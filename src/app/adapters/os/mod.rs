use os_api::{PriorityClass, ShortcutSpec};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const TEMP_SHORTCUT_ATTEMPTS: usize = 100;

pub fn set_current_process_priority(priority: PriorityClass) -> Result<(), String> {
    os_api::OS::set_current_process_priority(priority)
}

pub fn get_cpu_model() -> String {
    os_api::OS::get_cpu_model()
}

pub fn supports_hide_to_tray() -> bool {
    os_api::OS::supports_hide_to_tray()
}

pub fn open_directory(path: &Path) -> Result<(), String> {
    os_api::OS::open_directory(path)
}

pub fn current_exe_path() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|err| format!("failed to resolve current executable: {err}"))
}

pub fn current_user_desktop_dir() -> Result<PathBuf, String> {
    os_api::OS::current_user_desktop_dir()
}

pub fn shortcut_path_exists(path: &Path) -> Result<bool, String> {
    path.try_exists()
        .map_err(|err| format!("failed to check shortcut path '{}': {err}", path.display()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateShortcutNewError {
    AlreadyExists,
    ReserveFailed(String),
    WriteFailed(String),
}

trait ShortcutFileWriter {
    fn write_shortcut(&mut self, spec: ShortcutSpec) -> Result<(), String>;
}

struct OsShortcutFileWriter;

impl ShortcutFileWriter for OsShortcutFileWriter {
    fn write_shortcut(&mut self, spec: ShortcutSpec) -> Result<(), String> {
        os_api::OS::create_shortcut(spec)
    }
}

pub fn create_shortcut_new(spec: ShortcutSpec) -> Result<(), CreateShortcutNewError> {
    create_shortcut_new_with_writer(spec, &mut OsShortcutFileWriter)
}

fn create_shortcut_new_with_writer(
    spec: ShortcutSpec,
    writer: &mut impl ShortcutFileWriter,
) -> Result<(), CreateShortcutNewError> {
    let final_path = spec.shortcut_path.clone();
    if shortcut_path_exists(&final_path).map_err(CreateShortcutNewError::ReserveFailed)? {
        return Err(CreateShortcutNewError::AlreadyExists);
    }

    let temp_path = reserve_temp_shortcut_path(&final_path)?;
    let mut temp_spec = spec;
    temp_spec.shortcut_path = temp_path.clone();

    if let Err(err) = writer.write_shortcut(temp_spec) {
        let _ = fs::remove_file(&temp_path);
        return Err(CreateShortcutNewError::WriteFailed(err));
    }

    match move_temp_shortcut_without_replace(&temp_path, &final_path) {
        Ok(()) => Ok(()),
        Err(MoveShortcutError::AlreadyExists) => {
            let _ = fs::remove_file(&temp_path);
            Err(CreateShortcutNewError::AlreadyExists)
        }
        Err(MoveShortcutError::Failed(err)) => {
            let _ = fs::remove_file(&temp_path);
            Err(CreateShortcutNewError::WriteFailed(format!(
                "failed to move temporary shortcut '{}' to '{}': {err}",
                temp_path.display(),
                final_path.display()
            )))
        }
    }
}

fn reserve_temp_shortcut_path(final_path: &Path) -> Result<PathBuf, CreateShortcutNewError> {
    let parent = final_path.parent().ok_or_else(|| {
        CreateShortcutNewError::ReserveFailed(format!(
            "shortcut path '{}' has no parent directory",
            final_path.display()
        ))
    })?;
    let file_name = final_path.file_name().ok_or_else(|| {
        CreateShortcutNewError::ReserveFailed(format!(
            "shortcut path '{}' has no file name",
            final_path.display()
        ))
    })?;
    let nonce = temp_shortcut_nonce();

    for attempt in 0..TEMP_SHORTCUT_ATTEMPTS {
        let mut temp_name = OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".{nonce}.{attempt}.tmp"));
        let temp_path = parent.join(temp_name);

        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => {
                drop(file);
                return Ok(temp_path);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(CreateShortcutNewError::ReserveFailed(format!(
                    "failed to reserve temporary shortcut path '{}': {err}",
                    temp_path.display()
                )));
            }
        }
    }

    Err(CreateShortcutNewError::ReserveFailed(format!(
        "failed to reserve a unique temporary shortcut path in '{}'",
        parent.display()
    )))
}

fn temp_shortcut_nonce() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}

enum MoveShortcutError {
    AlreadyExists,
    Failed(io::Error),
}

#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;
#[cfg(target_os = "windows")]
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};

#[cfg(target_os = "windows")]
fn move_temp_shortcut_without_replace(
    temp_path: &Path,
    final_path: &Path,
) -> Result<(), MoveShortcutError> {
    let temp_path_wide = path_to_wide_null(temp_path);
    let final_path_wide = path_to_wide_null(final_path);

    let moved = unsafe {
        MoveFileExW(
            PCWSTR(temp_path_wide.as_ptr()),
            PCWSTR(final_path_wide.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    };

    if moved.is_ok() {
        return Ok(());
    }

    {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::AlreadyExists || final_path.try_exists().unwrap_or(false) {
            Err(MoveShortcutError::AlreadyExists)
        } else {
            Err(MoveShortcutError::Failed(err))
        }
    }
}

#[cfg(target_os = "windows")]
fn path_to_wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn move_temp_shortcut_without_replace(
    temp_path: &Path,
    final_path: &Path,
) -> Result<(), MoveShortcutError> {
    fs::hard_link(temp_path, final_path).map_err(|err| {
        if err.kind() == io::ErrorKind::AlreadyExists || final_path.try_exists().unwrap_or(false) {
            MoveShortcutError::AlreadyExists
        } else {
            MoveShortcutError::Failed(err)
        }
    })?;
    fs::remove_file(temp_path).map_err(MoveShortcutError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(path: PathBuf) -> Self {
            Self(path)
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeWriteMode {
        Success,
        FinalAppearsBeforeRename,
        FailAfterPartialWrite,
    }

    struct FakeShortcutWriter {
        final_path: PathBuf,
        mode: FakeWriteMode,
        write_paths: Vec<PathBuf>,
    }

    impl FakeShortcutWriter {
        fn new(final_path: PathBuf, mode: FakeWriteMode) -> Self {
            Self {
                final_path,
                mode,
                write_paths: Vec::new(),
            }
        }
    }

    impl ShortcutFileWriter for FakeShortcutWriter {
        fn write_shortcut(&mut self, spec: ShortcutSpec) -> Result<(), String> {
            self.write_paths.push(spec.shortcut_path.clone());
            match self.mode {
                FakeWriteMode::Success => {
                    fs::write(&spec.shortcut_path, b"new shortcut").unwrap();
                    Ok(())
                }
                FakeWriteMode::FinalAppearsBeforeRename => {
                    fs::write(&spec.shortcut_path, b"new shortcut").unwrap();
                    fs::write(&self.final_path, b"existing user shortcut").unwrap();
                    Ok(())
                }
                FakeWriteMode::FailAfterPartialWrite => {
                    fs::write(&spec.shortcut_path, b"partial shortcut").unwrap();
                    Err("simulated writer failure".to_string())
                }
            }
        }
    }

    fn unique_suffix() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{}-{nanos}", process::id())
    }

    fn temp_dir() -> (PathBuf, TempDirGuard) {
        let path = std::env::temp_dir().join(format!("codex-shortcut-new-{}", unique_suffix()));
        fs::create_dir_all(&path).unwrap();
        let guard = TempDirGuard::new(path.clone());
        (path, guard)
    }

    fn shortcut_spec(path: PathBuf) -> ShortcutSpec {
        ShortcutSpec {
            shortcut_path: path.clone(),
            target_path: path.with_file_name("cpu-affinity-tool.exe"),
            arguments: vec![
                "--run-rule".to_string(),
                "group-0".to_string(),
                "rule-0".to_string(),
            ],
            working_dir: path.parent().map(PathBuf::from),
            icon_path: None,
            icon_index: 0,
        }
    }

    #[test]
    fn test_create_shortcut_new_writer_receives_temp_path_not_final_path() {
        let (dir, _guard) = temp_dir();
        let final_path = dir.join("Rule.lnk");
        let mut writer = FakeShortcutWriter::new(final_path.clone(), FakeWriteMode::Success);

        create_shortcut_new_with_writer(shortcut_spec(final_path.clone()), &mut writer).unwrap();

        assert_eq!(writer.write_paths.len(), 1);
        assert_ne!(writer.write_paths[0], final_path);
        assert!(!writer.write_paths[0].exists());
        assert_eq!(fs::read(&final_path).unwrap(), b"new shortcut");
    }

    #[test]
    fn test_create_shortcut_new_preserves_final_path_that_appears_before_rename() {
        let (dir, _guard) = temp_dir();
        let final_path = dir.join("Rule.lnk");
        let mut writer =
            FakeShortcutWriter::new(final_path.clone(), FakeWriteMode::FinalAppearsBeforeRename);

        let result =
            create_shortcut_new_with_writer(shortcut_spec(final_path.clone()), &mut writer);

        assert_eq!(result, Err(CreateShortcutNewError::AlreadyExists));
        assert_eq!(fs::read(&final_path).unwrap(), b"existing user shortcut");
        assert_eq!(writer.write_paths.len(), 1);
        assert!(!writer.write_paths[0].exists());
    }

    #[test]
    fn test_create_shortcut_new_removes_temp_path_on_writer_failure() {
        let (dir, _guard) = temp_dir();
        let final_path = dir.join("Rule.lnk");
        let mut writer =
            FakeShortcutWriter::new(final_path.clone(), FakeWriteMode::FailAfterPartialWrite);

        let result = create_shortcut_new_with_writer(shortcut_spec(final_path), &mut writer);

        assert!(matches!(
            result,
            Err(CreateShortcutNewError::WriteFailed(_))
        ));
        assert_eq!(writer.write_paths.len(), 1);
        assert!(!writer.write_paths[0].exists());
    }

    #[test]
    fn test_create_shortcut_new_existing_final_path_returns_collision_without_writing() {
        let (dir, _guard) = temp_dir();
        let final_path = dir.join("Rule.lnk");
        fs::write(&final_path, b"existing user shortcut").unwrap();
        let mut writer = FakeShortcutWriter::new(final_path.clone(), FakeWriteMode::Success);

        let result =
            create_shortcut_new_with_writer(shortcut_spec(final_path.clone()), &mut writer);

        assert_eq!(result, Err(CreateShortcutNewError::AlreadyExists));
        assert_eq!(fs::read(&final_path).unwrap(), b"existing user shortcut");
        assert!(writer.write_paths.is_empty());
    }
}

#[cfg(target_os = "windows")]
pub fn set_taskbar_visible(hwnd: windows::Win32::Foundation::HWND, visible: bool) {
    os_api::OS::set_taskbar_visible(hwnd, visible);
}

#[cfg(target_os = "windows")]
pub fn restore_and_focus_window(hwnd: windows::Win32::Foundation::HWND) {
    os_api::OS::restore_and_focus(hwnd);
}
