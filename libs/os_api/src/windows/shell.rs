use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::null_mut;

use serde::Deserialize;
use windows::Win32::Foundation::{HLOCAL, HWND, LocalFree};
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    STGM_READ,
};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::UI::Shell::{
    CommandLineToArgvW, IShellLinkW, SLGP_UNCPRIORITY, SLR_NO_UI, ShellLink,
};
use windows::core::{Interface, PCWSTR};
use winreg::RegKey;
use winreg::enums::HKEY_CLASSES_ROOT;

use crate::{InstalledAppCatalogEntry, InstalledAppCatalogTarget, InstalledPackageRuntimeInfo};

use super::OS;
use super::common::{ComGuard, OsError, decode_ansi, expand_env, to_wide_z};

#[derive(Deserialize)]
struct StartAppRecord {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "AppID")]
    app_id: String,
}

#[derive(Deserialize)]
struct AppxPackageRuntimeInfoRecord {
    #[serde(rename = "PackageFamilyName")]
    package_family_name: String,
    #[serde(rename = "InstallLocation")]
    install_location: String,
}

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

fn classify_start_app_record(record: StartAppRecord) -> Option<InstalledAppCatalogEntry> {
    let name = record.name.trim().to_string();
    let app_id = record.app_id.trim().to_string();
    if name.is_empty() || app_id.is_empty() {
        return None;
    }

    if app_id.contains('!') {
        return Some(InstalledAppCatalogEntry {
            name,
            target: InstalledAppCatalogTarget::Aumid(app_id),
        });
    }

    let path = PathBuf::from(&app_id);
    let is_supported_exe = path.is_absolute()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        && path.exists();

    if is_supported_exe {
        return Some(InstalledAppCatalogEntry {
            name,
            target: InstalledAppCatalogTarget::Path(path),
        });
    }

    None
}

fn parse_start_apps_json(stdout: &str) -> Result<Vec<InstalledAppCatalogEntry>, String> {
    let value: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|e| format!("Failed to parse Start apps JSON: {e}"))?;

    let mut entries = Vec::new();
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                let record: StartAppRecord = serde_json::from_value(item)
                    .map_err(|e| format!("Invalid Start app record: {e}"))?;
                if let Some(entry) = classify_start_app_record(record) {
                    entries.push(entry);
                }
            }
        }
        serde_json::Value::Object(_) => {
            let record: StartAppRecord = serde_json::from_value(value)
                .map_err(|e| format!("Invalid Start app record: {e}"))?;
            if let Some(entry) = classify_start_app_record(record) {
                entries.push(entry);
            }
        }
        _ => return Err("Unexpected PowerShell JSON shape for Get-StartApps".into()),
    }

    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries.dedup_by(|left, right| left.name == right.name && left.target == right.target);
    Ok(entries)
}

fn run_hidden_powershell(script: &str, operation: &str) -> Result<std::process::Output, String> {
    Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW.0)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| format!("Failed to execute {operation}: {e}"))
}

fn list_supported_start_apps_powershell() -> Result<Vec<InstalledAppCatalogEntry>, String> {
    let output = run_hidden_powershell(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress",
        "Get-StartApps",
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Get-StartApps failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("Get-StartApps output was not valid UTF-8: {e}"))?;
    parse_start_apps_json(&stdout)
}

fn package_family_name_from_aumid(aumid: &str) -> Result<String, String> {
    let package_family_name = aumid
        .split('!')
        .next()
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| format!("Invalid AUMID (missing package family name): {aumid}"))?;

    Ok(package_family_name.to_string())
}

fn parse_appx_package_runtime_info_json(
    stdout: &str,
    aumid: &str,
) -> Result<InstalledPackageRuntimeInfo, String> {
    let value: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|e| format!("Failed to parse AppX package JSON: {e}"))?;

    let record: AppxPackageRuntimeInfoRecord = match value {
        serde_json::Value::Array(mut items) => {
            let item = items
                .drain(..)
                .next()
                .ok_or_else(|| format!("Package metadata not found for {aumid}"))?;
            serde_json::from_value(item).map_err(|e| format!("Invalid AppX package record: {e}"))?
        }
        serde_json::Value::Object(_) => serde_json::from_value(value)
            .map_err(|e| format!("Invalid AppX package record: {e}"))?,
        _ => return Err("Unexpected PowerShell JSON shape for Get-AppxPackage".into()),
    };

    let install_root = PathBuf::from(record.install_location.trim());
    if record.package_family_name.trim().is_empty() {
        return Err(format!(
            "Package metadata for {aumid} is missing PackageFamilyName"
        ));
    }
    if record.install_location.trim().is_empty() || !install_root.is_absolute() {
        return Err(format!(
            "Package metadata for {aumid} has invalid InstallLocation: {}",
            record.install_location
        ));
    }

    Ok(InstalledPackageRuntimeInfo {
        aumid: aumid.to_string(),
        package_family_name: record.package_family_name,
        install_root,
    })
}

fn resolve_installed_package_runtime_info_powershell(
    aumid: &str,
) -> Result<InstalledPackageRuntimeInfo, String> {
    let package_family_name = package_family_name_from_aumid(aumid)?;
    let script = format!(
        concat!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; ",
            "$ErrorActionPreference = 'Stop'; ",
            "$pkg = Get-AppxPackage | Where-Object {{ $_.PackageFamilyName -eq '{0}' }} ",
            "| Select-Object -First 1 PackageFamilyName,InstallLocation; ",
            "if ($null -eq $pkg) {{ throw 'Package not found for PackageFamilyName {0}' }}; ",
            "$pkg | ConvertTo-Json -Compress"
        ),
        package_family_name
    );

    let output = run_hidden_powershell(&script, "Get-AppxPackage")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Get-AppxPackage failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("Get-AppxPackage output was not valid UTF-8: {e}"))?;
    parse_appx_package_runtime_info_json(&stdout, aumid)
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

    pub fn list_supported_start_apps() -> Result<Vec<InstalledAppCatalogEntry>, String> {
        list_supported_start_apps_powershell()
    }

    pub fn resolve_installed_package_runtime_info(
        aumid: &str,
    ) -> Result<InstalledPackageRuntimeInfo, String> {
        resolve_installed_package_runtime_info_powershell(aumid)
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

    use super::{
        OS, StartAppRecord, classify_start_app_record, package_family_name_from_aumid,
        parse_appx_package_runtime_info_json, parse_start_apps_json, parse_url_file,
    };
    use crate::windows::common::{ComGuard, to_wide_z};
    use crate::{InstalledAppCatalogEntry, InstalledAppCatalogTarget, InstalledPackageRuntimeInfo};

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
    fn test_classify_start_app_record_accepts_aumid() {
        let entry = classify_start_app_record(StartAppRecord {
            name: "Spotify".into(),
            app_id: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
        })
        .unwrap();

        assert_eq!(
            entry,
            InstalledAppCatalogEntry {
                name: "Spotify".into(),
                target: InstalledAppCatalogTarget::Aumid(
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into()
                ),
            }
        );
    }

    #[test]
    fn test_classify_start_app_record_accepts_absolute_exe_path() {
        let exe_path = env::temp_dir().join(format!("codex-start-app-{}.exe", unique_suffix()));
        let _guard = TempFileGuard::new(exe_path.clone());
        fs::write(&exe_path, b"stub").unwrap();

        let entry = classify_start_app_record(StartAppRecord {
            name: "Tool".into(),
            app_id: exe_path.display().to_string(),
        })
        .unwrap();

        assert_eq!(
            entry,
            InstalledAppCatalogEntry {
                name: "Tool".into(),
                target: InstalledAppCatalogTarget::Path(exe_path),
            }
        );
    }

    #[test]
    fn test_parse_start_apps_json_rejects_unsupported_records() {
        let json = r#"[{"Name":"Docs","AppID":"https://docs.python.org/"},{"Name":"Help","AppID":"http://support.steampowered.com/"},{"Name":"Shortcut","AppID":"Chrome"}]"#;

        let entries = parse_start_apps_json(json).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_package_family_name_from_aumid_extracts_prefix() {
        let family =
            package_family_name_from_aumid("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify").unwrap();

        assert_eq!(family, "SpotifyAB.SpotifyMusic_zpdnekdrzrea0");
    }

    #[test]
    fn test_parse_appx_package_runtime_info_json_parses_install_root() {
        let json = r#"{"PackageFamilyName":"SpotifyAB.SpotifyMusic_zpdnekdrzrea0","InstallLocation":"C:\\Program Files\\WindowsApps\\SpotifyAB.SpotifyMusic_1.2.3.4_x64__zpdnekdrzrea0"}"#;

        let info = parse_appx_package_runtime_info_json(
            json,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
        )
        .unwrap();

        assert_eq!(
            info,
            InstalledPackageRuntimeInfo {
                aumid: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
                package_family_name: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into(),
                install_root: PathBuf::from(
                    r"C:\Program Files\WindowsApps\SpotifyAB.SpotifyMusic_1.2.3.4_x64__zpdnekdrzrea0"
                ),
            }
        );
    }

    #[test]
    fn test_parse_appx_package_runtime_info_json_rejects_invalid_install_root() {
        let json =
            r#"{"PackageFamilyName":"SpotifyAB.SpotifyMusic_zpdnekdrzrea0","InstallLocation":""}"#;

        let err = parse_appx_package_runtime_info_json(
            json,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
        )
        .unwrap_err();

        assert!(err.contains("InstallLocation"));
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
