use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows::Win32::Foundation::{HLOCAL, HWND, LocalFree};
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    STGM_READ,
};
use windows::Win32::UI::Shell::{
    CommandLineToArgvW, IShellLinkW, SLGP_UNCPRIORITY, SLR_NO_UI, ShellLink,
};
use windows::core::{Interface, PCWSTR};
use winreg::RegKey;
use winreg::enums::HKEY_CLASSES_ROOT;

use super::OS;
use super::common::{ComGuard, OsError, decode_ansi, expand_env, to_wide_z};

pub(super) fn parse_url_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
    let content = if bytes.starts_with(&[0xFF, 0xFE]) && bytes.len() >= 2 {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else if bytes.starts_with(&[0xFE, 0xFF]) && bytes.len() >= 2 {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else {
        match String::from_utf8(bytes.clone()) {
            Ok(s) => s,
            Err(_) => {
                decode_ansi(&bytes).ok_or("Failed to decode .url file using UTF-8 or ANSI")?
            }
        }
    };

    for line in content.lines() {
        if let Some(url) = line.strip_prefix("URL=") {
            return Ok(url.trim().to_string());
        }
    }

    Err("URL= not found".into())
}

fn resolve_url(path: &Path) -> Result<(PathBuf, Vec<String>), String> {
    let url = parse_url_file(path)?;
    let scheme = url
        .split(':')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Invalid URL (no scheme): {}", url))?;

    let exe =
        OS::get_program_path_for_uri(scheme).map_err(|e| format!("Failed to parse URL: {}", e))?;

    Ok((exe, vec![url]))
}

pub(super) fn split_windows_args(args: &str) -> Vec<String> {
    if args.is_empty() {
        return Vec::new();
    }

    let wide: Vec<u16> = OsStr::new(args).encode_wide().chain([0]).collect();
    let mut argc: i32 = 0;

    unsafe {
        let argv = CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut argc);
        if argv.is_null() || argc <= 0 {
            return vec![args.to_string()];
        }

        let mut out = Vec::with_capacity(argc as usize);

        for i in 0..argc {
            let p = (*argv.add(i as usize)).0;
            if p.is_null() {
                out.push(String::new());
                continue;
            }

            let mut len = 0usize;
            while *p.add(len) != 0 {
                len += 1;
            }

            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            out.push(s);
        }

        let _ = LocalFree(Some(HLOCAL(argv as *mut core::ffi::c_void)));
        out
    }
}

fn resolve_lnk(path: &Path) -> Result<(PathBuf, Vec<String>), String> {
    (|| unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(OsError::Win)?;
        let _com = ComGuard;

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let persist: IPersistFile = link.cast()?;

        let wide = to_wide_z(path.as_os_str());
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ)?;

        link.Resolve(HWND(null_mut()), SLR_NO_UI.0 as u32)?;

        let mut wbuf = [0u16; 32768];
        let mut find = WIN32_FIND_DATAW::default();
        link.GetPath(&mut wbuf, &mut find as *mut _, SLGP_UNCPRIORITY.0 as u32)?;
        let n = wbuf.iter().position(|&c| c == 0).unwrap_or(wbuf.len());
        let target_raw = String::from_utf16_lossy(&wbuf[..n]);
        let target = PathBuf::from(expand_env(&target_raw));

        let mut abuf = [0u16; 32768];
        link.GetArguments(&mut abuf)?;
        let an = abuf.iter().position(|&c| c == 0).unwrap_or(abuf.len());
        let args_str = String::from_utf16_lossy(&abuf[..an]);
        let args_expanded = expand_env(&args_str);
        let args_vec = split_windows_args(&args_expanded);

        Ok((target, args_vec))
    })()
    .map_err(|e: OsError| format!("resolve_lnk {:?} failed: {}", path, e))
}

fn get_program_path_for_uri_registry(uri_scheme: &str) -> Result<PathBuf, String> {
    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);

    let scheme_key = hkcr
        .open_subkey(uri_scheme)
        .map_err(|e| format!("Scheme {} not found: {}", uri_scheme, e))?;

    let _: String = scheme_key
        .get_value("URL Protocol")
        .map_err(|_| "Not a valid URI protocol".to_string())?;

    let command_key_path = format!(r"{}\shell\open\command", uri_scheme);
    let command_key = hkcr
        .open_subkey(command_key_path)
        .map_err(|e| format!("Command key not found: {}", e))?;

    let command: String = command_key
        .get_value("")
        .map_err(|e| format!("Failed to get command string: {}", e))?;

    let expanded = expand_env(&command);
    let parts = split_windows_args(&expanded);
    let first = parts.first().ok_or("Command string is empty")?;
    Ok(PathBuf::from(first))
}

impl OS {
    pub fn parse_dropped_file(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        let file_ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| format!("Failed to get file extension for {:?}", file_path))?;

        if file_ext.eq_ignore_ascii_case("url") {
            return resolve_url(&file_path);
        }
        if file_ext.eq_ignore_ascii_case("lnk") {
            return resolve_lnk(&file_path);
        }

        Ok((file_path, Vec::new()))
    }

    pub fn get_program_path_for_uri(uri_scheme: &str) -> Result<PathBuf, String> {
        get_program_path_for_uri_registry(uri_scheme)
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{Interface, PCWSTR};
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    use super::{OS, parse_url_file};
    use crate::windows::common::{ComGuard, to_wide_z};

    fn unique_suffix() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{}-{}", process::id(), nanos)
    }

    struct TempFileGuard {
        path: PathBuf,
    }

    impl TempFileGuard {
        fn new(path: PathBuf) -> Self {
            Self { path }
        }
    }

    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    struct RegistryKeyGuard {
        path: String,
    }

    impl Drop for RegistryKeyGuard {
        fn drop(&mut self) {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let _ = hkcu.delete_subkey_all(&self.path);
        }
    }

    fn write_shortcut(path: &PathBuf, target: &PathBuf, args: &str) -> Result<(), String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
            let _com = ComGuard;

            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            let persist: IPersistFile = link.cast().map_err(|e| e.to_string())?;

            let target_w = to_wide_z(target.as_os_str());
            link.SetPath(PCWSTR(target_w.as_ptr()))
                .map_err(|e| e.to_string())?;

            let args_w = to_wide_z(OsStr::new(args));
            link.SetArguments(PCWSTR(args_w.as_ptr()))
                .map_err(|e| e.to_string())?;

            let path_w = to_wide_z(path.as_os_str());
            persist
                .Save(PCWSTR(path_w.as_ptr()), true)
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    #[test]
    fn test_parse_url_file_decodes_utf8() {
        let path = env::temp_dir().join(format!("codex-url-{}.url", unique_suffix()));
        let _guard = TempFileGuard::new(path.clone());

        fs::write(&path, "[InternetShortcut]\nURL=https://example.com/path\n").unwrap();

        let url = parse_url_file(&path).unwrap();
        assert_eq!(url, "https://example.com/path");
    }

    #[test]
    fn test_parse_url_file_decodes_utf16le_bom() {
        let path = env::temp_dir().join(format!("codex-url-bom-{}.url", unique_suffix()));
        let _guard = TempFileGuard::new(path.clone());

        let mut bytes = vec![0xFF, 0xFE];
        for unit in "[InternetShortcut]\nURL=https://example.com/bom\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(&path, bytes).unwrap();

        let url = parse_url_file(&path).unwrap();
        assert_eq!(url, "https://example.com/bom");
    }

    #[test]
    fn test_parse_dropped_file_resolves_lnk_target_and_args() {
        let unique = unique_suffix();
        let target = env::temp_dir().join(format!("codex target {}.exe", unique));
        let target_guard = TempFileGuard::new(target.clone());
        fs::write(&target, b"stub").unwrap();

        let shortcut = env::temp_dir().join(format!("codex shortcut {}.lnk", unique));
        let shortcut_guard = TempFileGuard::new(shortcut.clone());
        let args = r#"--mode "quoted arg" "#;
        write_shortcut(&shortcut, &target, args).unwrap();

        let (resolved_target, resolved_args) = OS::parse_dropped_file(shortcut.clone()).unwrap();
        assert_eq!(resolved_target, target);
        assert_eq!(
            resolved_args,
            vec!["--mode".to_string(), "quoted arg".to_string()]
        );

        drop(shortcut_guard);
        drop(target_guard);
    }

    #[test]
    fn test_get_program_path_for_uri_reads_per_user_registration() {
        let unique = unique_suffix();
        let scheme = format!("codex-s08-{}", unique);
        let registry_path = format!(r"Software\Classes\{}", scheme);
        let _registry_guard = RegistryKeyGuard {
            path: registry_path.clone(),
        };

        let exe = env::temp_dir().join(format!("codex uri target {}.exe", unique));
        let command = format!("\"{}\" \"%1\"", exe.display());

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (scheme_key, _) = hkcu.create_subkey(&registry_path).unwrap();
        scheme_key.set_value("URL Protocol", &"").unwrap();
        let (command_key, _) = scheme_key.create_subkey(r"shell\open\command").unwrap();
        command_key.set_value("", &command).unwrap();

        let resolved = OS::get_program_path_for_uri(&scheme).unwrap();
        assert_eq!(resolved, exe);
    }
}
