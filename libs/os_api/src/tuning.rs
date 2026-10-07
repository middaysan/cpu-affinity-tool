//! Narrow recovery-storage and identity boundary. No gaming-setting writers.

pub const MAX_TUNING_JOURNAL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TUNING_MACHINE_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuningNativeError {
    Busy,
    Unsupported(&'static str),
    UnsafeStore(&'static str),
    Poisoned,
    Capacity { size: u64, limit: usize },
    Denied { operation: &'static str, code: u32 },
    Io { operation: &'static str, code: u32 },
}

impl std::fmt::Display for TuningNativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TuningNativeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningMachineSource {
    Tpm,
    Uefi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningMachineDomain {
    UnpackagedWin32,
}

#[derive(Clone, PartialEq, Eq)]
pub struct TuningMachineIdentity {
    pub(super) source: TuningMachineSource,
    pub(super) bytes: Vec<u8>,
}
impl TuningMachineIdentity {
    pub fn source(&self) -> TuningMachineSource {
        self.source
    }
    pub fn domain(&self) -> TuningMachineDomain {
        TuningMachineDomain::UnpackagedWin32
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl std::fmt::Debug for TuningMachineIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuningMachineIdentity")
            .field("source", &self.source)
            .field("byte_length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TuningTokenIdentity {
    pub(super) sid: String,
    pub(super) session_id: u32,
    pub(super) authentication_id: u64,
}
impl TuningTokenIdentity {
    pub fn sid(&self) -> &str {
        &self.sid
    }
    pub fn session_id(&self) -> u32 {
        self.session_id
    }
    pub fn authentication_id(&self) -> u64 {
        self.authentication_id
    }
}
impl std::fmt::Debug for TuningTokenIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningTokenIdentity { provenance: redacted }")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TuningProcessIdentity {
    pub(super) pid: u32,
    pub(super) creation_time: u64,
    pub(super) image_path: String,
    pub(super) token: TuningTokenIdentity,
}
impl TuningProcessIdentity {
    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn creation_time(&self) -> u64 {
        self.creation_time
    }
    pub fn image_path(&self) -> &str {
        &self.image_path
    }
    pub fn token(&self) -> &TuningTokenIdentity {
        &self.token
    }
}
impl std::fmt::Debug for TuningProcessIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuningProcessIdentity")
            .field("pid", &self.pid)
            .field("provenance", &"redacted")
            .finish()
    }
}

/// A result for one retained, initially verified native process object.
/// It does not establish the lifetime of its logon or any projected resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningProcessLifetime {
    VerifiedLive,
    PositivelyExited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningBootContinuity {
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningStoreAuthority {
    Canonical,
    #[cfg(any(test, feature = "tuning-test-controls"))]
    IsolatedTest,
}

#[cfg(all(windows, any(test, feature = "tuning-test-controls")))]
pub use crate::windows::tuning::{
    TuningLabArtifact, TuningLabArtifactKind, TuningLabCreation, TuningLabObjectId,
    TuningLabSnapshot,
};
#[cfg(windows)]
pub use crate::windows::tuning::{TuningProcessLifetimeGuard, TuningPublishPoint, TuningStore};

#[cfg(all(windows, any(test, feature = "tuning-test-controls")))]
pub use crate::windows::tuning::TuningTemporaryDirectory;
#[cfg(all(windows, any(test, feature = "tuning-test-controls")))]
pub use crate::windows::tuning::{
    TuningCanonicalContinuityCandidate, TuningContinuityCandidateData,
    TuningContinuityCandidateMode, TuningContinuityCandidateObservation,
};
#[cfg(all(windows, any(test, feature = "tuning-test-controls")))]
pub use crate::windows::tuning::{
    TuningMarkerNonce, TuningVolatileMarkerFixture, TuningVolatilityEvidence,
};
