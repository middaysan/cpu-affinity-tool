mod app_flow;
pub(in crate::app) mod editors;
pub(super) mod scalar;
pub(super) mod user_setting;
pub(super) mod value_setting;

use super::platform::catalog::{Catalog, CatalogError};
use super::platform::codec::{CodecError, EnvelopeKind, ModuleEnvelope};
use super::platform::contract::*;
use super::Tuning;
use scalar::{ScalarConfig, ScalarModule};
use user_setting::{UserConfig, UserModule};

fn machine() -> MachineIdentity {
    MachineIdentity {
        id: MachineId::new([1; 16]).unwrap(),
    }
}

fn machine_target() -> TargetIdentity {
    TargetIdentity::Machine(machine())
}

fn session() -> SessionIdentity {
    SessionIdentity {
        user: UserIdentity {
            machine: machine(),
            sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
        },
        session_id: 1,
        logon: LogonId::new(17).unwrap(),
        boot: BootId::new([2; 16]).unwrap(),
    }
}

fn catalog() -> (Catalog, ScalarModule, UserModule) {
    let scalar = ScalarModule::new(7);
    let user = UserModule::new(None, true);
    let mut catalog = Catalog::default();
    catalog.register(scalar.clone()).unwrap();
    catalog.register(user.clone()).unwrap();
    (catalog, scalar, user)
}

#[test]
fn stable_module_ids_reject_aliases_empty_and_unbounded_names() {
    assert!(FeatureId::new("power_plan").is_ok());
    for invalid in [
        "",
        "PowerPlan",
        "power-plan",
        " power_plan",
        "cpu sets",
        "1plan",
    ] {
        assert!(FeatureId::new(invalid).is_err(), "accepted {invalid:?}");
    }
    assert!(FeatureId::new("x".repeat(65)).is_err());
}

#[test]
fn identity_deserialization_cannot_bypass_validation() {
    assert!(serde_json::from_str::<FeatureId>("\"PowerPlan\"").is_err());
    assert!(serde_json::from_str::<ProfileId>("0").is_err());
    assert!(serde_json::from_str::<SchemaVersion>("0").is_err());
    assert!(UserSid::new("S-1-5-21").is_ok());
    assert!(UserSid::new("S-1-5-4294967296").is_err());
    assert!(ImagePath::new("game.exe").is_err());
    assert!(ImagePath::new(r"C:\Games\..\game.exe").is_err());
    assert!(ImagePath::new(r"C:\Games\game.exe").is_ok());
    assert!(MachineId::new([0; 16]).is_err());
    assert!(BootId::new([0; 16]).is_err());
}

#[test]
fn logon_boot_and_process_incarnation_are_part_of_target_identity() {
    let original = session();
    let mut relogon = original.clone();
    relogon.logon = LogonId::new(18).unwrap();
    let mut reboot = original.clone();
    reboot.boot = BootId::new([3; 16]).unwrap();
    assert_ne!(original, relogon);
    assert_ne!(original, reboot);
    let process = ProcessIdentity::new(
        original,
        42,
        123,
        ImagePath::new(r"C:\Games\game.exe").unwrap(),
    )
    .unwrap();
    let reused = ProcessIdentity::new(
        session(),
        42,
        124,
        ImagePath::new(r"C:\Games\game.exe").unwrap(),
    )
    .unwrap();
    assert_ne!(process, reused);
    assert!(ProcessIdentity::new(
        session(),
        0,
        123,
        ImagePath::new(r"C:\Games\game.exe").unwrap()
    )
    .is_err());
    assert!(ProcessIdentity::new(
        session(),
        42,
        0,
        ImagePath::new(r"C:\Games\game.exe").unwrap()
    )
    .is_err());
}

#[test]
fn catalog_rejects_duplicate_ids_and_invalid_descriptor_shapes() {
    let (mut catalog, scalar, _) = catalog();
    assert!(matches!(
        catalog.register(scalar),
        Err(CatalogError::DuplicateFeature(_))
    ));
    for descriptor in scalar::invalid_descriptors() {
        assert!(matches!(
            Catalog::default().register(ScalarModule::with_descriptor(descriptor)),
            Err(CatalogError::InvalidDescriptor(_))
        ));
    }
}

#[test]
fn production_catalog_is_empty_and_management_defaults_unmanaged() {
    assert_eq!(Tuning::default().catalog().descriptors().count(), 0);
    assert_eq!(ManagementChoice::default(), ManagementChoice::Unmanaged);
}

#[test]
fn codecs_roundtrip_and_preserve_unknown_envelopes_without_activation() {
    let (catalog, scalar, _) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let bytes = config.to_bytes().unwrap();
    assert_eq!(ModuleEnvelope::from_bytes(&bytes).unwrap(), config);
    catalog.validate_config(&config).unwrap();
    let mut unknown = config.clone();
    unknown.version = SchemaVersion::new(99).unwrap();
    let preserved = unknown.clone();
    assert!(matches!(
        catalog.validate_config(&unknown),
        Err(CatalogError::Codec(CodecError::UnsupportedVersion { .. }))
    ));
    assert_eq!(unknown, preserved);
    assert_eq!(scalar.writes(), 0);
    let mut wrong_kind = config.clone();
    wrong_kind.kind = EnvelopeKind::Snapshot;
    assert!(catalog.validate_config(&wrong_kind).is_err());
    let mut wrong_feature = config;
    wrong_feature.feature = FeatureId::new("future_module").unwrap();
    assert!(matches!(
        catalog.validate_config(&wrong_feature),
        Err(CatalogError::UnknownFeature(_))
    ));
}

#[test]
fn codec_limits_and_typed_config_validation_reject_untrusted_inputs() {
    assert!(ModuleEnvelope::from_bytes(&vec![b' '; 256 * 1024 + 1]).is_err());
    let (catalog, scalar, _) = catalog();
    let mut config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    config.payload = serde_json::json!({"value":12,"unexpected":true});
    assert!(catalog.validate_config(&config).is_err());
    config.payload = serde_json::json!({"value":999});
    assert!(matches!(
        catalog.validate_config(&config),
        Err(CatalogError::Module(_))
    ));
    let mut deep = serde_json::json!(null);
    for _ in 0..40 {
        deep = serde_json::json!([deep]);
    }
    config.payload = deep;
    assert!(config.to_bytes().is_err());
    assert_eq!(scalar.writes(), 0);
}

#[test]
fn unsupported_denied_scope_and_activation_short_circuit_backend_work() {
    let (catalog, scalar, _) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    for capability in [
        Capability::Unsupported("test build".into()),
        Capability::Denied("test context".into()),
    ] {
        scalar.set_capability(capability);
        assert!(catalog
            .prepare(&config, machine_target(), ActivationMode::Session)
            .is_err());
    }
    assert_eq!(scalar.reads(), 0);
    assert_eq!(scalar.writes(), 0);
    scalar.set_capability(Capability::Available);
    assert!(catalog
        .prepare(
            &config,
            TargetIdentity::Session(session()),
            ActivationMode::Session
        )
        .is_err());
    assert!(catalog
        .prepare(&config, machine_target(), ActivationMode::ForegroundSession)
        .is_err());
    assert_eq!(scalar.reads(), 0);
}

#[test]
fn both_module_shapes_prepare_classify_verify_and_restore_through_same_catalog() {
    let (catalog, scalar, user) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let operation = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    assert_eq!(operation.step_count(), 1);
    assert_eq!(operation.classify().unwrap(), StateMatch::Baseline);
    assert_eq!(scalar.writes(), 0, "prepare must be read-only");
    let snapshot = operation.snapshot_envelope().unwrap();
    operation.execute_step(0).unwrap();
    assert_eq!(
        operation.verify_step(0).unwrap(),
        Verification::Verified(Effect::Immediate)
    );
    assert_eq!(operation.classify().unwrap(), StateMatch::WrittenStep(0));
    let restore = catalog
        .prepare_restore(&snapshot, machine_target())
        .unwrap();
    restore.execute_step(0).unwrap();
    restore.verify_step(0).unwrap();
    assert_eq!(scalar.value(), 7);

    let target = TargetIdentity::Session(session());
    let config = user
        .config_envelope(&UserConfig {
            enabled: false,
            value: 9,
        })
        .unwrap();
    let operation = catalog
        .prepare(&config, target.clone(), ActivationMode::Manual)
        .unwrap();
    assert_eq!(operation.step_count(), 2);
    assert_eq!(operation.effect(), Effect::RestartRequired);
    assert_eq!(user.writes(), 0);
    let snapshot = operation.snapshot_envelope().unwrap();
    operation.execute_step(0).unwrap();
    assert_eq!(operation.classify().unwrap(), StateMatch::WrittenStep(0));
    let restore = catalog.prepare_restore(&snapshot, target).unwrap();
    assert_eq!(
        restore.step_count(),
        1,
        "compensate only the known first write"
    );
    restore.execute_step(0).unwrap();
    assert_eq!(
        restore.verify_step(0).unwrap(),
        Verification::Verified(Effect::RestartRequired)
    );
    assert_eq!(
        user.value(),
        None,
        "original absence is restored, not a default"
    );
    assert!(user.enabled(), "untouched second field stays intact");
}

#[test]
fn sharing_comparison_uses_typed_requests_target_and_first_baseline() {
    let (catalog, scalar, _) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let first = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    let same = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    assert!(first.same_request(&same));
    assert!(first.same_snapshot(&same));
    let different = scalar.config_envelope(&ScalarConfig { value: 13 }).unwrap();
    let different = catalog
        .prepare(&different, machine_target(), ActivationMode::Session)
        .unwrap();
    assert!(!first.same_request(&different));
    assert_eq!(first.resources(), same.resources());
    first.execute_step(0).unwrap();
    let later = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    assert!(first.same_request(&later));
    assert!(!first.same_snapshot(&later));
    assert_eq!(later.step_count(), 0, "no-op retains its actual baseline");
}

#[test]
fn drift_invalid_plan_cross_module_and_step_bounds_are_explicit() {
    let (catalog, scalar, user) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let operation = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    assert!(operation.execute_step(2).is_err());
    assert_eq!(scalar.writes(), 0);
    scalar.external_change(31);
    assert_eq!(operation.classify().unwrap(), StateMatch::ExternalChange);
    assert!(operation.execute_step(0).is_err());
    assert!(catalog
        .prepare_restore(&operation.snapshot_envelope().unwrap(), machine_target())
        .is_err());
    let user_config = user
        .config_envelope(&UserConfig {
            enabled: true,
            value: 9,
        })
        .unwrap();
    let other = catalog
        .prepare(
            &user_config,
            TargetIdentity::Session(session()),
            ActivationMode::Manual,
        )
        .unwrap();
    assert!(!operation.same_request(&other));
    assert!(!operation.same_snapshot(&other));
    scalar.set_invalid_plan(true);
    assert!(matches!(
        catalog.prepare(&config, machine_target(), ActivationMode::Session),
        Err(CatalogError::InvalidPlan(_))
    ));
}

#[test]
fn snapshot_version_mismatch_blocks_restore_without_a_write() {
    let (catalog, scalar, _) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let operation = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    let mut snapshot = operation.snapshot_envelope().unwrap();
    snapshot.version = SchemaVersion::new(99).unwrap();
    assert!(catalog
        .prepare_restore(&snapshot, machine_target())
        .is_err());
    assert_eq!(scalar.writes(), 0);
}

#[test]
fn updates_preserve_first_baseline_and_recognize_previous_written_states() {
    let (catalog, scalar, _) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let first = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    first.execute_step(0).unwrap();
    let next = scalar.config_envelope(&ScalarConfig { value: 13 }).unwrap();
    let updated = first.prepare_update(&next).unwrap();
    assert!(first.same_baseline(&updated));
    assert_eq!(updated.classify().unwrap(), StateMatch::WrittenStep(0));
    let before_write = updated.snapshot_envelope().unwrap();
    let recovered = catalog
        .prepare_restore(&before_write, machine_target())
        .unwrap();
    assert_eq!(recovered.step_count(), 1);
    updated.execute_step(0).unwrap();
    assert_eq!(updated.classify().unwrap(), StateMatch::WrittenStep(1));
    let all = updated
        .resources()
        .iter()
        .map(|claim| claim.key.clone())
        .collect::<Vec<_>>();
    let restore = updated.prepare_restore(&all).unwrap();
    restore.execute_step(0).unwrap();
    assert_eq!(
        scalar.value(),
        7,
        "never recapture the old app-written value 12"
    );
}

#[test]
fn released_resource_subset_preserves_other_shared_fields_even_after_drift() {
    let (catalog, _, user) = catalog();
    let target = TargetIdentity::Session(session());
    let config = user
        .config_envelope(&UserConfig {
            enabled: false,
            value: 9,
        })
        .unwrap();
    let operation = catalog
        .prepare(&config, target.clone(), ActivationMode::Manual)
        .unwrap();
    operation.execute_step(0).unwrap();
    operation.execute_step(1).unwrap();
    let value_key = operation.step_resources(0).unwrap()[0].clone();
    let enabled_key = operation.step_resources(1).unwrap()[0].clone();
    let snapshot = operation.snapshot_envelope().unwrap();
    let restore = catalog
        .prepare_restore_resources(&snapshot, target.clone(), std::slice::from_ref(&value_key))
        .unwrap();
    assert_eq!(restore.resources().len(), 1);
    assert_eq!(
        restore.step_resources(0).unwrap(),
        std::slice::from_ref(&value_key)
    );
    restore.execute_step(0).unwrap();
    assert_eq!(user.value(), None);
    assert!(!user.enabled());
    let restore_enabled = catalog
        .prepare_restore_resources(
            &snapshot,
            target.clone(),
            std::slice::from_ref(&enabled_key),
        )
        .unwrap();
    restore_enabled.execute_step(0).unwrap();
    assert!(user.enabled());

    // An unrelated shared field change must not invalidate a released value.
    let operation = catalog
        .prepare(&config, target.clone(), ActivationMode::Manual)
        .unwrap();
    operation.execute_step(0).unwrap();
    operation.execute_step(1).unwrap();
    user.external_enabled(true);
    assert!(
        operation
            .classify_resources(std::slice::from_ref(&value_key))
            .unwrap()
            != StateMatch::ExternalChange
    );
    operation
        .prepare_restore(std::slice::from_ref(&value_key))
        .unwrap()
        .execute_step(0)
        .unwrap();
    assert_eq!(user.value(), None);
    assert!(user.enabled());
    let foreign = ResourceKey {
        target: machine_target(),
        field: ResourceId::new("unknown_setting").unwrap(),
    };
    let reads = user.reads();
    assert!(catalog
        .prepare_restore_resources(&snapshot, target, &[foreign])
        .is_err());
    assert_eq!(
        user.reads(),
        reads,
        "reject invalid subset before observing"
    );

    let operation = catalog
        .prepare(
            &config,
            TargetIdentity::Session(session()),
            ActivationMode::Manual,
        )
        .unwrap();
    operation.execute_step(0).unwrap();
    operation.execute_step(1).unwrap();
    let drift = user_setting::NativeValue {
        kind: 99,
        bytes: vec![42, 0, 255],
    };
    user.external_value(Some(drift.clone()));
    assert_eq!(operation.classify().unwrap(), StateMatch::ExternalChange);
    assert!(matches!(
        operation
            .classify_resources(std::slice::from_ref(&enabled_key))
            .unwrap(),
        StateMatch::WrittenStep(_)
    ));
    operation
        .prepare_restore(std::slice::from_ref(&enabled_key))
        .unwrap()
        .execute_step(0)
        .unwrap();
    assert_eq!(user.native_value(), Some(drift));
    assert!(user.enabled());
}

#[test]
fn empty_release_is_a_noop_and_duplicate_subset_is_rejected_before_read() {
    let (catalog, scalar, _) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let operation = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    let snapshot = operation.snapshot_envelope().unwrap();
    let empty = catalog
        .prepare_restore_resources(&snapshot, machine_target(), &[])
        .unwrap();
    assert_eq!(empty.step_count(), 0);
    assert!(empty.resources().is_empty());
    assert_eq!(scalar.writes(), 0);
    let resource = operation.resources()[0].key.clone();
    let reads = scalar.reads();
    assert!(catalog
        .prepare_restore_resources(&snapshot, machine_target(), &[resource.clone(), resource])
        .is_err());
    assert_eq!(scalar.reads(), reads);
}

#[test]
fn snapshot_keeps_native_type_bytes_absence_and_target_incarnation() {
    let raw = user_setting::NativeValue {
        kind: 19,
        bytes: vec![0, 255, 1, 0, 64],
    };
    let user = UserModule::new_native(Some(raw.clone()), true);
    let mut catalog = Catalog::default();
    catalog.register(user.clone()).unwrap();
    let target = TargetIdentity::Session(session());
    let config = user
        .config_envelope(&UserConfig {
            enabled: false,
            value: 9,
        })
        .unwrap();
    let operation = catalog
        .prepare(&config, target.clone(), ActivationMode::Manual)
        .unwrap();
    let snapshot =
        ModuleEnvelope::from_bytes(&operation.snapshot_envelope().unwrap().to_bytes().unwrap())
            .unwrap();
    operation.execute_step(0).unwrap();
    operation.execute_step(1).unwrap();
    let mut different = session();
    different.logon = LogonId::new(99).unwrap();
    assert!(catalog
        .prepare_restore(&snapshot, TargetIdentity::Session(different))
        .is_err());
    let restore = catalog.prepare_restore(&snapshot, target).unwrap();
    restore.execute_step(0).unwrap();
    restore.execute_step(1).unwrap();
    assert_eq!(user.native_value(), Some(raw));
    assert!(user.enabled());
}

#[test]
fn read_only_catalog_path_needs_no_claims_and_observed_tokens_are_exact() {
    let mut descriptor = scalar::descriptor();
    descriptor.access = ModuleAccess::ReadOnly;
    descriptor.resources.clear();
    descriptor.activation_modes.clear();
    let readonly = ScalarModule::with_descriptor(descriptor);
    let mut catalog = Catalog::default();
    catalog.register(readonly.clone()).unwrap();
    let summary = catalog
        .observe(&FeatureId::new("test_scalar").unwrap(), &machine_target())
        .unwrap();
    assert_eq!(summary.summary, "7");
    assert_eq!(readonly.writes(), 0);
    let config = readonly
        .config_envelope(&ScalarConfig { value: 12 })
        .unwrap();
    assert!(matches!(
        catalog.prepare(&config, machine_target(), ActivationMode::Session),
        Err(CatalogError::ReadOnly)
    ));
    let (catalog, scalar, _) = super::testing::catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let operation = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    let keys = operation
        .resources()
        .iter()
        .map(|claim| claim.key.clone())
        .collect::<Vec<_>>();
    let before = operation.observe_resources(&keys).unwrap();
    assert_ne!(before[0].value, operation.resources()[0].desired);
    operation.execute_step(0).unwrap();
    let after = catalog
        .observe_resources(
            &FeatureId::new("test_scalar").unwrap(),
            &machine_target(),
            &keys,
        )
        .unwrap();
    assert_eq!(after[0].value, operation.resources()[0].desired);
}

#[test]
fn native_error_information_and_canonical_tokens_are_bounded() {
    let error = ModuleError::new(ErrorKind::Write, "write setting")
        .with_native(NativeError {
            domain: ResourceId::new("win32").unwrap(),
            code: 5,
        })
        .with_detail("Access denied")
        .unwrap();
    assert_eq!(error.native.as_ref().unwrap().code, 5);
    assert_eq!(error.detail(), Some("Access denied"));
    assert!(ModuleError::new(ErrorKind::Read, "read")
        .with_detail("x".repeat(1025))
        .is_err());
    let semantics = ResourceId::new("unsigned_scalar").unwrap();
    let version = SchemaVersion::new(1).unwrap();
    assert!(DesiredStateToken::new(semantics.clone(), version, vec![]).is_err());
    assert!(DesiredStateToken::new(semantics, version, vec![0; 4097]).is_err());
}

#[test]
fn descriptor_and_codec_metadata_are_reachable_and_consistent() {
    let mut descriptor = scalar::descriptor();
    descriptor.resources[0].scope = TargetScope::User;
    assert!(
        matches!(
            Catalog::default().register(ScalarModule::with_descriptor(descriptor)),
            Err(CatalogError::InvalidDescriptor(_))
        ),
        "machine target cannot reach a user resource"
    );
    for (feature, kind, version) in [
        (
            FeatureId::new("wrong_feature").unwrap(),
            EnvelopeKind::Config,
            SchemaVersion::new(1).unwrap(),
        ),
        (
            FeatureId::new("test_scalar").unwrap(),
            EnvelopeKind::Snapshot,
            SchemaVersion::new(1).unwrap(),
        ),
        (
            FeatureId::new("test_scalar").unwrap(),
            EnvelopeKind::Config,
            SchemaVersion::new(2).unwrap(),
        ),
    ] {
        assert!(matches!(
            Catalog::default()
                .register(ScalarModule::new(7).with_config_codec(feature, kind, version)),
            Err(CatalogError::InvalidDescriptor(_))
        ));
    }
    let target = TargetIdentity::Process(
        ProcessIdentity::new(
            session(),
            42,
            123,
            ImagePath::new(r"C:\Games\game.exe").unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(
        target.resource_target(TargetScope::Machine),
        Some(machine_target())
    );
    assert_eq!(
        target.resource_target(TargetScope::User),
        Some(TargetIdentity::User(session().user))
    );
    assert!(machine_target()
        .resource_target(TargetScope::User)
        .is_none());
}

#[test]
fn initial_and_updated_plans_reject_incompatible_snapshots_and_undeclared_step_writes() {
    let (catalog, scalar, _) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    scalar.set_invalid_snapshot(true);
    assert!(
        matches!(
            catalog.prepare(&config, machine_target(), ActivationMode::Session),
            Err(CatalogError::InvalidPlan(_))
        ),
        "snapshot must cover original claims before an executable operation exists"
    );
    scalar.set_invalid_snapshot(false);
    let operation = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    scalar.set_invalid_snapshot(true);
    let next = scalar.config_envelope(&ScalarConfig { value: 13 }).unwrap();
    assert!(matches!(
        operation.prepare_update(&next),
        Err(CatalogError::InvalidPlan(_))
    ));
    scalar.set_invalid_snapshot(false);
    scalar.set_invalid_step(true);
    assert!(matches!(
        catalog.prepare(&config, machine_target(), ActivationMode::Session),
        Err(CatalogError::InvalidPlan(_))
    ));
    assert_eq!(scalar.writes(), 0);
}

#[test]
fn capability_reason_and_later_denial_survive_without_backend_reads_or_writes() {
    let (catalog, scalar, _) = catalog();
    let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    scalar.set_capability(Capability::Denied("Wrong interactive user".into()));
    let error = match catalog.prepare(&config, machine_target(), ActivationMode::Session) {
        Err(CatalogError::Module(error)) => error,
        _ => panic!("expected denial"),
    };
    assert_eq!(error.detail(), Some("Wrong interactive user"));
    assert_eq!(scalar.reads(), 0);
    scalar.set_capability(Capability::Available);
    let operation = catalog
        .prepare(&config, machine_target(), ActivationMode::Session)
        .unwrap();
    let reads = scalar.reads();
    scalar.set_capability(Capability::Denied("Context changed".into()));
    assert!(operation.execute_step(0).is_err());
    assert!(catalog
        .prepare_restore(&operation.snapshot_envelope().unwrap(), machine_target())
        .is_err());
    assert_eq!(scalar.reads(), reads);
    assert_eq!(scalar.writes(), 0);
}

#[test]
fn failed_second_native_step_stays_a_failure_and_first_write_is_recoverable() {
    let (catalog, _, user) = catalog();
    let target = TargetIdentity::Session(session());
    let config = user
        .config_envelope(&UserConfig {
            enabled: false,
            value: 9,
        })
        .unwrap();
    let operation = catalog
        .prepare(&config, target.clone(), ActivationMode::Manual)
        .unwrap();
    let snapshot = operation.snapshot_envelope().unwrap();
    operation.execute_step(0).unwrap();
    user.fail_next_enabled_step();
    let result = operation.execute_step(1);
    assert!(
        matches!(
            result,
            Err(CatalogError::Module(ModuleError {
                kind: ErrorKind::Write,
                ..
            }))
        ),
        "second native failure cannot become success"
    );
    assert_eq!(user.writes(), 1);
    assert!(user.enabled());
    assert_eq!(operation.verify_step(1).unwrap(), Verification::Mismatch);
    assert_eq!(operation.classify().unwrap(), StateMatch::WrittenStep(0));
    let restore = catalog.prepare_restore(&snapshot, target).unwrap();
    assert_eq!(restore.step_count(), 1);
    restore.execute_step(0).unwrap();
    assert_eq!(
        restore.verify_step(0).unwrap(),
        Verification::Verified(Effect::RestartRequired)
    );
    assert_eq!(user.value(), None);
    assert!(user.enabled());
}

#[test]
fn subset_restore_snapshot_never_promotes_untouched_external_values_to_known_writes() {
    let (catalog, _, user) = catalog();
    let target = TargetIdentity::Session(session());
    let config = user
        .config_envelope(&UserConfig {
            enabled: false,
            value: 9,
        })
        .unwrap();
    let operation = catalog
        .prepare(&config, target.clone(), ActivationMode::Manual)
        .unwrap();
    operation.execute_step(0).unwrap();
    operation.execute_step(1).unwrap();
    let drift = user_setting::NativeValue {
        kind: 99,
        bytes: vec![42, 0, 255],
    };
    user.external_value(Some(drift.clone()));
    let enabled = operation.step_resources(1).unwrap()[0].clone();
    let subset_restore = operation.prepare_restore(&[enabled]).unwrap();
    subset_restore.execute_step(0).unwrap();
    let recovery = subset_restore.snapshot_envelope().unwrap();
    let writes = user.writes();
    assert!(
        matches!(
            catalog.prepare_restore(&recovery, target),
            Err(CatalogError::Module(ModuleError {
                kind: ErrorKind::ExternalChange,
                ..
            }))
        ),
        "subset preparation cannot launder unrelated drift into app-written history"
    );
    assert_eq!(user.native_value(), Some(drift));
    assert_eq!(user.writes(), writes);
}
