use crate::{
    OS,
    steam::{GameProcess, eligible_image},
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    UI::WindowsAndMessaging::*,
};

type MetadataCache = HashMap<u32, (Instant, Result<PathBuf, String>)>;
fn directory(app_id: u32) -> Result<PathBuf, String> {
    static CACHE: OnceLock<Mutex<MetadataCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(guard) = cache.lock()
        && let Some((at, value)) = guard.get(&app_id)
        && at.elapsed() < Duration::from_secs(15)
    {
        return value.clone();
    }
    let value = OS::get_program_path_for_uri("steam").and_then(|exe| {
        let root = exe.parent().ok_or("Steam handler has no directory")?;
        crate::steam::resolve_directory(root, app_id)
    });
    if let Ok(mut guard) = cache.lock() {
        if guard.len() >= 128 {
            guard.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(15));
            if guard.len() >= 128 {
                guard.clear();
            }
        }
        guard.insert(app_id, (Instant::now(), value.clone()));
    }
    value
}

pub fn window_processes() -> HashSet<u32> {
    unsafe extern "system" fn visit(window: HWND, param: LPARAM) -> windows::core::BOOL {
        let result = unsafe { &mut *(param.0 as *mut HashSet<u32>) };
        let mut rect = RECT::default();
        let visible = unsafe {
            IsWindowVisible(window).as_bool()
                && GetWindow(window, GW_OWNER).unwrap_or_default().0.is_null()
                && GetWindowLongW(window, GWL_EXSTYLE) & WS_EX_TOOLWINDOW.0 as i32 == 0
                && GetWindowRect(window, &mut rect).is_ok()
        };
        if unsafe { IsIconic(window).as_bool() } {
            let mut placement = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            if unsafe { GetWindowPlacement(window, &mut placement) }.is_ok() {
                rect = placement.rcNormalPosition;
            }
        }
        if visible
            && i64::from(rect.right) - i64::from(rect.left) >= 128
            && i64::from(rect.bottom) - i64::from(rect.top) >= 128
            && result.len() < 128
        {
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(window, Some(&mut pid));
            }
            if pid != 0 {
                result.insert(pid);
            }
        }
        true.into()
    }
    let mut result = HashSet::new();
    let _ = unsafe {
        EnumWindows(
            Some(visit),
            LPARAM((&mut result as *mut HashSet<u32>) as isize),
        )
    };
    result
}

pub fn verify_instance(pid: u32, token: u64) -> bool {
    crate::agent::ExactProcess::open(pid, token, false, false, true).is_ok()
}

fn inspect(pid: u32, windows: &HashSet<u32>) -> Option<GameProcess> {
    let (path, creation_token) = OS::get_process_image_path_and_instance_token(pid).ok()?;
    if !eligible_image(&path) {
        return None;
    }
    if !verify_instance(pid, creation_token) {
        return None;
    }
    Some(GameProcess {
        pid,
        creation_token,
        path,
        has_window: windows.contains(&pid),
    })
}

pub fn game_processes(
    app_id: u32,
    selected: Option<&Path>,
    names: &HashMap<u32, String>,
) -> Vec<GameProcess> {
    if let Some(selected) = selected {
        let windows: HashSet<_> = window_images().iter().map(|p| p.pid).collect();
        let Some(name) = selected.file_name() else {
            return Vec::new();
        };
        return names
            .iter()
            .filter(|(_, n)| n.eq_ignore_ascii_case(&name.to_string_lossy()))
            .filter_map(|(&pid, _)| inspect(pid, &windows))
            .filter(|p| {
                p.path
                    .canonicalize()
                    .ok()
                    .zip(selected.canonicalize().ok())
                    .is_some_and(|(a, b)| a == b)
            })
            .take(64)
            .collect();
    }
    let Ok(root) = directory(app_id) else {
        return Vec::new();
    };
    automatic_processes(&root)
}

fn automatic_processes(root: &Path) -> Vec<GameProcess> {
    window_images()
        .iter()
        .filter_map(|p| {
            if !crate::steam::canonical_path_within(&p.path, root) {
                return None;
            }
            crate::agent::ExactProcess::open(p.pid, p.creation_token, false, false, true).ok()?;
            Some(p.clone())
        })
        .collect()
}

fn window_images() -> std::sync::Arc<Vec<GameProcess>> {
    type Cache = Option<(Instant, std::sync::Arc<Vec<GameProcess>>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(guard) = cache.lock()
        && let Some((at, images)) = guard.as_ref()
        && at.elapsed() < Duration::from_secs(1)
    {
        return images.clone();
    }
    let images: std::sync::Arc<Vec<GameProcess>> = std::sync::Arc::new(
        window_processes()
            .into_iter()
            .filter_map(|pid| {
                let (path, creation_token) =
                    OS::get_process_image_path_and_instance_token(pid).ok()?;
                if !eligible_image(&path) {
                    return None;
                }
                Some(GameProcess {
                    pid,
                    creation_token,
                    path: path.canonicalize().ok()?,
                    has_window: true,
                })
            })
            .collect(),
    );
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), images.clone()));
    }
    images
}

pub fn process_choices(query: &str) -> Vec<GameProcess> {
    let windows = window_processes();
    let query = query.to_lowercase();
    let mut names: Vec<_> = OS::get_all_process_names()
        .into_iter()
        .filter(|(_, name)| eligible_image(Path::new(name)) && name.to_lowercase().contains(&query))
        .collect();
    names.sort_by_key(|(pid, name)| (!windows.contains(pid), name.to_lowercase(), *pid));
    crate::steam::rank_process_choices(
        names
            .into_iter()
            .take(4096)
            .filter_map(|(pid, _)| inspect(pid, &windows))
            .collect(),
        &query,
    )
}

pub fn launch(app_id: u32) -> Result<(), String> {
    if app_id == 0 {
        return Err("A nonzero Steam AppID is required".into());
    }
    super::agent::activate_steam_game(app_id)
}

pub fn game_directory(app_id: u32) -> Option<PathBuf> {
    directory(app_id).ok()
}

pub fn game_directory_result(app_id: u32) -> Result<PathBuf, String> {
    directory(app_id)
}

#[cfg(test)]
mod native_tests {
    use super::*;

    #[test]
    #[ignore = "Explicit desktop smoke; owns only a temporary child window/process"]
    fn owned_game_window_discovery_and_exact_fallback() {
        const MARKER: &str = "CPU_AFFINITY_STEAM_OWNED_WINDOW";
        if let Some(ready) = std::env::var_os(MARKER) {
            let title = windows::core::w!("Owned game discovery test");
            let window = unsafe {
                CreateWindowExW(
                    Default::default(),
                    windows::core::w!("STATIC"),
                    title,
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    20,
                    20,
                    420,
                    230,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap();
            std::fs::write(ready, b"ready").unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            while Instant::now() < deadline {
                let mut message = MSG::default();
                while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
                    unsafe {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = unsafe { DestroyWindow(window) };
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "owned-game-window-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("OwnedGame.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let ready = root.join("ready");
        let child = std::process::Command::new(&executable)
            .args([
                "--exact",
                "windows::steam::native_tests::owned_game_window_discovery_and_exact_fallback",
                "--ignored",
                "--nocapture",
            ])
            .env(MARKER, &ready)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let child = Child(child);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(ready.exists(), "Owned window did not become ready");
        let found = automatic_processes(&root.canonicalize().unwrap());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, child.0.id());
        assert!(
            game_processes(
                570,
                Some(&executable),
                &OS::get_all_process_names().into_iter().collect()
            )
            .iter()
            .any(|p| p.pid == child.0.id() && p.creation_token == found[0].creation_token)
        );
        assert!(
            !automatic_processes(&root.join("Other"))
                .iter()
                .any(|p| p.pid == child.0.id())
        );
        assert!(
            crate::agent::ExactProcess::open(
                child.0.id(),
                found[0].creation_token + 1,
                false,
                false,
                true
            )
            .is_err()
        );
        drop(child);
        std::fs::remove_dir_all(root).unwrap();
    }
}
