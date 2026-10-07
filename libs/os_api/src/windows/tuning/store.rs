use super::{
    identity, native_error,
    security::{Descriptor, SecurityPolicy},
    wide,
};
use crate::tuning::{MAX_TUNING_JOURNAL_BYTES, TuningNativeError, TuningStoreAuthority};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Write},
    marker::PhantomData,
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::{Component, Path, PathBuf},
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW, DELETE,
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_NONE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, FileDispositionInfo,
            GetFileInformationByHandle, GetFileType, GetVolumeInformationByHandleW,
            MOVE_FILE_FLAGS, MoveFileExW, OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL,
            REPLACE_FILE_FLAGS, ReplaceFileW, SetFileInformationByHandle,
        },
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_ProgramData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    },
    core::PCWSTR,
};

pub const JOURNAL_NAME: &str = "journal-v1.json";
const STAGING_NAME: &str = "journal-v1.staging";
const BACKUP_NAME: &str = "journal-v1.previous";
const LEASE_NAME: &str = "writer-v1.lock";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningPublishPoint {
    AfterStageSync,
    AfterPublish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectId {
    volume: u32,
    index: u64,
}
struct PinnedDirectory {
    path: PathBuf,
    file: File,
    id: ObjectId,
    managed: bool,
}

/// Native authority is opened, retained and dropped by one worker.
pub struct TuningStore {
    root: PathBuf,
    directories: Vec<PinnedDirectory>,
    lease: File,
    lease_id: ObjectId,
    policy: SecurityPolicy,
    max_bytes: usize,
    poisoned: bool,
    #[cfg(any(test, feature = "tuning-test-controls"))]
    fault: Option<TuningPublishPoint>,
    #[cfg(test)]
    before_backup_remove: Option<Box<dyn FnOnce()>>,
    _worker_only: PhantomData<Rc<()>>,
}

impl TuningStore {
    pub fn authority(&self) -> TuningStoreAuthority {
        #[cfg(any(test, feature = "tuning-test-controls"))]
        if self.policy.test_sid.is_some() {
            return TuningStoreAuthority::IsolatedTest;
        }
        TuningStoreAuthority::Canonical
    }
    /// This API never chooses a caller-supplied production path. Calling it is
    /// an explicit metadata initialization, not proof that elevated gates ran.
    pub fn open_canonical(max_bytes: usize) -> Result<Self, TuningNativeError> {
        validate_bound(max_bytes)?;
        if !identity::elevated()? {
            return Err(TuningNativeError::Denied {
                operation: "canonical store requires elevated token",
                code: 5,
            });
        }
        let value = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) }
            .map_err(|e| native_error("ProgramData known folder", e))?;
        let text = unsafe { value.to_string() };
        unsafe {
            CoTaskMemFree(Some(value.0.cast()));
        }
        let base = PathBuf::from(
            text.map_err(|_| TuningNativeError::Unsupported("ProgramData path encoding"))?,
        );
        let root = base.join("CpuAffinityTool").join("tuning");
        Self::open(root, base, SecurityPolicy { test_sid: None }, max_bytes)
    }
    #[cfg(any(test, feature = "tuning-test-controls"))]
    pub fn open_test_directory(root: &Path, max_bytes: usize) -> Result<Self, TuningNativeError> {
        validate_bound(max_bytes)?;
        let base = std::env::temp_dir();
        if root.parent() != Some(base.as_path())
            || !root.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("cpu-affinity-tuning-test-")
            })
        {
            return Err(TuningNativeError::UnsafeStore(
                "isolated temporary store path required",
            ));
        }
        let sid = crate::windows::OS::tuning_current_token_identity()?
            .sid()
            .to_owned();
        Self::open(
            root.to_owned(),
            base,
            SecurityPolicy {
                test_sid: Some(sid),
            },
            max_bytes,
        )
    }
    pub fn read(&mut self) -> Result<Option<Vec<u8>>, TuningNativeError> {
        self.validate_authority()?;
        self.validate_artifacts()?;
        self.read_journal()
    }
    pub fn publish(&mut self, bytes: &[u8]) -> Result<(), TuningNativeError> {
        if self.poisoned {
            return Err(TuningNativeError::Poisoned);
        }
        if bytes.len() > self.max_bytes {
            return Err(TuningNativeError::Capacity {
                size: bytes.len() as u64,
                limit: self.max_bytes,
            });
        }
        let result = self.publish_inner(bytes);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub fn validate_authority(&mut self) -> Result<(), TuningNativeError> {
        if self.poisoned {
            return Err(TuningNativeError::Poisoned);
        }
        let result = self.validate_objects();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    #[cfg(any(test, feature = "tuning-test-controls"))]
    pub fn fail_publication_at(&mut self, point: TuningPublishPoint) {
        self.fault = Some(point);
    }

    fn open(
        root: PathBuf,
        existing_base: PathBuf,
        policy: SecurityPolicy,
        max_bytes: usize,
    ) -> Result<Self, TuningNativeError> {
        let descriptor = Descriptor::new(policy.test_sid.as_deref())?;
        let mut directories = Vec::new();
        let mut path = PathBuf::new();
        let mut components = root.components();
        match components.next() {
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), std::path::Prefix::Disk(_)) =>
            {
                path.push(prefix.as_os_str())
            }
            _ => {
                return Err(TuningNativeError::Unsupported(
                    "local drive absolute store required",
                ));
            }
        }
        if components.next() != Some(Component::RootDir) {
            return Err(TuningNativeError::UnsafeStore("absolute store required"));
        }
        path.push("\\");
        let parts: Vec<_> = components.collect();
        for index in 0..=parts.len() {
            if index != 0 {
                match parts[index - 1] {
                    Component::Normal(value) => path.push(value),
                    _ => {
                        return Err(TuningNativeError::UnsafeStore(
                            "normalized store components required",
                        ));
                    }
                }
            }
            let managed = path != existing_base && path.starts_with(&existing_base);
            if managed {
                let value = wide(path.as_os_str())?;
                let attrs = descriptor.attributes();
                if let Err(error) =
                    unsafe { CreateDirectoryW(PCWSTR(value.as_ptr()), Some(&attrs)) }
                {
                    let error = native_error("CreateDirectoryW store", error);
                    if !matches!(error, TuningNativeError::Io { code: 183, .. }) {
                        return Err(error);
                    }
                }
            }
            let file = open_directory(&path)?;
            let info = checked_info(&file, true)?;
            // The nonshipping fixture proves managed-object primitives, not
            // privileged security of the user's writable temp ancestors.
            if managed || policy.test_sid.is_none() {
                policy.validate(raw(&file), managed)?;
            }
            if index == 0 {
                validate_volume(&file)?;
            }
            directories.push(PinnedDirectory {
                path: path.clone(),
                file,
                id: object_id(&info),
                managed,
            });
        }
        let lease_path = root.join(LEASE_NAME);
        let lease = open_file(
            &lease_path,
            GENERIC_READ.0 | GENERIC_WRITE.0 | READ_CONTROL.0,
            FILE_SHARE_NONE,
            OPEN_ALWAYS,
            Some(&descriptor),
        )?;
        let lease_id = object_id(&checked_info(&lease, false)?);
        policy.validate(raw(&lease), true)?;
        let mut result = Self {
            root,
            directories,
            lease,
            lease_id,
            policy,
            max_bytes,
            poisoned: false,
            #[cfg(any(test, feature = "tuning-test-controls"))]
            fault: None,
            #[cfg(test)]
            before_backup_remove: None,
            _worker_only: PhantomData,
        };
        result.validate_authority()?;
        result.validate_artifacts()?;
        Ok(result)
    }

    fn validate_objects(&self) -> Result<(), TuningNativeError> {
        for entry in &self.directories {
            if object_id(&checked_info(&entry.file, true)?) != entry.id {
                return Err(TuningNativeError::UnsafeStore("held directory changed"));
            }
            if entry.managed || self.policy.test_sid.is_none() {
                self.policy.validate(raw(&entry.file), entry.managed)?;
            }
            let reopened = open_directory(&entry.path)?;
            if object_id(&checked_info(&reopened, true)?) != entry.id {
                return Err(TuningNativeError::UnsafeStore(
                    "directory namespace changed",
                ));
            }
            if entry.managed || self.policy.test_sid.is_none() {
                self.policy.validate(raw(&reopened), entry.managed)?;
            }
        }
        if object_id(&checked_info(&self.lease, false)?) != self.lease_id {
            return Err(TuningNativeError::UnsafeStore("held lease changed"));
        }
        self.policy.validate(raw(&self.lease), true)?;
        let path = self.root.join(LEASE_NAME);
        let reopened = open_file(
            &path,
            FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            OPEN_EXISTING,
            None,
        )?;
        if object_id(&checked_info(&reopened, false)?) != self.lease_id {
            return Err(TuningNativeError::UnsafeStore("lease namespace changed"));
        }
        self.policy.validate(raw(&reopened), true)
    }

    fn validate_artifacts(&self) -> Result<(), TuningNativeError> {
        let entries =
            fs::read_dir(&self.root).map_err(|e| io_error("list journal artifacts", e))?;
        for (index, entry) in entries.enumerate() {
            if index > 3 {
                return Err(TuningNativeError::UnsafeStore("unknown managed artifacts"));
            }
            let name = entry
                .map_err(|e| io_error("journal artifact", e))?
                .file_name();
            if name != OsStr::new(JOURNAL_NAME) && name != OsStr::new(LEASE_NAME) {
                return Err(TuningNativeError::UnsafeStore(
                    "pending or unknown journal artifact",
                ));
            }
        }
        Ok(())
    }

    fn journal_file(&self) -> Result<Option<File>, TuningNativeError> {
        let file = match open_file(
            &self.root.join(JOURNAL_NAME),
            GENERIC_READ.0 | READ_CONTROL.0,
            FILE_SHARE_READ,
            OPEN_EXISTING,
            None,
        ) {
            Ok(file) => file,
            Err(TuningNativeError::Io { code: 2, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        checked_info(&file, false)?;
        self.policy.validate(raw(&file), true)?;
        Ok(Some(file))
    }

    fn read_journal(&self) -> Result<Option<Vec<u8>>, TuningNativeError> {
        let Some(file) = self.journal_file()? else {
            return Ok(None);
        };
        let info = checked_info(&file, false)?;
        let size = (info.nFileSizeHigh as u64) << 32 | info.nFileSizeLow as u64;
        if size > self.max_bytes as u64 {
            return Err(TuningNativeError::Capacity {
                size,
                limit: self.max_bytes,
            });
        }
        let mut bytes = Vec::with_capacity(size as usize);
        file.take(self.max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| io_error("read journal", e))?;
        if bytes.len() as u64 != size {
            return Err(TuningNativeError::UnsafeStore("journal length changed"));
        }
        Ok(Some(bytes))
    }

    fn publish_inner(&mut self, bytes: &[u8]) -> Result<(), TuningNativeError> {
        self.validate_authority()?;
        self.validate_artifacts()?;
        let old = self
            .journal_file()?
            .map(|file| {
                let info = checked_info(&file, false)?;
                let size = (info.nFileSizeHigh as u64) << 32 | info.nFileSizeLow as u64;
                if size > self.max_bytes as u64 {
                    return Err(TuningNativeError::Capacity {
                        size,
                        limit: self.max_bytes,
                    });
                }
                Ok(object_id(&info))
            })
            .transpose()?;
        let descriptor = Descriptor::new(self.policy.test_sid.as_deref())?;
        let staging_path = self.root.join(STAGING_NAME);
        let mut staging = open_file(
            &staging_path,
            GENERIC_READ.0 | GENERIC_WRITE.0 | READ_CONTROL.0,
            FILE_SHARE_NONE,
            CREATE_NEW,
            Some(&descriptor),
        )?;
        let staging_id = object_id(&checked_info(&staging, false)?);
        self.policy.validate(raw(&staging), true)?;
        staging
            .write_all(bytes)
            .map_err(|e| io_error("write journal staging", e))?;
        staging
            .sync_all()
            .map_err(|e| io_error("sync journal staging", e))?;
        if object_id(&checked_info(&staging, false)?) != staging_id {
            return Err(TuningNativeError::UnsafeStore("staging object changed"));
        }
        self.policy.validate(raw(&staging), true)?;
        self.inject(TuningPublishPoint::AfterStageSync)?;
        drop(staging);
        self.validate_objects()?;
        let current = self
            .journal_file()?
            .map(|file| checked_info(&file, false).map(|info| object_id(&info)))
            .transpose()?;
        if current != old {
            return Err(TuningNativeError::UnsafeStore(
                "journal object changed before publication",
            ));
        }
        let stage_proof = open_file(
            &staging_path,
            GENERIC_READ.0 | READ_CONTROL.0,
            FILE_SHARE_READ,
            OPEN_EXISTING,
            None,
        )?;
        if object_id(&checked_info(&stage_proof, false)?) != staging_id {
            return Err(TuningNativeError::UnsafeStore("staging namespace changed"));
        }
        self.policy.validate(raw(&stage_proof), true)?;
        drop(stage_proof);
        let source = wide(staging_path.as_os_str())?;
        let target = wide(self.root.join(JOURNAL_NAME).as_os_str())?;
        let backup = wide(self.root.join(BACKUP_NAME).as_os_str())?;
        unsafe {
            if old.is_some() {
                ReplaceFileW(
                    PCWSTR(target.as_ptr()),
                    PCWSTR(source.as_ptr()),
                    PCWSTR(backup.as_ptr()),
                    REPLACE_FILE_FLAGS(0),
                    None,
                    None,
                )
            } else {
                MoveFileExW(
                    PCWSTR(source.as_ptr()),
                    PCWSTR(target.as_ptr()),
                    MOVE_FILE_FLAGS(0),
                )
            }
        }
        .map_err(|e| native_error("publish journal", e))?;
        self.inject(TuningPublishPoint::AfterPublish)?;
        self.validate_objects()?;
        let result_file = self
            .journal_file()?
            .ok_or(TuningNativeError::UnsafeStore("published journal missing"))?;
        if object_id(&checked_info(&result_file, false)?) != staging_id {
            return Err(TuningNativeError::UnsafeStore("published object mismatch"));
        }
        drop(result_file);
        if self.read_journal()?.as_deref() != Some(bytes) {
            return Err(TuningNativeError::UnsafeStore("published bytes mismatch"));
        }
        if old.is_some() {
            // Only the authenticated exact previous object of this publication
            // is eligible for cleanup; arbitrary artifact deletion is forbidden.
            let previous = open_file(
                &self.root.join(BACKUP_NAME),
                GENERIC_READ.0 | READ_CONTROL.0 | DELETE.0,
                FILE_SHARE_READ,
                OPEN_EXISTING,
                None,
            )?;
            if Some(object_id(&checked_info(&previous, false)?)) != old {
                return Err(TuningNativeError::UnsafeStore("previous object mismatch"));
            }
            self.policy.validate(raw(&previous), true)?;
            #[cfg(test)]
            if let Some(hook) = self.before_backup_remove.take() {
                hook();
            }
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            unsafe {
                SetFileInformationByHandle(
                    raw(&previous),
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            }
            .map_err(|e| native_error("delete verified previous journal handle", e))?;
            drop(previous);
        }
        self.validate_artifacts()?;
        Ok(())
    }

    fn inject(&mut self, point: TuningPublishPoint) -> Result<(), TuningNativeError> {
        #[cfg(any(test, feature = "tuning-test-controls"))]
        if self.fault == Some(point) {
            self.fault = None;
            return Err(TuningNativeError::Io {
                operation: "injected publication failure",
                code: 0,
            });
        }
        let _ = point;
        Ok(())
    }
}

fn validate_bound(max: usize) -> Result<(), TuningNativeError> {
    if max == 0 || max > MAX_TUNING_JOURNAL_BYTES {
        Err(TuningNativeError::Unsupported("journal byte bound"))
    } else {
        Ok(())
    }
}
fn raw(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}
fn io_error(operation: &'static str, error: std::io::Error) -> TuningNativeError {
    super::code_error(operation, error.raw_os_error().unwrap_or(-1) as u32)
}
fn object_id(info: &BY_HANDLE_FILE_INFORMATION) -> ObjectId {
    ObjectId {
        volume: info.dwVolumeSerialNumber,
        index: (info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64,
    }
}
fn checked_info(
    file: &File,
    directory: bool,
) -> Result<BY_HANDLE_FILE_INFORMATION, TuningNativeError> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(raw(file), &mut info) }
        .map_err(|e| native_error("GetFileInformationByHandle", e))?;
    if unsafe { GetFileType(raw(file)) } != FILE_TYPE_DISK
        || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(TuningNativeError::UnsafeStore(
            "unexpected object type/reparse/hardlink",
        ));
    }
    Ok(info)
}
pub(super) fn validate_volume(file: &File) -> Result<(), TuningNativeError> {
    let mut flags = 0;
    let mut name = [0u16; 32];
    unsafe {
        GetVolumeInformationByHandleW(
            raw(file),
            None,
            None,
            None,
            Some(&mut flags),
            Some(&mut name),
        )
    }
    .map_err(|e| native_error("store volume information", e))?;
    let end = name.iter().position(|x| *x == 0).unwrap_or(name.len());
    if String::from_utf16_lossy(&name[..end]) != "NTFS" || flags & 8 == 0 {
        return Err(TuningNativeError::Unsupported(
            "local NTFS ACL store required",
        ));
    }
    Ok(())
}
fn open_directory(path: &Path) -> Result<File, TuningNativeError> {
    let value = wide(path.as_os_str())?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(value.as_ptr()),
            FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|e| native_error("open store directory", e))?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}
fn open_file(
    path: &Path,
    access: u32,
    sharing: FILE_SHARE_MODE,
    disposition: windows::Win32::Storage::FileSystem::FILE_CREATION_DISPOSITION,
    descriptor: Option<&Descriptor>,
) -> Result<File, TuningNativeError> {
    let value = wide(path.as_os_str())?;
    let attrs = descriptor.map(Descriptor::attributes);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(value.as_ptr()),
            access,
            sharing,
            attrs.as_ref().map(|x| x as *const _),
            disposition,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|e| native_error("open journal object", e))?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::TuningTemporaryDirectory as TempRoot;
    use super::*;
    use std::fs;
    #[test]
    fn preexisting_fixture_collision_is_never_owned_or_cleaned() {
        let root = TempRoot::new().unwrap();
        fs::write(root.path().join("unrelated"), b"retained prior evidence").unwrap();
        assert!(TempRoot::try_new_at(root.path().to_owned()).is_err());
        assert_eq!(
            fs::read(root.path().join("unrelated")).unwrap(),
            b"retained prior evidence"
        );
    }
    #[test]
    fn exclusive_lease_survives_lock_file_existence_then_reopens_after_drop() {
        let root = TempRoot::new().unwrap();
        let first = TuningStore::open_test_directory(root.path(), 128).unwrap();
        assert!(matches!(
            TuningStore::open_test_directory(root.path(), 128),
            Err(TuningNativeError::Busy)
        ));
        drop(first);
        assert!(TuningStore::open_test_directory(root.path(), 128).is_ok());
    }
    #[test]
    fn bounded_roundtrip_and_replacement_preserve_exact_bytes() {
        let root = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 128).unwrap();
        assert_eq!(store.read().unwrap(), None);
        store.publish(b"first\0exact").unwrap();
        assert_eq!(store.read().unwrap().unwrap(), b"first\0exact");
        store.publish(b"replacement").unwrap();
        assert_eq!(store.read().unwrap().unwrap(), b"replacement");
    }
    #[test]
    fn oversized_publication_keeps_previous_journal() {
        let root = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 8).unwrap();
        store.publish(b"baseline").unwrap();
        assert!(matches!(
            store.publish(b"too large"),
            Err(TuningNativeError::Capacity { .. })
        ));
        assert_eq!(store.read().unwrap().unwrap(), b"baseline");
    }
    #[test]
    fn orphan_staging_is_preserved_and_never_becomes_absent() {
        let root = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 128).unwrap();
        fs::write(root.path().join(STAGING_NAME), b"unfinished evidence").unwrap();
        assert!(matches!(
            store.read(),
            Err(TuningNativeError::UnsafeStore(_))
        ));
        assert_eq!(
            fs::read(root.path().join(STAGING_NAME)).unwrap(),
            b"unfinished evidence"
        );
    }
    #[test]
    fn synced_stage_failure_retains_artifact_and_poison_blocks_retry() {
        let root = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 128).unwrap();
        store.publish(b"old").unwrap();
        store.fail_publication_at(TuningPublishPoint::AfterStageSync);
        assert!(store.publish(b"intent").is_err());
        assert_eq!(fs::read(root.path().join(JOURNAL_NAME)).unwrap(), b"old");
        assert_eq!(fs::read(root.path().join(STAGING_NAME)).unwrap(), b"intent");
        assert_eq!(store.publish(b"retry"), Err(TuningNativeError::Poisoned));
    }
    #[test]
    fn temporary_factory_cannot_select_an_arbitrary_path() {
        assert!(matches!(
            TuningStore::open_test_directory(Path::new("C:\\arbitrary"), 128),
            Err(TuningNativeError::UnsafeStore(_))
        ));
    }
    #[test]
    fn oversized_existing_record_is_preserved_and_cannot_be_overwritten() {
        let root = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 8).unwrap();
        assert_eq!(store.authority(), TuningStoreAuthority::IsolatedTest);
        store.publish(b"old").unwrap();
        fs::write(
            root.path().join(JOURNAL_NAME),
            b"oversized retained evidence",
        )
        .unwrap();
        assert!(matches!(
            store.read(),
            Err(TuningNativeError::Capacity { .. })
        ));
        assert!(matches!(
            store.publish(b"new"),
            Err(TuningNativeError::Capacity { .. })
        ));
        assert_eq!(
            fs::read(root.path().join(JOURNAL_NAME)).unwrap(),
            b"oversized retained evidence"
        );
    }
    #[test]
    fn preexisting_unprotected_directory_is_rejected_without_repair() {
        let root = TempRoot::unprotected().unwrap();
        fs::write(root.path().join("unrelated"), b"untouched").unwrap();
        assert!(matches!(
            TuningStore::open_test_directory(root.path(), 128),
            Err(TuningNativeError::UnsafeStore(_))
        ));
        assert_eq!(
            fs::read(root.path().join("unrelated")).unwrap(),
            b"untouched"
        );
        assert!(!root.path().join(LEASE_NAME).exists());
    }
    #[test]
    fn journal_hardlink_refuses_reads_and_preserves_both_names() {
        let root = TempRoot::new().unwrap();
        let alias = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 128).unwrap();
        store.publish(b"evidence").unwrap();
        fs::hard_link(root.path().join(JOURNAL_NAME), alias.path().join("alias")).unwrap();
        assert!(matches!(
            store.read(),
            Err(TuningNativeError::UnsafeStore(_))
        ));
        assert_eq!(fs::read(alias.path().join("alias")).unwrap(), b"evidence");
    }
    #[test]
    fn timeout_does_not_transfer_the_worker_held_native_lease() {
        use std::{sync::mpsc, thread, time::Duration};
        let root = TempRoot::new().unwrap();
        let path = root.path().to_owned();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let store = TuningStore::open_test_directory(&path, 128).unwrap();
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            drop(store);
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            TuningStore::open_test_directory(root.path(), 128),
            Err(TuningNativeError::Busy)
        ));
        assert!(!worker.is_finished());
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(TuningStore::open_test_directory(root.path(), 128).is_ok());
    }
    #[test]
    fn exclusive_lease_is_shared_across_processes() {
        use std::{os::windows::process::CommandExt, process::Command};
        let root = TempRoot::new().unwrap();
        let _store = TuningStore::open_test_directory(root.path(), 128).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "windows::tuning::store::tests::native_lease_child_probe",
                "--nocapture",
            ])
            .env("CPU_AFFINITY_TUNING_TEST_CHILD_ROOT", root.path())
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
    }
    #[test]
    fn native_lease_child_probe() {
        if let Some(path) = std::env::var_os("CPU_AFFINITY_TUNING_TEST_CHILD_ROOT") {
            assert!(matches!(
                TuningStore::open_test_directory(Path::new(&path), 128),
                Err(TuningNativeError::Busy)
            ));
        }
    }
    #[test]
    fn failed_after_replacement_keeps_intent_and_previous_evidence() {
        let root = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 128).unwrap();
        store.publish(b"old").unwrap();
        store.fail_publication_at(TuningPublishPoint::AfterPublish);
        assert!(store.publish(b"intent").is_err());
        assert_eq!(fs::read(root.path().join(JOURNAL_NAME)).unwrap(), b"intent");
        assert_eq!(fs::read(root.path().join(BACKUP_NAME)).unwrap(), b"old");
        assert_eq!(store.publish(b"retry"), Err(TuningNativeError::Poisoned));
        drop(store);
        assert!(matches!(
            TuningStore::open_test_directory(root.path(), 128),
            Err(TuningNativeError::UnsafeStore(_))
        ));
    }
    #[test]
    fn failed_after_first_publication_retains_record_and_poison() {
        let root = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 128).unwrap();
        store.fail_publication_at(TuningPublishPoint::AfterPublish);
        assert!(store.publish(b"possible intent").is_err());
        assert_eq!(
            fs::read(root.path().join(JOURNAL_NAME)).unwrap(),
            b"possible intent"
        );
        assert_eq!(store.publish(b"retry"), Err(TuningNativeError::Poisoned));
        drop(store);
        // A later lease holder can inspect the exact retained record; native
        // storage never turns this post-publication failure into absence.
        let mut later = TuningStore::open_test_directory(root.path(), 128).unwrap();
        assert_eq!(later.read().unwrap().unwrap(), b"possible intent");
    }
    #[test]
    fn backup_cleanup_cannot_delete_a_namespace_replacement() {
        use std::sync::{Arc, Mutex};
        let root = TempRoot::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), 128).unwrap();
        store.publish(b"old").unwrap();
        let path = root.path().to_owned();
        let swapped = Arc::new(Mutex::new(false));
        let observe = Arc::clone(&swapped);
        store.before_backup_remove = Some(Box::new(move || {
            if fs::rename(path.join(BACKUP_NAME), path.join("retained-original")).is_ok() {
                fs::write(path.join(BACKUP_NAME), b"unrelated replacement").unwrap();
                *observe.lock().unwrap() = true;
            }
        }));
        let _result = store.publish(b"new");
        assert!(
            !*swapped.lock().unwrap(),
            "authenticated backup handle must prevent namespace replacement"
        );
        assert_eq!(store.read().unwrap().unwrap(), b"new");
    }
}
