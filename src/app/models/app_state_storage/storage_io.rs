use super::state_path::STATE_FILE_NAME;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const TEMP_FILE_ATTEMPTS: usize = 64;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) trait StateFilesystem {
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn create_dir_all(&mut self, path: &Path) -> io::Result<()>;
    fn path_exists(&self, path: &Path) -> io::Result<bool>;
    fn stage_new_and_sync(&mut self, path: &Path, contents: &[u8]) -> io::Result<()>;
    fn publish_staged(&mut self, staged_path: &Path, destination: &Path) -> io::Result<()>;
    fn remove_file(&mut self, path: &Path) -> io::Result<()>;
    fn copy_file_new_and_sync(&mut self, source: &Path, destination: &Path) -> io::Result<()>;
    fn sync_parent(&mut self, path: &Path) -> io::Result<()>;
}

#[derive(Default)]
pub(super) struct RealStateFilesystem;

impl StateFilesystem for RealStateFilesystem {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn path_exists(&self, path: &Path) -> io::Result<bool> {
        path.try_exists()
    }

    fn stage_new_and_sync(&mut self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(contents)?;
        file.sync_all()
    }

    fn publish_staged(&mut self, staged_path: &Path, destination: &Path) -> io::Result<()> {
        publish_staged_file(staged_path, destination)
    }

    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn copy_file_new_and_sync(&mut self, source: &Path, destination: &Path) -> io::Result<()> {
        let mut source_file = File::open(source)?;
        let mut destination_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;

        if let Err(error) = io::copy(&mut source_file, &mut destination_file) {
            drop(destination_file);
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        if let Err(error) = destination_file.sync_all() {
            drop(destination_file);
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }

    fn sync_parent(&mut self, path: &Path) -> io::Result<()> {
        sync_parent_directory(path)
    }
}

pub(super) fn read_state_file_with_filesystem(
    path: &Path,
    filesystem: &impl StateFilesystem,
) -> Option<String> {
    filesystem.read_to_string(path).ok()
}

pub(super) fn save_to_path_with_filesystem<T: Serialize>(
    value: &T,
    path: &Path,
    filesystem: &mut impl StateFilesystem,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(value)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    filesystem.create_dir_all(parent)?;

    let staged_path = stage_state_file(filesystem, path, json.as_bytes())?;
    if let Err(error) = filesystem.publish_staged(&staged_path, path) {
        let _ = filesystem.remove_file(&staged_path);
        return Err(Box::new(error));
    }

    // Linux needs an explicit directory sync after rename for durable name publication.
    // Windows publishing uses write-through replacement APIs below.
    filesystem.sync_parent(path)?;
    Ok(())
}

pub(super) fn backup_state_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut filesystem = RealStateFilesystem;
    backup_state_file_with_filesystem(path, &mut filesystem)
}

pub(super) fn backup_state_file_with_filesystem(
    path: &Path,
    filesystem: &mut impl StateFilesystem,
) -> Result<(), Box<dyn std::error::Error>> {
    copy_rotated_state_file(path, "old", "", filesystem)
}

pub(super) fn backup_pre_v6_state_file_with_filesystem(
    path: &Path,
    filesystem: &mut impl StateFilesystem,
) -> Result<(), Box<dyn std::error::Error>> {
    copy_rotated_state_file(path, "pre-v6", "-", filesystem)
}

fn copy_rotated_state_file(
    path: &Path,
    suffix: &str,
    counter_separator: &str,
    filesystem: &mut impl StateFilesystem,
) -> Result<(), Box<dyn std::error::Error>> {
    if !filesystem.path_exists(path)? {
        return Ok(());
    }

    let backup_path = rotated_backup_path(path, suffix, counter_separator, filesystem)?;
    filesystem.copy_file_new_and_sync(path, &backup_path)?;
    filesystem.sync_parent(&backup_path)?;
    Ok(())
}

fn stage_state_file(
    filesystem: &mut impl StateFilesystem,
    destination: &Path,
    contents: &[u8],
) -> io::Result<PathBuf> {
    for _ in 0..TEMP_FILE_ATTEMPTS {
        let staged_path = temporary_state_path(destination);
        match filesystem.stage_new_and_sync(&staged_path, contents) {
            Ok(()) => return Ok(staged_path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                let _ = filesystem.remove_file(&staged_path);
                return Err(error);
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique staged state file",
    ))
}

fn temporary_state_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(STATE_FILE_NAME);
    let nonce = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    destination.with_file_name(format!(".{file_name}.tmp-{}-{nonce}", std::process::id()))
}

fn rotated_backup_path(
    path: &Path,
    suffix: &str,
    counter_separator: &str,
    filesystem: &impl StateFilesystem,
) -> Result<PathBuf, io::Error> {
    let mut backup_path = PathBuf::from(path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(STATE_FILE_NAME);

    let mut backup_name = format!("{file_name}.{suffix}");
    backup_path.set_file_name(&backup_name);

    let mut counter = 1;
    while filesystem.path_exists(&backup_path)? {
        backup_name = format!("{file_name}.{suffix}{counter_separator}{counter}");
        backup_path.set_file_name(&backup_name);
        counter += 1;
    }

    Ok(backup_path)
}

#[cfg(target_os = "windows")]
fn publish_staged_file(staged_path: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH, REPLACEFILE_WRITE_THROUGH,
    };

    let staged_wide = staged_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    let result = if destination.try_exists()? {
        // ReplaceFileW replaces an existing destination without the delete-then-rename
        // window that std::fs::rename has on Windows.
        unsafe {
            ReplaceFileW(
                PCWSTR(destination_wide.as_ptr()),
                PCWSTR(staged_wide.as_ptr()),
                PCWSTR::null(),
                REPLACEFILE_WRITE_THROUGH,
                None,
                None,
            )
        }
    } else {
        unsafe {
            MoveFileExW(
                PCWSTR(staged_wide.as_ptr()),
                PCWSTR(destination_wide.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
    };

    result.map_err(|_| io::Error::last_os_error())
}

#[cfg(not(target_os = "windows"))]
fn publish_staged_file(staged_path: &Path, destination: &Path) -> io::Result<()> {
    // The staged file is in the destination directory, so rename is an atomic same-filesystem
    // replacement on the Linux beta path.
    fs::rename(staged_path, destination)
}

#[cfg(target_os = "windows")]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::{
        backup_pre_v6_state_file_with_filesystem, backup_state_file_with_filesystem,
        save_to_path_with_filesystem, StateFilesystem,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct FakeFilesystem {
        files: BTreeMap<PathBuf, Vec<u8>>,
        staged_paths: Vec<PathBuf>,
        fail_stage: bool,
        fail_publish: bool,
        fail_copy: bool,
    }

    impl FakeFilesystem {
        fn write_file(&mut self, path: &Path, contents: impl AsRef<[u8]>) {
            self.files
                .insert(path.to_path_buf(), contents.as_ref().to_vec());
        }

        fn read_file(&self, path: &Path) -> String {
            String::from_utf8(self.files[path].clone()).unwrap()
        }
    }

    impl StateFilesystem for FakeFilesystem {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.files
                .get(path)
                .map(|contents| String::from_utf8(contents.clone()).unwrap())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing file"))
        }

        fn create_dir_all(&mut self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn path_exists(&self, path: &Path) -> io::Result<bool> {
            Ok(self.files.contains_key(path))
        }

        fn stage_new_and_sync(&mut self, path: &Path, contents: &[u8]) -> io::Result<()> {
            self.staged_paths.push(path.to_path_buf());
            if self.fail_stage {
                self.files.insert(path.to_path_buf(), b"partial".to_vec());
                return Err(io::Error::other("simulated staging failure"));
            }
            if self.files.contains_key(path) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "staged path exists",
                ));
            }
            self.files.insert(path.to_path_buf(), contents.to_vec());
            Ok(())
        }

        fn publish_staged(&mut self, staged_path: &Path, destination: &Path) -> io::Result<()> {
            if self.fail_publish {
                return Err(io::Error::other("simulated publish failure"));
            }
            let contents = self
                .files
                .remove(staged_path)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing staged file"))?;
            self.files.insert(destination.to_path_buf(), contents);
            Ok(())
        }

        fn remove_file(&mut self, path: &Path) -> io::Result<()> {
            self.files.remove(path);
            Ok(())
        }

        fn copy_file_new_and_sync(&mut self, source: &Path, destination: &Path) -> io::Result<()> {
            if self.fail_copy {
                return Err(io::Error::other("simulated backup failure"));
            }
            if self.files.contains_key(destination) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "backup path exists",
                ));
            }
            let contents =
                self.files.get(source).cloned().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "missing source file")
                })?;
            self.files.insert(destination.to_path_buf(), contents);
            Ok(())
        }

        fn sync_parent(&mut self, _path: &Path) -> io::Result<()> {
            Ok(())
        }
    }

    fn state_path() -> PathBuf {
        PathBuf::from("state/state.json")
    }

    #[test]
    fn staging_failure_preserves_existing_state_and_removes_partial_temp_file() {
        let path = state_path();
        let mut filesystem = FakeFilesystem {
            fail_stage: true,
            ..Default::default()
        };
        filesystem.write_file(&path, "original state");

        let result = save_to_path_with_filesystem(&json!({ "new": true }), &path, &mut filesystem);

        assert!(result.is_err());
        assert_eq!(filesystem.read_file(&path), "original state");
        assert_eq!(filesystem.staged_paths.len(), 1);
        assert_eq!(filesystem.staged_paths[0].parent(), path.parent());
        assert!(!filesystem.files.contains_key(&filesystem.staged_paths[0]));
    }

    #[test]
    fn publish_failure_preserves_existing_state_and_removes_staged_file() {
        let path = state_path();
        let mut filesystem = FakeFilesystem {
            fail_publish: true,
            ..Default::default()
        };
        filesystem.write_file(&path, "original state");

        let result = save_to_path_with_filesystem(&json!({ "new": true }), &path, &mut filesystem);

        assert!(result.is_err());
        assert_eq!(filesystem.read_file(&path), "original state");
        assert_eq!(filesystem.staged_paths.len(), 1);
        assert!(!filesystem.files.contains_key(&filesystem.staged_paths[0]));
    }

    #[test]
    fn corrupt_state_backup_copies_instead_of_moving_the_original() {
        let path = state_path();
        let mut filesystem = FakeFilesystem::default();
        filesystem.write_file(&path, "{broken json");

        backup_state_file_with_filesystem(&path, &mut filesystem).unwrap();

        assert_eq!(filesystem.read_file(&path), "{broken json");
        assert_eq!(
            filesystem.read_file(&path.with_file_name("state.json.old")),
            "{broken json"
        );
    }

    #[test]
    fn pre_v6_backup_failure_leaves_original_for_a_later_retry() {
        let path = state_path();
        let mut filesystem = FakeFilesystem {
            fail_copy: true,
            ..Default::default()
        };
        filesystem.write_file(&path, "legacy state");

        assert!(backup_pre_v6_state_file_with_filesystem(&path, &mut filesystem).is_err());
        assert_eq!(filesystem.read_file(&path), "legacy state");
        assert!(!filesystem
            .files
            .contains_key(&path.with_file_name("state.json.pre-v6")));

        filesystem.fail_copy = false;
        backup_pre_v6_state_file_with_filesystem(&path, &mut filesystem).unwrap();

        assert_eq!(filesystem.read_file(&path), "legacy state");
        assert_eq!(
            filesystem.read_file(&path.with_file_name("state.json.pre-v6")),
            "legacy state"
        );
    }

    #[test]
    fn corrupt_recovery_preserves_original_when_backup_or_default_publish_fails() {
        let path = state_path();
        let mut backup_failure = FakeFilesystem {
            fail_copy: true,
            ..Default::default()
        };
        backup_failure.write_file(&path, "{broken json");

        let recovered = super::super::AppStateStorage::load_from_path_with_filesystem(
            &path,
            &mut backup_failure,
        );

        assert_eq!(recovered.version, super::super::CURRENT_APP_STATE_VERSION);
        assert_eq!(backup_failure.read_file(&path), "{broken json");
        assert!(!backup_failure
            .files
            .contains_key(&path.with_file_name("state.json.old")));

        let mut publish_failure = FakeFilesystem {
            fail_publish: true,
            ..Default::default()
        };
        publish_failure.write_file(&path, "{broken json");

        let recovered = super::super::AppStateStorage::load_from_path_with_filesystem(
            &path,
            &mut publish_failure,
        );

        assert_eq!(recovered.version, super::super::CURRENT_APP_STATE_VERSION);
        assert_eq!(publish_failure.read_file(&path), "{broken json");
        assert_eq!(
            publish_failure.read_file(&path.with_file_name("state.json.old")),
            "{broken json"
        );
    }

    #[test]
    fn pre_v6_save_retries_the_backup_after_a_failure() {
        let path = state_path();
        let mut state = super::super::schema_refresh::build_default_state();
        state.pending_pre_v6_backup = true;
        let mut filesystem = FakeFilesystem {
            fail_copy: true,
            ..Default::default()
        };
        filesystem.write_file(&path, "legacy state");

        assert!(state
            .try_save_to_path_with_filesystem(&path, &mut filesystem)
            .is_err());
        assert!(state.pending_pre_v6_backup);
        assert_eq!(filesystem.read_file(&path), "legacy state");

        filesystem.fail_copy = false;
        state
            .try_save_to_path_with_filesystem(&path, &mut filesystem)
            .unwrap();

        assert!(!state.pending_pre_v6_backup);
        assert_eq!(
            filesystem.read_file(&path.with_file_name("state.json.pre-v6")),
            "legacy state"
        );
    }
}
