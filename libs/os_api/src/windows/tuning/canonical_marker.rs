//! Explicit nonshipping protected-continuity candidate. No startup/CLI wiring.
//!
//! Pure shape/decision tests and private-hive characterization cannot issue
//! canonical authority or admit shipping recovery/setting operations.

mod capsule;
mod control;
mod driver;
mod native_context;
mod native_driver;
mod native_key;
mod native_plumbing;
mod native_token;
mod primitives;
mod registry_descriptor;
mod registry_policy;
mod security_descriptor;

use super::store::TuningStore;
use crate::tuning::{TuningNativeError, TuningStoreAuthority};
use std::{marker::PhantomData, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningContinuityCandidateMode {
    ObserveExisting,
    InitializeOnlyWhenCanonicalJournalAbsent,
}

/// Data from an explicitly invoked native candidate; not a shipping permit.
#[derive(Clone, PartialEq, Eq)]
pub struct TuningContinuityCandidateData {
    capsule: capsule::DecodedCapsule,
}
impl TuningContinuityCandidateData {
    pub fn producer(&self) -> &'static str {
        "windows_hklm_software_volatile_marker"
    }
    pub fn version(&self) -> u32 {
        self.capsule.version
    }
    pub fn domain(&self) -> &'static str {
        "cpu_affinity_tool_tuning"
    }
    pub fn view(&self) -> &'static str {
        "native64"
    }
    pub fn nonce(&self) -> [u8; 32] {
        self.capsule.nonce
    }
}
impl std::fmt::Debug for TuningContinuityCandidateData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningContinuityCandidateData { value: redacted }")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuningContinuityCandidateObservation {
    Observed(TuningContinuityCandidateData),
    Unknown,
}

/// Opaque worker-owned candidate borrowing the actual retained canonical lease.
#[cfg_attr(
    feature = "tuning-test-controls",
    doc = r#"
The nonshipping API can be composed without executing it in documentation tests:

```no_run
use os_api::tuning::{TuningStore, TuningNativeError, TuningContinuityCandidateMode};
fn inspect(store: &mut TuningStore) -> Result<(), TuningNativeError> {
    let mut candidate = store.canonical_continuity_candidate(
        TuningContinuityCandidateMode::ObserveExisting)?;
    let _ = candidate.observe()?;
    // Readonly retained context, after a successful native observation. This
    // accessor cannot construct a candidate or confer shipping authority.
    let _machine = candidate.retained_machine_identity();
    Ok(())
}
```

The retained candidate cannot be sent to another worker:

```compile_fail
fn require_send<T: Send>() {}
require_send::<os_api::tuning::TuningCanonicalContinuityCandidate<'static>>();
```

The actual store/lease cannot be dropped while its candidate is still used:

```compile_fail
use os_api::tuning::{TuningStore, TuningNativeError, TuningContinuityCandidateMode};
fn inspect(mut store: TuningStore) -> Result<(), TuningNativeError> {
    let mut candidate = store.canonical_continuity_candidate(
        TuningContinuityCandidateMode::ObserveExisting)?;
    drop(store);
    let _ = candidate.observe()?;
    Ok(())
}
```
"#
)]
pub struct TuningCanonicalContinuityCandidate<'lease> {
    store: &'lease mut TuningStore,
    context: native_context::NativeContext,
    session: driver::DriverSession<native_key::NativeKey>,
    _worker_only: PhantomData<Rc<()>>,
}
impl std::fmt::Debug for TuningCanonicalContinuityCandidate<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningCanonicalContinuityCandidate { native_authority: opaque }")
    }
}
impl TuningCanonicalContinuityCandidate<'_> {
    /// The retained native context tuple. This is not a fresh observation or permit.
    /// Use only after successful observations; no caller identity is accepted.
    #[cfg(feature = "tuning-test-controls")]
    pub fn retained_machine_identity(&self) -> &crate::tuning::TuningMachineIdentity {
        self.context.retained_machine_identity()
    }
    pub fn observe(&mut self) -> Result<TuningContinuityCandidateObservation, TuningNativeError> {
        let mut native = native_driver::NativeDriver {
            store: self.store,
            context: &self.context,
        };
        match self.session.observe(&mut native).map_err(candidate_error)? {
            driver::DriverObservation::ObservedShape(capsule) => {
                // This sole constructor follows actual native engine/context,
                // namespace/security/capsule and positive volatility checks.
                Ok(TuningContinuityCandidateObservation::Observed(
                    TuningContinuityCandidateData { capsule },
                ))
            }
            driver::DriverObservation::Unknown => Ok(TuningContinuityCandidateObservation::Unknown),
        }
    }
}
impl TuningStore {
    pub fn canonical_continuity_candidate(
        &mut self,
        mode: TuningContinuityCandidateMode,
    ) -> Result<TuningCanonicalContinuityCandidate<'_>, TuningNativeError> {
        if self.authority() != TuningStoreAuthority::Canonical {
            return Err(TuningNativeError::UnsafeStore(
                "canonical continuity candidate requires canonical store",
            ));
        }
        let context = native_context::NativeContext::capture(self).map_err(candidate_error)?;
        let session = {
            let mut native = native_driver::NativeDriver {
                store: self,
                context: &context,
            };
            driver::begin(&mut native, mode).map_err(candidate_error)?
        };
        Ok(TuningCanonicalContinuityCandidate {
            store: self,
            context,
            session,
            _worker_only: PhantomData,
        })
    }
}

fn candidate_error(error: driver::DriverFailure) -> TuningNativeError {
    match error {
        driver::DriverFailure::Native(error) => error,
        driver::DriverFailure::Unknown(reason) => TuningNativeError::Unsupported(match reason {
            UnknownReason::MalformedCapsule => "canonical continuity malformed capsule",
            UnknownReason::QueryBound => "canonical continuity query bounds",
            UnknownReason::UnsafeSecurity => "canonical continuity unsafe security",
            UnknownReason::Context => "canonical continuity context unproven",
            UnknownReason::MissingComponent => "canonical continuity missing component",
            UnknownReason::Initialization => "canonical continuity initialization refused",
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UnknownReason {
    MalformedCapsule,
    QueryBound,
    UnsafeSecurity,
    Context,
    MissingComponent,
    Initialization,
}

#[cfg(test)]
#[path = "canonical_marker/candidate_error_tests.rs"]
mod candidate_error_tests;
#[cfg(test)]
#[path = "canonical_marker/capsule_tests.rs"]
mod capsule_tests;
#[cfg(test)]
#[path = "canonical_marker/control_tests.rs"]
mod control_tests;
#[cfg(test)]
#[path = "canonical_marker/creator_owner_native_tests.rs"]
mod creator_owner_native_tests;
#[cfg(test)]
#[path = "canonical_marker/descriptor_tests.rs"]
mod descriptor_tests;
#[cfg(test)]
#[path = "canonical_marker/driver_tests.rs"]
mod driver_tests;
#[cfg(test)]
#[path = "canonical_marker/fixture_tests.rs"]
mod fixture_tests;
#[cfg(test)]
#[path = "canonical_marker/native_key_tests.rs"]
mod native_key_tests;
#[cfg(test)]
#[path = "canonical_marker/native_plumbing_tests.rs"]
mod native_plumbing_tests;
#[cfg(test)]
#[path = "canonical_marker/native_token_tests.rs"]
mod native_token_tests;
#[cfg(test)]
#[path = "canonical_marker/policy_tests.rs"]
mod policy_tests;
#[cfg(test)]
#[path = "canonical_marker/primitive_tests.rs"]
mod primitive_tests;
