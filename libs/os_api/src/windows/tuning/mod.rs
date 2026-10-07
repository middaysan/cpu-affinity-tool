mod identity;
pub use identity::TuningProcessLifetimeGuard;
#[cfg(any(test, feature = "tuning-test-controls"))]
mod canonical_marker;
#[cfg(any(test, feature = "tuning-test-controls"))]
pub use canonical_marker::{
    TuningCanonicalContinuityCandidate, TuningContinuityCandidateData,
    TuningContinuityCandidateMode, TuningContinuityCandidateObservation,
};
#[cfg(any(test, feature = "tuning-test-controls"))]
mod marker;
#[cfg(test)]
mod relative_io;
#[cfg(test)]
mod relative_publication;
mod security;
mod store;
#[cfg(any(test, feature = "tuning-test-controls"))]
mod test_support;
#[cfg(any(test, feature = "tuning-test-controls"))]
pub use marker::{TuningMarkerNonce, TuningVolatileMarkerFixture, TuningVolatilityEvidence};
pub use store::{TuningPublishPoint, TuningStore};
#[cfg(any(test, feature = "tuning-test-controls"))]
pub use test_support::TuningTemporaryDirectory;

use crate::tuning::TuningNativeError;
pub(super) fn code_error(operation: &'static str, code: u32) -> TuningNativeError {
    match code {
        5 => TuningNativeError::Denied { operation, code },
        32 | 33 => TuningNativeError::Busy,
        _ => TuningNativeError::Io { operation, code },
    }
}
pub(super) fn native_error(
    operation: &'static str,
    error: windows::core::Error,
) -> TuningNativeError {
    let raw = error.code().0 as u32;
    code_error(
        operation,
        if raw & 0xffff0000 == 0x80070000 {
            raw & 0xffff
        } else {
            raw
        },
    )
}
pub(super) fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>, TuningNativeError> {
    use std::os::windows::ffi::OsStrExt;
    let bytes: Vec<u16> = value.encode_wide().collect();
    if bytes.len() > 32760 || bytes.contains(&0) {
        return Err(TuningNativeError::UnsafeStore("invalid native path"));
    }
    Ok(bytes.into_iter().chain([0]).collect())
}
