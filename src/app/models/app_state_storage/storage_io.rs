use super::state_path::STATE_FILE_NAME;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const TEMP_FILE_ATTEMPTS: usize = 64;
pub(super) const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) enum StateRead {
    Text(String),
    Oversized { size: usize },
}
pub(crate) enum StateQualification {
    Unavailable,
    #[cfg(any(windows, test))]
    Blocked,
    #[cfg(windows)]
    Receipt(os_api::tuning::TuningConfigReceipt),
}
impl StateQualification {
    pub(super) fn is_blocked(&self) -> bool {
        #[cfg(any(windows, test))]
        if matches!(self, Self::Blocked) {
            return true;
        }
        false
    }
}
pub(crate) enum StatePublication {
    Unavailable,
    #[cfg(windows)]
    Receipt(os_api::tuning::TuningConfigReceipt),
}
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) trait StateFilesystem {
    fn qualify_read(&self, _path: &Path) -> StateQualification {
        StateQualification::Unavailable
    }
    fn prepare_save(
        &mut self,
        path: &Path,
        #[cfg(windows)] expected: Option<&os_api::tuning::TuningConfigReceipt>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.qualify_read(path).is_blocked() {
            return Err("qualified state evidence is blocked; save refused".into());
        }
        #[cfg(windows)]
        if expected.is_some() {
            return Err("qualified state source unavailable; save refused".into());
        }
        Ok(())
    }
    fn publish_qualified(
        &mut self,
        _path: &Path,
        _bytes: &[u8],
        #[cfg(windows)] _expected: Option<&os_api::tuning::TuningConfigReceipt>,
    ) -> Result<StatePublication, Box<dyn std::error::Error>> {
        Ok(StatePublication::Unavailable)
    }
    fn read_bounded(&self, path: &Path) -> io::Result<StateRead>;
    fn create_dir_all(&mut self, path: &Path) -> io::Result<()>;
    fn path_exists(&self, path: &Path) -> io::Result<bool>;
    fn stage_new_and_sync(&mut self, path: &Path, contents: &[u8]) -> io::Result<()>;
    fn publish_staged(&mut self, staged_path: &Path, destination: &Path) -> io::Result<()>;
    fn remove_file(&mut self, path: &Path) -> io::Result<()>;
    fn copy_file_new_and_sync(&mut self, source: &Path, destination: &Path) -> io::Result<()>;
    fn sync_parent(&mut self, path: &Path) -> io::Result<()>;
}

#[derive(Default)]
pub(super) struct RealStateFilesystem {
    // This guard belongs to one synchronous load/save call, never shared state.
    // Its no-write/no-delete leaf stays open across backup and until the native
    // source releases it at its checked atomic publication boundary.
    #[cfg(windows)]
    source: std::cell::RefCell<Option<(PathBuf, os_api::tuning::TuningConfigSource)>>,
    #[cfg(windows)]
    unavailable: std::cell::Cell<bool>,
    #[cfg(windows)]
    blocked: std::cell::Cell<bool>,
}

impl StateFilesystem for RealStateFilesystem {
    #[cfg(windows)]
    fn qualify_read(&self, path: &Path) -> StateQualification {
        use os_api::tuning::TuningConfigSource;
        if self.blocked.get() {
            return StateQualification::Blocked;
        }
        let mut retained = self.source.borrow_mut();
        if retained.is_none() {
            if self.unavailable.get() {
                return StateQualification::Unavailable;
            }
            match TuningConfigSource::open(path) {
                Ok(source) => *retained = Some((path.to_path_buf(), source)),
                Err(error) if unavailable_source(&error) => {
                    self.unavailable.set(true);
                    return StateQualification::Unavailable;
                }
                Err(_) => {
                    self.blocked.set(true);
                    return StateQualification::Blocked;
                }
            }
        }
        let (selected, source) = retained.as_mut().expect("retained source");
        if selected != path {
            self.blocked.set(true);
            return StateQualification::Blocked;
        }
        match source.read() {
            Ok(Some(receipt)) => StateQualification::Receipt(receipt),
            Ok(None) => StateQualification::Unavailable,
            Err(_) => {
                self.blocked.set(true);
                StateQualification::Blocked
            }
        }
    }
    #[cfg(windows)]
    fn prepare_save(
        &mut self,
        path: &Path,
        expected: Option<&os_api::tuning::TuningConfigReceipt>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let current = match self.qualify_read(path) {
            StateQualification::Receipt(receipt) => Some(receipt),
            StateQualification::Unavailable => None,
            StateQualification::Blocked => {
                return Err("qualified state evidence is blocked; save refused".into());
            }
        };
        if expected.is_some() && current.as_ref() != expected {
            self.blocked.set(true);
            return Err("qualified state revision changed; save refused".into());
        }
        // The same source/leaf remains retained through every backup and publish.
        Ok(())
    }
    #[cfg(windows)]
    fn publish_qualified(
        &mut self,
        path: &Path,
        bytes: &[u8],
        expected: Option<&os_api::tuning::TuningConfigReceipt>,
    ) -> Result<StatePublication, Box<dyn std::error::Error>> {
        let current = match self.qualify_read(path) {
            StateQualification::Receipt(receipt) => Some(receipt),
            StateQualification::Unavailable => None,
            StateQualification::Blocked => {
                return Err("qualified state evidence is blocked; publication refused".into());
            }
        };
        if expected.is_some() && current.as_ref() != expected {
            self.blocked.set(true);
            return Err("qualified state revision changed; save refused".into());
        }
        let mut retained = self.source.borrow_mut();
        let Some((_, source)) = retained.as_mut() else {
            return Ok(StatePublication::Unavailable);
        };
        match source.publish(current.as_ref(), bytes) {
            Ok(receipt) => Ok(StatePublication::Receipt(receipt)),
            Err(error) => {
                self.blocked.set(true);
                Err(Box::new(error))
            }
        }
    }
    fn read_bounded(&self, path: &Path) -> io::Result<StateRead> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take((MAX_STATE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Ok(StateRead::Oversized { size: bytes.len() });
        }
        String::from_utf8(bytes)
            .map(StateRead::Text)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "state UTF-8"))
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

#[cfg(windows)]
fn unavailable_source(error: &os_api::tuning::TuningNativeError) -> bool {
    use os_api::tuning::TuningNativeError as Native;
    matches!(
        error,
        Native::Unsupported(_)
            | Native::UnsafeStore(_)
            | Native::Denied { .. }
            | Native::Io { code: 2 | 3, .. }
    )
}

pub(super) fn read_state_file_with_filesystem(
    path: &Path,
    filesystem: &impl StateFilesystem,
) -> io::Result<StateRead> {
    filesystem.read_bounded(path)
}

#[cfg(test)]
pub(super) fn save_to_path_with_filesystem<T: Serialize>(
    value: &T,
    path: &Path,
    filesystem: &mut impl StateFilesystem,
) -> Result<(), Box<dyn std::error::Error>> {
    let serialized = serialize_state_bounded(value)?;
    publish_state_bytes_with_filesystem(&serialized, path, filesystem)
}

pub(super) fn serialize_state_bounded<T: Serialize>(
    value: &T,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    struct BoundedWriter(Vec<u8>);
    impl Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self
                .0
                .len()
                .checked_add(bytes.len())
                .is_none_or(|size| size > MAX_STATE_BYTES)
            {
                return Err(io::Error::other("serialized state exceeds read limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = BoundedWriter(Vec::new());
    serde_json::to_writer_pretty(&mut output, value)?;
    Ok(output.0)
}

pub(super) fn publish_state_bytes_with_filesystem(
    serialized: &[u8],
    path: &Path,
    filesystem: &mut impl StateFilesystem,
) -> Result<(), Box<dyn std::error::Error>> {
    if serialized.len() > MAX_STATE_BYTES {
        return Err("serialized state exceeds read limit".into());
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    filesystem.create_dir_all(parent)?;

    let staged_path = stage_state_file(filesystem, path, serialized)?;
    if let Err(error) = filesystem.publish_staged(&staged_path, path) {
        let _ = filesystem.remove_file(&staged_path);
        return Err(Box::new(error));
    }

    // Linux needs an explicit directory sync after rename for durable name publication.
    // The Windows path has already synchronized staged file contents; its parent-directory
    // sync is intentionally a no-op because Rust cannot portably open directories there.
    filesystem.sync_parent(path)?;
    Ok(())
}

#[cfg(test)]
pub(super) fn backup_state_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut filesystem = RealStateFilesystem::default();
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
        MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH, REPLACE_FILE_FLAGS,
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
                // REPLACEFILE_WRITE_THROUGH is documented as unsupported. The staged
                // file contents were synchronized before this atomic replacement.
                REPLACE_FILE_FLAGS(0),
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
        mutation_count: usize,
        fail_stage: bool,
        fail_publish: bool,
        fail_copy: bool,
        blocked_qualification: bool,
        fail_qualified_publish: bool,
        qualified_publishes: usize,
        fail_save_guard: bool,
        save_guards: usize,
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
        fn prepare_save(
            &mut self,
            _path: &Path,
            #[cfg(windows)] _expected: Option<&os_api::tuning::TuningConfigReceipt>,
        ) -> Result<(), Box<dyn std::error::Error>> {
            self.save_guards += 1;
            if self.fail_save_guard || self.blocked_qualification {
                Err("original qualified revision changed".into())
            } else {
                Ok(())
            }
        }
        fn publish_qualified(
            &mut self,
            _path: &Path,
            _bytes: &[u8],
            #[cfg(windows)] _expected: Option<&os_api::tuning::TuningConfigReceipt>,
        ) -> Result<super::StatePublication, Box<dyn std::error::Error>> {
            self.qualified_publishes += 1;
            if self.fail_qualified_publish {
                Err("uncertain qualified publication".into())
            } else {
                Ok(super::StatePublication::Unavailable)
            }
        }
        fn qualify_read(&self, _path: &Path) -> super::StateQualification {
            if self.blocked_qualification {
                super::StateQualification::Blocked
            } else {
                super::StateQualification::Unavailable
            }
        }
        fn read_bounded(&self, path: &Path) -> io::Result<super::StateRead> {
            self.files
                .get(path)
                .map(|contents| {
                    if contents.len() > super::MAX_STATE_BYTES {
                        super::StateRead::Oversized {
                            size: super::MAX_STATE_BYTES + 1,
                        }
                    } else {
                        super::StateRead::Text(String::from_utf8(contents.clone()).unwrap())
                    }
                })
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing file"))
        }

        fn create_dir_all(&mut self, _path: &Path) -> io::Result<()> {
            self.mutation_count += 1;
            Ok(())
        }

        fn path_exists(&self, path: &Path) -> io::Result<bool> {
            Ok(self.files.contains_key(path))
        }

        fn stage_new_and_sync(&mut self, path: &Path, contents: &[u8]) -> io::Result<()> {
            self.mutation_count += 1;
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
            self.mutation_count += 1;
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
            self.mutation_count += 1;
            self.files.remove(path);
            Ok(())
        }

        fn copy_file_new_and_sync(&mut self, source: &Path, destination: &Path) -> io::Result<()> {
            self.mutation_count += 1;
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
            self.mutation_count += 1;
            Ok(())
        }
    }

    fn state_path() -> PathBuf {
        PathBuf::from("state/state.json")
    }

    fn p1_state(version: &str, tuning: Option<&str>, extension: Option<&str>) -> String {
        let base = super::super::schema_refresh::build_default_state();
        let mut value = serde_json::to_value(base).unwrap();
        value["version"] = json!(11);
        let mut text = serde_json::to_string(&value).unwrap();
        text = text.replace("\"version\":11", &format!("\"version\":{version}"));
        text.pop();
        if let Some(tuning) = tuning {
            text.push_str(&format!(",\"tuning\":{tuning}"));
        }
        if let Some(extension) = extension {
            text.push_str(&format!(",\"future_state\":{extension}"));
        }
        text.push('}');
        text
    }
    fn p1_config() -> String {
        serde_json::to_string(&json!({"version":1,"lineage":vec![1;32],"profiles":[],"associations":[],"allocations":{"next_profile":1,"next_owner":1,"next_config_generation":1},"activations":[],"withdrawals":[]})).unwrap()
    }
    // These exercise the production recovery/order seam with refusal results.
    // The fake cannot construct a native receipt, proof or successful publication.
    #[test]
    fn p1b_qualified_corrupt_recovery_refuses_replaced_source_before_backup() {
        let path = state_path();
        let replacement = p1_state("11", Some(&p1_config()), None);
        let mut fs = FakeFilesystem {
            fail_save_guard: true,
            ..Default::default()
        };
        fs.write_file(&path, &replacement);
        let state = super::super::AppStateStorage::recover_corrupt_state(
            super::super::schema_refresh::build_default_state(),
            &path,
            &mut fs,
            true,
        );
        assert!(state.save_blocked());
        assert_eq!(fs.mutation_count, 0);
        assert_eq!(fs.qualified_publishes, 0);
        assert_eq!(fs.read_file(&path), replacement);
        assert_eq!(fs.files.len(), 1);
        let mut clone = state.clone();
        clone.mark_ready_for_current_schema_save(
            crate::app::features::rules::PersistedRuleIdentities::default(),
        );
        assert!(clone
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .is_err());
        assert_eq!(fs.save_guards, 1);
        assert_eq!(fs.mutation_count, 0);
    }

    #[test]
    fn p1b_qualified_corrupt_recovery_uncertainty_is_sticky_without_fallback() {
        let path = state_path();
        let original = "{broken json";
        let mut fs = FakeFilesystem {
            fail_qualified_publish: true,
            ..Default::default()
        };
        fs.write_file(&path, original);
        let state = super::super::AppStateStorage::recover_corrupt_state(
            super::super::schema_refresh::build_default_state(),
            &path,
            &mut fs,
            true,
        );
        assert!(state.save_blocked());
        assert_eq!(fs.read_file(&path), original);
        assert_eq!(
            fs.read_file(&path.with_file_name("state.json.old")),
            original
        );
        assert_eq!(fs.qualified_publishes, 1);
        assert_eq!(fs.save_guards, 1);
        assert_eq!(fs.mutation_count, 2); // Only the original backup and its sync.
        assert!(fs.staged_paths.is_empty());
        let mut clone = state.clone();
        clone.mark_ready_for_current_schema_save(
            crate::app::features::rules::PersistedRuleIdentities::default(),
        );
        assert!(clone
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .is_err());
        assert_eq!(fs.qualified_publishes, 1);
        assert_eq!(fs.save_guards, 1);
    }

    #[test]
    fn p1b_pre_v6_revision_refusal_precedes_every_backup_mutation() {
        let path = state_path();
        let replacement = p1_state("11", Some(&p1_config()), None);
        let mut state = super::super::schema_refresh::build_default_state();
        state.pending_pre_v6_backup = true;
        let mut fs = FakeFilesystem {
            fail_save_guard: true,
            fail_qualified_publish: true,
            ..Default::default()
        };
        fs.write_file(&path, &replacement);
        assert!(state
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .is_err());
        assert!(state.save_blocked());
        assert!(state.pending_pre_v6_backup);
        assert_eq!(fs.mutation_count, 0);
        assert_eq!(fs.qualified_publishes, 0);
        assert_eq!(fs.read_file(&path), replacement);
        assert_eq!(fs.files.len(), 1);
        assert!(fs.staged_paths.is_empty());
    }
    #[test]
    fn p1b_poisoned_qualification_precedes_missing_leaf_recovery_and_is_sticky() {
        let path = state_path();
        let mut fs = FakeFilesystem {
            blocked_qualification: true,
            ..Default::default()
        };
        let state = super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert!(state.save_blocked());
        let mut cloned = state.clone();
        cloned.mark_ready_for_current_schema_save(
            crate::app::features::rules::PersistedRuleIdentities::default(),
        );
        assert!(cloned
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .is_err());
        assert_eq!(fs.mutation_count, 0);
        assert!(fs.staged_paths.is_empty());
        assert!(fs.files.is_empty());
    }
    #[test]
    fn p1b_saved_byte_parser_cannot_promote_legacy_or_unknown_configuration() {
        use crate::app::features::tuning::platform::configured_intent::saved_configuration;
        let known = p1_config();
        assert!(saved_configuration(p1_state("11", Some(&known), None).as_bytes()).is_some());
        for version in ["10", "12", "20"] {
            assert!(
                saved_configuration(p1_state(version, Some(&known), None).as_bytes()).is_none(),
                "{version}"
            );
        }
        assert!(saved_configuration(p1_state("11", None, None).as_bytes()).is_none());
        let mut unknown: serde_json::Value = serde_json::from_str(&known).unwrap();
        unknown["future"] = json!(true);
        assert!(
            saved_configuration(p1_state("11", Some(&unknown.to_string()), None).as_bytes())
                .is_none()
        );
    }
    #[test]
    fn p1b_uncertain_qualified_publication_never_uses_ordinary_cleanup_or_retry() {
        let path = state_path();
        let original = p1_state("11", Some(&p1_config()), None);
        let mut fs = FakeFilesystem {
            fail_qualified_publish: true,
            ..Default::default()
        };
        fs.write_file(&path, &original);
        let mut state =
            super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        state.theme_index = 2;
        assert!(state
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .is_err());
        assert!(state.save_blocked());
        let mut cloned = state.clone();
        cloned.mark_ready_for_current_schema_save(
            crate::app::features::rules::PersistedRuleIdentities::default(),
        );
        assert!(cloned
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .is_err());
        assert_eq!(fs.qualified_publishes, 1);
        assert_eq!(fs.mutation_count, 0);
        assert!(fs.staged_paths.is_empty());
        assert_eq!(fs.read_file(&path), original);
    }
    #[cfg(windows)]
    #[test]
    fn p1b_wire_or_fixture_storage_never_mints_native_receipt_proof() {
        fn immutable_send_sync<T: Clone + Send + Sync>() {}
        immutable_send_sync::<
            crate::app::features::tuning::platform::configured_intent::NativeConfigurationProof,
        >();
        let path = state_path();
        let mut fs = FakeFilesystem::default();
        fs.write_file(&path, p1_state("11", Some(&p1_config()), None));
        let state = super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert!(state.native_configuration().is_none());
        let text = serde_json::to_string(&state).unwrap();
        let mut cloned: super::super::AppStateStorage = serde_json::from_str(&text).unwrap();
        cloned.theme_index = 2;
        assert!(cloned.native_configuration().is_none());
        assert!(!text.contains("receipt") && !text.contains("volume_guid"));
    }
    fn assert_blocked_original_without_mutation(original: &str) {
        let path = state_path();
        let mut fs = FakeFilesystem::default();
        fs.write_file(&path, original);
        let state = super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert!(
            state.save_blocked(),
            "preserved evidence must block every save"
        );
        assert_eq!(state.preserved_evidence(), Some(original));
        assert_eq!(fs.read_file(&path), original);
        assert_eq!(fs.files.len(), 1);
        assert_eq!(fs.mutation_count, 0);
        let mut cloned = state.clone();
        cloned.mark_ready_for_current_schema_save(
            crate::app::features::rules::RulesContext::from_storage(&cloned)
                .to_persisted_identities(),
        );
        assert!(cloned
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .is_err());
        assert_eq!(cloned.preserved_evidence(), Some(original));
        assert_eq!(fs.read_file(&path), original);
        assert_eq!(fs.files.len(), 1);
        assert_eq!(fs.mutation_count, 0);
    }

    #[test]
    fn p1_storage_tuning_evidence_survives_base_decode_failure_and_duplicate_tuning() {
        for tuning in ["{}".to_owned(), p1_config()] {
            let original = p1_state("11", Some(&tuning), None)
                .replace("\"theme_index\":0", "\"theme_index\":null");
            assert_blocked_original_without_mutation(&original);
        }
        let mut duplicate = p1_state("11", Some(&p1_config()), None);
        duplicate.pop();
        duplicate.push_str(",\"tuning\":{}}");
        assert_blocked_original_without_mutation(&duplicate);
    }

    #[test]
    fn p1_storage_duplicate_top_extensions_are_blocked_without_rewrite() {
        for duplicate_key in ["future_state", "future_\\u0073tate"] {
            let mut original = p1_state("11", None, Some("{\"first\":1e+2}"));
            original.pop();
            original.push_str(&format!(",\"{duplicate_key}\":{{\"second\":2}}}}"));
            assert_blocked_original_without_mutation(&original);
        }
    }

    #[test]
    fn p1_storage_output_bound_precedes_upgrade_backup_and_all_publication() {
        for version in ["5", "11"] {
            let path = state_path();
            let mut value: serde_json::Value =
                serde_json::from_str(&p1_state(version, None, None)).unwrap();
            value["groups"] = json!([{
                "name": "", "cores": [0, 1], "programs": [],
                "is_hidden": false, "run_all_button": true
            }]);
            let compact = serde_json::to_string(&value).unwrap();
            value["groups"][0]["name"] =
                json!("x".repeat(super::MAX_STATE_BYTES - compact.len() - 1));
            let original = serde_json::to_string(&value).unwrap();
            assert_eq!(original.len(), super::MAX_STATE_BYTES - 1);
            let mut fs = FakeFilesystem::default();
            fs.write_file(&path, &original);
            let mut state =
                super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
            assert!(!state.save_blocked());
            assert_eq!(fs.mutation_count, 0);
            state.mark_ready_for_current_schema_save(
                crate::app::features::rules::RulesContext::from_storage(&state)
                    .to_persisted_identities(),
            );
            assert!(serde_json::to_string_pretty(&state).unwrap().len() > super::MAX_STATE_BYTES);
            assert!(
                state
                    .try_save_to_path_with_filesystem(&path, &mut fs)
                    .is_err(),
                "cannot publish state that the bounded reader rejects"
            );
            assert_eq!(fs.mutation_count, 0);
            assert_eq!(fs.read_file(&path), original);
            assert_eq!(fs.files.len(), 1);
            assert!(fs.staged_paths.is_empty());
            assert_eq!(state.pending_pre_v6_backup, version == "5");
        }
    }
    #[test]
    fn p1_storage_v11_round_trip_uses_real_save_seam_and_old_missing_tuning_stays_off() {
        use crate::app::features::tuning::config::TuningLoad;
        let path = state_path();
        let original = p1_state("11", Some(&p1_config()), None);
        let mut fs = FakeFilesystem::default();
        fs.write_file(&path, &original);
        let mut state =
            super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert!(matches!(state.tuning.decode(), TuningLoad::Known(_)));
        assert_eq!(state.loaded_version, 11);
        assert_eq!(fs.read_file(&path), original);
        state.theme_index = 2;
        state
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .unwrap();
        assert_eq!(fs.staged_paths.len(), 1);
        let state = super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert!(matches!(state.tuning.decode(), TuningLoad::Known(_)));
        for version in ["3", "6", "9", "10"] {
            let raw = p1_state(version, None, None);
            let mut fs = FakeFilesystem::default();
            fs.write_file(&path, &raw);
            let mut state =
                super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
            assert_eq!(state.tuning.decode(), TuningLoad::Off);
            assert_eq!(fs.read_file(&path), raw);
            state.mark_ready_for_current_schema_save(
                crate::app::features::rules::RulesContext::from_storage(&state)
                    .to_persisted_identities(),
            );
            state
                .try_save_to_path_with_filesystem(&path, &mut fs)
                .unwrap();
            let saved = fs.read_file(&path);
            assert!(serde_json::from_str::<serde_json::Value>(&saved)
                .unwrap()
                .get("tuning")
                .is_none());
            assert_eq!(
                super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs)
                    .tuning
                    .decode(),
                TuningLoad::Off
            );
        }
    }
    #[test]
    fn p1_storage_unknown_raw_envelope_and_top_extensions_survive_unrelated_save_exactly() {
        let path = state_path();
        let tuning = "{ \"version\":99, \"profile\":{\"future\":1e+2,\"text\":\"\\u0061\"} }";
        let extension = "{ \"n\":1e+2, \"escape\":\"\\u0062\" }";
        let mut fs = FakeFilesystem::default();
        fs.write_file(&path, p1_state("11", Some(tuning), Some(extension)));
        let mut state =
            super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert_eq!(
            state.tuning.decode(),
            crate::app::features::tuning::config::TuningLoad::Unknown
        );
        state.theme_index = 1;
        state
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .unwrap();
        let saved = fs.read_file(&path);
        assert!(saved.contains(tuning), "unknown tuning bytes preserved");
        assert!(
            saved.contains(extension),
            "top-level extension bytes preserved"
        );
        let state = super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert_eq!(state.tuning.raw(), Some(tuning));
    }
    #[test]
    fn p1_storage_future_and_malformed_explicit_version_never_recover_or_publish() {
        let path = state_path();
        for version in [
            "12",
            "4294967296",
            "18446744073709551615",
            "null",
            "\"11\"",
            "-1",
            "11.5",
            "{}",
        ] {
            let original = p1_state(version, None, None);
            let mut fs = FakeFilesystem::default();
            fs.write_file(&path, &original);
            let state =
                super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
            assert!(
                state.save_blocked(),
                "explicit unsupported version is blocked"
            );
            assert_eq!(state.preserved_evidence(), Some(original.as_str()));
            assert_eq!(fs.files.len(), 1);
            assert!(fs.staged_paths.is_empty());
            let mut cloned = state.clone();
            cloned.theme_index = 2;
            cloned.mark_ready_for_current_schema_save(
                crate::app::features::rules::RulesContext::from_storage(&cloned)
                    .to_persisted_identities(),
            );
            assert!(cloned
                .try_save_to_path_with_filesystem(&path, &mut fs)
                .is_err());
            assert_eq!(fs.read_file(&path), original);
            assert_eq!(fs.files.len(), 1);
            assert!(fs.staged_paths.is_empty());
        }
    }
    #[test]
    fn p1_storage_malformed_or_oversized_tuning_retains_evidence_and_refuses_save() {
        let path = state_path();
        for tuning in [
            "true".to_owned(),
            "{}".to_owned(),
            format!(
                "{{\"version\":99,\"data\":\"{}\"}}",
                "x".repeat(4 * 1024 * 1024)
            ),
        ] {
            let original = p1_state("11", Some(&tuning), None);
            let mut fs = FakeFilesystem::default();
            fs.write_file(&path, &original);
            let mut state =
                super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
            assert!(state.save_blocked());
            assert_eq!(state.tuning.raw(), Some(tuning.as_str()));
            assert!(state
                .try_save_to_path_with_filesystem(&path, &mut fs)
                .is_err());
            assert_eq!(fs.read_file(&path), original);
            assert_eq!(fs.files.len(), 1);
            assert!(fs.staged_paths.is_empty());
        }
    }
    #[test]
    fn p1_storage_old_tuning_extension_cannot_activate_after_upgrade() {
        let path = state_path();
        let original = p1_state("10", Some(&p1_config()), None);
        let mut fs = FakeFilesystem::default();
        fs.write_file(&path, &original);
        let mut state =
            super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert!(state.save_blocked());
        state.mark_ready_for_current_schema_save(
            crate::app::features::rules::RulesContext::from_storage(&state)
                .to_persisted_identities(),
        );
        assert!(state
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .is_err());
        assert_eq!(fs.read_file(&path), original);
        assert!(fs.staged_paths.is_empty());
    }
    #[test]
    fn p1_storage_whole_state_bound_retains_disk_without_recovery_or_large_memory_copy() {
        let path = state_path();
        let raw = format!(
            "{{\"version\":18446744073709551615,\"data\":\"{}\"}}",
            "x".repeat(16 * 1024 * 1024)
        );
        let mut fs = FakeFilesystem::default();
        fs.write_file(&path, &raw);
        let mut state =
            super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);
        assert!(state.save_blocked());
        assert!(
            state.preserved_evidence().is_none(),
            "oversize retains disk plus finite size only"
        );
        let refusal = state
            .try_save_to_path_with_filesystem(&path, &mut fs)
            .unwrap_err()
            .to_string();
        assert_eq!(
            refusal,
            format!(
                "state evidence exceeds read limit (at least {} bytes); save refused",
                super::MAX_STATE_BYTES + 1
            )
        );
        assert_eq!(fs.files.len(), 1);
        assert_eq!(fs.read_file(&path), raw);
        assert!(fs.staged_paths.is_empty());
    }

    #[test]
    fn p1_storage_duplicate_explicit_versions_retain_evidence_without_recovery() {
        let path = state_path();
        for versions in [("20", "2"), ("null", "10"), ("10", "11"), ("10", "10")] {
            let mut original = p1_state(versions.0, None, None);
            original.pop();
            original.push_str(&format!(",\"version\":{}}}", versions.1));
            let mut fs = FakeFilesystem::default();
            fs.write_file(&path, &original);
            let state =
                super::super::AppStateStorage::load_from_path_with_filesystem(&path, &mut fs);

            assert!(state.save_blocked(), "ambiguous version header is blocked");
            assert_eq!(state.preserved_evidence(), Some(original.as_str()));
            assert_eq!(fs.read_file(&path), original);
            assert_eq!(fs.files.len(), 1);
            assert!(fs.staged_paths.is_empty());

            let mut cloned = state.clone();
            cloned.mark_ready_for_current_schema_save(
                crate::app::features::rules::RulesContext::from_storage(&cloned)
                    .to_persisted_identities(),
            );
            assert!(cloned
                .try_save_to_path_with_filesystem(&path, &mut fs)
                .is_err());
            assert_eq!(fs.read_file(&path), original);
            assert_eq!(fs.files.len(), 1);
            assert!(fs.staged_paths.is_empty());
        }
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
        assert!(!recovered.save_blocked());
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
        assert!(!recovered.save_blocked());
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
