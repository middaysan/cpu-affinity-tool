use super::*;
use crate::app::features::system_tweaks::{
    execution_bridge::{ExecutionObservation, Member, ProcessFacts},
    platform::controller::{LifetimeEvidence, RecoveryIntent, SessionSource},
};
use crate::app::features::system_tweaks::{
    platform::registry::{FieldChange, Module, ModuleObservation, Registry},
    presentation::ModuleEditor,
};
use std::sync::Arc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

struct Save(Arc<AtomicBool>);
impl super::super::StatePersistence for Save {
    fn save(
        &mut self,
        _: &mut crate::app::models::AppStateStorage,
    ) -> Result<super::super::service::SavedState, SaveError> {
        if self.0.load(Ordering::Relaxed) {
            Err(SaveError::Storage)
        } else {
            Ok(super::super::service::SavedState::Unavailable)
        }
    }
}
fn service() -> (HeadlessTuning, Arc<AtomicBool>) {
    let fail = Arc::new(AtomicBool::new(false));
    let mut service = super::super::Tuning::from_registry(Registry::default()).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(Save(fail.clone())),
    );
    service.admit_test_modules();
    (service, fail)
}

#[test]
fn background_policy_save_failure_keeps_previous_state_and_default_stays_off() {
    let (mut service, fail) = service();
    assert!(policies(&service).is_empty());
    assert!(!snapshot(&service).enabled);
    service
        .configure_background(false, Config::default())
        .unwrap();
    let old = service.state().tuning.raw().unwrap().to_string();
    let mut config = Config::default();
    config
        .excluded_roles
        .push(os_api::background::Role::DeviceCensus);
    fail.store(true, Ordering::Relaxed);
    assert_eq!(
        service.configure_background(false, config),
        Err(SaveError::Storage)
    );
    assert_eq!(service.state().tuning.raw(), Some(old.as_str()));
    assert!(!service.background.needs_work());
}

// Test-only module exercises the session/controller integration; it cannot be
// registered in the production registry or qualify a native system role.
struct Fixture {
    value: Mutex<Vec<u8>>,
    exited: AtomicBool,
}
impl ModuleEditor for Fixture {
    fn availability(&self) -> super::super::platforms::TweakAvailability {
        super::super::platforms::WINDOWS11.for_platform(
            super::super::platforms::OsPlatform::Windows {
                major: 10,
                minor: 0,
                build: 22621,
                workstation: true,
            },
        )
    }
    fn feature(&self) -> FeatureId {
        feature()
    }
    fn default_config(&self) -> super::super::platform::codec::ModuleEnvelope {
        Config::default().envelope()
    }
    fn accepts_config(&self, envelope: &super::super::platform::codec::ModuleEnvelope) -> bool {
        configured(envelope).is_ok()
    }
    fn show(
        &self,
        _: &mut eframe::egui::Ui,
        _: &mut super::super::platform::codec::ModuleEnvelope,
    ) -> Result<bool, ()> {
        Ok(false)
    }
}
impl Module for Fixture {
    fn requires_session_target(&self) -> bool {
        true
    }
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            id: feature(),
            title: "fixture",
            description: "fixture",
            access: ModuleAccess::Managed,
            target_scopes: vec![TargetScope::Process],
            activation_modes: vec![ActivationMode::Session],
            effect: Effect::Immediate,
            config_version: SchemaVersion::new(1).unwrap(),
        }
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn observe(&self, _: &TargetIdentity) -> ModuleObservation {
        ModuleObservation {
            capability: Capability::Available,
            effect: Effect::Immediate,
            summary: ObservationSummary {
                summary: "fixture".into(),
                effect: Effect::Immediate,
            },
        }
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        vec![]
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        _: &super::super::platform::codec::ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        Ok(vec![FieldChange {
            key: ResourceKey {
                target: target.clone(),
                field: ResourceId::new("fixture").unwrap(),
            },
            desired: vec![12],
        }])
    }
    fn read(&self, _: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn write(&self, _: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        let mut value = self.value.lock().unwrap();
        if value.as_slice() != expected {
            return Err(ModuleError::new(ErrorKind::ExternalChange, "fixture"));
        }
        *value = desired.to_vec();
        Ok(())
    }
    fn target_has_expired(&self, _: &TargetIdentity) -> Result<bool, ModuleError> {
        Ok(self.exited.load(Ordering::Relaxed))
    }
}
fn owned_preview() -> (HeadlessTuning, Arc<Fixture>, Instant, OwnerKey) {
    let (mut service, _) = service();
    service
        .configure_background(false, Config::default())
        .unwrap();
    let module = Arc::new(Fixture {
        value: Mutex::new(vec![7]),
        exited: AtomicBool::new(false),
    });
    let now = Instant::now();
    let source = facts(1);
    let owner = OwnerKey::SessionTarget {
        profile: ProfileId::new(1).unwrap(),
        feature: feature(),
        source: SessionSource::Preview {
            pid: source.pid,
            token: source.token,
        },
        target_pid: 20,
        target_token: 120,
    };
    service
        .controller
        .acquire_session(
            owner.clone(),
            module.clone(),
            TargetIdentity::Process {
                pid: 20,
                creation_token: 120,
                image_path: "C:\\fixture.exe".into(),
            },
            Config::default().envelope(),
            ActivationFacts {
                pid: source.pid,
                token: source.token,
                image: source.image.to_string(),
                trigger: ActivationEligibility::Running,
            },
        )
        .unwrap();
    service.background.start_preview(source, now).unwrap();
    service.background.inspect_requested = false;
    service.background.next_scan = Some(now + Duration::from_secs(1000));
    service
        .background
        .acquired
        .insert(owner.clone(), Config::default().envelope());
    (service, module, now, owner)
}

#[test]
fn background_preview_expiry_restores_live_target_not_the_trigger_process() {
    let (mut service, module, now, _) = owned_preview();
    assert_eq!(*module.value.lock().unwrap(), vec![12]);
    tick(
        &mut service,
        &ExecutionObservation::Unknown(super::super::execution_bridge::UnknownReason::Unavailable),
        now + Duration::from_secs(600),
    );
    assert_eq!(*module.value.lock().unwrap(), vec![7]);
    assert!(service.controller.owners().is_empty());
    assert!(service.background.preview.is_none());
}
#[test]
fn background_preview_lifetime_does_not_depend_on_foreground_and_undo_clears_visible_state() {
    let (mut service, module, now, owner) = owned_preview();
    tick(
        &mut service,
        &ExecutionObservation::Unknown(
            super::super::execution_bridge::UnknownReason::ForegroundRace,
        ),
        now,
    );
    assert!(snapshot(&service).preview_until.is_some());
    assert_eq!(
        service.controller.status(&owner),
        Some(ModuleStatus::VerifiedActive(Effect::Immediate))
    );
    service.undo_background().unwrap();
    let snapshot = snapshot(&service);
    assert!(snapshot.preview_until.is_none());
    assert_eq!(snapshot.changed_targets, 0);
    assert!(snapshot.status.starts_with("Off"), "{}", snapshot.status);
    assert_eq!(*module.value.lock().unwrap(), vec![7]);
}

#[test]
fn background_restart_restores_retained_preview_and_never_resumes_it() {
    let (mut service, module, now, _) = owned_preview();
    service.background = Runtime::default();
    tick(
        &mut service,
        &ExecutionObservation::Unknown(super::super::execution_bridge::UnknownReason::Unavailable),
        now,
    );
    assert_eq!(*module.value.lock().unwrap(), vec![7]);
    assert!(service.controller.owners().is_empty());
}

#[test]
fn background_target_exit_retires_without_writing_to_replacement() {
    let (mut service, module, now, _) = owned_preview();
    module.exited.store(true, Ordering::Relaxed);
    tick(
        &mut service,
        &ExecutionObservation::Unknown(super::super::execution_bridge::UnknownReason::Unavailable),
        now + Duration::from_secs(600),
    );
    assert_eq!(
        *module.value.lock().unwrap(),
        vec![12],
        "exited target must not be restored through a reused PID"
    );
    assert!(service.controller.owners().is_empty());
}

#[test]
fn background_external_change_survives_expiry_and_retains_recovery_evidence() {
    let (mut service, module, now, owner) = owned_preview();
    *module.value.lock().unwrap() = vec![99];
    tick(
        &mut service,
        &ExecutionObservation::Unknown(super::super::execution_bridge::UnknownReason::Unavailable),
        now + Duration::from_secs(600),
    );
    assert_eq!(*module.value.lock().unwrap(), vec![99]);
    assert!(service.controller.owners().contains(&owner));
    assert_eq!(
        service.controller.status(&owner),
        Some(ModuleStatus::ExternalChange)
    );
    assert!(service
        .background
        .snapshot
        .status
        .contains("Recovery needs attention"));
}

fn facts(pid: u32) -> ProcessFacts {
    ProcessFacts {
        pid,
        token: 100 + pid as u64,
        image: Arc::from("C:\\Games\\game.exe"),
        #[cfg(windows)]
        native: None,
    }
}
fn member(group: &str, rule: &str) -> Member {
    Member {
        key: crate::app::models::AppToRun::new_installed(
            "game".into(),
            "game".into(),
            os_api::PriorityClass::Normal,
            false,
        )
        .get_key(),
        group: crate::app::shared::ids::GroupId(group.into()),
        rule: crate::app::shared::ids::RuleId(rule.into()),
        instances: vec![facts(30)],
    }
}

#[test]
fn background_game_membership_uses_rule_identity_after_group_move() {
    let state = crate::app::models::AppStateStorage::test_fixture(|_| {});
    let observation = ExecutionObservation::fixture(&state, 1, vec![member("new", "r")], facts(99));
    let config = Config {
        games: vec![GameRule {
            group: "old".into(),
            rule: "r".into(),
        }],
        ..Default::default()
    };
    let sources = game_sources(&config, &observation);
    assert_eq!(sources.len(), 1);
    assert!(
        matches!(&sources[0].0, SessionSource::Game{ group,rule,pid:30,.. } if group == "new" && rule == "r")
    );
}

#[test]
fn background_alt_tab_does_not_end_running_session_but_game_exit_does() {
    assert_eq!(
        retention_intent(true, false, LifetimeEvidence::Live),
        RecoveryIntent::Keep
    );
    assert_eq!(
        retention_intent(true, false, LifetimeEvidence::Unknown),
        RecoveryIntent::Unknown
    );
    assert_eq!(
        retention_intent(true, false, LifetimeEvidence::Exited),
        RecoveryIntent::Restore
    );
    assert_eq!(
        retention_intent(false, false, LifetimeEvidence::Unknown),
        RecoveryIntent::Restore
    );
    assert_eq!(
        retention_intent(true, true, LifetimeEvidence::Live),
        RecoveryIntent::Restore
    );
}

#[test]
fn background_preview_is_finite_and_does_not_extend_when_reopened() {
    let now = std::time::Instant::now();
    let mut runtime = Runtime::default();
    runtime.start_preview(facts(1), now).unwrap();
    let deadline = runtime.preview.as_ref().unwrap().deadline;
    assert!(runtime
        .start_preview(facts(1), now + std::time::Duration::from_secs(20))
        .is_err());
    assert_eq!(runtime.preview.as_ref().unwrap().deadline, deadline);
    assert!(!runtime.preview_expired(now + std::time::Duration::from_secs(599)));
    assert!(runtime.preview_expired(now + std::time::Duration::from_secs(600)));
}

#[test]
fn background_duplicate_rule_in_another_group_cannot_be_selected_twice() {
    let mut config = Config::default();
    config.games = vec![
        GameRule {
            group: "a".into(),
            rule: "r".into(),
        },
        GameRule {
            group: "b".into(),
            rule: "r".into(),
        },
    ];
    assert!(!config.valid());
}

#[test]
fn background_identity_verified_row_is_still_not_mutation_admitted() {
    let row = os_api::background::Candidate {
        role: os_api::background::Role::DiagnosticsTelemetry,
        pid: 60,
        process: None,
        session: Some(0),
        verified: true,
        proof: None,
        reason: String::new(),
    };
    assert_eq!(
        candidate_skip(&row, &Config::default(), &[]),
        Some(row.role.qualification_note().into())
    );
    let mut denied = row;
    denied.verified = false;
    denied.reason = "Denied: OpenProcess (5)".into();
    let reason = candidate_skip(&denied, &Config::default(), &[]).unwrap();
    assert!(reason.contains("Denied"), "{reason}");
    assert!(reason.contains("not qualified"), "{reason}");
}
