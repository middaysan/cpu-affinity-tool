//! The field controller's durable boundary, independently reloaded after a real process exits.
use super::{
    persistence::{source_identifier, DiskRecoveryStore, RecoveryDocument, RecoveryStore},
    platform::{
        contract::*,
        controller::{Controller, OwnerKey, RecoveryIntent},
        registry::{Module, Registry},
    },
    simple_tests::Setting,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Memory {
    document: Option<RecoveryDocument>,
    publications: usize,
    fail: Option<usize>,
}
struct Store(Arc<Mutex<Memory>>);
impl RecoveryStore for Store {
    fn load(&mut self) -> Result<Option<RecoveryDocument>, ModuleError> {
        Ok(self.0.lock().unwrap().document.clone())
    }
    fn publish(&mut self, document: &RecoveryDocument) -> Result<(), ModuleError> {
        let mut memory = self.0.lock().unwrap();
        memory.publications += 1;
        if memory.fail == Some(memory.publications) {
            return Err(ModuleError::new(ErrorKind::Write, "recovery publication"));
        }
        memory.document = Some(document.clone());
        Ok(())
    }
}
fn owner(id: u64) -> OwnerKey {
    OwnerKey::Manual {
        profile: ProfileId::new(id).unwrap(),
        feature: FeatureId::new("scalar").unwrap(),
    }
}
fn setup(user: bool) -> (Arc<Setting>, Registry, Arc<Mutex<Memory>>, Controller) {
    let module = Setting::new("scalar", user);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let memory = Arc::new(Mutex::new(Memory::default()));
    let controller = Controller::restore(
        Box::new(Store(memory.clone())),
        "C:/owned/state.json".into(),
        &registry,
    )
    .unwrap();
    (module, registry, memory, controller)
}
#[test]
fn durable_prewrite_failure_writes_nothing_and_keeps_evidence() {
    let (module, _, memory, mut controller) = setup(false);
    memory.lock().unwrap().fail = Some(1);
    assert!(controller
        .acquire(
            owner(1),
            module.clone(),
            TargetIdentity::Machine,
            module.config(12)
        )
        .is_err());
    assert_eq!(module.value(), vec![7]);
    assert_eq!(*module.writes.lock().unwrap(), 0);
    assert!(!controller.is_empty());
    assert!(!controller.can_admit());
}
#[test]
fn durable_valid_empty_projection_has_no_owner_and_releases_previous_fields() {
    let (module, _, memory, mut controller) = setup(false);
    let mut empty = module.config(12);
    empty.payload["empty"] = true.into();
    controller
        .acquire(
            owner(1),
            module.clone(),
            TargetIdentity::Machine,
            empty.clone(),
        )
        .unwrap();
    assert!(controller.is_empty());
    assert_eq!(*module.writes.lock().unwrap(), 0);
    assert!(memory.lock().unwrap().document.is_none());
    controller
        .acquire(
            owner(1),
            module.clone(),
            TargetIdentity::Machine,
            module.config(12),
        )
        .unwrap();
    controller
        .acquire(owner(1), module.clone(), TargetIdentity::Machine, empty)
        .unwrap();
    assert_eq!(module.value(), vec![7]);
    assert!(controller.is_empty());
}
#[test]
fn durable_session_projected_user_field_retains_exact_activation_and_trigger() {
    let (module, registry, memory, mut controller) = setup(true);
    let session = OwnerKey::Session {
        profile: ProfileId::new(1).unwrap(),
        feature: FeatureId::new("scalar").unwrap(),
        group: "group_1".into(),
        rule: "rule_1".into(),
        pid: 42,
        token: 142,
    };
    let activation = super::persistence::ActivationFacts {
        pid: 42,
        token: 142,
        image: "fixture.exe".into(),
        trigger: ActivationEligibility::Foreground,
    };
    controller
        .acquire_session(
            session.clone(),
            module.clone(),
            module.targets()[0].clone(),
            module.config(12),
            activation.clone(),
        )
        .unwrap();
    drop(controller);
    let loaded = Controller::restore(
        Box::new(Store(memory)),
        "C:/owned/state.json".into(),
        &registry,
    )
    .unwrap();
    assert_eq!(
        loaded.owner_record(&session).unwrap().activation,
        Some(activation)
    );
    assert!(!loaded.can_admit());
    assert_eq!(module.value(), vec![12]);
}
struct ExitedProcess {
    setting: Arc<Setting>,
    exited: std::sync::atomic::AtomicBool,
}
impl Module for ExitedProcess {
    fn descriptor(&self) -> ModuleDescriptor {
        let mut descriptor = self.setting.descriptor();
        descriptor.target_scopes = vec![TargetScope::Process];
        descriptor
    }
    fn editor(&self) -> &dyn super::presentation::ModuleEditor {
        self.setting.as_ref()
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        vec![]
    }
    fn observe(&self, target: &TargetIdentity) -> super::platform::registry::ModuleObservation {
        self.setting.observe(target)
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &super::platform::codec::ModuleEnvelope,
    ) -> Result<Vec<super::platform::registry::FieldChange>, ModuleError> {
        self.setting.fields(target, config)
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        if self.exited.load(std::sync::atomic::Ordering::SeqCst) {
            Err(ModuleError::new(ErrorKind::Read, "process exited"))
        } else {
            self.setting.read(key)
        }
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        self.setting.write(key, expected, desired)
    }
}
#[test]
fn durable_exact_positive_exit_retires_process_field_without_reading_or_writing_dead_pid() {
    use super::platform::controller::LifetimeEvidence;
    let setting = Setting::new("scalar", false);
    let module = Arc::new(ExitedProcess {
        setting: setting.clone(),
        exited: std::sync::atomic::AtomicBool::new(false),
    });
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let memory = Arc::new(Mutex::new(Memory::default()));
    let mut controller = Controller::restore(
        Box::new(Store(memory.clone())),
        "C:/owned/state.json".into(),
        &registry,
    )
    .unwrap();
    let session = OwnerKey::Session {
        profile: ProfileId::new(1).unwrap(),
        feature: FeatureId::new("scalar").unwrap(),
        group: "group_1".into(),
        rule: "rule_1".into(),
        pid: 42,
        token: 142,
    };
    let facts = super::persistence::ActivationFacts {
        pid: 42,
        token: 142,
        image: "fixture.exe".into(),
        trigger: ActivationEligibility::Running,
    };
    let target = TargetIdentity::Process {
        pid: 42,
        creation_token: 142,
        image_path: "fixture.exe".into(),
    };
    controller
        .acquire_session(
            session.clone(),
            module.clone(),
            target,
            setting.config(12),
            facts,
        )
        .unwrap();
    module
        .exited
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(!controller
        .release_if_exited(&session, LifetimeEvidence::Unknown)
        .unwrap());
    assert!(!controller.is_empty());
    assert!(controller
        .release_if_exited(&session, LifetimeEvidence::Exited)
        .unwrap());
    assert!(controller.is_empty());
    assert_eq!(*setting.writes.lock().unwrap(), 1);
    assert!(memory
        .lock()
        .unwrap()
        .document
        .as_ref()
        .unwrap()
        .fields
        .is_empty());
}
#[test]
fn durable_equal_share_conflict_and_subset_release_are_published() {
    let (module, _, memory, mut controller) = setup(false);
    controller
        .acquire(
            owner(1),
            module.clone(),
            TargetIdentity::Machine,
            module.config(12),
        )
        .unwrap();
    controller
        .acquire(
            owner(2),
            module.clone(),
            TargetIdentity::Machine,
            module.config(12),
        )
        .unwrap();
    let before = memory.lock().unwrap().publications;
    assert_eq!(
        memory.lock().unwrap().document.as_ref().unwrap().fields[0]
            .owners
            .len(),
        2
    );
    assert!(controller
        .acquire(
            owner(3),
            module.clone(),
            TargetIdentity::Machine,
            module.config(13)
        )
        .is_err());
    assert_eq!(memory.lock().unwrap().publications, before);
    assert_eq!(*module.writes.lock().unwrap(), 1);
    controller.release(&owner(1)).unwrap();
    assert_eq!(
        memory.lock().unwrap().document.as_ref().unwrap().fields[0].owners,
        vec![owner(2)]
    );
    assert_eq!(module.value(), vec![12]);
    controller.release(&owner(2)).unwrap();
    assert_eq!(module.value(), vec![7]);
    assert!(memory
        .lock()
        .unwrap()
        .document
        .as_ref()
        .unwrap()
        .fields
        .is_empty());
}
#[test]
fn durable_reload_partial_fields_and_unknown_drift_never_recapture() {
    let (module, registry, memory, mut controller) = setup(true);
    *module.fail_at.lock().unwrap() = Some(ResourceId::new("second").unwrap());
    assert!(controller
        .acquire(
            owner(1),
            module.clone(),
            module.targets()[0].clone(),
            module.config(12)
        )
        .is_err());
    assert_eq!(module.value(), vec![12]);
    drop(controller);
    *module.fail_at.lock().unwrap() = None;
    let mut loaded = Controller::restore(
        Box::new(Store(memory.clone())),
        "C:/owned/state.json".into(),
        &registry,
    )
    .unwrap();
    loaded
        .recover_owner(&owner(1), RecoveryIntent::Unknown)
        .unwrap();
    assert!(!loaded.can_admit());
    assert_eq!(*module.writes.lock().unwrap(), 1);
    loaded
        .recover_owner(&owner(1), RecoveryIntent::Restore)
        .unwrap();
    assert_eq!(module.value(), vec![7]);
    assert!(!module
        .fields
        .lock()
        .unwrap()
        .contains_key(&ResourceId::new("second").unwrap()));
    loaded
        .acquire(
            owner(1),
            module.clone(),
            module.targets()[0].clone(),
            module.config(12),
        )
        .unwrap();
    module
        .fields
        .lock()
        .unwrap()
        .insert(ResourceId::new("value").unwrap(), vec![19]);
    drop(loaded);
    let mut drift = Controller::restore(
        Box::new(Store(memory.clone())),
        "C:/owned/state.json".into(),
        &registry,
    )
    .unwrap();
    assert!(drift
        .recover_owner(&owner(1), RecoveryIntent::Restore)
        .is_err());
    assert_eq!(module.value(), vec![19]);
    assert!(!drift.can_admit());
    assert!(!drift.is_empty());
}
#[test]
fn durable_after_write_and_restore_publication_failures_reload_original() {
    for restoring in [false, true] {
        let (module, registry, memory, mut controller) = setup(false);
        if restoring {
            controller
                .acquire(
                    owner(1),
                    module.clone(),
                    TargetIdentity::Machine,
                    module.config(12),
                )
                .unwrap();
            let next = memory.lock().unwrap().publications + 2;
            memory.lock().unwrap().fail = Some(next);
            assert!(controller.release(&owner(1)).is_err());
            assert_eq!(module.value(), vec![7]);
        } else {
            memory.lock().unwrap().fail = Some(3);
            assert!(controller
                .acquire(
                    owner(1),
                    module.clone(),
                    TargetIdentity::Machine,
                    module.config(12)
                )
                .is_err());
            assert_eq!(module.value(), vec![12]);
        }
        assert!(!controller.is_empty());
        assert!(!controller.can_admit());
        drop(controller);
        memory.lock().unwrap().fail = None;
        let mut loaded = Controller::restore(
            Box::new(Store(memory)),
            "C:/owned/state.json".into(),
            &registry,
        )
        .unwrap();
        loaded
            .recover_owner(&owner(1), RecoveryIntent::Restore)
            .unwrap();
        assert_eq!(module.value(), vec![7]);
        assert!(loaded.is_empty());
    }
}
struct OwnedDirectory(PathBuf);
impl OwnedDirectory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cat-recovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        let target = self.0.join("target.bin");
        if target.exists() {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(target)
            {
                use std::io::Write;
                let _ = file.write_all(&7u32.to_le_bytes());
                let _ = file.sync_all();
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn durable_disk_lease_ambiguous_artifact_unknown_and_source_mismatch_block() {
    let directory = OwnedDirectory::new();
    let store = DiskRecoveryStore::open(&directory.0).unwrap();
    assert!(DiskRecoveryStore::open(&directory.0).is_err());
    drop(store);
    std::fs::write(directory.0.join("recovery-v1.staging"), b"evidence").unwrap();
    assert!(DiskRecoveryStore::open(&directory.0).is_err());
    std::fs::remove_file(directory.0.join("recovery-v1.staging")).unwrap();
    std::fs::write(
        directory.0.join("recovery-v1.json"),
        br#"{"version":99,"fields":[]}"#,
    )
    .unwrap();
    let registry = Registry::default();
    assert!(Controller::restore(
        Box::new(DiskRecoveryStore::open(&directory.0).unwrap()),
        "C:/owned/state.json".into(),
        &registry
    )
    .is_err());
    std::fs::remove_file(directory.0.join("recovery-v1.json")).unwrap();
    let (module, registry, _, _) = setup(false);
    let source = source_identifier(&directory.0.join("state.json")).unwrap();
    let mut controller = Controller::restore(
        Box::new(DiskRecoveryStore::open(&directory.0).unwrap()),
        source,
        &registry,
    )
    .unwrap();
    controller
        .acquire(
            owner(1),
            module,
            TargetIdentity::Machine,
            Setting::new("scalar", false).config(12),
        )
        .unwrap();
    drop(controller);
    assert!(Controller::restore(
        Box::new(DiskRecoveryStore::open(&directory.0).unwrap()),
        "C:/foreign/state.json".into(),
        &registry
    )
    .is_err());
}
struct SavedFile(PathBuf);

#[test]
fn durable_saved_off_reconciles_loaded_session_before_quit() {
    use super::config::TuningEnvelope;
    use crate::app::models::{AppStateStorage, RunningApps};
    let directory = OwnedDirectory::new();
    let path = directory.0.join("state.json");
    let mut state = AppStateStorage::test_fixture(|_| {});
    state.try_save_to_path(&path).unwrap();
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let owner = OwnerKey::Session {
        profile: ProfileId::new(1).unwrap(),
        feature: FeatureId::new("scalar").unwrap(),
        group: "group_1".into(),
        rule: "rule_1".into(),
        pid: 42,
        token: 142,
    };
    let mut controller = Controller::restore(
        Box::new(DiskRecoveryStore::open(&directory.0).unwrap()),
        source_identifier(&path).unwrap(),
        &registry,
    )
    .unwrap();
    controller
        .acquire_session(
            owner,
            module.clone(),
            TargetIdentity::Machine,
            module.config(12),
            super::persistence::ActivationFacts {
                pid: 42,
                token: 142,
                image: "fixture.exe".into(),
                trigger: ActivationEligibility::Running,
            },
        )
        .unwrap();
    drop(controller);
    state.tuning = TuningEnvelope::default();
    state.try_save_to_path(&path).unwrap();
    let mut service =
        super::Tuning::from_registry(registry).headless(state, Box::new(SavedFile(path)));
    service
        .start_recovery(Box::new(DiskRecoveryStore::open(&directory.0).unwrap()))
        .unwrap();
    let runtime = super::session_reducer::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(RunningApps::default())),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while module.value() != vec![7] && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let before_quit = module.value();
    runtime.begin_shutdown();
    while !runtime.snapshot().shutdown_complete && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        before_quit,
        vec![7],
        "independently saved Off must reconcile the loaded owner before Quit or another user edit"
    );
    assert!(runtime.snapshot().shutdown_complete);
}

impl super::StatePersistence for SavedFile {
    fn save(
        &mut self,
        state: &mut crate::app::models::AppStateStorage,
    ) -> Result<super::service::SavedState, super::SaveError> {
        state
            .try_save_to_path(&self.0)
            .map_err(|_| super::SaveError::Storage)?;
        Ok(super::service::SavedState::Unavailable)
    }
    fn source_path(&self) -> Option<PathBuf> {
        Some(self.0.clone())
    }
    fn recovery_directory(&self) -> Option<PathBuf> {
        self.0.parent().map(std::path::Path::to_path_buf)
    }
}
fn first_saved_state_initializes_recovery(version: u32, staging: bool) {
    use super::config::ModulePreference;
    use crate::app::{features::rules::RulesContext, models::AppStateStorage};
    let directory = OwnedDirectory::new();
    let path = directory.0.join("state.json");
    let original = AppStateStorage::test_fixture(|state| {
        state.version = version;
        state.rule_identities = None;
    });
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    let mut seeded = AppStateStorage::load_from_path(&path);
    let identities = RulesContext::from_storage(&seeded).to_persisted_identities();
    seeded.rule_identities = Some(identities);
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    if staging {
        std::fs::write(
            directory.0.join("recovery-v1.staging"),
            b"retained evidence",
        )
        .unwrap();
    }
    let mut service =
        super::Tuning::from_registry(registry).headless(seeded, Box::new(SavedFile(path.clone())));
    service.start_selected_recovery();
    assert!(!service.controller.is_durable());
    assert!(!service.native_admitted);
    assert!(service.operation_error.is_some());
    let profile = service
        .create_profile(
            "First save".into(),
            vec![ModulePreference {
                feature: FeatureId::new("scalar").unwrap(),
                enabled: true,
                mode: ActivationMode::Manual,
                config: module.config(12),
            }],
        )
        .unwrap();
    let saved = AppStateStorage::read_saved_for_tuning(&path).unwrap();
    assert_eq!(saved.version, 11);
    assert!(saved.rule_identities.is_some());
    if staging {
        assert!(!service.controller.is_durable());
        assert!(!service.native_admitted);
        assert!(service.operation_error.is_some());
        assert_eq!(
            std::fs::read(directory.0.join("recovery-v1.staging")).unwrap(),
            b"retained evidence"
        );
        assert!(!directory.0.join("recovery-v1.json").exists());
        assert_eq!(*module.writes.lock().unwrap(), 0);
    } else {
        assert!(
            service.controller.is_durable(),
            "successful first save must retry initialization"
        );
        assert!(service.native_admitted);
        assert!(
            service.operation_error.is_none(),
            "verified initialization clears stale startup error"
        );
        service
            .activate_manual(
                profile,
                FeatureId::new("scalar").unwrap(),
                TargetIdentity::Machine,
            )
            .unwrap();
        assert_eq!(module.value(), vec![12]);
        service.disable_profile(profile).unwrap();
        assert_eq!(module.value(), vec![7]);
    }
}
#[test]
fn durable_review_seeded_identities_first_save_initializes_without_restart() {
    first_saved_state_initializes_recovery(11, false);
}
#[test]
fn durable_review_v10_first_save_initializes_without_restart() {
    first_saved_state_initializes_recovery(10, false);
}
#[test]
fn durable_review_first_save_keeps_ambiguous_staging_blocked() {
    first_saved_state_initializes_recovery(11, true);
}
fn loaded_session_shutdown_keeps_evidence_when_guard_closed(reason: &str) {
    use super::{config::ModulePreference, platform::controller::ShutdownResult};
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup};
    let directory = OwnedDirectory::new();
    let path = directory.0.join("state.json");
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let app = AppToRun::new_installed(
        "fixture".into(),
        "fixture".into(),
        os_api::PriorityClass::Normal,
        false,
    );
    let mut state = AppStateStorage::test_fixture(|state| {
        state.groups = vec![CoreGroup {
            name: "Games".into(),
            cores: vec![],
            programs: vec![app],
            is_hidden: false,
            run_all_button: false,
        }];
    });
    state.try_save_to_path(&path).unwrap();
    let mut service = super::Tuning::from_registry(registry.clone())
        .headless(state, Box::new(SavedFile(path.clone())));
    let group = service.rules().group_id_for_index(0).unwrap();
    let rule = service.rules().rule_id_for_index(0, 0).unwrap();
    let profile = service
        .create_profile(
            "Session".into(),
            vec![ModulePreference {
                feature: FeatureId::new("scalar").unwrap(),
                enabled: true,
                mode: ActivationMode::Session,
                config: module.config(12),
            }],
        )
        .unwrap();
    service
        .bind_rule(
            group.clone(),
            rule.clone(),
            profile,
            ActivationEligibility::Running,
        )
        .unwrap();
    let saved = service.state().clone();
    drop(service);
    let owner = OwnerKey::Session {
        profile,
        feature: FeatureId::new("scalar").unwrap(),
        group: group.0,
        rule: rule.0,
        pid: 42,
        token: 142,
    };
    let mut controller = Controller::restore(
        Box::new(DiskRecoveryStore::open(&directory.0).unwrap()),
        source_identifier(&path).unwrap(),
        &registry,
    )
    .unwrap();
    controller
        .acquire_session(
            owner,
            module.clone(),
            TargetIdentity::Machine,
            module.config(12),
            super::persistence::ActivationFacts {
                pid: 42,
                token: 142,
                image: "fixture.exe".into(),
                trigger: ActivationEligibility::Running,
            },
        )
        .unwrap();
    drop(controller);
    let mut service =
        super::Tuning::from_registry(registry).headless(saved, Box::new(SavedFile(path.clone())));
    service
        .start_recovery(Box::new(DiskRecoveryStore::open(&directory.0).unwrap()))
        .unwrap();
    match reason {
        "closed" => service.native_admitted = false,
        "unavailable" => std::fs::remove_file(&path).unwrap(),
        "legacy" => service.legacy_recovery = true,
        _ => unreachable!(),
    }
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    runtime.begin_shutdown();
    let started = std::time::Instant::now();
    while !runtime.snapshot().shutdown_complete {
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(
        runtime.snapshot().shutdown_outcome,
        Some(ShutdownResult::RecoveryRequired)
    );
    assert_eq!(
        module.value(),
        vec![12],
        "closed restoration guard must not write"
    );
    assert_eq!(*module.writes.lock().unwrap(), 1);
    let journal: RecoveryDocument =
        serde_json::from_slice(&std::fs::read(directory.0.join("recovery-v1.json")).unwrap())
            .unwrap();
    assert_eq!(journal.fields.len(), 1);
    assert_eq!(journal.fields[0].baseline, vec![7]);
    assert_eq!(journal.fields[0].current, vec![12]);
    assert_eq!(journal.owners.len(), 1);
}
#[test]
fn durable_review_closed_native_gate_cannot_restore_loaded_session_on_quit() {
    loaded_session_shutdown_keeps_evidence_when_guard_closed("closed");
}
#[test]
fn durable_review_unavailable_saved_source_cannot_restore_loaded_session_on_quit() {
    loaded_session_shutdown_keeps_evidence_when_guard_closed("unavailable");
}
#[test]
fn durable_review_legacy_block_cannot_restore_loaded_session_on_quit() {
    loaded_session_shutdown_keeps_evidence_when_guard_closed("legacy");
}
#[test]
fn durable_public_manual_target_is_saved_before_write_and_fresh_disk_controls_recovery() {
    use super::config::{ModulePreference, TuningLoad};
    let directory = OwnedDirectory::new();
    let path = directory.0.join("state.json");
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let mut state = crate::app::models::AppStateStorage::test_fixture(|_| {});
    state.try_save_to_path(&path).unwrap();
    let mut service = super::Tuning::from_registry(registry.clone())
        .headless(state, Box::new(SavedFile(path.clone())));
    service
        .start_recovery(Box::new(DiskRecoveryStore::open(&directory.0).unwrap()))
        .unwrap();
    let profile = service
        .create_profile(
            "Saved".into(),
            vec![ModulePreference {
                feature: FeatureId::new("scalar").unwrap(),
                enabled: true,
                mode: ActivationMode::Manual,
                config: module.config(12),
            }],
        )
        .unwrap();
    service
        .activate_manual(
            profile,
            FeatureId::new("scalar").unwrap(),
            TargetIdentity::Machine,
        )
        .unwrap();
    let saved = crate::app::models::AppStateStorage::read_saved_for_tuning(&path).unwrap();
    assert!(
        matches!(saved.tuning.decode(),TuningLoad::Known(config) if config.manual_targets.len()==1 && config.manual_targets[0].target==TargetIdentity::Machine)
    );
    assert_eq!(module.value(), vec![12]);
    drop(service);
    let mut independent =
        crate::app::models::AppStateStorage::read_saved_for_tuning(&path).unwrap();
    let TuningLoad::Known(mut config) = independent.tuning.decode() else {
        panic!("saved config");
    };
    config.profiles[0].modules[0].enabled = false;
    independent.tuning = super::config::TuningEnvelope::from_config(&config).unwrap();
    independent.try_save_to_path(&path).unwrap();
    let mut service = super::Tuning::from_registry(registry.clone())
        .headless(saved, Box::new(SavedFile(path.clone())));
    assert!(service
        .start_recovery(Box::new(DiskRecoveryStore::open(&directory.0).unwrap()))
        .is_err());
    assert_eq!(module.value(), vec![12]);
    drop(service);
    let mut fresh =
        super::Tuning::from_registry(registry).headless(independent, Box::new(SavedFile(path)));
    fresh
        .start_recovery(Box::new(DiskRecoveryStore::open(&directory.0).unwrap()))
        .unwrap();
    assert_eq!(module.value(), vec![7]);
}

#[cfg(all(windows, feature = "tuning-test-controls"))]
mod subprocess {
    use super::super::{
        platform::{
            codec::ModuleEnvelope,
            registry::{FieldChange, ModuleObservation},
        },
        presentation::ModuleEditor,
    };
    use super::*;
    use std::path::Path;
    struct FileSetting {
        path: PathBuf,
        editor: Arc<Setting>,
    }
    impl Module for FileSetting {
        fn descriptor(&self) -> ModuleDescriptor {
            self.editor.descriptor()
        }
        fn editor(&self) -> &dyn ModuleEditor {
            self.editor.as_ref()
        }
        fn targets(&self) -> Vec<TargetIdentity> {
            vec![TargetIdentity::Machine]
        }
        fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
            self.editor.observe(target)
        }
        fn fields(
            &self,
            target: &TargetIdentity,
            config: &ModuleEnvelope,
        ) -> Result<Vec<FieldChange>, ModuleError> {
            Ok(vec![FieldChange {
                key: ResourceKey {
                    target: target.clone(),
                    field: ResourceId::new("value").unwrap(),
                },
                desired: (config.payload["value"].as_u64().unwrap() as u32)
                    .to_le_bytes()
                    .to_vec(),
            }])
        }
        fn read(&self, _: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
            std::fs::read(&self.path)
                .map_err(|_| ModuleError::new(ErrorKind::Read, "temporary target"))
        }
        fn write(
            &self,
            key: &ResourceKey,
            expected: &[u8],
            desired: &[u8],
        ) -> Result<(), ModuleError> {
            use std::io::Write;
            if self.read(key)? != expected {
                return Err(ModuleError::new(
                    ErrorKind::ExternalChange,
                    "temporary target",
                ));
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)
                .unwrap();
            file.write_all(desired).unwrap();
            file.sync_all().unwrap();
            if std::env::var("CAT_RECOVERY_BOUNDARY").as_deref() == Ok("physical") {
                std::process::exit(73);
            }
            Ok(())
        }
    }
    struct BoundaryStore(DiskRecoveryStore, String);
    impl RecoveryStore for BoundaryStore {
        fn load(&mut self) -> Result<Option<RecoveryDocument>, ModuleError> {
            self.0.load()
        }
        fn publish(&mut self, document: &RecoveryDocument) -> Result<(), ModuleError> {
            self.0.publish(document)?;
            if document.fields.iter().any(|f| {
                (self.1 == "intent" && f.attempted.is_some())
                    || (self.1 == "verified"
                        && f.current == 12u32.to_le_bytes()
                        && f.attempted.is_none())
            }) {
                std::process::exit(73);
            }
            Ok(())
        }
    }
    fn registry(path: &Path) -> (Registry, Arc<FileSetting>) {
        let module = Arc::new(FileSetting {
            path: path.into(),
            editor: Setting::new("scalar", false),
        });
        let mut registry = Registry::default();
        registry.register(module.clone()).unwrap();
        (registry, module)
    }
    #[test]
    #[ignore = "Child process entered only by the owned recovery fixture"]
    fn recovery_subprocess_child() {
        let Some(directory) = std::env::var_os("CAT_RECOVERY_DIRECTORY") else {
            return;
        };
        let directory = PathBuf::from(directory);
        let (registry, module) = registry(&directory.join("target.bin"));
        let source = source_identifier(&directory.join("state.json")).unwrap();
        let store = BoundaryStore(
            DiskRecoveryStore::open(&directory).unwrap(),
            std::env::var("CAT_RECOVERY_BOUNDARY").unwrap(),
        );
        let mut controller = Controller::restore(Box::new(store), source, &registry).unwrap();
        controller
            .acquire(
                owner(1),
                module.clone(),
                TargetIdentity::Machine,
                module.editor.default_config(),
            )
            .unwrap();
        panic!("child missed forced boundary");
    }
    pub(super) fn run(boundary: &str) {
        let directory = OwnedDirectory::new();
        std::fs::write(directory.0.join("target.bin"), 7u32.to_le_bytes()).unwrap();
        std::fs::write(
            directory.0.join("state.json"),
            br#"{"saved_manual_target":"Machine","value":12}"#,
        )
        .unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::features::tuning::recovery_tests::subprocess::recovery_subprocess_child",
                "--ignored",
                "--nocapture",
            ])
            .env("CAT_RECOVERY_DIRECTORY", &directory.0)
            .env("CAT_RECOVERY_BOUNDARY", boundary)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73));
        let before = std::fs::read(directory.0.join("target.bin")).unwrap();
        assert_eq!(
            before,
            if boundary == "intent" {
                7u32.to_le_bytes()
            } else {
                12u32.to_le_bytes()
            }
        );
        let (registry, _) = registry(&directory.0.join("target.bin"));
        let source = source_identifier(&directory.0.join("state.json")).unwrap();
        let mut controller = Controller::restore(
            Box::new(DiskRecoveryStore::open(&directory.0).unwrap()),
            source,
            &registry,
        )
        .unwrap();
        controller
            .recover_owner(&owner(1), RecoveryIntent::Restore)
            .unwrap();
        assert_eq!(
            std::fs::read(directory.0.join("target.bin")).unwrap(),
            7u32.to_le_bytes()
        );
        assert!(controller.is_empty());
        if boundary == "intent" {
            assert_eq!(
                std::fs::read(directory.0.join("target.bin")).unwrap(),
                before
            );
        }
    }
}
#[cfg(all(windows, feature = "tuning-test-controls"))]
#[test]
#[ignore = "Isolated subprocess recovery; run through the recovery script"]
fn forced_exit_after_intent_recovers_without_a_target_write() {
    subprocess::run("intent");
}
#[cfg(all(windows, feature = "tuning-test-controls"))]
#[test]
#[ignore = "Isolated subprocess recovery; run through the recovery script"]
fn forced_exit_after_physical_write_restores_exact_original_baseline() {
    subprocess::run("physical");
}
#[cfg(all(windows, feature = "tuning-test-controls"))]
#[test]
#[ignore = "Isolated subprocess recovery; run through the recovery script"]
fn forced_exit_after_verified_progress_reloads_and_restores_original() {
    subprocess::run("verified");
}
