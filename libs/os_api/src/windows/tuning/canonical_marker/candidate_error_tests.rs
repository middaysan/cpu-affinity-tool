//! Pure diagnostic mapping only; no native authority or candidate execution.
use super::*;

fn require_unknown_mapping(reason: UnknownReason, expected: &'static str) {
    assert_eq!(
        candidate_error(driver::DriverFailure::Unknown(reason)),
        TuningNativeError::Unsupported(expected)
    );
}

#[test]
fn malformed_capsule_keeps_its_fixed_diagnostic_reason() {
    require_unknown_mapping(
        UnknownReason::MalformedCapsule,
        "canonical continuity malformed capsule",
    );
}
#[test]
fn query_bound_keeps_its_fixed_diagnostic_reason() {
    require_unknown_mapping(
        UnknownReason::QueryBound,
        "canonical continuity query bounds",
    );
}
#[test]
fn unsafe_security_keeps_its_fixed_diagnostic_reason() {
    require_unknown_mapping(
        UnknownReason::UnsafeSecurity,
        "canonical continuity unsafe security",
    );
}
#[test]
fn context_keeps_its_fixed_diagnostic_reason() {
    require_unknown_mapping(
        UnknownReason::Context,
        "canonical continuity context unproven",
    );
}
#[test]
fn missing_component_keeps_its_fixed_diagnostic_reason() {
    require_unknown_mapping(
        UnknownReason::MissingComponent,
        "canonical continuity missing component",
    );
}
#[test]
fn initialization_keeps_its_fixed_diagnostic_reason() {
    require_unknown_mapping(
        UnknownReason::Initialization,
        "canonical continuity initialization refused",
    );
}
#[test]
fn native_errors_preserve_exact_class_reason_and_code() {
    for error in [
        TuningNativeError::Busy,
        TuningNativeError::Unsupported("unrecognized native producer"),
        TuningNativeError::UnsafeStore("fixed native refusal"),
        TuningNativeError::Poisoned,
        TuningNativeError::Capacity {
            size: 4194305,
            limit: 4194304,
        },
        TuningNativeError::Denied {
            operation: "fixed denied operation",
            code: 5,
        },
        TuningNativeError::Io {
            operation: "fixed I/O operation",
            code: 1018,
        },
    ] {
        assert_eq!(
            candidate_error(driver::DriverFailure::Native(error.clone())),
            error
        );
    }
}
