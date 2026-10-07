//! Isolated publication protocol characterization. No canonical authority.
//! Fixed no-replace protocol, bounded observations and retained worker objects.
use super::{
    relative_io::{CONTENT_BYTES, Directory, Kind, Node, OpenMode, RelativeName, Sharing},
    test_support::TuningTemporaryDirectory,
};
use crate::tuning::TuningNativeError;
use std::{fmt, path::Path};

pub(super) type Result<T> = std::result::Result<T, TuningNativeError>;
pub(super) const JOURNAL: &str = "journal-v1.json";
pub(super) const STAGE: &str = "journal-v1.staging";
pub(super) const PREVIOUS: &str = "journal-v1.previous";
pub(super) const LEASE: &str = "writer-v1.lock";
pub(super) const CAPACITY: usize = CONTENT_BYTES;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Read {
    Absent,
    Present(Vec<u8>),
}
impl fmt::Debug for Read {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("PublicationRead::Absent"),
            Self::Present(_) => f.write_str("PublicationRead::Present(..)"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Boundary {
    StageSynced,
    OldMoved,
    NewMoved,
    Verified,
    OldDeleted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Old,
    Stage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnerState {
    Ready,
    Poisoned,
    Unresolved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Retention {
    Terminal,
    Unresolved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CleanState {
    Absent,
    Present,
}

/// Pure shape only. The private native reader obtains every fact by bounded
/// relative enumeration/open; this helper produces no native admission.
pub(super) fn clean_state(
    journal: bool,
    stage: bool,
    previous: bool,
    unknown: bool,
) -> Result<CleanState> {
    if stage || previous || unknown {
        return Err(TuningNativeError::UnsafeStore(
            "relative publication artifacts",
        ));
    }
    Ok(if journal {
        CleanState::Present
    } else {
        CleanState::Absent
    })
}
pub(super) fn retention_for(error: &TuningNativeError) -> Retention {
    if matches!(
        error,
        TuningNativeError::UnsafeStore("relative pending completion")
    ) {
        Retention::Unresolved
    } else {
        Retention::Terminal
    }
}
/// An unresolved request never becomes terminal because a later call is refused.
pub(super) fn latch_retention(previous: Retention, error: &TuningNativeError) -> Retention {
    if previous == Retention::Unresolved {
        Retention::Unresolved
    } else {
        retention_for(error)
    }
}
/// Shared ownership disposal seam, including partly constructed contexts.
/// Unexpected pending retains all authority and native-used objects until the
/// owning process terminates. This test-only policy never grants admission.
pub(super) fn dispose_context<T>(owned: Box<T>, retention: Retention) {
    match retention {
        Retention::Terminal => drop(owned),
        Retention::Unresolved => {
            let _retained = Box::into_raw(owned);
        }
    }
}

/// Private fixed-operation seam. The native adapter borrows the actual owner
/// context: all old/stage handles and expected bytes remain owned there. Fakes
/// can provide only traces/shapes, never handles or canonical evidence.
pub(super) trait PublicationDriver {
    fn check_clean(&mut self) -> Result<()>;
    fn authenticate_old(&mut self) -> Result<bool>;
    fn create_stage(&mut self) -> Result<()>;
    fn write_sync_stage(&mut self, bytes: &[u8]) -> Result<()>;
    /// Reopen the fixed source NAME and compare complete bytes and actual ID
    /// with its retained handle immediately before the corresponding rename.
    fn check_source_name(&mut self, source: Source) -> Result<()>;
    fn rename_old_to_previous(&mut self) -> Result<()>;
    fn rename_stage_to_journal(&mut self) -> Result<()>;
    /// Reopen journal NAME and compare it with stage ID/full expected bytes;
    /// called after publication and immediately before old-object cleanup.
    fn verify_new_name(&mut self) -> Result<()>;
    fn delete_old_handle(&mut self) -> Result<()>;
    fn boundary(&mut self, boundary: Boundary) -> Result<()>;
}
pub(super) fn protocol<D: PublicationDriver>(driver: &mut D, bytes: &[u8]) -> Result<()> {
    if bytes.len() > CAPACITY {
        return Err(TuningNativeError::Capacity {
            size: bytes.len() as u64,
            limit: CAPACITY,
        });
    }
    driver.check_clean()?;
    let old = driver.authenticate_old()?;
    driver.create_stage()?;
    driver.write_sync_stage(bytes)?;
    driver.boundary(Boundary::StageSynced)?;
    if old {
        driver.check_source_name(Source::Old)?;
        driver.rename_old_to_previous()?;
        driver.boundary(Boundary::OldMoved)?;
    }
    driver.check_source_name(Source::Stage)?;
    driver.rename_stage_to_journal()?;
    driver.boundary(Boundary::NewMoved)?;
    driver.verify_new_name()?;
    driver.boundary(Boundary::Verified)?;
    if old {
        driver.verify_new_name()?;
        driver.delete_old_handle()?;
        driver.boundary(Boundary::OldDeleted)?;
    }
    driver.check_clean()
}

type BoundaryHook = Box<dyn FnMut(Boundary) -> Result<()>>;
struct Context {
    // Declaration order is terminal-drop order: in-flight Nodes/hooks close
    // before the lease and root, so their lifetime remains inside that lease.
    old: Option<Node>,
    stage: Option<Node>,
    old_bytes: Vec<u8>,
    expected: Vec<u8>,
    poisoned: bool,
    retention: Retention,
    hook: Option<BoundaryHook>,
    lease: Option<Node>,
    root: Directory,
}
pub(super) struct PublicationOwner {
    context: Option<Box<Context>>,
}
impl fmt::Debug for PublicationOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PublicationOwner(..)")
    }
}
impl PublicationOwner {
    /// Genuine existing parent-owned root plus exclusive worker-held lease.
    pub(super) fn fixture(root: &TuningTemporaryDirectory) -> Result<Self> {
        Self::from_directory(Directory::fixture(root)?)
    }
    /// Parent supplies its actual owned TempRoot's path and creation identity.
    /// Child borrows this fixture and never cleans its parent-owned directory.
    pub(super) fn fixture_observed_root(path: &Path, id: (u32, u64)) -> Result<Self> {
        Self::from_directory(Directory::fixture_observed_root(path, id)?)
    }
    fn from_directory(root: Directory) -> Result<Self> {
        let mut context = Box::new(Context {
            root,
            lease: None,
            old: None,
            stage: None,
            old_bytes: Vec::new(),
            expected: Vec::new(),
            poisoned: false,
            retention: Retention::Terminal,
            hook: None,
        });
        let name = RelativeName::new(LEASE)?;
        match context.root.open(
            &name,
            Kind::File,
            OpenMode::OpenOrCreate,
            Sharing::ExclusiveLease,
        ) {
            Ok(lease) => {
                context.lease = Some(lease);
                Ok(Self {
                    context: Some(context),
                })
            }
            Err(error) => {
                dispose_context(context, retention_for(&error));
                Err(error)
            }
        }
    }
    pub(super) fn read(&mut self) -> Result<Read> {
        let result = self
            .context
            .as_mut()
            .expect("owned publication context")
            .read_clean();
        result.map_err(|error| self.record_failure(error))
    }
    pub(super) fn publish(&mut self, bytes: &[u8]) -> Result<()> {
        let context = self.context.as_mut().expect("owned publication context");
        if context.poisoned {
            return Err(TuningNativeError::Poisoned);
        }
        if bytes.len() > CAPACITY {
            return Err(TuningNativeError::Capacity {
                size: bytes.len() as u64,
                limit: CAPACITY,
            });
        }
        // Last successful stage proof refers to the current journal stream.
        // Close it before acquiring the next old HANDLE, so exact old deletion
        // cannot be delayed by a stale handle from the previous publication.
        context.old = None;
        context.stage = None;
        context.old_bytes.clear();
        context.expected.clear();
        let result = protocol(&mut NativeDriver { context }, bytes);
        result.map_err(|error| self.record_failure(error))
    }
    pub(super) fn set_boundary_hook<F>(&mut self, hook: F)
    where
        F: FnMut(Boundary) -> Result<()> + 'static,
    {
        self.context
            .as_mut()
            .expect("owned publication context")
            .hook = Some(Box::new(hook));
    }
    pub(super) fn state(&self) -> OwnerState {
        let context = self.context.as_ref().expect("owned publication context");
        if context.retention == Retention::Unresolved {
            OwnerState::Unresolved
        } else if context.poisoned {
            OwnerState::Poisoned
        } else {
            OwnerState::Ready
        }
    }
    /// Synthetic fixed failure after genuine lease acquisition; no actual SDK
    /// pending is requested or claimed. Uses the native error ownership path.
    pub(super) fn inject_pending_after_lease_for_test(&mut self) -> Result<()> {
        Err(self.record_failure(TuningNativeError::UnsafeStore(
            "relative pending completion",
        )))
    }
    fn record_failure(&mut self, error: TuningNativeError) -> TuningNativeError {
        let context = self.context.as_mut().expect("owned publication context");
        context.poisoned = true;
        context.retention = latch_retention(context.retention, &error);
        error
    }
}
impl Drop for PublicationOwner {
    fn drop(&mut self) {
        if let Some(context) = self.context.take() {
            let retention = context.retention;
            dispose_context(context, retention);
        }
    }
}

fn artifacts_error() -> TuningNativeError {
    TuningNativeError::UnsafeStore("relative publication artifacts")
}
fn source_error() -> TuningNativeError {
    TuningNativeError::UnsafeStore("relative publication source changed")
}
impl Context {
    fn verify_authority(&self) -> Result<()> {
        if self.poisoned {
            return Err(TuningNativeError::Poisoned);
        }
        self.root.as_node().verify()?;
        self.lease.as_ref().ok_or_else(artifacts_error)?.verify()
    }
    /// Enumerate all normal entries, refusing duplicates/unknowns. The actual
    /// lease must be named in this root and match its held ID. Enumeration is
    /// bounded observation, not an atomic absence or directory snapshot proof.
    fn clean_journal(&mut self, writable: bool) -> Result<Option<Node>> {
        self.verify_authority()?;
        let journal_name = RelativeName::new(JOURNAL)?;
        let lease_name = RelativeName::new(LEASE)?;
        let stage_name = RelativeName::new(STAGE)?;
        let previous_name = RelativeName::new(PREVIOUS)?;
        let mut journal = None;
        let mut lease_seen = false;
        let mut stage_seen = false;
        let mut previous_seen = false;
        let mut unknown = false;
        for entry in self.root.enumerate()? {
            if entry.name == journal_name {
                if journal.replace(entry).is_some() {
                    unknown = true;
                }
            } else if entry.name == lease_name {
                if lease_seen
                    || !self
                        .root
                        .entry_matches(&entry, self.lease.as_ref().ok_or_else(artifacts_error)?)?
                {
                    unknown = true;
                }
                lease_seen = true;
            } else if entry.name == stage_name {
                stage_seen = true;
            } else if entry.name == previous_name {
                previous_seen = true;
            } else {
                unknown = true;
            }
        }
        if !lease_seen {
            return Err(artifacts_error());
        }
        let state = clean_state(journal.is_some(), stage_seen, previous_seen, unknown)?;
        let opened = if writable {
            self.root.open(
                &journal_name,
                Kind::File,
                OpenMode::Existing,
                Sharing::Renameable,
            )
        } else {
            self.root.open_read_only_file(&journal_name)
        };
        let node = match (state, opened) {
            (CleanState::Absent, Err(TuningNativeError::Io { code: 2, .. })) => None,
            (CleanState::Absent, Ok(_)) => return Err(artifacts_error()),
            (CleanState::Present, Ok(node)) => {
                if !self
                    .root
                    .entry_matches(journal.as_ref().ok_or_else(artifacts_error)?, &node)?
                {
                    return Err(artifacts_error());
                }
                Some(node)
            }
            (CleanState::Present, Err(TuningNativeError::Io { code: 2, .. })) => {
                return Err(artifacts_error());
            }
            (_, Err(error)) => return Err(error),
        };
        self.verify_authority()?;
        Ok(node)
    }
    fn read_clean(&mut self) -> Result<Read> {
        let result = match self.clean_journal(false)? {
            Some(mut journal) => Read::Present(journal.read()?),
            None => Read::Absent,
        };
        self.verify_authority()?;
        Ok(result)
    }
    fn check_source_name(&mut self, source: Source) -> Result<()> {
        self.verify_authority()?;
        let (name, retained, expected) = match source {
            Source::Old => (JOURNAL, self.old.as_ref(), &self.old_bytes),
            Source::Stage => (STAGE, self.stage.as_ref(), &self.expected),
        };
        let retained = retained.ok_or_else(source_error)?;
        retained.verify()?;
        let mut current = self.root.open_read_only_file(&RelativeName::new(name)?)?;
        if current.id() != retained.id() || current.read()? != *expected {
            return Err(source_error());
        }
        retained.verify()?;
        self.verify_authority()
    }
    fn verify_new_name(&mut self) -> Result<()> {
        self.verify_authority()?;
        let stage = self.stage.as_ref().ok_or_else(source_error)?;
        stage.verify()?;
        let mut current = self
            .root
            .open_read_only_file(&RelativeName::new(JOURNAL)?)?;
        if current.id() != stage.id() || current.read()? != self.expected {
            return Err(source_error());
        }
        stage.verify()?;
        self.verify_authority()
    }
}
struct NativeDriver<'a> {
    context: &'a mut Context,
}
impl PublicationDriver for NativeDriver<'_> {
    fn check_clean(&mut self) -> Result<()> {
        self.context.read_clean()?;
        if self.context.stage.is_some() {
            self.context.verify_new_name()?;
        }
        Ok(())
    }
    fn authenticate_old(&mut self) -> Result<bool> {
        if let Some(mut old) = self.context.clean_journal(true)? {
            let bytes = old.read()?;
            self.context.old = Some(old);
            self.context.old_bytes = bytes;
        }
        self.context.verify_authority()?;
        Ok(self.context.old.is_some())
    }
    fn create_stage(&mut self) -> Result<()> {
        self.context.verify_authority()?;
        let stage = self.context.root.open(
            &RelativeName::new(STAGE)?,
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Renameable,
        )?;
        // Adopt the real SDK-created object into the entire retained context
        // before any subsequent operation can return an unresolved result.
        self.context.stage = Some(stage);
        self.context.verify_authority()
    }
    fn write_sync_stage(&mut self, bytes: &[u8]) -> Result<()> {
        self.context.verify_authority()?;
        self.context.expected = bytes.to_vec();
        let stage = self.context.stage.as_mut().ok_or_else(source_error)?;
        stage.write(bytes)?;
        if stage.read()? != bytes {
            return Err(source_error());
        }
        self.context.verify_authority()
    }
    fn check_source_name(&mut self, source: Source) -> Result<()> {
        self.context.check_source_name(source)
    }
    fn rename_old_to_previous(&mut self) -> Result<()> {
        self.context.verify_authority()?;
        self.context
            .old
            .as_ref()
            .ok_or_else(source_error)?
            .rename_no_replace(&self.context.root, &RelativeName::new(PREVIOUS)?)?;
        self.context.verify_authority()
    }
    fn rename_stage_to_journal(&mut self) -> Result<()> {
        self.context.verify_authority()?;
        self.context
            .stage
            .as_ref()
            .ok_or_else(source_error)?
            .rename_no_replace(&self.context.root, &RelativeName::new(JOURNAL)?)?;
        self.context.verify_authority()
    }
    fn verify_new_name(&mut self) -> Result<()> {
        self.context.verify_new_name()
    }
    fn delete_old_handle(&mut self) -> Result<()> {
        self.context.verify_authority()?;
        self.context
            .old
            .take()
            .ok_or_else(source_error)?
            .delete_owned()?;
        self.context.verify_authority()
    }
    fn boundary(&mut self, boundary: Boundary) -> Result<()> {
        self.context.verify_authority()?;
        if let Some(hook) = self.context.hook.as_mut() {
            hook(boundary)?;
        }
        self.context.verify_authority()
    }
}

#[path = "relative_publication/native_tests.rs"]
mod native_tests;
#[path = "relative_publication/pure_tests.rs"]
mod pure_tests;
