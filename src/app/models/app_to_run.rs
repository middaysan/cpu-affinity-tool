use std::borrow::Borrow;
use std::fmt;
use std::path::{Path, PathBuf};

use os_api::PriorityClass;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    Path {
        dropped_path: PathBuf,
        bin_path: PathBuf,
    },
    Installed {
        aumid: String,
    },
}

#[derive(Debug, Serialize)]
struct AppRuntimeKeyPayload {
    target_kind: &'static str,
    target_id: String,
    args: Vec<String>,
    priority: PriorityClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppRuntimeKey(String);

impl AppRuntimeKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn from_parts(
        target_kind: &'static str,
        target_id: String,
        args: &[String],
        priority: PriorityClass,
    ) -> Self {
        let payload = AppRuntimeKeyPayload {
            target_kind,
            target_id,
            args: args.to_vec(),
            priority,
        };

        let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
            format!(
                "{{\"target_kind\":\"{}\",\"target_id\":\"{}\"}}",
                payload.target_kind, payload.target_id
            )
        });

        Self(encoded)
    }
}

impl Borrow<str> for AppRuntimeKey {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for AppRuntimeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct AppToRun {
    /// Display name of the application
    pub name: String,
    /// Launch identity for the application
    pub launch_target: LaunchTarget,
    /// Command-line arguments to pass to the application
    pub args: Vec<String>,
    /// Additional process names to track (e.g. "discord.exe")
    #[serde(default)]
    pub additional_processes: Vec<String>,
    /// Whether the application should start automatically on application startup
    pub autorun: bool,
    /// Process priority class to assign to the application
    pub priority: PriorityClass,
}

#[derive(Deserialize)]
struct AppToRunV5 {
    name: String,
    launch_target: LaunchTarget,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    additional_processes: Vec<String>,
    autorun: bool,
    priority: PriorityClass,
}

#[derive(Deserialize)]
struct AppToRunV4 {
    name: String,
    dropped_path: PathBuf,
    #[serde(default)]
    args: Vec<String>,
    bin_path: PathBuf,
    #[serde(default)]
    additional_processes: Vec<String>,
    autorun: bool,
    priority: PriorityClass,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AppToRunSerde {
    V5(AppToRunV5),
    V4(AppToRunV4),
}

impl<'de> Deserialize<'de> for AppToRun {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match AppToRunSerde::deserialize(deserializer)? {
            AppToRunSerde::V5(v5) => Ok(Self {
                name: v5.name,
                launch_target: v5.launch_target,
                args: v5.args,
                additional_processes: v5.additional_processes,
                autorun: v5.autorun,
                priority: v5.priority,
            }),
            AppToRunSerde::V4(v4) => Ok(Self {
                name: v4.name,
                launch_target: LaunchTarget::Path {
                    dropped_path: v4.dropped_path,
                    bin_path: v4.bin_path,
                },
                args: v4.args,
                additional_processes: v4.additional_processes,
                autorun: v4.autorun,
                priority: v4.priority,
            }),
        }
    }
}

impl AppToRun {
    pub fn new_path(
        dropped_path: PathBuf,
        args: Vec<String>,
        bin_path: PathBuf,
        priority: PriorityClass,
        autorun: bool,
    ) -> Self {
        let name = dropped_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or("Unknown")
            .to_string();

        Self {
            name,
            launch_target: LaunchTarget::Path {
                dropped_path,
                bin_path,
            },
            args,
            additional_processes: Vec::new(),
            autorun,
            priority,
        }
    }

    pub fn new_installed(
        name: String,
        aumid: String,
        priority: PriorityClass,
        autorun: bool,
    ) -> Self {
        Self {
            name,
            launch_target: LaunchTarget::Installed { aumid },
            args: Vec::new(),
            additional_processes: Vec::new(),
            autorun,
            priority,
        }
    }

    pub fn bin_path(&self) -> Option<&Path> {
        match &self.launch_target {
            LaunchTarget::Path { bin_path, .. } => Some(bin_path.as_path()),
            LaunchTarget::Installed { .. } => None,
        }
    }

    pub fn bin_path_mut(&mut self) -> Option<&mut PathBuf> {
        match &mut self.launch_target {
            LaunchTarget::Path { bin_path, .. } => Some(bin_path),
            LaunchTarget::Installed { .. } => None,
        }
    }

    pub fn dropped_path(&self) -> Option<&Path> {
        match &self.launch_target {
            LaunchTarget::Path { dropped_path, .. } => Some(dropped_path.as_path()),
            LaunchTarget::Installed { .. } => None,
        }
    }

    pub fn installed_aumid(&self) -> Option<&str> {
        match &self.launch_target {
            LaunchTarget::Installed { aumid } => Some(aumid.as_str()),
            LaunchTarget::Path { .. } => None,
        }
    }

    pub fn is_path_target(&self) -> bool {
        matches!(self.launch_target, LaunchTarget::Path { .. })
    }

    pub fn is_installed_target(&self) -> bool {
        matches!(self.launch_target, LaunchTarget::Installed { .. })
    }

    pub fn is_args_editable(&self) -> bool {
        self.is_path_target()
    }

    pub fn launch_target_label(&self) -> String {
        match &self.launch_target {
            LaunchTarget::Path { bin_path, .. } => bin_path.display().to_string(),
            LaunchTarget::Installed { aumid } => format!("Installed app ({aumid})"),
        }
    }

    pub fn launch_target_detail(&self) -> String {
        match &self.launch_target {
            LaunchTarget::Path {
                dropped_path,
                bin_path,
            } => format!("{} (src: {})", bin_path.display(), dropped_path.display()),
            LaunchTarget::Installed { aumid } => format!("Installed app AUMID: {aumid}"),
        }
    }

    pub fn runtime_key(&self) -> AppRuntimeKey {
        AppRuntimeKey::from(self)
    }

    pub fn get_key(&self) -> AppRuntimeKey {
        self.runtime_key()
    }

    pub fn display(&self) -> String {
        match &self.launch_target {
            LaunchTarget::Path {
                dropped_path,
                bin_path,
            } => format!(
                "{} {}(src: {}) P({:?})",
                bin_path.display(),
                self.args.join(" "),
                dropped_path.display(),
                self.priority
            ),
            LaunchTarget::Installed { aumid } => {
                format!("Installed({aumid}) P({:?})", self.priority)
            }
        }
    }

    fn target_id(&self) -> String {
        match &self.launch_target {
            LaunchTarget::Path { bin_path, .. } => normalized_path_identity(bin_path),
            LaunchTarget::Installed { aumid } => aumid.clone(),
        }
    }
}

impl From<&AppToRun> for AppRuntimeKey {
    fn from(app: &AppToRun) -> Self {
        let target_kind = if app.is_installed_target() {
            "installed"
        } else {
            "path"
        };

        Self::from_parts(target_kind, app.target_id(), &app.args, app.priority)
    }
}

#[cfg(target_os = "windows")]
fn normalized_path_identity(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('/', "\\");
    normalized
        .strip_prefix(r"\\?\")
        .unwrap_or(&normalized)
        .to_ascii_lowercase()
}

#[cfg(not(target_os = "windows"))]
fn normalized_path_identity(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::{AppToRun, LaunchTarget};
    use os_api::PriorityClass;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn test_runtime_key_distinguishes_targets_and_priority() {
        let path = AppToRun::new_path(
            PathBuf::from(r"C:\App.lnk"),
            vec!["--debug".into()],
            PathBuf::from(r"C:\App.exe"),
            PriorityClass::Normal,
            false,
        );
        let installed = AppToRun::new_installed(
            "Spotify".into(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            PriorityClass::Normal,
            false,
        );
        let high = AppToRun::new_installed(
            "Spotify".into(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            PriorityClass::High,
            false,
        );

        assert_ne!(path.runtime_key(), installed.runtime_key());
        assert_ne!(installed.runtime_key(), high.runtime_key());
    }

    #[test]
    fn test_runtime_key_normalizes_windows_path_identity() {
        let first = AppToRun::new_path(
            PathBuf::from(r"C:\Shortcuts\Spotify.lnk"),
            vec![],
            PathBuf::from(r"C:\Program Files\Spotify\Spotify.exe"),
            PriorityClass::Normal,
            false,
        );
        let second = AppToRun::new_path(
            PathBuf::from(r"D:\Pinned\Spotify.lnk"),
            vec![],
            PathBuf::from(r"c:/program files/spotify/Spotify.exe"),
            PriorityClass::Normal,
            false,
        );

        assert_eq!(first.runtime_key(), second.runtime_key());
    }

    #[test]
    fn test_runtime_key_encoded_contract_stays_stable() {
        let path = AppToRun::new_path(
            PathBuf::from(r"C:\App.lnk"),
            vec!["--debug".into()],
            PathBuf::from(r"C:\App.exe"),
            PriorityClass::Normal,
            false,
        );
        let installed = AppToRun::new_installed(
            "Spotify".into(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            PriorityClass::High,
            false,
        );

        assert_eq!(
            path.runtime_key().as_str(),
            r#"{"target_kind":"path","target_id":"c:\\app.exe","args":["--debug"],"priority":"Normal"}"#
        );
        assert_eq!(
            installed.runtime_key().as_str(),
            r#"{"target_kind":"installed","target_id":"SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify","args":[],"priority":"High"}"#
        );
    }

    #[test]
    fn test_v4_shape_deserializes_to_path_target() {
        let value = json!({
            "name": "Sample",
            "dropped_path": r"C:\Sample.lnk",
            "args": ["--fullscreen"],
            "bin_path": r"C:\Sample.exe",
            "additional_processes": ["helper.exe"],
            "autorun": true,
            "priority": "Normal"
        });

        let app: AppToRun = serde_json::from_value(value).unwrap();
        assert!(matches!(
            app.launch_target,
            LaunchTarget::Path {
                dropped_path,
                bin_path,
            } if dropped_path == PathBuf::from(r"C:\Sample.lnk")
                && bin_path == PathBuf::from(r"C:\Sample.exe")
        ));
        assert_eq!(app.args, vec!["--fullscreen".to_string()]);
        assert_eq!(app.additional_processes, vec!["helper.exe".to_string()]);
    }

    #[test]
    fn test_v5_shape_deserializes_to_installed_target() {
        let value = json!({
            "name": "Spotify",
            "launch_target": {
                "Installed": {
                    "aumid": "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"
                }
            },
            "args": [],
            "additional_processes": [],
            "autorun": false,
            "priority": "Normal"
        });

        let app: AppToRun = serde_json::from_value(value).unwrap();
        assert_eq!(
            app.installed_aumid(),
            Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify")
        );
        assert!(app.is_installed_target());
    }
}
