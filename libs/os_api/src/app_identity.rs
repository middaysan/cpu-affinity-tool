//! Read-only application layout evidence shared by bounded discovery and launch.
use std::path::{Path, PathBuf};

pub fn discord_launcher_args(args: &[String]) -> bool {
    matches!(args, [flag, process] if flag.eq_ignore_ascii_case("--processStart") && process.eq_ignore_ascii_case("Discord.exe"))
}

pub fn discord_launcher_target(path: &Path, args: &[String]) -> bool {
    path.file_name().is_some_and(|name| {
        (name.eq_ignore_ascii_case("Update.exe") && discord_launcher_args(args))
            || (name.eq_ignore_ascii_case("Discord.exe")
                && (args.is_empty() || discord_launcher_args(args)))
    })
}

pub fn is_version_directory(name: &str) -> bool {
    let Some(version) = name.strip_prefix("app-") else {
        return false;
    };
    let parts: Vec<_> = version.split('.').collect();
    name.len() <= 80
        && parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u32>().is_ok()
        })
}

pub fn canonical_scope(root: &Path, image: &Path) -> Option<(PathBuf, PathBuf)> {
    Some((root.canonicalize().ok()?, image.canonicalize().ok()?))
}

pub fn verify_current_instance(pid: u32, token: u64) -> bool {
    #[cfg(windows)]
    {
        crate::agent::ExactProcess::open(pid, token, false, false, true).is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, token);
        false
    }
}

pub fn qualify_installation(path: &Path, discord_launcher: bool) -> bool {
    #[cfg(windows)]
    {
        crate::windows::app_identity::qualify_installation(path, discord_launcher)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, discord_launcher);
        false
    }
}

/// Import-time product classification; not cryptographic publisher authentication.
pub fn qualify_catalog_image(path: &Path, vendors: &[&str]) -> bool {
    #[cfg(windows)]
    {
        crate::windows::app_identity::qualify_catalog_image(path, vendors)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, vendors);
        false
    }
}

pub fn activate_catalog_client(
    path: &Path,
    args: &[String],
    vendors: &[&str],
) -> Result<(), String> {
    if !qualify_catalog_image(path, vendors) {
        return Err("Application product could not be verified".into());
    }
    spawn_ordinary(path, args)
}

/// Launches only the caller's verified application path as an ordinary desktop child.
/// No requested affinity or priority is applied to an intermediate launcher.
pub fn launch_ordinary(path: &Path, args: &[String]) -> Result<(), String> {
    if !discord_launcher_target(path, args) || !qualify_installation(path, true) {
        return Err("Discord launcher installation or arguments could not be verified".into());
    }
    spawn_ordinary(path, args)
}

/// Standard client activation when its owned processes have no focusable window.
pub fn activate_client(path: &Path, args: &[String]) -> Result<(), String> {
    if path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name.eq_ignore_ascii_case("Discord"))
        && discord_launcher_target(path, args)
        && qualify_installation(path, true)
    {
        return launch_ordinary(path, args);
    }
    if !qualify_installation(path, false) {
        return Err("Application installation could not be verified".into());
    }
    // Focus opens the client normally, without reusing an imported silent-start flag.
    spawn_ordinary(path, &[])
}
fn spawn_ordinary(path: &Path, args: &[String]) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut child = crate::agent::launch_executable(&crate::agent::LaunchSpec {
            executable: path.into(),
            args: args.into(),
            cwd: path.parent().map(PathBuf::from),
            cpus: None,
            priority: crate::PriorityClass::Normal,
            elevated: false,
            capture: false,
            environment: Default::default(),
        })?;
        child.resume()
    }
    #[cfg(not(windows))]
    {
        let _ = (path, args);
        Err("Windows application launch is unavailable".into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn s107_version_directories_are_finite_path_components() {
        for name in ["app-1.0.999", "app-0.0.1"] {
            assert!(super::is_version_directory(name));
        }
        for name in [
            "app-",
            "app-1.2.3\\evil",
            "app-..",
            "app-1.2.3.4",
            "app-x.y.z",
        ] {
            assert!(!super::is_version_directory(name));
        }
    }
}
