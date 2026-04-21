use super::StateStorageMode;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(super) const STATE_FILE_NAME: &str = "state.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedStateLocation {
    pub(super) mode: StateStorageMode,
    pub(super) state_path: PathBuf,
}

impl ResolvedStateLocation {
    fn data_dir(&self) -> PathBuf {
        self.state_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

static RESOLVED_STATE_LOCATION: OnceLock<ResolvedStateLocation> = OnceLock::new();

pub(super) fn get_state_path() -> PathBuf {
    get_resolved_state_location().state_path.clone()
}

pub(super) fn get_state_dir() -> PathBuf {
    get_resolved_state_location().data_dir()
}

pub(super) fn get_state_storage_mode() -> StateStorageMode {
    get_resolved_state_location().mode
}

pub(super) fn get_resolved_state_location() -> &'static ResolvedStateLocation {
    RESOLVED_STATE_LOCATION.get_or_init(resolve_state_location)
}

fn resolve_state_location() -> ResolvedStateLocation {
    let sidecar_state_path = current_sidecar_state_path();
    let platform_state_path = current_platform_state_path();

    resolve_state_location_with(
        sidecar_state_path.clone(),
        sidecar_state_path.exists(),
        platform_state_path,
        |dir| std::fs::create_dir_all(dir).is_ok(),
    )
}

fn resolve_state_location_with(
    sidecar_state_path: PathBuf,
    sidecar_exists: bool,
    platform_state_path: Option<PathBuf>,
    mut ensure_platform_dir: impl FnMut(&Path) -> bool,
) -> ResolvedStateLocation {
    if sidecar_exists {
        return ResolvedStateLocation {
            mode: StateStorageMode::LegacySidecar,
            state_path: sidecar_state_path,
        };
    }

    if let Some(platform_state_path) = platform_state_path {
        if let Some(parent) = platform_state_path.parent() {
            if ensure_platform_dir(parent) {
                return ResolvedStateLocation {
                    mode: StateStorageMode::PlatformData,
                    state_path: platform_state_path,
                };
            }
        }
    }

    ResolvedStateLocation {
        mode: StateStorageMode::LegacySidecar,
        state_path: sidecar_state_path,
    }
}

fn current_sidecar_state_path() -> PathBuf {
    std::env::current_exe()
        .map(|mut path| {
            path.set_file_name(STATE_FILE_NAME);
            path
        })
        .unwrap_or_else(|_| STATE_FILE_NAME.into())
}

fn current_platform_state_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        platform_state_path_for(
            "windows",
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            None,
            None,
        )
    }

    #[cfg(target_os = "linux")]
    {
        platform_state_path_for(
            "linux",
            None,
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        )
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

fn platform_state_path_for(
    platform: &str,
    local_app_data: Option<PathBuf>,
    xdg_data_home: Option<PathBuf>,
    home_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    match platform {
        "windows" => local_app_data.map(|path| {
            let base = path
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_string();
            PathBuf::from(format!(r"{base}\CpuAffinityTool\{STATE_FILE_NAME}"))
        }),
        "linux" => xdg_data_home
            .or_else(|| home_dir.map(|path| path.join(".local").join("share")))
            .map(|path| path.join("cpu-affinity-tool").join(STATE_FILE_NAME)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::StateStorageMode;
    use super::{
        platform_state_path_for, resolve_state_location_with, ResolvedStateLocation,
        STATE_FILE_NAME,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), unique));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_resolve_state_location_prefers_existing_sidecar_state() {
        let exe_dir = TempDirGuard::new("cpu_affinity_tool_sidecar");
        let sidecar_state_path = exe_dir.path.join(STATE_FILE_NAME);
        fs::write(&sidecar_state_path, "{}").unwrap();

        let platform_state_path = exe_dir.path.join("platform").join(STATE_FILE_NAME);
        let resolved = resolve_state_location_with(
            sidecar_state_path.clone(),
            true,
            Some(platform_state_path),
            |_| true,
        );

        assert_eq!(
            resolved,
            ResolvedStateLocation {
                mode: StateStorageMode::LegacySidecar,
                state_path: sidecar_state_path,
            }
        );
    }

    #[test]
    fn test_resolve_state_location_uses_platform_path_when_sidecar_missing() {
        let exe_dir = TempDirGuard::new("cpu_affinity_tool_platform");
        let sidecar_state_path = exe_dir.path.join(STATE_FILE_NAME);
        let platform_state_path = exe_dir.path.join("data").join(STATE_FILE_NAME);

        let resolved = resolve_state_location_with(
            sidecar_state_path,
            false,
            Some(platform_state_path.clone()),
            |_| true,
        );

        assert_eq!(
            resolved,
            ResolvedStateLocation {
                mode: StateStorageMode::PlatformData,
                state_path: platform_state_path,
            }
        );
    }

    #[test]
    fn test_resolve_state_location_falls_back_to_sidecar_if_platform_dir_unavailable() {
        let exe_dir = TempDirGuard::new("cpu_affinity_tool_fallback");
        let sidecar_state_path = exe_dir.path.join(STATE_FILE_NAME);
        let platform_state_path = exe_dir.path.join("data").join(STATE_FILE_NAME);

        let resolved = resolve_state_location_with(
            sidecar_state_path.clone(),
            false,
            Some(platform_state_path),
            |_| false,
        );

        assert_eq!(
            resolved,
            ResolvedStateLocation {
                mode: StateStorageMode::LegacySidecar,
                state_path: sidecar_state_path,
            }
        );
    }

    #[test]
    fn test_resolve_state_location_falls_back_to_sidecar_if_platform_path_missing() {
        let exe_dir = TempDirGuard::new("cpu_affinity_tool_missing_platform");
        let sidecar_state_path = exe_dir.path.join(STATE_FILE_NAME);

        let resolved =
            resolve_state_location_with(sidecar_state_path.clone(), false, None, |_| true);

        assert_eq!(
            resolved,
            ResolvedStateLocation {
                mode: StateStorageMode::LegacySidecar,
                state_path: sidecar_state_path,
            }
        );
    }

    #[test]
    fn test_platform_state_path_for_windows_uses_local_app_data() {
        let path = platform_state_path_for(
            "windows",
            Some(PathBuf::from(r"C:\Users\Admin\AppData\Local")),
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\Admin\AppData\Local\CpuAffinityTool\state.json")
        );
    }

    #[test]
    fn test_platform_state_path_for_linux_prefers_xdg_data_home() {
        let path = platform_state_path_for(
            "linux",
            None,
            Some(PathBuf::from("/home/alice/.xdg-data")),
            Some(PathBuf::from("/home/alice")),
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from("/home/alice/.xdg-data/cpu-affinity-tool/state.json")
        );
    }

    #[test]
    fn test_platform_state_path_for_linux_falls_back_to_home_share_dir() {
        let path = platform_state_path_for("linux", None, None, Some(PathBuf::from("/home/alice")))
            .unwrap();

        assert_eq!(
            path,
            PathBuf::from("/home/alice/.local/share/cpu-affinity-tool/state.json")
        );
    }
}
