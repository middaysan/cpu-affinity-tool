use super::store_io::native::{Directory, Node};
use super::{
    identity, native_error,
    security::{Descriptor, SecurityPolicy},
    store_io::{self, Disposition, Entry, Kind, OpenMode, Sharing},
};
use crate::tuning::{MAX_TUNING_JOURNAL_BYTES, TuningNativeError, TuningStoreAuthority};
#[cfg(any(test, feature = "tuning-test-controls"))]
use std::path::Path;
use std::{
    fs::File,
    marker::PhantomData,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::{Component, PathBuf},
    rc::Rc,
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::GetVolumeInformationByHandleW,
    System::Com::CoTaskMemFree,
    UI::Shell::{FOLDERID_ProgramData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
};
pub const JOURNAL_NAME: &str = "journal-v1.json";
const STAGING_NAME: &str = "journal-v1.staging";
const BACKUP_NAME: &str = "journal-v1.previous";
const LEASE_NAME: &str = "writer-v1.lock";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningPublishPoint {
    AfterStageSync,
    AfterOldMove,
    AfterPublish,
    BeforeBackupCleanup,
    AfterBackupCleanup,
}
#[cfg(test)]
type PublicationHook = Box<dyn FnMut(TuningPublishPoint) -> Result<(), TuningNativeError>>;
struct PinnedDirectory {
    name: Option<Vec<u16>>,
    directory: Directory,
    scope: Option<bool>,
}
/// Opening locals are the actual authority frame while descendant native IO
/// runs. A nonterminal request parks the same worker without unwinding this
/// chain or any newly delivered lease handle in its owned output allocation.
struct OpeningContext {
    // Lease closes before all ancestor handles.
    lease: Node,
    directories: Vec<PinnedDirectory>,
    policy: Rc<SecurityPolicy>,
    max_bytes: usize,
}
/// Native authority is opened, retained and dropped by one worker.
/// All descendant operations use the retained root. An unresolved request
/// retains the entire worker indefinitely; process exit is the release bound.
pub struct TuningStore {
    // Publication nodes are call locals, so close before the retained lease.
    lease: Node,
    directories: Vec<PinnedDirectory>,
    _policy: Rc<SecurityPolicy>,
    max_bytes: usize,
    poisoned: bool,
    #[cfg(any(test, feature = "tuning-test-controls"))]
    fault: Option<TuningPublishPoint>,
    #[cfg(test)]
    publication_hook: Option<PublicationHook>,
    #[cfg(test)]
    after_read_snapshot: Option<Box<dyn FnMut()>>,
    #[cfg(test)]
    after_previous_read: Option<Box<dyn FnMut()>>,
    _worker_only: PhantomData<Rc<()>>,
}
impl TuningStore {
    pub fn authority(&self) -> TuningStoreAuthority {
        #[cfg(any(test, feature = "tuning-test-controls"))]
        if self._policy.test_sid.is_some() {
            return TuningStoreAuthority::IsolatedTest;
        }
        TuningStoreAuthority::Canonical
    }
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
        let (base, policy) = test_selection(root, max_bytes)?;
        Self::open(root.to_owned(), base, policy, max_bytes)
    }
    pub fn read(&mut self) -> Result<Option<Vec<u8>>, TuningNativeError> {
        if self.poisoned {
            return Err(TuningNativeError::Poisoned);
        }
        let result = (|| {
            self.validate_objects()?;
            let mut journal = self.clean_journal(true)?;
            let bytes = match journal.as_mut() {
                Some(node) => Some(node.read(self.max_bytes)?),
                None => None,
            };
            #[cfg(test)]
            if let Some(hook) = self.after_read_snapshot.as_mut() {
                hook();
            }
            match (journal.as_ref(), bytes.as_ref()) {
                (Some(node), Some(bytes)) => {
                    let fresh = self
                        .clean_journal(true)?
                        .ok_or(TuningNativeError::UnsafeStore("journal presence changed"))?;
                    if fresh.id() != node.id() {
                        return Err(TuningNativeError::UnsafeStore(
                            "journal read namespace changed",
                        ));
                    }
                    self.check_name(JOURNAL_NAME, node, bytes)?;
                }
                (None, None) => {
                    if self.clean_journal(true)?.is_some() {
                        return Err(TuningNativeError::UnsafeStore("journal absence changed"));
                    }
                }
                _ => return Err(TuningNativeError::UnsafeStore("journal read evidence")),
            }
            self.validate_objects()?;
            Ok(bytes)
        })();
        // Capacity is a terminal nonmutating bound refusal; a caller may
        // independently test publication against the retained oversized record.
        if result
            .as_ref()
            .is_err_and(|e| !matches!(e, TuningNativeError::Capacity { .. }))
        {
            self.poisoned = true;
        }
        result
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
    #[cfg(test)]
    pub(super) fn set_publication_hook<
        F: FnMut(TuningPublishPoint) -> Result<(), TuningNativeError> + 'static,
    >(
        &mut self,
        hook: F,
    ) {
        self.publication_hook = Some(Box::new(hook));
    }
    #[cfg(test)]
    pub(super) fn set_after_read_snapshot_hook<F: FnMut() + 'static>(&mut self, hook: F) {
        self.after_read_snapshot = Some(Box::new(hook));
    }
    #[cfg(test)]
    pub(super) fn set_after_previous_read_hook<F: FnMut() + 'static>(&mut self, hook: F) {
        self.after_previous_read = Some(Box::new(hook));
    }
    #[cfg(test)]
    pub(super) fn retain_nonterminal_for_test<F: FnOnce()>(&mut self, entered: F) -> ! {
        assert_eq!(self.authority(), TuningStoreAuthority::IsolatedTest);
        self.validate_authority()
            .expect("isolated retained authority");
        store_io::retain_nonterminal(Box::new(self), entered)
    }
    #[cfg(test)]
    pub(super) fn retain_opening_for_test<F: FnOnce()>(
        root: &Path,
        max_bytes: usize,
        entered: F,
    ) -> ! {
        let (base, policy) = test_selection(root, max_bytes).expect("isolated opening selection");
        let mut opening = Self::prepare_open(root.to_owned(), base, policy, max_bytes)
            .expect("actual isolated opening and lease");
        store_io::retain_nonterminal(Box::new(&mut opening), entered)
    }
    fn open(
        root: PathBuf,
        existing_base: PathBuf,
        policy: SecurityPolicy,
        max_bytes: usize,
    ) -> Result<Self, TuningNativeError> {
        let opening = Self::prepare_open(root, existing_base, policy, max_bytes)?;
        let mut store = Self {
            lease: opening.lease,
            directories: opening.directories,
            _policy: opening.policy,
            max_bytes: opening.max_bytes,
            poisoned: false,
            #[cfg(any(test, feature = "tuning-test-controls"))]
            fault: None,
            #[cfg(test)]
            publication_hook: None,
            #[cfg(test)]
            after_read_snapshot: None,
            #[cfg(test)]
            after_previous_read: None,
            _worker_only: PhantomData,
        };
        store.validate_authority()?;
        store.clean_journal(true)?;
        Ok(store)
    }
    fn prepare_open(
        root: PathBuf,
        existing_base: PathBuf,
        policy: SecurityPolicy,
        max_bytes: usize,
    ) -> Result<OpeningContext, TuningNativeError> {
        let policy = Rc::new(policy);
        let descriptor = Rc::new(Descriptor::new(policy.test_sid.as_deref())?);
        let mut parts = root.components();
        let (letter, drive) = match parts.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                std::path::Prefix::Disk(letter) => (letter, PathBuf::from(prefix.as_os_str())),
                _ => {
                    return Err(TuningNativeError::Unsupported(
                        "local drive absolute store required",
                    ));
                }
            },
            _ => {
                return Err(TuningNativeError::Unsupported(
                    "local drive absolute store required",
                ));
            }
        };
        if parts.next() != Some(Component::RootDir) {
            return Err(TuningNativeError::UnsafeStore("absolute store required"));
        }
        let mut path = drive;
        path.push("\\");
        let anchor = if policy.test_sid.is_none() {
            Directory::canonical_bootstrap(letter, Rc::clone(&descriptor), Rc::clone(&policy))?
        } else {
            Directory::bootstrap(&path, Rc::clone(&descriptor), Rc::clone(&policy), None)?
        };
        let root_scope = if policy.test_sid.is_none() {
            Some(false)
        } else {
            None
        };
        let mut directories = vec![PinnedDirectory {
            name: None,
            directory: anchor,
            scope: root_scope,
        }];
        for part in parts {
            let Component::Normal(value) = part else {
                return Err(TuningNativeError::UnsafeStore(
                    "normalized store components required",
                ));
            };
            let name: Vec<_> = value.encode_wide().collect();
            store_io::component(&name)?;
            path.push(value);
            let managed = path != existing_base && path.starts_with(&existing_base);
            let scope = if managed || policy.test_sid.is_none() {
                Some(managed)
            } else {
                None
            };
            let parent = &directories.last().expect("retained volume root").directory;
            let (node, _) = parent.open(
                &name,
                Kind::Directory,
                if managed {
                    OpenMode::OpenOrCreate
                } else {
                    OpenMode::Existing
                },
                Sharing::Retained,
                true,
                scope,
            )?;
            directories.push(PinnedDirectory {
                name: Some(name),
                directory: node.into_directory()?,
                scope,
            });
        }
        let final_root = &directories
            .last()
            .ok_or(TuningNativeError::UnsafeStore("store root missing"))?
            .directory;
        let (lease, _) = final_root.open(
            &units(LEASE_NAME),
            Kind::File,
            OpenMode::OpenOrCreate,
            Sharing::ExclusiveLease,
            false,
            Some(true),
        )?;
        lease.verify()?;
        Ok(OpeningContext {
            lease,
            directories,
            policy,
            max_bytes,
        })
    }
    fn root(&self) -> &Directory {
        &self
            .directories
            .last()
            .expect("retained store root")
            .directory
    }
    fn validate_objects(&self) -> Result<(), TuningNativeError> {
        let mut current = None;
        for (index, entry) in self.directories.iter().enumerate() {
            entry.directory.verify()?;
            if index != 0 {
                let parent = current.as_ref().unwrap_or(&self.directories[0].directory);
                let name = entry
                    .name
                    .as_deref()
                    .ok_or(TuningNativeError::UnsafeStore("store directory name"))?;
                let (node, _) = parent.open(
                    name,
                    Kind::Directory,
                    OpenMode::Existing,
                    Sharing::Retained,
                    true,
                    entry.scope,
                )?;
                if node.id() != entry.directory.id() {
                    return Err(TuningNativeError::UnsafeStore(
                        "directory namespace changed",
                    ));
                }
                current = Some(node.into_directory()?);
            }
        }
        self.lease.verify()?;
        let lease_name = units(LEASE_NAME);
        let mut lease_seen = false;
        for entry in self.root().entries()? {
            if entry.name == lease_name {
                if lease_seen || !self.root().entry_matches(&entry, &self.lease)? {
                    return Err(TuningNativeError::UnsafeStore("lease namespace changed"));
                }
                lease_seen = true;
            }
        }
        if !lease_seen {
            return Err(TuningNativeError::UnsafeStore("lease namespace changed"));
        }
        Ok(())
    }
    fn clean_journal(&self, read_only: bool) -> Result<Option<Node>, TuningNativeError> {
        self.validate_objects()?;
        let journal_name = units(JOURNAL_NAME);
        let lease_name = units(LEASE_NAME);
        let mut journal: Option<Entry> = None;
        let mut lease_seen = false;
        for entry in self.root().entries()? {
            if entry.name == journal_name {
                if journal.replace(entry).is_some() {
                    return Err(artifact_error());
                }
            } else if entry.name == lease_name {
                if lease_seen || !self.root().entry_matches(&entry, &self.lease)? {
                    return Err(artifact_error());
                }
                lease_seen = true;
            } else {
                return Err(artifact_error());
            }
        }
        if !lease_seen {
            return Err(artifact_error());
        }
        let opened = self.root().open(
            &journal_name,
            Kind::File,
            OpenMode::Existing,
            Sharing::Renameable,
            read_only,
            Some(true),
        );
        let result = match (journal, opened) {
            (None, Err(TuningNativeError::Io { code: 2, .. })) => None,
            (Some(entry), Ok((node, _))) => {
                if !self.root().entry_matches(&entry, &node)? {
                    return Err(artifact_error());
                }
                Some(node)
            }
            (_, Err(error)) => return Err(error),
            _ => return Err(artifact_error()),
        };
        self.validate_objects()?;
        Ok(result)
    }
    /// Fresh NAME -> actual retained full ID and full bounded bytes. A retained
    /// handle's self-identity alone never proves its current namespace link.
    fn check_name(
        &self,
        name: &str,
        retained: &Node,
        bytes: &[u8],
    ) -> Result<(), TuningNativeError> {
        self.validate_objects()?;
        retained.verify()?;
        let (mut current, _) = self.root().open(
            &units(name),
            Kind::File,
            OpenMode::Existing,
            Sharing::Renameable,
            true,
            Some(true),
        )?;
        if current.id() != retained.id() || current.read(self.max_bytes)? != bytes {
            return Err(TuningNativeError::UnsafeStore(
                "journal source namespace or bytes changed",
            ));
        }
        retained.verify()?;
        self.validate_objects()
    }
    fn publish_inner(&mut self, bytes: &[u8]) -> Result<(), TuningNativeError> {
        self.validate_objects()?;
        let mut old = self.clean_journal(false)?;
        let old_bytes = match old.as_mut() {
            Some(node) => node.read(self.max_bytes)?,
            None => Vec::new(),
        };
        self.validate_objects()?;
        let (mut stage, created) = self.root().open(
            &units(STAGING_NAME),
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Renameable,
            false,
            Some(true),
        )?;
        if created != Disposition::Created {
            return Err(TuningNativeError::UnsafeStore("store staging disposition"));
        }
        stage.write_synced(bytes, self.max_bytes)?;
        if stage.read(self.max_bytes)? != bytes {
            return Err(TuningNativeError::UnsafeStore("staging bytes changed"));
        }
        self.inject(TuningPublishPoint::AfterStageSync)?;
        if let Some(previous) = old.as_ref() {
            self.check_name(JOURNAL_NAME, previous, &old_bytes)?;
            previous.rename_no_replace(self.root(), &units(BACKUP_NAME))?;
            self.inject(TuningPublishPoint::AfterOldMove)?;
        }
        self.check_name(STAGING_NAME, &stage, bytes)?;
        stage.rename_no_replace(self.root(), &units(JOURNAL_NAME))?;
        self.inject(TuningPublishPoint::AfterPublish)?;
        self.check_name(JOURNAL_NAME, &stage, bytes)?;
        if let Some(mut previous) = old {
            self.inject(TuningPublishPoint::BeforeBackupCleanup)?;
            if previous.read(self.max_bytes)? != old_bytes {
                return Err(TuningNativeError::UnsafeStore("previous bytes changed"));
            }
            #[cfg(test)]
            if let Some(hook) = self.after_previous_read.as_mut() {
                hook();
            }
            // Verify after the complete previous-byte read and its observable
            // boundary. The fresh journal NAME -> stage ID/full-byte proof is
            // the last high-level observation before exact-old-handle cleanup.
            self.check_name(JOURNAL_NAME, &stage, bytes)?;
            previous.delete_owned()?;
            self.inject(TuningPublishPoint::AfterBackupCleanup)?;
        }
        self.clean_journal(true)?;
        self.check_name(JOURNAL_NAME, &stage, bytes)?;
        Ok(())
    }
    fn inject(&mut self, point: TuningPublishPoint) -> Result<(), TuningNativeError> {
        #[cfg(test)]
        if let Some(hook) = self.publication_hook.as_mut() {
            hook(point)?;
        }
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
#[cfg(any(test, feature = "tuning-test-controls"))]
fn test_selection(
    root: &Path,
    max_bytes: usize,
) -> Result<(PathBuf, SecurityPolicy), TuningNativeError> {
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
    Ok((
        base,
        SecurityPolicy {
            test_sid: Some(sid),
        },
    ))
}
fn units(name: &str) -> Vec<u16> {
    name.encode_utf16().collect()
}
fn artifact_error() -> TuningNativeError {
    TuningNativeError::UnsafeStore("pending or unknown journal artifact")
}
#[cfg(test)]
#[path = "store/native_migration_tests.rs"]
mod native_migration_tests;
fn validate_bound(max: usize) -> Result<(), TuningNativeError> {
    if max == 0 || max > MAX_TUNING_JOURNAL_BYTES {
        Err(TuningNativeError::Unsupported("journal byte bound"))
    } else {
        Ok(())
    }
}
pub(super) fn validate_volume(file: &File) -> Result<(), TuningNativeError> {
    let mut flags = 0;
    let mut name = [0_u16; 32];
    unsafe {
        GetVolumeInformationByHandleW(
            HANDLE(file.as_raw_handle()),
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
        store.set_publication_hook(move |point| {
            if point == TuningPublishPoint::BeforeBackupCleanup {
                fs::rename(path.join(BACKUP_NAME), path.join("retained-original"))
                    .expect("move owned previous exact object");
                fs::write(path.join(BACKUP_NAME), b"unrelated replacement").unwrap();
                *observe.lock().unwrap() = true;
            }
            Ok(())
        });
        let result = store.publish(b"new");
        assert!(
            *swapped.lock().unwrap(),
            "real verified boundary permits the owned previous-name substitution"
        );
        assert!(
            result.is_err(),
            "foreign artifact prevents clean publication"
        );
        assert!(
            !root.path().join("retained-original").exists(),
            "only the exact moved original object is cleaned"
        );
        assert_eq!(
            fs::read(root.path().join(BACKUP_NAME)).unwrap(),
            b"unrelated replacement"
        );
        assert_eq!(fs::read(root.path().join(JOURNAL_NAME)).unwrap(), b"new");
        assert_eq!(store.read(), Err(TuningNativeError::Poisoned));
    }
}
