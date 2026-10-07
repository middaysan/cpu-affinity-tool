//! Owned isolated native-test fixtures. Never enabled by release builds.
use super::{native_error, security::Descriptor, wide};
use crate::tuning::TuningNativeError;
use std::{
    fs::{self, File},
    marker::PhantomData,
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::{Path, PathBuf},
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::HANDLE,
        Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, CreateFileW, DELETE,
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, FileDispositionInfo,
            GetFileInformationByHandle, GetFileType, OPEN_EXISTING, READ_CONTROL,
            SetFileInformationByHandle,
        },
    },
    core::PCWSTR,
};

/// Creation must succeed for a new random root. Cleanup proves the original
/// opened directory identity, retains DELETE authority and removes only bounded
/// flat native objects by handle. Unknown/replaced/nested objects are retained.
pub struct TuningTemporaryDirectory {
    path: PathBuf,
    id: (u32, u64),
    _worker_only: PhantomData<Rc<()>>,
}
impl TuningTemporaryDirectory {
    pub fn new() -> Result<Self, TuningNativeError> {
        for _ in 0..8 {
            let mut nonce = [0; 32];
            unsafe { BCryptGenRandom(None, &mut nonce, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
                .ok()
                .map_err(|e| native_error("temporary directory nonce", e))?;
            let suffix: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
            let path = std::env::temp_dir().join(format!("cpu-affinity-tuning-test-{suffix}"));
            match Self::create_at(path, true) {
                Err(TuningNativeError::Io { code: 183, .. }) => continue,
                other => return other,
            }
        }
        Err(TuningNativeError::Unsupported(
            "temporary directory collision limit",
        ))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Test-only shape of the ID captured at this root's actual creation.
    /// This exposes no handle or native/canonical authority.
    #[cfg(test)]
    pub(super) fn original_identity(&self) -> (u32, u64) {
        self.id
    }
    #[cfg(test)]
    pub(super) fn unprotected() -> Result<Self, TuningNativeError> {
        let new = Self::new()?;
        let path = new.path.clone();
        new.cleanup()?;
        // Deliberately inherited fixture for rejection characterization. This
        // new creation is still owned; it does not relax managed-store policy.
        Self::create_at(path, false)
    }
    #[cfg(test)]
    pub(super) fn try_new_at(path: PathBuf) -> Result<Self, TuningNativeError> {
        Self::create_at(path, true)
    }
    fn create_at(path: PathBuf, protected: bool) -> Result<Self, TuningNativeError> {
        if path.parent() != Some(std::env::temp_dir().as_path())
            || !path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("cpu-affinity-tuning-test-")
            })
        {
            return Err(TuningNativeError::UnsafeStore(
                "owned temporary root required",
            ));
        }
        let parent = open(std::env::temp_dir().as_path(), false, true)?;
        object(&parent, true)?;
        super::store::validate_volume(&parent)?;
        let sid = crate::windows::OS::tuning_current_token_identity()?
            .sid()
            .to_owned();
        let descriptor = if protected {
            Some(Descriptor::new(Some(&sid))?)
        } else {
            None
        };
        let attrs = descriptor.as_ref().map(Descriptor::attributes);
        let text = wide(path.as_os_str())?;
        unsafe { CreateDirectoryW(PCWSTR(text.as_ptr()), attrs.as_ref().map(|x| x as *const _)) }
            .map_err(|e| native_error("create new owned temporary root", e))?;
        // ERROR_ALREADY_EXISTS is never accepted as creation ownership.
        let file = open(&path, false, true)?;
        super::store::validate_volume(&file)?;
        let id = object(&file, true)?;
        Ok(Self {
            path,
            id,
            _worker_only: PhantomData,
        })
    }
    pub fn cleanup(mut self) -> Result<(), TuningNativeError> {
        let result = self.cleanup_inner();
        // Never retry an uncertain cleanup from Drop.
        self.path.clear();
        result
    }
    fn cleanup_inner(&self) -> Result<(), TuningNativeError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if self.path.parent() != Some(std::env::temp_dir().as_path()) {
            return Err(TuningNativeError::UnsafeStore(
                "temporary cleanup boundary changed",
            ));
        }
        let root = open(&self.path, true, true)?;
        if object(&root, true)? != self.id {
            return Err(TuningNativeError::UnsafeStore(
                "temporary root identity changed",
            ));
        }
        let mut children = Vec::new();
        for entry in fs::read_dir(&self.path)
            .map_err(|_| TuningNativeError::UnsafeStore("temporary cleanup enumeration"))?
        {
            if children.len() >= 32 {
                return Err(TuningNativeError::Unsupported(
                    "temporary cleanup entry limit",
                ));
            }
            let entry =
                entry.map_err(|_| TuningNativeError::UnsafeStore("temporary cleanup entry"))?;
            if entry.path().parent() != Some(self.path.as_path()) {
                return Err(TuningNativeError::UnsafeStore(
                    "temporary child boundary changed",
                ));
            }
            let file = open(&entry.path(), true, false)?;
            object(&file, false)?;
            children.push(file);
        }
        // The retained root handle denies namespace substitution. Child handles
        // deny delete sharing and mark those exact opened objects for deletion.
        for child in children {
            remove(child)?;
        }
        remove(root)
    }
}
impl Drop for TuningTemporaryDirectory {
    fn drop(&mut self) {
        let _ = self.cleanup_inner();
    }
}
fn raw(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}
fn open(path: &Path, delete: bool, directory: bool) -> Result<File, TuningNativeError> {
    let text = wide(path.as_os_str())?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(text.as_ptr()),
            FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0 | if delete { DELETE.0 } else { 0 },
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0)
                },
            None,
        )
    }
    .map_err(|e| native_error("open owned temporary object", e))?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}
fn object(file: &File, directory: bool) -> Result<(u32, u64), TuningNativeError> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(raw(file), &mut info) }
        .map_err(|e| native_error("temporary object identity", e))?;
    if unsafe { GetFileType(raw(file)) } != FILE_TYPE_DISK
        || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != directory
    {
        return Err(TuningNativeError::UnsafeStore(
            "unexpected temporary object type",
        ));
    }
    Ok((
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}
fn remove(file: File) -> Result<(), TuningNativeError> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            raw(&file),
            FileDispositionInfo,
            &disposition as *const _ as *const _,
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|e| native_error("remove owned temporary object", e))?;
    drop(file);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_owned_flat_fixture_is_removed_by_retained_handles() {
        let root = TuningTemporaryDirectory::new().unwrap();
        let path = root.path().to_owned();
        fs::write(path.join("own-target"), b"own temporary data").unwrap();
        root.cleanup().unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn cleanup_does_not_delete_a_replaced_directory_incarnation() {
        let original = TuningTemporaryDirectory::new().unwrap();
        fs::write(original.path().join("old-target"), b"owned original").unwrap();
        let destination = TuningTemporaryDirectory::new().unwrap();
        let moved_path = destination.path().to_owned();
        destination.cleanup().unwrap();
        assert_eq!(
            original.path().parent(),
            Some(std::env::temp_dir().as_path())
        );
        assert_eq!(moved_path.parent(), Some(std::env::temp_dir().as_path()));
        fs::rename(original.path(), &moved_path).unwrap();
        // Same exact created object, now moved by this test; retain its actual
        // original ID for eventual owned cleanup at the new bounded path.
        let moved = TuningTemporaryDirectory {
            path: moved_path,
            id: original.id,
            _worker_only: PhantomData,
        };
        let replacement = TuningTemporaryDirectory::try_new_at(original.path().to_owned()).unwrap();
        fs::write(
            replacement.path().join("evidence"),
            b"replacement must survive",
        )
        .unwrap();
        drop(original);
        assert_eq!(
            fs::read(replacement.path().join("evidence")).unwrap(),
            b"replacement must survive"
        );
        moved.cleanup().unwrap();
        replacement.cleanup().unwrap();
    }
}
