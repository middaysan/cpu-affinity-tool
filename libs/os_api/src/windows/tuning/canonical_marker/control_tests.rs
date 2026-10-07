use super::{TuningContinuityCandidateMode as Mode, control::*};

fn fresh() -> MarkerFacts {
    MarkerFacts {
        disposition: Disposition::New,
        security_verified: true,
        namespace_verified: true,
        values: 0,
        subkeys: 0,
    }
}
#[test]
fn observe_missing_chain_opens_without_create_or_set() {
    for component in [
        Component::Software,
        Component::Application,
        Component::Tuning,
        Component::Marker,
    ] {
        assert!(missing_action(Mode::ObserveExisting, component, true).is_err());
        assert!(
            missing_action(
                Mode::InitializeOnlyWhenCanonicalJournalAbsent,
                component,
                false
            )
            .is_err()
        );
    }
    let mode = Mode::InitializeOnlyWhenCanonicalJournalAbsent;
    assert!(missing_action(mode, Component::Software, true).is_err());
    assert_eq!(
        missing_action(mode, Component::Application, true),
        Ok(MissingAction::CreateSingleNonvolatile)
    );
    assert_eq!(
        missing_action(mode, Component::Tuning, true),
        Ok(MissingAction::CreateSingleNonvolatile)
    );
    assert_eq!(
        missing_action(mode, Component::Marker, true),
        Ok(MissingAction::CreateSingleVolatile)
    );
}
#[test]
fn fresh_init_validates_handle_and_namespace_before_only_set() {
    let mode = Mode::InitializeOnlyWhenCanonicalJournalAbsent;
    assert_eq!(may_set_new_capsule(mode, true, fresh()), Ok(true));
    assert!(may_set_new_capsule(Mode::ObserveExisting, true, fresh()).is_err());
    assert!(may_set_new_capsule(mode, false, fresh()).is_err());
    for alteration in 0..4 {
        let mut facts = fresh();
        match alteration {
            0 => facts.security_verified = false,
            1 => facts.namespace_verified = false,
            2 => facts.values = 1,
            _ => facts.subkeys = 1,
        }
        assert!(may_set_new_capsule(mode, true, facts).is_err());
    }
}
#[test]
fn existing_disposition_never_reinitializes_unknown_capsule() {
    for mode in [
        Mode::ObserveExisting,
        Mode::InitializeOnlyWhenCanonicalJournalAbsent,
    ] {
        for values in [0, 1, 2, u32::MAX] {
            let mut facts = fresh();
            facts.disposition = Disposition::Existing;
            facts.values = values;
            assert_eq!(may_set_new_capsule(mode, true, facts), Ok(false));
        }
    }
}
#[test]
fn failed_probe_artifacts_block_later_launch_probes() {
    let mut existing = fresh();
    existing.disposition = Disposition::Existing;
    existing.values = 1;
    assert!(may_probe(existing, true, false));
    assert_eq!(probe_result(1021), Volatility::Positive);
    for code in [0, 2, 5, 87, 1018, 1722, u32::MAX] {
        assert_eq!(probe_result(code), Volatility::Unknown);
    }
    assert!(!may_probe(existing, true, true));
    assert!(!may_probe(existing, false, false));
    // A successful nonvolatile child creation is retained by the fixture;
    // the later guard sees its count and cannot create a second artifact.
    for count in [1, 16, u32::MAX] {
        let mut reopened = existing;
        reopened.subkeys = count;
        assert!(!may_probe(reopened, true, false));
    }
    let mut extra_value = existing;
    extra_value.values = 2;
    assert!(!may_probe(extra_value, true, false));
}
#[test]
fn standard_unknown_fixture_or_non64_context_has_zero_registry_calls() {
    let valid = ContextFacts {
        native64_build: true,
        canonical_lease: true,
        elevated: true,
        high_or_system_integrity: true,
        exact_machine_and_token: true,
    };
    assert_eq!(admit_context(valid), Ok(()));
    for alteration in 0..5 {
        let mut denied = valid;
        match alteration {
            0 => denied.native64_build = false,
            1 => denied.canonical_lease = false,
            2 => denied.elevated = false,
            3 => denied.high_or_system_integrity = false,
            _ => denied.exact_machine_and_token = false,
        }
        // The future native driver must call this gate before any registry API;
        // these are decision facts, not actual elevation or fixture authority.
        assert!(admit_context(denied).is_err());
    }
}
