//! Bounded Steam metadata and exact game-image selection. No Steam SDK or hooks.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameProcess {
    pub pid: u32,
    pub creation_token: u64,
    pub path: PathBuf,
    pub has_window: bool,
}

pub fn parse_app_id(url: &str) -> Option<u32> {
    let url = url.to_ascii_lowercase();
    let rest = url
        .strip_prefix("steam://rungameid/")
        .or_else(|| url.strip_prefix("steam://run/"))?;
    let digits = rest.strip_suffix('/').unwrap_or(rest);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|id| *id != 0)
}

/// Supported complete Steam launch forms. Extra arguments are refused rather than lost.
pub fn parse_launch_args(args: &[String]) -> Option<u32> {
    match args {
        [url] => parse_app_id(url),
        [flag, id]
            if flag.eq_ignore_ascii_case("-applaunch")
                && !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_digit()) =>
        {
            id.parse::<u32>().ok().filter(|id| *id != 0)
        }
        _ => None,
    }
}
pub fn is_game_launch_args(args: &[String]) -> bool {
    args.iter().any(|a| {
        let a = a.to_ascii_lowercase();
        a == "-applaunch" || a.starts_with("steam://run/") || a.starts_with("steam://rungameid/")
    })
}
/// Conservative automatic role heuristic; a user may explicitly resolve a generic launcher.
pub fn automatic_image(path: &Path) -> bool {
    let name = image_name(path).trim_end_matches(".exe").to_string();
    eligible_image(path)
        && !["launcher", "updater", "bootstrap", "config", "helper"]
            .iter()
            .any(|part| name.contains(part))
}
/// Filter before the display bound, so typing can find processes on a busy host.
pub fn rank_process_choices(mut rows: Vec<GameProcess>, query: &str) -> Vec<GameProcess> {
    let query = query.to_lowercase();
    rows.retain(|p| {
        p.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
            .contains(&query)
    });
    rows.sort_by_key(|p| {
        (
            !p.has_window,
            p.path.to_string_lossy().to_lowercase(),
            p.pid,
        )
    });
    rows.truncate(256);
    rows
}

fn image_name(path: &Path) -> String {
    path.to_string_lossy()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}
pub fn eligible_image(path: &Path) -> bool {
    let name = image_name(path);
    let Some(stem) = name.strip_suffix(".exe") else {
        return false;
    };
    !matches!(
        stem,
        "steam"
            | "steamwebhelper"
            | "steamservice"
            | "steamerrorreporter"
            | "eadesktop"
            | "ealauncher"
            | "epicgameslauncher"
            | "upc"
            | "ubisoftconnect"
            | "setup"
            | "install"
            | "uninstall"
    ) && ![
        "easyanticheat",
        "beservice",
        "battleye",
        "crashreport",
        "crashpad",
        "unins",
    ]
    .iter()
    .any(|prefix| stem.starts_with(prefix))
}

pub fn path_within(path: &Path, root: &Path) -> bool {
    canonical_path_within(
        &path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
        &root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
    )
}

/// Component comparison for paths already resolved by the native file boundary.
pub fn canonical_path_within(path: &Path, root: &Path) -> bool {
    fn key(path: &Path) -> String {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_start_matches(r"\\?\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }
    let root = key(root);
    let path = key(path);
    !root.is_empty()
        && path
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug)]
enum Vdf {
    Text(String),
    Object(std::collections::BTreeMap<String, Vdf>),
}
#[cfg(any(target_os = "windows", test))]
impl Vdf {
    fn field(&self, key: &str) -> Option<&Vdf> {
        if let Self::Object(m) = self {
            m.get(&key.to_ascii_lowercase())
        } else {
            None
        }
    }
    fn text(&self) -> Option<&str> {
        if let Self::Text(s) = self {
            Some(s)
        } else {
            None
        }
    }
}
#[cfg(any(target_os = "windows", test))]
fn parse_vdf(input: &str) -> Result<Vdf, String> {
    if input.len() > 1024 * 1024 {
        return Err("Steam metadata exceeds 1 MiB".into());
    }
    let mut tokens = Vec::new();
    let mut chars = input.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
            continue;
        }
        if ch == '{' || ch == '}' {
            tokens.push(ch.to_string());
        } else if ch == '"' {
            let mut value = String::new();
            let mut closed = false;
            while let Some(c) = chars.next() {
                if c == '"' {
                    closed = true;
                    break;
                }
                if c == '\\' {
                    let next = chars.next().ok_or("Incomplete Steam escape")?;
                    if next != '\\' && next != '"' {
                        value.push('\\');
                    }
                    value.push(next);
                } else {
                    value.push(c);
                }
            }
            if !closed || value.contains('\0') {
                return Err("Invalid Steam string".into());
            }
            tokens.push(format!("={value}"));
        } else {
            return Err("Unsupported Steam metadata syntax".into());
        }
        if tokens.len() > 32768 {
            return Err("Too many Steam metadata fields".into());
        }
    }
    fn object(
        tokens: &[String],
        at: &mut usize,
        depth: usize,
        nested: bool,
    ) -> Result<Vdf, String> {
        if depth > 8 {
            return Err("Steam metadata nesting exceeds limit".into());
        }
        let mut fields = std::collections::BTreeMap::new();
        while *at < tokens.len() {
            if tokens[*at] == "}" {
                if !nested {
                    return Err("Unexpected Steam brace".into());
                }
                *at += 1;
                return Ok(Vdf::Object(fields));
            }
            let key = tokens[*at]
                .strip_prefix('=')
                .ok_or("Missing Steam field name")?
                .to_ascii_lowercase();
            *at += 1;
            let token = tokens.get(*at).ok_or("Missing Steam value")?;
            *at += 1;
            let value = if token == "{" {
                object(tokens, at, depth + 1, true)?
            } else {
                Vdf::Text(token.strip_prefix('=').ok_or("Invalid Steam value")?.into())
            };
            if fields.insert(key, value).is_some() {
                return Err("Duplicate Steam metadata field".into());
            }
        }
        if nested {
            Err("Incomplete Steam metadata".into())
        } else {
            Ok(Vdf::Object(fields))
        }
    }
    object(&tokens, &mut 0, 0, false)
}
#[cfg(any(target_os = "windows", test))]
fn manifest_directory(input: &str, app_id: u32) -> Result<String, String> {
    let data = parse_vdf(input)?;
    let app = data.field("AppState").ok_or("Missing AppState")?;
    let id = app
        .field("appid")
        .and_then(Vdf::text)
        .and_then(|s| s.parse::<u32>().ok());
    let flags = app
        .field("StateFlags")
        .and_then(Vdf::text)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let dir = app
        .field("installdir")
        .and_then(Vdf::text)
        .ok_or("Missing game directory")?;
    if id != Some(app_id)
        || flags & 4 == 0
        || dir.is_empty()
        || dir.len() > 255
        || dir == "."
        || dir == ".."
        || dir.contains(['/', '\\', ':'])
    {
        return Err("Steam game is not fully installed or has an invalid directory".into());
    }
    Ok(dir.into())
}

#[cfg(target_os = "windows")]
pub(crate) fn resolve_directory(steam_root: &Path, app_id: u32) -> Result<PathBuf, String> {
    use std::io::Read;
    fn read(path: &Path) -> Result<String, String> {
        let mut s = String::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(1024 * 1024 + 1)
            .read_to_string(&mut s)
            .map_err(|e| e.to_string())?;
        if s.len() > 1024 * 1024 {
            return Err("Steam metadata exceeds 1 MiB".into());
        }
        Ok(s)
    }
    let mut libraries = vec![steam_root.to_path_buf()];
    if let Ok(text) = read(&steam_root.join("steamapps/libraryfolders.vdf")) {
        let parsed = parse_vdf(&text)?;
        if let Some(Vdf::Object(folders)) = parsed.field("libraryfolders") {
            for (key, value) in folders.iter().take(64) {
                if key.parse::<u32>().is_err() {
                    continue;
                }
                if let Some(p) = value
                    .field("path")
                    .and_then(Vdf::text)
                    .or_else(|| value.text())
                {
                    let p = PathBuf::from(p);
                    if p.is_absolute() && !libraries.contains(&p) {
                        libraries.push(p);
                    }
                }
            }
        }
    }
    let mut found = None;
    for library in &libraries {
        let manifest = library.join(format!("steamapps/appmanifest_{app_id}.acf"));
        let Ok(text) = read(&manifest) else {
            continue;
        };
        let Ok(dir) = manifest_directory(&text, app_id) else {
            continue;
        };
        let Ok(root) = library.join("steamapps/common").join(dir).canonicalize() else {
            continue;
        };
        // A junction must not turn one game's authority into an entire library/drive.
        let broad: Vec<_> = libraries
            .iter()
            .flat_map(|lib| {
                [
                    lib.clone(),
                    lib.join("steamapps"),
                    lib.join("steamapps/common"),
                ]
            })
            .collect();
        if root.parent().is_none()
            || broad
                .iter()
                .filter_map(|p| p.canonicalize().ok())
                .any(|p| p == root || canonical_path_within(&p, &root))
        {
            continue;
        }
        if found.as_ref().is_some_and(|p| p != &root) {
            return Err("Game is installed in multiple Steam libraries; choose its process".into());
        }
        found = Some(root);
    }
    found.ok_or_else(|| {
        "Steam game installation not found. Start the game, then choose its process.".into()
    })
}

#[cfg(target_os = "windows")]
pub fn verify_instance(pid: u32, token: u64) -> bool {
    crate::windows::steam::verify_instance(pid, token)
}
#[cfg(not(target_os = "windows"))]
pub fn verify_instance(_pid: u32, _token: u64) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn game_processes(
    _app_id: u32,
    _selected: Option<&Path>,
    _names: &std::collections::HashMap<u32, String>,
) -> Vec<GameProcess> {
    Vec::new()
}
#[cfg(not(target_os = "windows"))]
pub fn process_choices(_query: &str) -> Vec<GameProcess> {
    Vec::new()
}
#[cfg(not(target_os = "windows"))]
pub fn launch(_app_id: u32) -> Result<(), String> {
    Err("Steam game rules currently require Windows".into())
}

#[cfg(target_os = "windows")]
pub use crate::windows::steam::{game_processes, launch, process_choices};

#[cfg(target_os = "windows")]
pub use crate::windows::steam::game_directory;
#[cfg(not(target_os = "windows"))]
pub fn game_directory(_app_id: u32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s103_launch_argument_forms_are_complete_and_lossless() {
        assert_eq!(
            parse_launch_args(&["-applaunch".into(), "570".into()]),
            Some(570)
        );
        assert_eq!(parse_launch_args(&["steam://run/570/".into()]), Some(570));
        assert_eq!(
            parse_launch_args(&["-applaunch".into(), "570".into(), "-foo".into()]),
            None
        );
        assert!(is_game_launch_args(&["-applaunch".into(), "oops".into()]));
    }
    #[test]
    fn s103_explicit_selection_can_resolve_a_generic_launcher() {
        assert!(eligible_image(Path::new("Launcher.exe")));
        assert!(!automatic_image(Path::new("Launcher.exe")));
        assert!(!automatic_image(Path::new("GameLauncher.exe")));
        assert!(automatic_image(Path::new("Game-Win64-Shipping.exe")));
        assert!(!eligible_image(Path::new("steam.exe")));
    }
    #[test]
    fn s103_picker_filters_before_cap_and_prioritizes_windows() {
        let mut rows: Vec<_> = (1..=300)
            .map(|pid| GameProcess {
                pid,
                creation_token: 1,
                path: format!("background{pid}.exe").into(),
                has_window: false,
            })
            .collect();
        rows.push(GameProcess {
            pid: 999,
            creation_token: 1,
            path: "my-game.exe".into(),
            has_window: true,
        });
        assert_eq!(rank_process_choices(rows.clone(), "")[0].pid, 999);
        assert_eq!(rank_process_choices(rows, "my-game")[0].pid, 999);
    }

    #[test]
    fn app_ids_are_native_games_not_arbitrary_protocol_commands() {
        assert_eq!(parse_app_id("steam://rungameid/1867240"), Some(1867240));
        assert_eq!(parse_app_id("steam://run/570/"), Some(570));
        for bad in [
            "steam://run/0",
            "steam://run/18446744073709551615",
            "steam://run/570;quit",
            "steam://open/main",
            "https://run/570",
            "steam://run/570//-evil",
            "steam://run/570?x=1",
        ] {
            assert_eq!(parse_app_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn manifest_binds_app_id_and_rejects_traversal() {
        assert_eq!(
            manifest_directory(
                r#""AppState" { "appid" "570" "StateFlags" "4" "installdir" "dota 2 beta" }"#,
                570
            )
            .unwrap(),
            "dota 2 beta"
        );
        for fields in [
            r#""appid" "571" "StateFlags" "4" "installdir" "game""#,
            r#""appid" "570" "StateFlags" "1" "installdir" "game""#,
            r#""appid" "570" "StateFlags" "4" "installdir" "../other""#,
            r#""appid" "570" "StateFlags" "4" "installdir" "C:\\game""#,
        ] {
            assert!(manifest_directory(&format!("\"AppState\" {{ {fields} }}"), 570).is_err());
        }
    }

    #[test]
    fn metadata_is_bounded_and_ambiguous_keys_are_refused() {
        assert!(parse_vdf(r#""AppState" { "appid" "570" "appid" "571" }"#).is_err());
        assert!(parse_vdf(&"{".repeat(100)).is_err());
        assert!(parse_vdf(r#""AppState" { "appid" "570""#).is_err());
    }

    #[test]
    fn infrastructure_is_never_automatic_game_membership() {
        for name in [
            "steam.exe",
            "steamwebhelper.exe",
            "EasyAntiCheat_EOS.exe",
            "BEService.exe",
            "CrashReportClient.exe",
            "setup.exe",
            "EADesktop.exe",
        ] {
            assert!(!eligible_image(Path::new(name)), "{name}");
        }
        assert!(eligible_image(Path::new("Game-Win64-Shipping.exe")));
        assert!(eligible_image(Path::new("SteamWorldDig2.exe")));
    }

    #[test]
    fn directory_membership_has_real_component_boundaries() {
        assert!(path_within(
            Path::new(r"C:\Games\WARDOGS\bin\game.exe"),
            Path::new(r"c:\games\wardogs")
        ));
        assert!(!path_within(
            Path::new(r"C:\Games\WARDOGS2\game.exe"),
            Path::new(r"C:\Games\WARDOGS")
        ));
        assert!(path_within(
            Path::new(r"\\?\C:\Games\WARDOGS\game.exe"),
            Path::new(r"C:\Games\WARDOGS")
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn local_installation_resolution_checks_manifest_and_missing_library() {
        let root = std::env::temp_dir().join(format!(
            "steam-metadata-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("steamapps/common/Game/bin")).unwrap();
        std::fs::write(
            root.join("steamapps/appmanifest_570.acf"),
            r#""AppState" { "appid" "570" "StateFlags" "4" "installdir" "Game" }"#,
        )
        .unwrap();
        assert_eq!(
            resolve_directory(&root, 570).unwrap(),
            root.join("steamapps/common/Game").canonicalize().unwrap()
        );
        assert!(resolve_directory(&root, 571).is_err());
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            r#""libraryfolders" { "0" { "path" "E:\\UnavailableSteamTest" } }"#,
        )
        .unwrap();
        assert!(resolve_directory(&root, 570).is_ok());
        std::fs::write(
            root.join("steamapps/appmanifest_570.acf"),
            r#""AppState" { "appid" "571" "StateFlags" "4" "installdir" "Game" }"#,
        )
        .unwrap();
        assert!(resolve_directory(&root, 570).is_err());
        let other = root.join("other-library");
        std::fs::create_dir_all(other.join("steamapps/common/Valid")).unwrap();
        std::fs::write(
            other.join("steamapps/appmanifest_570.acf"),
            r#""AppState" { "appid" "570" "StateFlags" "4" "installdir" "Valid" }"#,
        )
        .unwrap();
        let other_text = other.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!("\"libraryfolders\" {{ \"1\" {{ \"path\" \"{other_text}\" }} }}"),
        )
        .unwrap();
        assert_eq!(
            resolve_directory(&root, 570).unwrap(),
            other.join("steamapps/common/Valid").canonicalize().unwrap(),
            "Stale invalid first manifest must not hide a valid library"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
