use std::collections::HashSet;
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
use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::{InstalledAppCatalogEntry, InstalledAppCatalogTarget, InstalledPackageRuntimeInfo};

use super::OS;
use super::common::{ComGuard, OsError, decode_ansi, expand_env, to_wide_z};

#[derive(Deserialize)]
struct AppsFolderRecord {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Path")]
    path: String,
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
        let target = normalize_existing_windows_path(&PathBuf::from(expand_env(&target_raw)));

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

fn strip_windows_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();

    if let Some(stripped) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{stripped}"));
    }

    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }

    path.to_path_buf()
}

fn normalize_existing_windows_path(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .map(|canonical| strip_windows_verbatim_prefix(&canonical))
        .unwrap_or_else(|_| path.to_path_buf())
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

fn is_supported_absolute_exe_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        && path.exists()
}

fn classify_catalog_record(name: &str, raw_target: &str) -> Option<InstalledAppCatalogEntry> {
    let name = name.trim().to_string();
    let raw_target = raw_target.trim().to_string();
    if name.is_empty() || raw_target.is_empty() {
        return None;
    }

    if raw_target.contains('!') {
        return Some(InstalledAppCatalogEntry {
            name,
            target: InstalledAppCatalogTarget::Aumid(raw_target),
        });
    }

    let path = PathBuf::from(&raw_target);
    if is_supported_absolute_exe_path(&path) {
        return Some(InstalledAppCatalogEntry {
            name,
            target: InstalledAppCatalogTarget::Path(normalize_existing_windows_path(&path)),
        });
    }

    None
}

fn target_identity(target: &InstalledAppCatalogTarget) -> String {
    match target {
        InstalledAppCatalogTarget::Aumid(aumid) => format!("aumid:{}", aumid.to_lowercase()),
        InstalledAppCatalogTarget::Path(path) => format!(
            "path:{}",
            normalize_existing_windows_path(path)
                .to_string_lossy()
                .to_lowercase()
        ),
    }
}

fn sort_catalog_entries(entries: &mut [InstalledAppCatalogEntry]) {
    entries.sort_by_cached_key(|entry| entry.name.to_lowercase());
}

fn dedup_catalog_entries(entries: &mut Vec<InstalledAppCatalogEntry>) {
    let mut identities = HashSet::new();
    entries.retain(|entry| identities.insert(target_identity(&entry.target)));
}

fn merge_catalog_sources(
    primary: Vec<InstalledAppCatalogEntry>,
    secondary: Vec<InstalledAppCatalogEntry>,
) -> Vec<InstalledAppCatalogEntry> {
    let mut identities: HashSet<String> = primary
        .iter()
        .map(|entry| target_identity(&entry.target))
        .collect();
    let mut merged = primary;

    for entry in secondary {
        if identities.insert(target_identity(&entry.target)) {
            merged.push(entry);
        }
    }

    sort_catalog_entries(&mut merged);
    merged
}

fn parse_apps_folder_json(stdout: &str) -> Result<Vec<InstalledAppCatalogEntry>, String> {
    let value: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|e| format!("Failed to parse AppsFolder JSON: {e}"))?;

    let mut entries = Vec::new();
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                let record: AppsFolderRecord = serde_json::from_value(item)
                    .map_err(|e| format!("Invalid AppsFolder record: {e}"))?;
                if let Some(entry) = classify_catalog_record(&record.name, &record.path) {
                    entries.push(entry);
                }
            }
        }
        serde_json::Value::Object(_) => {
            let record: AppsFolderRecord = serde_json::from_value(value)
                .map_err(|e| format!("Invalid AppsFolder record: {e}"))?;
            if let Some(entry) = classify_catalog_record(&record.name, &record.path) {
                entries.push(entry);
            }
        }
        _ => return Err("Unexpected PowerShell JSON shape for AppsFolder enumeration".into()),
    }

    dedup_catalog_entries(&mut entries);
    sort_catalog_entries(&mut entries);
    Ok(entries)
}

fn run_hidden_powershell(script: &str, operation: &str) -> Result<std::process::Output, String> {
    Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW.0)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| format!("Failed to execute {operation}: {e}"))
}

fn list_apps_folder_entries_powershell() -> Result<Vec<InstalledAppCatalogEntry>, String> {
    let output = run_hidden_powershell(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $apps = (New-Object -ComObject Shell.Application).NameSpace('shell:::{4234d49b-0245-4df3-b780-3893943456e1}').Items(); $apps | Select-Object @{n='Name';e={$_.Name}},@{n='Path';e={$_.Path}} | ConvertTo-Json -Compress",
        "AppsFolder enumeration",
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("AppsFolder enumeration failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("AppsFolder output was not valid UTF-8: {e}"))?;

    if stdout.trim().is_empty() || stdout.trim() == "null" {
        return Ok(Vec::new());
    }

    parse_apps_folder_json(&stdout)
}

fn unquote_wrapped_text(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|trimmed| trimmed.strip_suffix('"'))
        .unwrap_or(value)
}

fn is_ignored_start_menu_entry_name(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    lower.starts_with("uninstall ") || lower.starts_with("remove ") || lower.starts_with("repair ")
}

fn classify_start_menu_entry(path: &Path) -> Option<InstalledAppCatalogEntry> {
    let display_name = path.file_stem()?.to_str()?.trim().to_string();
    if display_name.is_empty() || is_ignored_start_menu_entry_name(&display_name) {
        return None;
    }

    let extension = path.extension().and_then(|ext| ext.to_str())?;
    match extension.to_ascii_lowercase().as_str() {
        "exe" if is_supported_absolute_exe_path(path) => Some(InstalledAppCatalogEntry {
            name: display_name,
            target: InstalledAppCatalogTarget::Path(normalize_existing_windows_path(path)),
        }),
        "lnk" => {
            let (target, args) = resolve_lnk(path).ok()?;
            if !args.is_empty() || !is_supported_absolute_exe_path(&target) {
                return None;
            }

            Some(InstalledAppCatalogEntry {
                name: display_name,
                target: InstalledAppCatalogTarget::Path(normalize_existing_windows_path(&target)),
            })
        }
        _ => None,
    }
}

fn collect_start_menu_entries_from_dir(root: &Path) -> Vec<InstalledAppCatalogEntry> {
    let mut entries = Vec::new();
    let mut pending_dirs = vec![root.to_path_buf()];

    while let Some(current_dir) = pending_dirs.pop() {
        let Ok(children) = fs::read_dir(&current_dir) else {
            continue;
        };

        for child in children.flatten() {
            let path = child.path();
            if path.is_dir() {
                pending_dirs.push(path);
                continue;
            }

            if let Some(entry) = classify_start_menu_entry(&path) {
                entries.push(entry);
            }
        }
    }

    entries
}

fn list_start_menu_entries() -> Result<Vec<InstalledAppCatalogEntry>, String> {
    let mut entries = Vec::new();
    let roots = [
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join(r"Microsoft\Windows\Start Menu\Programs")),
        std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .map(|path| path.join(r"Microsoft\Windows\Start Menu\Programs")),
    ];

    for root in roots.into_iter().flatten() {
        if root.is_dir() {
            entries.extend(collect_start_menu_entries_from_dir(&root));
        }
    }

    dedup_catalog_entries(&mut entries);
    sort_catalog_entries(&mut entries);
    Ok(entries)
}

fn classify_app_paths_entry(
    display_name: &str,
    default_value: &str,
) -> Option<InstalledAppCatalogEntry> {
    let expanded = expand_env(default_value.trim());
    let candidate = PathBuf::from(unquote_wrapped_text(expanded.trim()));
    if !is_supported_absolute_exe_path(&candidate) {
        return None;
    }

    let name = display_name.trim().to_string();
    if name.is_empty() {
        return None;
    }

    Some(InstalledAppCatalogEntry {
        name,
        target: InstalledAppCatalogTarget::Path(normalize_existing_windows_path(&candidate)),
    })
}

fn app_paths_display_name(key_name: &str, default_path: &Path) -> String {
    Path::new(key_name)
        .file_stem()
        .or_else(|| default_path.file_stem())
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
        .unwrap_or_else(|| default_path.display().to_string())
}

fn collect_app_paths_entries_from_root(
    hive: RegKey,
    subkey_path: &str,
) -> Result<Vec<InstalledAppCatalogEntry>, String> {
    let root = match hive.open_subkey(subkey_path) {
        Ok(root) => root,
        Err(_) => return Ok(Vec::new()),
    };

    let mut entries = Vec::new();

    for key_name in root.enum_keys().flatten() {
        let Ok(app_key) = root.open_subkey(&key_name) else {
            continue;
        };
        let Ok(default_value) = app_key.get_value::<String, _>("") else {
            continue;
        };

        let expanded = expand_env(default_value.trim());
        let candidate = PathBuf::from(unquote_wrapped_text(expanded.trim()));
        if !is_supported_absolute_exe_path(&candidate) {
            continue;
        }

        let friendly_name = app_key
            .get_value::<String, _>("FriendlyAppName")
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| app_paths_display_name(&key_name, &candidate));

        if let Some(entry) = classify_app_paths_entry(&friendly_name, &default_value) {
            entries.push(entry);
        }
    }

    Ok(entries)
}

fn list_app_paths_entries() -> Result<Vec<InstalledAppCatalogEntry>, String> {
    let hives_and_paths = [
        (
            RegKey::predef(HKEY_CURRENT_USER),
            r"Software\Microsoft\Windows\CurrentVersion\App Paths",
        ),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            r"Software\Microsoft\Windows\CurrentVersion\App Paths",
        ),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths",
        ),
    ];

    let mut entries = Vec::new();
    for (hive, subkey_path) in hives_and_paths {
        entries.extend(collect_app_paths_entries_from_root(hive, subkey_path)?);
    }

    dedup_catalog_entries(&mut entries);
    sort_catalog_entries(&mut entries);
    Ok(entries)
}

fn merge_catalog_results(
    apps_folder: Result<Vec<InstalledAppCatalogEntry>, String>,
    start_menu: Result<Vec<InstalledAppCatalogEntry>, String>,
    app_paths: Result<Vec<InstalledAppCatalogEntry>, String>,
) -> Result<Vec<InstalledAppCatalogEntry>, String> {
    let mut merged: Option<Vec<InstalledAppCatalogEntry>> = None;
    let mut errors = Vec::new();

    for (source_name, source_entries) in [
        ("AppsFolder", apps_folder),
        ("Start Menu", start_menu),
        ("App Paths", app_paths),
    ] {
        match source_entries {
            Ok(entries) => {
                merged = Some(match merged.take() {
                    Some(existing) => merge_catalog_sources(existing, entries),
                    None => {
                        let mut entries = entries;
                        dedup_catalog_entries(&mut entries);
                        sort_catalog_entries(&mut entries);
                        entries
                    }
                });
            }
            Err(error) => errors.push(format!("{source_name} ({error})")),
        }
    }

    match merged {
        Some(entries) => Ok(entries),
        None => Err(format!(
            "Failed to enumerate installed apps from {}",
            errors.join(", ")
        )),
    }
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
        merge_catalog_results(
            list_apps_folder_entries_powershell(),
            list_start_menu_entries(),
            list_app_paths_entries(),
        )
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
    use std::path::{Path, PathBuf};
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
        OS, app_paths_display_name, classify_app_paths_entry, classify_catalog_record,
        classify_start_menu_entry, collect_start_menu_entries_from_dir, dedup_catalog_entries,
        is_ignored_start_menu_entry_name, merge_catalog_sources, normalize_existing_windows_path,
        package_family_name_from_aumid, parse_apps_folder_json,
        parse_appx_package_runtime_info_json, parse_url_file, strip_windows_verbatim_prefix,
        target_identity,
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

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(path: PathBuf) -> Self {
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
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
        assert_eq!(resolved_target, normalize_existing_windows_path(&target));
        assert_eq!(
            resolved_args,
            vec!["--mode".to_string(), "quoted arg".to_string()]
        );

        drop(shortcut_guard);
        drop(target_guard);
    }

    #[test]
    fn test_strip_windows_verbatim_prefix_handles_local_and_unc_forms() {
        assert_eq!(
            strip_windows_verbatim_prefix(Path::new(r"\\?\C:\Tools\app.exe")),
            PathBuf::from(r"C:\Tools\app.exe")
        );
        assert_eq!(
            strip_windows_verbatim_prefix(Path::new(r"\\?\UNC\server\share\tool.exe")),
            PathBuf::from(r"\\server\share\tool.exe")
        );
    }

    #[test]
    fn test_normalize_existing_windows_path_keeps_missing_path_unchanged() {
        let missing = PathBuf::from(r"C:\definitely-missing-codex-test-path.exe");
        assert_eq!(normalize_existing_windows_path(&missing), missing);
    }

    #[test]
    fn test_classify_catalog_record_accepts_aumid() {
        let entry =
            classify_catalog_record("Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify")
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
    fn test_classify_catalog_record_accepts_absolute_exe_path() {
        let exe_path = env::temp_dir().join(format!("codex-start-app-{}.exe", unique_suffix()));
        let _guard = TempFileGuard::new(exe_path.clone());
        fs::write(&exe_path, b"stub").unwrap();

        let entry = classify_catalog_record("Tool", &exe_path.display().to_string()).unwrap();

        assert_eq!(
            entry,
            InstalledAppCatalogEntry {
                name: "Tool".into(),
                target: InstalledAppCatalogTarget::Path(normalize_existing_windows_path(&exe_path)),
            }
        );
    }

    #[test]
    fn test_parse_apps_folder_json_rejects_unsupported_records() {
        let json = r#"[{"Name":"Docs","Path":"https://docs.python.org/"},{"Name":"Help","Path":"http://support.steampowered.com/"},{"Name":"Shortcut","Path":"Chrome"}]"#;

        let entries = parse_apps_folder_json(json).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_classify_app_paths_entry_accepts_valid_absolute_exe() {
        let exe_path = env::temp_dir().join(format!("codex-app-paths-{}.exe", unique_suffix()));
        let _guard = TempFileGuard::new(exe_path.clone());
        fs::write(&exe_path, b"stub").unwrap();

        let entry =
            classify_app_paths_entry("Discord", &format!("\"{}\"", exe_path.display())).unwrap();

        assert_eq!(
            entry,
            InstalledAppCatalogEntry {
                name: "Discord".into(),
                target: InstalledAppCatalogTarget::Path(normalize_existing_windows_path(&exe_path)),
            }
        );
    }

    #[test]
    fn test_classify_app_paths_entry_rejects_invalid_targets() {
        assert!(classify_app_paths_entry("Docs", "https://docs.python.org/").is_none());
        assert!(classify_app_paths_entry("Shortcut", "chrome.exe").is_none());
    }

    #[test]
    fn test_is_ignored_start_menu_entry_name_filters_obvious_non_launchers() {
        assert!(is_ignored_start_menu_entry_name("Uninstall RustDesk"));
        assert!(is_ignored_start_menu_entry_name("remove Tool"));
        assert!(is_ignored_start_menu_entry_name("Repair Discord"));
        assert!(!is_ignored_start_menu_entry_name("RustDesk"));
    }

    #[test]
    fn test_classify_start_menu_entry_accepts_resolved_shortcut_without_args() {
        let unique = unique_suffix();
        let temp_dir = env::temp_dir().join(format!("codex-start-menu-{}", unique));
        fs::create_dir_all(&temp_dir).unwrap();
        let _dir_guard = TempDirGuard::new(temp_dir.clone());

        let target = temp_dir.join("RustDesk.exe");
        fs::write(&target, b"stub").unwrap();
        let shortcut = temp_dir.join("RustDesk.lnk");
        write_shortcut(&shortcut, &target, "").unwrap();

        let entry = classify_start_menu_entry(&shortcut).unwrap();
        assert_eq!(
            entry,
            InstalledAppCatalogEntry {
                name: "RustDesk".into(),
                target: InstalledAppCatalogTarget::Path(normalize_existing_windows_path(&target)),
            }
        );
    }

    #[test]
    fn test_classify_start_menu_entry_rejects_shortcut_with_args() {
        let unique = unique_suffix();
        let temp_dir = env::temp_dir().join(format!("codex-start-menu-args-{}", unique));
        fs::create_dir_all(&temp_dir).unwrap();
        let _dir_guard = TempDirGuard::new(temp_dir.clone());

        let target = temp_dir.join("Launcher.exe");
        fs::write(&target, b"stub").unwrap();
        let shortcut = temp_dir.join("Launcher.lnk");
        write_shortcut(&shortcut, &target, "--profile default").unwrap();

        assert!(classify_start_menu_entry(&shortcut).is_none());
    }

    #[test]
    fn test_classify_start_menu_entry_accepts_direct_exe() {
        let exe_path =
            env::temp_dir().join(format!("codex-start-menu-direct-{}.exe", unique_suffix()));
        let _guard = TempFileGuard::new(exe_path.clone());
        fs::write(&exe_path, b"stub").unwrap();

        let entry = classify_start_menu_entry(&exe_path).unwrap();
        assert_eq!(entry.name, exe_path.file_stem().unwrap().to_string_lossy());
        assert_eq!(
            entry.target,
            InstalledAppCatalogTarget::Path(normalize_existing_windows_path(&exe_path))
        );
    }

    #[test]
    fn test_classify_start_menu_entry_rejects_non_launch_safe_files() {
        let unique = unique_suffix();
        let temp_dir = env::temp_dir().join(format!("codex-start-menu-bad-{}", unique));
        fs::create_dir_all(&temp_dir).unwrap();
        let _dir_guard = TempDirGuard::new(temp_dir.clone());

        let url = temp_dir.join("Docs.url");
        fs::write(&url, "[InternetShortcut]\nURL=https://example.com/\n").unwrap();
        let broken_lnk = temp_dir.join("Broken.lnk");
        fs::write(&broken_lnk, b"not-a-shortcut").unwrap();
        let uninstall_exe = temp_dir.join("Uninstall Tool.exe");
        fs::write(&uninstall_exe, b"stub").unwrap();

        assert!(classify_start_menu_entry(&url).is_none());
        assert!(classify_start_menu_entry(&broken_lnk).is_none());
        assert!(classify_start_menu_entry(&uninstall_exe).is_none());
    }

    #[test]
    fn test_collect_start_menu_entries_from_dir_recurses_and_filters() {
        let unique = unique_suffix();
        let temp_dir = env::temp_dir().join(format!("codex-start-menu-tree-{}", unique));
        let nested = temp_dir.join("RustDesk");
        fs::create_dir_all(&nested).unwrap();
        let _dir_guard = TempDirGuard::new(temp_dir.clone());

        let target = temp_dir.join("RustDesk.exe");
        fs::write(&target, b"stub").unwrap();
        let shortcut = nested.join("RustDesk.lnk");
        write_shortcut(&shortcut, &target, "").unwrap();
        let uninstall = nested.join("Uninstall RustDesk.lnk");
        write_shortcut(&uninstall, &target, "").unwrap();

        let entries = collect_start_menu_entries_from_dir(&temp_dir);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.name == "RustDesk"));
        assert!(
            entries
                .iter()
                .all(|entry| !entry.name.starts_with("Uninstall "))
        );
    }

    #[test]
    fn test_app_paths_display_name_falls_back_to_exe_stem() {
        let exe = PathBuf::from(r"C:\Tools\Discord.exe");
        assert_eq!(app_paths_display_name("Discord.exe", &exe), "Discord");
    }

    #[test]
    fn test_target_identity_normalizes_aumid_and_path() {
        let exe_path = env::temp_dir().join(format!("codex-identity-{}.exe", unique_suffix()));
        let _guard = TempFileGuard::new(exe_path.clone());
        fs::write(&exe_path, b"stub").unwrap();

        assert_eq!(
            target_identity(&InstalledAppCatalogTarget::Aumid("Test.App!Main".into())),
            "aumid:test.app!main"
        );
        assert_eq!(
            target_identity(&InstalledAppCatalogTarget::Path(exe_path.clone())),
            format!(
                "path:{}",
                normalize_existing_windows_path(&exe_path)
                    .display()
                    .to_string()
                    .to_lowercase()
            )
        );
    }

    #[test]
    fn test_merge_catalog_sources_dedups_by_target_identity_and_keeps_precedence_order() {
        let exe_path = env::temp_dir().join(format!("codex-merge-{}.exe", unique_suffix()));
        let _guard = TempFileGuard::new(exe_path.clone());
        fs::write(&exe_path, b"stub").unwrap();

        let normalized_path = normalize_existing_windows_path(&exe_path);
        let merged = merge_catalog_sources(
            vec![
                InstalledAppCatalogEntry {
                    name: "Spotify".into(),
                    target: InstalledAppCatalogTarget::Aumid(
                        "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
                    ),
                },
                InstalledAppCatalogEntry {
                    name: "Friendly Discord".into(),
                    target: InstalledAppCatalogTarget::Path(normalized_path.clone()),
                },
            ],
            vec![InstalledAppCatalogEntry {
                name: "Start Menu Discord".into(),
                target: InstalledAppCatalogTarget::Path(normalized_path.clone()),
            }],
        );
        let merged = merge_catalog_sources(
            merged,
            vec![
                InstalledAppCatalogEntry {
                    name: "Duplicate Spotify".into(),
                    target: InstalledAppCatalogTarget::Aumid(
                        "spotifyab.spotifymusic_zpdnekdrzrea0!spotify".into(),
                    ),
                },
                InstalledAppCatalogEntry {
                    name: "Discord".into(),
                    target: InstalledAppCatalogTarget::Path(exe_path),
                },
            ],
        );

        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|entry| entry.name == "Spotify"));
        assert!(merged.iter().any(|entry| entry.name == "Friendly Discord"));
    }

    #[test]
    fn test_dedup_catalog_entries_collapses_duplicates_inside_one_source() {
        let exe_path = env::temp_dir().join(format!("codex-dedup-{}.exe", unique_suffix()));
        let _guard = TempFileGuard::new(exe_path.clone());
        fs::write(&exe_path, b"stub").unwrap();

        let mut entries = vec![
            InstalledAppCatalogEntry {
                name: "Discord".into(),
                target: InstalledAppCatalogTarget::Path(normalize_existing_windows_path(&exe_path)),
            },
            InstalledAppCatalogEntry {
                name: "Discord Duplicate".into(),
                target: InstalledAppCatalogTarget::Path(exe_path),
            },
        ];

        dedup_catalog_entries(&mut entries);
        assert_eq!(entries.len(), 1);
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
