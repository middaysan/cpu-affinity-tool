use super::{
    platform::{
        codec::{EnvelopeKind, ModuleEnvelope},
        contract::*,
        controller::{Controller, OwnerKey},
        registry::{FieldChange, Module, ModuleObservation, Registry},
    },
    presentation::ModuleEditor,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub(crate) struct Setting {
    id: FeatureId,
    pub(super) fields: Arc<Mutex<BTreeMap<ResourceId, Vec<u8>>>>,
    pub(super) writes: Arc<Mutex<usize>>,
    actions: Arc<Mutex<Vec<String>>>,
    pub(super) fail_at: Mutex<Option<ResourceId>>,
    observation_gate: Mutex<Option<Arc<(Mutex<bool>, std::sync::Condvar)>>>,
    observation_entered: std::sync::atomic::AtomicBool,
    user: bool,
}
impl Setting {
    pub(crate) fn new(name: &str, user: bool) -> Arc<Self> {
        Arc::new(Self {
            id: FeatureId::new(name).unwrap(),
            fields: Arc::new(Mutex::new(BTreeMap::from([
                (ResourceId::new("value").unwrap(), vec![7]),
                (ResourceId::new("unrelated").unwrap(), vec![99]),
            ]))),
            writes: Arc::new(Mutex::new(0)),
            actions: Arc::new(Mutex::new(vec![])),
            fail_at: Mutex::new(None),
            observation_gate: Mutex::new(None),
            observation_entered: std::sync::atomic::AtomicBool::new(false),
            user,
        })
    }
    pub(crate) fn config(&self, desired: u8) -> ModuleEnvelope {
        ModuleEnvelope {
            feature: self.id.clone(),
            kind: EnvelopeKind::Config,
            version: SchemaVersion::new(1).unwrap(),
            target: None,
            payload: serde_json::json!({"value": desired}),
        }
    }
    pub(super) fn value(&self) -> Vec<u8> {
        self.fields
            .lock()
            .unwrap()
            .get(&ResourceId::new("value").unwrap())
            .cloned()
            .unwrap_or_default()
    }
}
impl ModuleEditor for Setting {
    fn platform_support(&self) -> super::platforms::PlatformSupport {
        if self.id.as_str() == "unsupported_tweak" {
            super::platforms::PlatformSupport {
                windows10_min_build: None,
                windows11_min_build: None,
            }
        } else {
            super::platforms::WIN10_11
        }
    }
    fn tweak(&self) -> Option<super::presentation::TweakSpec> {
        matches!(
            self.id.as_str(),
            "simple_tweak" | "display_refresh" | "unsupported_tweak"
        )
        .then_some(super::presentation::TweakSpec {
            order: 1,
            section: super::presentation::TweakSection::Games,
            title: "Test tweak",
            scope: "Computer",
            apply_label: "Apply",
            help: "Test manual setting",
        })
    }
    fn tweak_ready(&self, _: &TargetIdentity) -> Result<bool, ModuleError> {
        Ok(self.value() == vec![12])
    }
    fn feature(&self) -> FeatureId {
        self.id.clone()
    }
    fn default_config(&self) -> ModuleEnvelope {
        self.config(12)
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        config.feature == self.id
            && config
                .payload
                .get("value")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|v| v <= 255)
    }
    fn show(&self, _: &mut eframe::egui::Ui, _: &mut ModuleEnvelope) -> Result<bool, ()> {
        Ok(false)
    }
}
impl Module for Setting {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            id: self.id.clone(),
            title: "Test setting",
            description: "Test",
            access: ModuleAccess::Managed,
            target_scopes: vec![if self.user {
                TargetScope::User
            } else {
                TargetScope::Machine
            }],
            activation_modes: vec![ActivationMode::Manual, ActivationMode::Session],
            effect: if self.user {
                Effect::RestartRequired
            } else {
                Effect::Immediate
            },
            config_version: SchemaVersion::new(1).unwrap(),
        }
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        vec![if self.user {
            TargetIdentity::User {
                sid: "S-1-5-21-1".into(),
                logon: 1,
            }
        } else {
            TargetIdentity::Machine
        }]
    }
    fn observe(&self, _: &TargetIdentity) -> ModuleObservation {
        if let Some(gate) = self.observation_gate.lock().unwrap().clone() {
            self.observation_entered
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let (lock, changed) = &*gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }
        ModuleObservation {
            capability: Capability::Available,
            summary: ObservationSummary {
                summary: format!("{:?}", self.value()),
                effect: self.descriptor().effect,
            },
            effect: self.descriptor().effect,
        }
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        if !self.accepts_config(config) {
            return Err(ModuleError::new(ErrorKind::InvalidConfig, "config"));
        }
        if config.payload.get("empty") == Some(&serde_json::Value::Bool(true)) {
            return Ok(vec![]);
        }
        let mut changes = vec![FieldChange {
            key: ResourceKey {
                target: target.clone(),
                field: ResourceId::new("value").unwrap(),
            },
            desired: vec![config.payload["value"].as_u64().unwrap() as u8],
        }];
        if self.user || config.payload.get("second").is_some() {
            changes.push(FieldChange {
                key: ResourceKey {
                    target: target.clone(),
                    field: ResourceId::new("second").unwrap(),
                },
                desired: vec![config
                    .payload
                    .get("second")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(1) as u8],
            });
        }
        Ok(changes)
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        Ok(self
            .fields
            .lock()
            .unwrap()
            .get(&key.field)
            .cloned()
            .unwrap_or_default())
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        if self.fail_at.lock().unwrap().as_ref() == Some(&key.field) {
            return Err(ModuleError::new(ErrorKind::Write, "fixture write"));
        }
        let mut fields = self.fields.lock().unwrap();
        if fields.get(&key.field).map_or(&[][..], Vec::as_slice) != expected {
            return Err(ModuleError::new(ErrorKind::ExternalChange, "write"));
        }
        if desired.is_empty() {
            fields.remove(&key.field);
        } else {
            fields.insert(key.field.clone(), desired.to_vec());
        }
        *self.writes.lock().unwrap() += 1;
        Ok(())
    }
    fn action(&self, _: super::presentation::ModuleAction) -> Result<(), ModuleError> {
        self.actions
            .lock()
            .unwrap()
            .push(std::thread::current().name().unwrap_or("unknown").into());
        Ok(())
    }
}
fn owner(id: u64) -> OwnerKey {
    OwnerKey::Manual {
        profile: ProfileId::new(id).unwrap(),
        feature: FeatureId::new("scalar").unwrap(),
    }
}

#[test]
fn simple_controller_shares_equal_fields_and_conflicts_without_writes() {
    let module = Setting::new("scalar", false);
    let mut controller = Controller::default();
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
    assert_eq!(*module.writes.lock().unwrap(), 1);
    assert!(controller
        .acquire(
            owner(3),
            module.clone(),
            TargetIdentity::Machine,
            module.config(13)
        )
        .is_err());
    assert_eq!(*module.writes.lock().unwrap(), 1);
    controller.release(&owner(1)).unwrap();
    assert_eq!(module.value(), vec![12]);
    controller.release(&owner(2)).unwrap();
    assert_eq!(module.value(), vec![7]);
    assert!(controller.is_empty());
}
#[test]
fn simple_controller_updates_without_recapturing_first_baseline() {
    let module = Setting::new("scalar", false);
    let mut controller = Controller::default();
    for value in [12, 13] {
        controller
            .acquire(
                owner(1),
                module.clone(),
                TargetIdentity::Machine,
                module.config(value),
            )
            .unwrap();
    }
    controller.release(&owner(1)).unwrap();
    assert_eq!(module.value(), vec![7]);
    assert_eq!(*module.writes.lock().unwrap(), 3);
}
#[test]
fn simple_controller_user_absence_restoration_preserves_unrelated_fields_and_drift() {
    let module = Setting::new("user_setting", true);
    module
        .fields
        .lock()
        .unwrap()
        .remove(&ResourceId::new("value").unwrap());
    let target = module.targets()[0].clone();
    let mut controller = Controller::default();
    controller
        .acquire(owner(1), module.clone(), target.clone(), module.config(12))
        .unwrap();
    module
        .fields
        .lock()
        .unwrap()
        .insert(ResourceId::new("unrelated").unwrap(), vec![100]);
    controller.release(&owner(1)).unwrap();
    assert!(!module
        .fields
        .lock()
        .unwrap()
        .contains_key(&ResourceId::new("value").unwrap()));
    assert_eq!(
        module.fields.lock().unwrap()[&ResourceId::new("unrelated").unwrap()],
        vec![100]
    );
    controller
        .acquire(owner(1), module.clone(), target, module.config(12))
        .unwrap();
    module
        .fields
        .lock()
        .unwrap()
        .insert(ResourceId::new("value").unwrap(), vec![19]);
    assert!(controller.release(&owner(1)).is_err());
    assert!(!controller.is_empty());
    assert_eq!(module.value(), vec![19]);
}
#[test]
fn simple_registry_uses_the_registered_modules_editor() {
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    assert!(registry.register(module.clone()).is_err());
    assert!(registry
        .get(&module.id)
        .unwrap()
        .editor()
        .accepts_config(&module.config(12)));
}

#[test]
fn simple_controller_unknown_or_live_process_does_not_restore_positive_exit_does() {
    use super::platform::controller::LifetimeEvidence;
    let module = Setting::new("scalar", false);
    let mut controller = Controller::default();
    controller
        .acquire(
            owner(1),
            module.clone(),
            TargetIdentity::Machine,
            module.config(12),
        )
        .unwrap();
    for evidence in [LifetimeEvidence::Unknown, LifetimeEvidence::Live] {
        assert!(!controller.release_if_exited(&owner(1), evidence).unwrap());
        assert_eq!(module.value(), vec![12]);
    }
    assert!(controller
        .release_if_exited(&owner(1), LifetimeEvidence::Exited)
        .unwrap());
    assert_eq!(module.value(), vec![7]);
}

struct MemorySave {
    saved: Arc<Mutex<Vec<String>>>,
    fail: bool,
}

#[test]
fn unsupported_platform_tweak_refuses_before_save_or_native_write() {
    let module = Setting::new("unsupported_tweak", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let saved = Arc::new(Mutex::new(vec![]));
    let mut service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(MemorySave {
            saved: saved.clone(),
            fail: false,
        }),
    );
    service.admit_test_modules();
    assert_eq!(
        service.apply_tweak(module.id.clone(), TargetIdentity::Machine),
        Err(super::SaveError::Invalid)
    );
    let mut controller = Controller::default();
    let result = controller.acquire(
        owner(1),
        module.clone(),
        TargetIdentity::Machine,
        module.config(12),
    );
    assert_eq!(result.unwrap_err().kind, ErrorKind::Unsupported);
    assert!(saved.lock().unwrap().is_empty());
    assert_eq!(*module.writes.lock().unwrap(), 0);
    assert_eq!(module.value(), vec![7]);
    assert!(service.controller.is_empty());
}

#[test]
fn unsupported_platform_legacy_manual_activation_never_saves_target() {
    use super::config::{ModulePreference, Profile, TuningConfig, TuningEnvelope};
    let module = Setting::new("unsupported_tweak", false);
    let profile = ProfileId::new(1).unwrap();
    let mut config = TuningConfig::default();
    config.next_profile = 2;
    config.profiles.push(Profile {
        version: 1,
        id: profile,
        name: "Existing settings".into(),
        modules: vec![ModulePreference {
            feature: module.id.clone(),
            enabled: true,
            mode: ActivationMode::Manual,
            config: module.config(12),
        }],
    });
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let saved = Arc::new(Mutex::new(vec![]));
    let mut service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|state| {
            state.tuning = TuningEnvelope::from_config(&config).unwrap()
        }),
        Box::new(MemorySave {
            saved: saved.clone(),
            fail: false,
        }),
    );
    service.admit_test_modules();
    assert_eq!(
        service.activate_manual(profile, module.id.clone(), TargetIdentity::Machine),
        Err(super::SaveError::Invalid)
    );
    assert!(saved.lock().unwrap().is_empty());
    assert_eq!(*module.writes.lock().unwrap(), 0);
    assert!(service.controller.is_empty());
}

#[test]
fn manual_tweak_one_click_saves_without_association_and_exactly_restores() {
    let module = Setting::new("simple_tweak", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let mut service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(MemorySave {
            saved: Arc::new(Mutex::new(vec![])),
            fail: false,
        }),
    );
    service.admit_test_modules();
    service
        .apply_tweak(module.id.clone(), TargetIdentity::Machine)
        .unwrap();
    assert_eq!(module.value(), vec![12]);
    let super::config::TuningLoad::Known(config) = service.state().tuning.decode() else {
        panic!()
    };
    assert!(config.associations.is_empty());
    assert_eq!(config.manual_targets.len(), 1);
    assert!(config.profiles.iter().all(super::service::is_tweak_profile));
    service.undo_tweak(&module.id).unwrap();
    assert_eq!(module.value(), vec![7]);
    assert!(service.controller.is_empty());
}

#[test]
fn manual_tweak_already_suitable_makes_no_write_or_undo_owner() {
    let module = Setting::new("simple_tweak", false);
    module
        .fields
        .lock()
        .unwrap()
        .insert(ResourceId::new("value").unwrap(), vec![12]);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let mut service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(MemorySave {
            saved: Arc::new(Mutex::new(vec![])),
            fail: false,
        }),
    );
    service.admit_test_modules();
    service
        .apply_tweak(module.id.clone(), TargetIdentity::Machine)
        .unwrap();
    assert_eq!(*module.writes.lock().unwrap(), 0);
    assert!(service.controller.is_empty());
    assert!(service.state().tuning.is_off());
}

#[test]
fn manual_tweak_save_failure_never_writes_native_setting() {
    let module = Setting::new("simple_tweak", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let mut service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(MemorySave {
            saved: Arc::new(Mutex::new(vec![])),
            fail: true,
        }),
    );
    service.admit_test_modules();
    assert_eq!(
        service.apply_tweak(module.id.clone(), TargetIdentity::Machine),
        Err(super::SaveError::Storage)
    );
    assert_eq!(module.value(), vec![7]);
    assert_eq!(*module.writes.lock().unwrap(), 0);
    assert!(service.state().tuning.is_off());
}
#[test]
fn manual_tweak_display_failed_withdrawal_keeps_deadline_and_retries() {
    struct SwitchSave(Arc<std::sync::atomic::AtomicBool>);
    impl super::StatePersistence for SwitchSave {
        fn save(
            &mut self,
            _: &mut crate::app::models::AppStateStorage,
        ) -> Result<super::service::SavedState, super::SaveError> {
            if self.0.load(std::sync::atomic::Ordering::SeqCst) {
                Err(super::SaveError::Storage)
            } else {
                Ok(super::service::SavedState::Unavailable)
            }
        }
    }
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let module = Setting::new("display_refresh", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let mut service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(SwitchSave(fail.clone())),
    );
    service.admit_test_modules();
    service
        .apply_tweak(module.id.clone(), TargetIdentity::Machine)
        .unwrap();
    service.display_confirmation = Some(std::time::Instant::now());
    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(service.expire_display_tweak(false));
    assert!(service.display_confirmation.is_some());
    assert_eq!(module.value(), vec![12]);
    assert_eq!(service.undo_all_tweaks(), Err(super::SaveError::Storage));
    assert!(service.display_confirmation.is_some());
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(service.expire_display_tweak(false));
    assert!(service.display_confirmation.is_none());
    assert_eq!(module.value(), vec![7]);
    assert!(service.controller.is_empty());
}

impl super::StatePersistence for MemorySave {
    fn save(
        &mut self,
        state: &mut crate::app::models::AppStateStorage,
    ) -> Result<super::service::SavedState, super::SaveError> {
        if self.fail {
            return Err(super::SaveError::Storage);
        }
        self.saved
            .lock()
            .unwrap()
            .push(serde_json::to_string(state).unwrap());
        Ok(super::service::SavedState::Unavailable)
    }
}
fn settle(runtime: &super::TuningRuntime) -> Arc<super::TuningSnapshot> {
    let start = std::time::Instant::now();
    loop {
        let snapshot = runtime.snapshot();
        if !matches!(
            snapshot.operation,
            super::TuningOperationState::Pending { .. }
        ) {
            return snapshot;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(3),
            "worker did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
fn submit(
    runtime: &super::TuningRuntime,
    request: super::TuningRequest,
) -> Arc<super::TuningSnapshot> {
    runtime.request(request).unwrap();
    settle(runtime)
}
#[test]
fn simple_disabled_modules_keep_only_state_saves_without_observation_or_recovery() {
    struct SaveOnly;
    impl super::StatePersistence for SaveOnly {
        fn save(
            &mut self,
            _: &mut crate::app::models::AppStateStorage,
        ) -> Result<super::service::SavedState, super::SaveError> {
            Ok(super::service::SavedState::Unavailable)
        }
        fn source_path(&self) -> Option<std::path::PathBuf> {
            panic!("Disabled modules must not inspect the native recovery source");
        }
    }
    let module = Setting::new("disabled_setting", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let raw = r#"{ "version": 77, "future": { "keep": true } }"#;
    let saved = crate::app::models::AppStateStorage::test_fixture(|state| {
        state.tuning = serde_json::from_str(raw).unwrap();
    });
    let runtime = super::TuningRuntime::start(
        super::Tuning::from_registry(registry)
            .headless(saved, Box::new(SaveOnly))
            .without_modules(),
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    assert!(!runtime.modules_enabled());
    assert_eq!(runtime.poll_interval(), None);
    let initial = runtime.snapshot();
    assert!(
        initial.profiles.is_empty()
            && initial.catalog.is_empty()
            && initial.readonly_cards.is_empty()
    );
    assert_eq!(
        runtime.request(super::TuningRequest::ModuleAction {
            feature: module.id.clone(),
            action: super::presentation::ModuleAction::Refresh,
        }),
        Err(super::SaveError::Invalid)
    );
    let mut candidate = (*initial.state).clone();
    candidate.theme_index = 2;
    let changed = submit(
        &runtime,
        super::TuningRequest::SaveBase {
            candidate: Box::new(candidate),
            expected_revision: initial.revision,
        },
    );
    assert_eq!(changed.state.theme_index, 2);
    assert_eq!(changed.state.tuning.raw(), Some(raw));
    runtime.begin_shutdown();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !runtime.snapshot().shutdown_complete && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(
        runtime.snapshot().shutdown_outcome,
        Some(super::platform::controller::ShutdownResult::Complete)
    );
    assert!(!module
        .observation_entered
        .load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(*module.writes.lock().unwrap(), 0);
}
#[test]
fn simple_public_scalar_and_user_shapes_save_activate_edit_disable_and_restore() {
    use super::{config::ModulePreference, TuningOperationState, TuningRequest as Request};
    for user in [false, true] {
        let module = Setting::new(if user { "user_setting" } else { "scalar" }, user);
        if user {
            module
                .fields
                .lock()
                .unwrap()
                .remove(&ResourceId::new("value").unwrap());
        }
        let baseline = module
            .fields
            .lock()
            .unwrap()
            .get(&ResourceId::new("value").unwrap())
            .cloned();
        let saved = Arc::new(Mutex::new(vec![]));
        let mut registry = Registry::default();
        registry.register(module.clone()).unwrap();
        let mut service = super::Tuning::from_registry(registry).headless(
            crate::app::models::AppStateStorage::test_fixture(|_| {}),
            Box::new(MemorySave {
                saved: saved.clone(),
                fail: false,
            }),
        );
        service.admit_test_modules();
        let runtime = super::TuningRuntime::start(
            service,
            Arc::new(tokio::sync::RwLock::new(Default::default())),
        );
        let first = submit(
            &runtime,
            Request::CreateProfile {
                name: "Gaming".into(),
            },
        );
        assert_eq!(first.operation, TuningOperationState::Idle);
        let profile = first.profiles[0].id;
        let preference = ModulePreference {
            feature: module.id.clone(),
            enabled: true,
            mode: ActivationMode::Manual,
            config: module.config(12),
        };
        submit(
            &runtime,
            Request::SetModulePreference {
                profile,
                preference: Box::new(preference.clone()),
                expected: None,
            },
        );
        let active = submit(
            &runtime,
            Request::ActivateManual {
                profile,
                feature: module.id.clone(),
                target: module.targets()[0].clone(),
            },
        );
        assert_eq!(module.value(), vec![12]);
        assert_eq!(
            active.profiles[0].modules[0].status,
            Some(ModuleStatus::VerifiedActive(module.descriptor().effect))
        );
        let edited = ModulePreference {
            config: module.config(13),
            ..preference.clone()
        };
        submit(
            &runtime,
            Request::SetModulePreference {
                profile,
                preference: Box::new(edited),
                expected: Some(preference),
            },
        );
        assert_eq!(module.value(), vec![13]);
        submit(&runtime, Request::DisableProfile { profile });
        assert_eq!(
            module
                .fields
                .lock()
                .unwrap()
                .get(&ResourceId::new("value").unwrap())
                .cloned(),
            baseline
        );
        let text = saved.lock().unwrap().last().unwrap().clone();
        let reloaded: crate::app::models::AppStateStorage = serde_json::from_str(&text).unwrap();
        assert!(
            matches!(reloaded.tuning.decode(),super::config::TuningLoad::Known(config) if !config.profiles[0].modules[0].enabled)
        );
        runtime.begin_shutdown();
    }
}
#[test]
fn simple_public_module_action_dispatches_on_worker_without_profiles() {
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(MemorySave {
            saved: Default::default(),
            fail: false,
        }),
    );
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    let snapshot = submit(
        &runtime,
        super::TuningRequest::ModuleAction {
            feature: module.id.clone(),
            action: super::presentation::ModuleAction::Refresh,
        },
    );
    assert!(snapshot.profiles.is_empty());
    assert_eq!(*module.actions.lock().unwrap(), vec!["tuning"]);
    runtime.begin_shutdown();
}
#[test]
fn simple_public_stale_base_save_cannot_overwrite_newer_saved_profile() {
    let saved = Arc::new(Mutex::new(vec![]));
    let state = crate::app::models::AppStateStorage::test_fixture(|_| {});
    let service = super::Tuning::from_registry(Registry::default()).headless(
        state.clone(),
        Box::new(MemorySave {
            saved: saved.clone(),
            fail: false,
        }),
    );
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    submit(
        &runtime,
        super::TuningRequest::CreateProfile {
            name: "Saved".into(),
        },
    );
    let mut candidate = state;
    candidate.theme_index = 2;
    let snapshot = submit(
        &runtime,
        super::TuningRequest::SaveBase {
            candidate: Box::new(candidate),
            expected_revision: 0,
        },
    );
    assert!(matches!(
        snapshot.operation,
        super::TuningOperationState::Refused {
            error: super::SaveError::Invalid,
            ..
        }
    ));
    assert_eq!(snapshot.profiles[0].name, "Saved");
    assert_eq!(snapshot.state.theme_index, 0);
    assert_eq!(saved.lock().unwrap().len(), 1);
    runtime.begin_shutdown();
}
#[test]
fn simple_public_failed_save_retains_candidate_source_and_rule_identities() {
    let state = crate::app::models::AppStateStorage::test_fixture(|_| {});
    let ids = state.rule_identities.clone();
    let service = super::Tuning::from_registry(Registry::default()).headless(
        state.clone(),
        Box::new(MemorySave {
            saved: Default::default(),
            fail: true,
        }),
    );
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    let mut candidate = state;
    candidate.theme_index = 2;
    let snapshot = submit(
        &runtime,
        super::TuningRequest::SaveBase {
            candidate: Box::new(candidate),
            expected_revision: 0,
        },
    );
    assert!(matches!(
        snapshot.operation,
        super::TuningOperationState::Refused {
            error: super::SaveError::Storage,
            ..
        }
    ));
    assert_eq!(snapshot.revision, 0);
    assert_eq!(snapshot.state.theme_index, 0);
    assert_eq!(snapshot.state.rule_identities, ids);
    runtime.begin_shutdown();
}

#[test]
fn simple_config_preserves_unknown_nested_fields_and_blocks_duplicate_or_legacy_evidence() {
    use super::config::{TuningEnvelope, TuningLoad};
    let module = Setting::new("scalar", false);
    let mut config = serde_json::json!({"version":3,"next_profile":2,"profiles":[{"version":1,"id":1,"name":"Saved","modules":[{"feature":"scalar","enabled":true,"mode":"Manual","config":module.config(12)}]}],"associations":[]});
    config["profiles"][0]["future"] = serde_json::json!(true);
    let raw = serde_json::to_string(&config).unwrap();
    let envelope: TuningEnvelope = serde_json::from_str(&raw).unwrap();
    assert_eq!(envelope.decode(), TuningLoad::Unknown);
    assert_eq!(envelope.raw(), Some(raw.as_str()));
    let duplicate = r#"{"version":3,"version":3,"profiles":[],"associations":[],"next_profile":1}"#;
    let envelope: TuningEnvelope = serde_json::from_str(duplicate).unwrap();
    assert_eq!(envelope.decode(), TuningLoad::Blocked);
    let legacy = r#"{"version":2,"lineage":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"profiles":[],"associations":[],"allocations":{"next_profile":1,"next_owner":2,"next_config_generation":2},"activations":[{"retained":"old lifecycle evidence"}],"withdrawals":[],"transitions":[]}"#;
    let envelope: TuningEnvelope = serde_json::from_str(legacy).unwrap();
    assert_eq!(envelope.decode(), TuningLoad::Unknown);
    assert_eq!(envelope.raw(), Some(legacy));
}

#[test]
fn simple_controller_partial_write_retains_first_baseline_and_restores_only_owned_fields() {
    let module = Setting::new("scalar", false);
    let mut controller = Controller::default();
    let mut config = module.config(12);
    config.payload["second"] = serde_json::json!(4);
    *module.fail_at.lock().unwrap() = Some(ResourceId::new("second").unwrap());
    assert!(controller
        .acquire(owner(1), module.clone(), TargetIdentity::Machine, config)
        .is_err());
    assert_eq!(module.value(), vec![12]);
    assert_eq!(
        controller.status(&owner(1)),
        Some(ModuleStatus::PartialFailure)
    );
    assert_eq!(
        controller
            .fields()
            .iter()
            .find(|f| f.key.field.as_str() == "value")
            .unwrap()
            .baseline,
        vec![7]
    );
    let fields = controller.fields();
    let value = fields
        .iter()
        .find(|f| f.key.field.as_str() == "value")
        .unwrap();
    assert_eq!(value.written, vec![12]);
    assert!(value.attempted.is_none());
    assert_eq!(value.owners, vec![owner(1)]);
    *module.fail_at.lock().unwrap() = None;
    controller.release(&owner(1)).unwrap();
    assert!(controller.is_empty());
    assert_eq!(module.value(), vec![7]);
    assert!(!module
        .fields
        .lock()
        .unwrap()
        .contains_key(&ResourceId::new("second").unwrap()));
    assert_eq!(
        module.fields.lock().unwrap()[&ResourceId::new("unrelated").unwrap()],
        vec![99]
    );
}

#[test]
fn simple_controller_owner_admission_is_bounded_before_any_additional_write() {
    let module = Setting::new("scalar", false);
    let mut controller = Controller::default();
    for id in 1..=256 {
        controller
            .acquire(
                owner(id),
                module.clone(),
                TargetIdentity::Machine,
                module.config(12),
            )
            .unwrap();
    }
    assert!(controller
        .acquire(
            owner(257),
            module.clone(),
            TargetIdentity::Machine,
            module.config(12)
        )
        .is_err());
    assert_eq!(controller.owners().len(), 256);
    assert_eq!(*module.writes.lock().unwrap(), 1);
}

#[test]
fn simple_public_equal_and_conflicting_profile_owners_keep_visible_conflict_and_first_baseline() {
    use super::{config::ModulePreference, TuningRequest as Request};
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let mut service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(MemorySave {
            saved: Default::default(),
            fail: false,
        }),
    );
    service.admit_test_modules();
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    let mut profiles = vec![];
    for (name, value) in [("First", 12), ("Shared", 12), ("Conflict", 13)] {
        let snapshot = submit(&runtime, Request::CreateProfile { name: name.into() });
        let profile = snapshot.profiles.last().unwrap().id;
        submit(
            &runtime,
            Request::SetModulePreference {
                profile,
                preference: Box::new(ModulePreference {
                    feature: module.id.clone(),
                    enabled: true,
                    mode: ActivationMode::Manual,
                    config: module.config(value),
                }),
                expected: None,
            },
        );
        profiles.push(profile);
    }
    for profile in &profiles[..2] {
        submit(
            &runtime,
            Request::ActivateManual {
                profile: *profile,
                feature: module.id.clone(),
                target: TargetIdentity::Machine,
            },
        );
    }
    let conflict = submit(
        &runtime,
        Request::ActivateManual {
            profile: profiles[2],
            feature: module.id.clone(),
            target: TargetIdentity::Machine,
        },
    );
    assert!(matches!(
        conflict.operation,
        super::TuningOperationState::Refused {
            error: super::SaveError::Recovering,
            ..
        }
    ));
    assert_eq!(
        conflict.profiles[2].modules[0].status,
        Some(ModuleStatus::PartialFailure)
    );
    assert_eq!(*module.writes.lock().unwrap(), 1);
    assert_eq!(module.value(), vec![12]);
    submit(
        &runtime,
        Request::DisableProfile {
            profile: profiles[0],
        },
    );
    assert_eq!(module.value(), vec![12]);
    submit(
        &runtime,
        Request::DisableProfile {
            profile: profiles[1],
        },
    );
    assert_eq!(module.value(), vec![7]);
    runtime.begin_shutdown();
}

#[test]
fn simple_public_session_unknown_and_missing_membership_require_exact_positive_exit() {
    use super::{
        config::ModulePreference,
        execution_bridge::{ExecutionObservation, Member, ProcessFacts, UnknownReason},
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup};
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let app = AppToRun::new_installed(
        "fixture".into(),
        "fixture".into(),
        os_api::PriorityClass::Normal,
        false,
    );
    let state = AppStateStorage::test_fixture(|state| {
        state.groups = vec![CoreGroup {
            name: "Games".into(),
            cores: vec![],
            programs: vec![app.clone()],
            is_hidden: false,
            run_all_button: false,
        }];
        state.rule_identities = None;
    });
    let mut service = super::Tuning::from_registry(registry).headless(
        state,
        Box::new(MemorySave {
            saved: Default::default(),
            fail: false,
        }),
    );
    service.admit_test_modules();
    let group = service.rules().group_id_for_index(0).unwrap();
    let rule = service.rules().rule_id_for_index(0, 0).unwrap();
    let profile = service
        .create_profile(
            "Session".into(),
            vec![ModulePreference {
                feature: module.id.clone(),
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
    let facts = ProcessFacts {
        pid: 42,
        token: 142,
        image: Arc::from("fixture.exe"),
        #[cfg(windows)]
        native: None,
    };
    let observed = ExecutionObservation::fixture(
        &saved,
        1,
        vec![Member {
            key: app.get_key(),
            group,
            rule,
            instances: vec![facts.clone()],
        }],
        facts.clone(),
    );
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    let refresh = || {
        submit(
            &runtime,
            super::TuningRequest::ModuleAction {
                feature: module.id.clone(),
                action: super::presentation::ModuleAction::Refresh,
            },
        );
    };
    runtime.observe_fixture(observed, vec![]);
    refresh();
    assert_eq!(module.value(), vec![12]);
    runtime.observe_fixture(ExecutionObservation::Unknown(UnknownReason::Denied), vec![]);
    refresh();
    assert_eq!(module.value(), vec![12]);
    runtime.observe_fixture(
        ExecutionObservation::fixture(&saved, 2, vec![], facts),
        vec![],
    );
    refresh();
    assert_eq!(module.value(), vec![12]);
    runtime.observe_fixture(
        ExecutionObservation::Unknown(UnknownReason::Denied),
        vec![(42, 999)],
    );
    refresh();
    assert_eq!(
        module.value(),
        vec![12],
        "another incarnation cannot release this owner"
    );
    runtime.observe_fixture(
        ExecutionObservation::Unknown(UnknownReason::Denied),
        vec![(42, 142)],
    );
    refresh();
    assert_eq!(module.value(), vec![7]);
    runtime.begin_shutdown();
}

#[test]
fn simple_public_refresh_reports_external_drift_without_restoration_or_recapture() {
    use super::{config::ModulePreference, TuningRequest as Request};
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let mut service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(MemorySave {
            saved: Default::default(),
            fail: false,
        }),
    );
    service.admit_test_modules();
    let profile = service
        .create_profile(
            "Saved".into(),
            vec![ModulePreference {
                feature: module.id.clone(),
                enabled: true,
                mode: ActivationMode::Manual,
                config: module.config(12),
            }],
        )
        .unwrap();
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    submit(
        &runtime,
        Request::ActivateManual {
            profile,
            feature: module.id.clone(),
            target: TargetIdentity::Machine,
        },
    );
    module
        .fields
        .lock()
        .unwrap()
        .insert(ResourceId::new("value").unwrap(), vec![19]);
    let refreshed = submit(
        &runtime,
        Request::ModuleAction {
            feature: module.id.clone(),
            action: super::presentation::ModuleAction::Refresh,
        },
    );
    assert_eq!(
        refreshed.profiles[0].modules[0].status,
        Some(ModuleStatus::ExternalChange)
    );
    assert_eq!(module.value(), vec![19]);
    assert_eq!(*module.writes.lock().unwrap(), 1);
    submit(&runtime, Request::DisableProfile { profile });
    assert_eq!(module.value(), vec![19]);
    runtime.begin_shutdown();
}

struct SlowSave {
    gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
    entered: Arc<std::sync::atomic::AtomicBool>,
}
impl super::StatePersistence for SlowSave {
    fn save(
        &mut self,
        _: &mut crate::app::models::AppStateStorage,
    ) -> Result<super::service::SavedState, super::SaveError> {
        self.entered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (lock, changed) = &*self.gate;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = changed.wait(open).unwrap();
        }
        Ok(super::service::SavedState::Unavailable)
    }
}
#[test]
fn simple_public_shutdown_during_save_keeps_worker_and_reports_pending_until_terminal() {
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let service = super::Tuning::from_registry(Registry::default()).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(SlowSave {
            gate: gate.clone(),
            entered: entered.clone(),
        }),
    );
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    let started = std::time::Instant::now();
    runtime
        .request(super::TuningRequest::CreateProfile {
            name: "Saved".into(),
        })
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_millis(200));
    while !entered.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    runtime.begin_shutdown();
    assert!(!runtime.snapshot().shutdown_complete);
    runtime.expire_shutdown_for_test();
    assert_eq!(
        runtime.snapshot().shutdown_outcome,
        Some(super::platform::controller::ShutdownResult::TimedOut)
    );
    assert!(!runtime.snapshot().shutdown_complete);
    assert_eq!(
        runtime
            .request(super::TuningRequest::CreateProfile {
                name: "Late".into()
            })
            .unwrap_err(),
        super::SaveError::Recovering
    );
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    let start = std::time::Instant::now();
    while !runtime.snapshot().shutdown_complete {
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let terminal = runtime.snapshot();
    assert_eq!(terminal.profiles[0].name, "Saved");
    assert_eq!(
        terminal.shutdown_outcome,
        Some(super::platform::controller::ShutdownResult::Complete)
    );
}

#[test]
fn simple_review_partial_session_failure_retains_actor_for_exit_and_disable_cleanup() {
    use super::{
        config::ModulePreference,
        execution_bridge::{ExecutionObservation, Member, ProcessFacts, UnknownReason},
        TuningRequest as Request,
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup};
    for disable in [false, true] {
        let module = Setting::new("user_setting", true);
        *module.fail_at.lock().unwrap() = Some(ResourceId::new("second").unwrap());
        let mut registry = Registry::default();
        registry.register(module.clone()).unwrap();
        let app = AppToRun::new_installed(
            "fixture".into(),
            "fixture".into(),
            os_api::PriorityClass::Normal,
            false,
        );
        let state = AppStateStorage::test_fixture(|state| {
            state.groups = vec![CoreGroup {
                name: "Games".into(),
                cores: vec![],
                programs: vec![app.clone()],
                is_hidden: false,
                run_all_button: false,
            }];
            state.rule_identities = None;
        });
        let mut service = super::Tuning::from_registry(registry).headless(
            state,
            Box::new(MemorySave {
                saved: Default::default(),
                fail: false,
            }),
        );
        service.admit_test_modules();
        let group = service.rules().group_id_for_index(0).unwrap();
        let rule = service.rules().rule_id_for_index(0, 0).unwrap();
        let profile = service
            .create_profile(
                "Session".into(),
                vec![ModulePreference {
                    feature: module.id.clone(),
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
        let facts = ProcessFacts {
            pid: 42,
            token: 142,
            image: Arc::from("fixture.exe"),
            #[cfg(windows)]
            native: None,
        };
        let observed = ExecutionObservation::fixture(
            &saved,
            1,
            vec![Member {
                key: app.get_key(),
                group,
                rule,
                instances: vec![facts.clone()],
            }],
            facts,
        );
        let runtime = super::TuningRuntime::start(
            service,
            Arc::new(tokio::sync::RwLock::new(Default::default())),
        );
        runtime.observe_fixture(observed, vec![]);
        let partial = submit(
            &runtime,
            Request::ModuleAction {
                feature: module.id.clone(),
                action: super::presentation::ModuleAction::Refresh,
            },
        );
        assert_eq!(module.value(), vec![12]);
        assert_eq!(
            partial.profiles[0].modules[0].status,
            Some(ModuleStatus::PartialFailure)
        );
        *module.fail_at.lock().unwrap() = None;
        runtime.observe_fixture(
            ExecutionObservation::Unknown(UnknownReason::Denied),
            if disable { vec![] } else { vec![(42, 142)] },
        );
        if disable {
            submit(&runtime, Request::DisableProfile { profile });
        } else {
            submit(
                &runtime,
                Request::ModuleAction {
                    feature: module.id.clone(),
                    action: super::presentation::ModuleAction::Refresh,
                },
            );
        }
        assert_eq!(
            module.value(),
            vec![7],
            "partial session must retain its actor for cleanup"
        );
        assert!(!module
            .fields
            .lock()
            .unwrap()
            .contains_key(&ResourceId::new("second").unwrap()));
        assert_eq!(*module.writes.lock().unwrap(), 2);
        runtime.begin_shutdown();
    }
}

#[test]
fn simple_review_blocked_idle_observation_keeps_ui_requests_and_shutdown_responsive() {
    let module = Setting::new("scalar", false);
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    *module.observation_gate.lock().unwrap() = Some(gate.clone());
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let service = super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(MemorySave {
            saved: Default::default(),
            fail: false,
        }),
    );
    let runtime = Arc::new(super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    ));
    let entered = std::time::Instant::now();
    while !module
        .observation_entered
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        assert!(entered.elapsed() < std::time::Duration::from_secs(3));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let ui = runtime.clone();
    let caller = std::thread::spawn(move || {
        let before = ui.snapshot();
        let request = ui.request(super::TuningRequest::CreateProfile {
            name: "Queued".into(),
        });
        ui.begin_shutdown();
        sender
            .send((before.shutdown_complete, request, ui.snapshot()))
            .unwrap();
    });
    let responsive = receiver.recv_timeout(std::time::Duration::from_millis(300));
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    caller.join().unwrap();
    assert!(
        responsive.is_ok(),
        "native observation held a UI-facing mutex"
    );
    let (complete, request, snapshot) = responsive.unwrap();
    assert!(!complete);
    assert!(request.is_ok());
    assert!(!snapshot.shutdown_complete);
    assert!(matches!(
        snapshot.operation,
        super::TuningOperationState::Pending { .. }
    ));
}

#[test]
fn simple_review_foreground_uses_current_saved_trigger_and_positive_foreground_facts() {
    use super::{
        config::ModulePreference,
        execution_bridge::{ExecutionObservation, Member, ProcessFacts},
        TuningRequest as Request,
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup};
    let module = Setting::new("scalar", false);
    let mut registry = Registry::default();
    registry.register(module.clone()).unwrap();
    let app = AppToRun::new_installed(
        "fixture".into(),
        "fixture".into(),
        os_api::PriorityClass::Normal,
        false,
    );
    let state = AppStateStorage::test_fixture(|state| {
        state.groups = vec![CoreGroup {
            name: "Games".into(),
            cores: vec![],
            programs: vec![app.clone()],
            is_hidden: false,
            run_all_button: false,
        }];
        state.rule_identities = None;
    });
    let mut service = super::Tuning::from_registry(registry).headless(
        state,
        Box::new(MemorySave {
            saved: Default::default(),
            fail: false,
        }),
    );
    service.admit_test_modules();
    let group = service.rules().group_id_for_index(0).unwrap();
    let rule = service.rules().rule_id_for_index(0, 0).unwrap();
    let profile = service
        .create_profile(
            "Session".into(),
            vec![ModulePreference {
                feature: module.id.clone(),
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
    let facts = ProcessFacts {
        pid: 42,
        token: 142,
        image: Arc::from("fixture.exe"),
        #[cfg(windows)]
        native: None,
    };
    let background = ProcessFacts {
        pid: 99,
        token: 199,
        image: Arc::from("other.exe"),
        #[cfg(windows)]
        native: None,
    };
    let member = Member {
        key: app.get_key(),
        group: group.clone(),
        rule: rule.clone(),
        instances: vec![facts.clone()],
    };
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    let refresh = || {
        submit(
            &runtime,
            Request::ModuleAction {
                feature: module.id.clone(),
                action: super::presentation::ModuleAction::Refresh,
            },
        )
    };
    runtime.observe_fixture(
        ExecutionObservation::fixture(&saved, 1, vec![member.clone()], background.clone()),
        vec![],
    );
    refresh();
    assert_eq!(module.value(), vec![12]);
    let changed = submit(
        &runtime,
        Request::BindRule {
            group,
            rule,
            profile,
            eligibility: ActivationEligibility::Foreground,
        },
    );
    runtime.observe_fixture(
        ExecutionObservation::fixture(&changed.state, 2, vec![member.clone()], background),
        vec![],
    );
    refresh();
    assert_eq!(
        module.value(),
        vec![7],
        "new saved foreground trigger must release background running intent"
    );
    runtime.observe_fixture(
        ExecutionObservation::fixture(&changed.state, 3, vec![member], facts.clone()),
        vec![],
    );
    refresh();
    assert_eq!(module.value(), vec![12]);
    runtime.observe_fixture(
        ExecutionObservation::fixture(&changed.state, 4, vec![], facts),
        vec![],
    );
    refresh();
    assert_eq!(
        module.value(),
        vec![12],
        "missing membership is not positive foreground loss"
    );
    runtime.begin_shutdown();
}
