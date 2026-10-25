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

#[test]
fn background_checkbox_cannot_save_or_activate_hidden_legacy_core_placement() {
    let (mut service, _) = service();
    let legacy = Config {
        games: vec![GameRule {
            group: "g".into(),
            rule: "r".into(),
        }],
        cpu_mode: CpuMode::EfficientCores,
        manual_ids: vec![7],
        ..Default::default()
    };
    service.configure_background(false, legacy).unwrap();
    let before = service.state().tuning.raw().unwrap().to_owned();
    let (_, _, active) = policies(&service).pop().unwrap();
    assert_eq!(active.cpu_mode, CpuMode::PriorityEco);
    assert!(active.manual_ids.is_empty());
    assert_eq!(
        service.state().tuning.raw(),
        Some(before.as_str()),
        "reading policy must retain old recovery-compatible state"
    );
    let mut saved = service.configured().unwrap();
    let draft = rule_policy(&saved, "r").unwrap();
    put_rule_policy(
        &mut saved,
        GameRule {
            group: "moved".into(),
            rule: "r".into(),
        },
        &draft,
    )
    .unwrap();
    let stored = rule_policy(&saved, "r").unwrap();
    assert_eq!(stored.config.cpu_mode, CpuMode::PriorityEco);
    assert!(stored.config.manual_ids.is_empty());
    assert!(stored.config.manual_topology.is_empty());
    assert_eq!(stored.config.games[0].group, "g");
}

#[test]
fn background_per_rule_defaults_off_and_splits_legacy_selection_without_resetting_other_game() {
    use super::super::config::TuningConfig;
    let mut saved = TuningConfig::default();
    let first = GameRule {
        group: "old-group".into(),
        rule: "first".into(),
    };
    let second = GameRule {
        group: "old-group".into(),
        rule: "second".into(),
    };
    assert_eq!(rule_policy(&saved, "first").unwrap(), RulePolicy::default());
    let legacy = Config {
        games: vec![first.clone(), second.clone()],
        cpu_mode: CpuMode::EfficientCores,
        ..Default::default()
    };
    saved.profiles.push(Profile {
        version: 1,
        id: ProfileId::new(1).unwrap(),
        name: PROFILE_NAME.into(),
        modules: vec![ModulePreference {
            feature: feature(),
            enabled: true,
            mode: ActivationMode::Session,
            config: legacy.envelope(),
        }],
    });
    saved.next_profile = 2;
    let mut draft = rule_policy(&saved, "first").unwrap();
    assert!(draft.enabled);
    draft.enabled = false;
    draft.config.cpu_mode = CpuMode::PriorityEco;
    put_rule_policy(&mut saved, first, &draft).unwrap();
    assert_eq!(
        rule_policy(&saved, "first").unwrap().config.cpu_mode,
        CpuMode::PriorityEco
    );
    assert!(!rule_policy(&saved, "first").unwrap().enabled);
    assert!(rule_policy(&saved, "second").unwrap().enabled);
    assert_eq!(
        rule_policy(&saved, "second").unwrap().config.cpu_mode,
        CpuMode::EfficientCores
    );
    assert_eq!(saved.profiles.len(), 2);
    let existing = saved.clone();
    put_rule_policy(
        &mut saved,
        GameRule {
            group: "new-group".into(),
            rule: "first".into(),
        },
        &draft,
    )
    .unwrap();
    assert_eq!(
        saved, existing,
        "group moves must preserve the saved session identity"
    );
}

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
    service_with_registry(Registry::default())
}
fn service_with_registry(registry: Registry) -> (HeadlessTuning, Arc<AtomicBool>) {
    let fail = Arc::new(AtomicBool::new(false));
    let mut service = super::super::Tuning::from_registry(registry).headless(
        crate::app::models::AppStateStorage::test_fixture(|_| {}),
        Box::new(Save(fail.clone())),
    );
    service.admit_test_modules();
    (service, fail)
}

struct GamesFixture {
    service: HeadlessTuning,
    module: Arc<Fixture>,
    now: Instant,
    group: crate::app::shared::ids::GroupId,
    destination: crate::app::shared::ids::GroupId,
    rules: Vec<crate::app::shared::ids::RuleId>,
    owners: Vec<OwnerKey>,
}
impl GamesFixture {
    fn new() -> Self {
        use crate::app::models::{AppToRun, CoreGroup, LaunchTarget};
        let module = Arc::new(Fixture {
            value: Mutex::new(vec![7]),
            exited: AtomicBool::new(false),
        });
        let mut registry = Registry::default();
        registry.register(module.clone()).unwrap();
        let (mut service, _) = service_with_registry(registry);
        service
            .save_base(|state| {
                state.groups = vec![
                    CoreGroup {
                        name: "Games".into(),
                        cores: vec![0],
                        is_hidden: false,
                        run_all_button: false,
                        programs: ["first", "second"]
                            .into_iter()
                            .map(|name| AppToRun {
                                name: name.into(),
                                launch_target: LaunchTarget::Path {
                                    dropped_path: format!("C:\\{name}.exe").into(),
                                    bin_path: format!("C:\\{name}.exe").into(),
                                },
                                args: vec![],
                                additional_processes: vec![],
                                manage_descendants: false,
                                autorun: false,
                                priority: os_api::PriorityClass::Normal,
                            })
                            .collect(),
                    },
                    CoreGroup {
                        name: "Moved".into(),
                        cores: vec![1],
                        is_hidden: false,
                        run_all_button: false,
                        programs: vec![],
                    },
                ];
                Ok(())
            })
            .unwrap();
        let group = service.rules().group_id_for_index(0).unwrap();
        let destination = service.rules().group_id_for_index(1).unwrap();
        let rules: Vec<_> = (0..2)
            .map(|index| service.rules().rule_id_for_index(0, index).unwrap())
            .collect();
        for (index, rule) in rules.iter().enumerate() {
            let app = service.state().groups[0].programs[index].clone();
            service
                .edit_rule_background(
                    &group,
                    rule,
                    app,
                    Some(RulePolicy {
                        enabled: true,
                        ..Default::default()
                    }),
                )
                .unwrap();
        }
        let now = Instant::now();
        let mut owners = vec![];
        for (index, rule) in rules.iter().enumerate() {
            let (profile, _, config) = policies(&service)
                .into_iter()
                .find(|(_, _, config)| config.games.iter().any(|game| game.rule == rule.0))
                .unwrap();
            let source = facts(30 + index as u32);
            let owner = OwnerKey::SessionTarget {
                profile,
                feature: feature(),
                source: SessionSource::Game {
                    group: group.0.clone(),
                    rule: rule.0.clone(),
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
                    config.envelope(),
                    ActivationFacts {
                        pid: source.pid,
                        token: source.token,
                        image: source.image.to_string(),
                        trigger: ActivationEligibility::Running,
                    },
                )
                .unwrap();
            service
                .background
                .source_exit
                .insert((source.pid, source.token), false);
            service
                .background
                .acquired
                .insert(owner.clone(), config.envelope());
            owners.push(owner);
        }
        service.background.next_scan = Some(now + Duration::from_secs(1000));
        Self {
            service,
            module,
            now,
            group,
            destination,
            rules,
            owners,
        }
    }
    fn legacy_shared() -> Self {
        let mut fixture = Self::new();
        let mut saved = fixture.service.configured().unwrap();
        let config = Config {
            games: fixture
                .rules
                .iter()
                .map(|rule| GameRule {
                    group: fixture.group.0.clone(),
                    rule: rule.0.clone(),
                })
                .collect(),
            ..Default::default()
        };
        saved.profiles.truncate(1);
        saved.profiles[0].modules[0].config = config.envelope();
        fixture.service.undo_background().unwrap();
        fixture.service.save_config(saved).unwrap();
        for owner in &mut fixture.owners {
            let OwnerKey::SessionTarget {
                profile, source, ..
            } = owner
            else {
                unreachable!()
            };
            *profile = ProfileId::new(1).unwrap();
            let (pid, token) = source.instance();
            fixture
                .service
                .controller
                .acquire_session(
                    owner.clone(),
                    fixture.module.clone(),
                    TargetIdentity::Process {
                        pid: 20,
                        creation_token: 120,
                        image_path: "C:\\fixture.exe".into(),
                    },
                    config.envelope(),
                    ActivationFacts {
                        pid,
                        token,
                        image: facts(pid).image.to_string(),
                        trigger: ActivationEligibility::Running,
                    },
                )
                .unwrap();
            fixture
                .service
                .background
                .acquired
                .insert(owner.clone(), config.envelope());
        }
        fixture.service.background.next_scan = Some(fixture.now + Duration::from_secs(1000));
        fixture
    }
    fn tick_unknown(&mut self) {
        tick(
            &mut self.service,
            &ExecutionObservation::Unknown(
                super::super::execution_bridge::UnknownReason::Unavailable,
            ),
            self.now,
        );
    }
}

#[test]
fn s92_profile_cleanup_waits_for_recovery_admission() {
    let mut fixture = GamesFixture::new();
    fixture.service.undo_background().unwrap();
    fixture.service.native_admitted = false;
    fixture.service.remove_group(&fixture.group).unwrap();
    assert_eq!(fixture.service.configured().unwrap().profiles.len(), 2);
    fixture.service.admit_test_modules();
    fixture.service.save_base(|_| Ok(())).unwrap();
    assert!(fixture.service.configured().unwrap().profiles.is_empty());
}

#[test]
fn s92_profile_cleanup_reuses_the_cap_and_keeps_live_recovery_and_custom_profiles() {
    use super::super::config::TuningConfig;
    let mut saved = TuningConfig::default();
    let draft = RulePolicy {
        enabled: true,
        ..Default::default()
    };
    for index in 0..64 {
        put_rule_policy(
            &mut saved,
            GameRule {
                group: "g".into(),
                rule: format!("r{index}"),
            },
            &draft,
        )
        .unwrap();
    }
    assert!(put_rule_policy(
        &mut saved,
        GameRule {
            group: "g".into(),
            rule: "new".into()
        },
        &draft
    )
    .is_err());
    // Custom profile names are not authority to discard user data.
    saved.profiles[4].name = "Custom background policy".into();
    let original_live = saved.profiles[3].clone();
    let original_recovery = saved.profiles[6].clone();
    let live = ["r3".into()].into();
    let retained = original_recovery.id;
    assert!(prune_deleted_rule_profiles(&mut saved, &live, |id| id == retained));
    assert_eq!(saved.profiles.len(), 3);
    assert!(saved.profiles.contains(&original_live));
    assert!(saved.profiles.contains(&original_recovery));
    assert!(!prune_deleted_rule_profiles(&mut saved, &live, |id| id == retained));
    put_rule_policy(
        &mut saved,
        GameRule {
            group: "g".into(),
            rule: "new".into(),
        },
        &draft,
    )
    .unwrap();
    assert_eq!(saved.profiles.len(), 4);
}

#[test]
fn s92_legacy_profile_lookup_decodes_the_document_once() {
    let fixture = GamesFixture::new();
    let policies = policies(&fixture.service);
    let saved = fixture.service.configured().unwrap();
    let mut reads = 0;
    assert_eq!(
        legacy_profile(&policies, || {
            reads += 1;
            TuningLoad::Known(Box::new(saved.clone()))
        }),
        None
    );
    assert_eq!(
        reads, 1,
        "normal per-rule profiles must not trigger full JSON decode per policy"
    );
}

#[test]
fn s92_deleted_background_profiles_release_capacity_after_recovery_retires() {
    let mut fixture = GamesFixture::new();
    fixture.service.remove_group(&fixture.group).unwrap();
    assert_eq!(
        fixture.service.configured().unwrap().profiles.len(),
        2,
        "pending owners must retain the original recovery profiles"
    );
    fixture.tick_unknown();
    fixture.service.save_base(|_| Ok(())).unwrap();
    assert!(
        fixture.service.configured().unwrap().profiles.is_empty(),
        "deleted rules must not permanently consume the shared profile capacity"
    );
}

#[test]
fn s92_deleted_background_profiles_are_pruned_without_an_owner() {
    let mut fixture = GamesFixture::new();
    fixture.service.undo_background().unwrap();
    fixture
        .service
        .remove_rule(&fixture.group, &fixture.rules[0])
        .unwrap();
    assert_eq!(fixture.service.configured().unwrap().profiles.len(), 1);
    fixture.service.remove_group(&fixture.group).unwrap();
    assert!(fixture.service.configured().unwrap().profiles.is_empty());
}

#[test]
fn background_removing_live_game_rule_releases_only_its_owner_before_last_undo() {
    let mut fixture = GamesFixture::new();
    fixture
        .service
        .remove_rule(&fixture.group, &fixture.rules[0])
        .unwrap();
    fixture.tick_unknown();
    assert_eq!(
        fixture.service.controller.owners(),
        [fixture.owners[1].clone()].into(),
        "Removing a saved trigger must withdraw its background ownership even while the process is live"
    );
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![12]);
    fixture.service.undo_background().unwrap();
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
    assert!(fixture.service.controller.owners().is_empty());
}

#[test]
fn background_removing_group_restores_live_targets_without_waiting_for_game_exit() {
    let mut fixture = GamesFixture::new();
    fixture.service.remove_group(&fixture.group).unwrap();
    fixture.tick_unknown();
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
    assert!(fixture.service.controller.owners().is_empty());
}

#[test]
fn background_moving_rule_and_unknown_observation_preserve_shared_original_baseline() {
    let mut fixture = GamesFixture::new();
    fixture
        .service
        .move_rule_at(&fixture.group, &fixture.rules[0], &fixture.destination, 0)
        .unwrap();
    fixture.tick_unknown();
    assert_eq!(fixture.service.controller.owners().len(), 2);
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![12]);
    assert_eq!(fixture.service.controller.fields()[0].baseline, vec![7]);
    fixture
        .service
        .background
        .source_exit
        .insert((30, facts(30).token), true);
    fixture.tick_unknown();
    assert_eq!(
        fixture.service.controller.owners(),
        [fixture.owners[1].clone()].into()
    );
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![12]);
    fixture.service.undo_background().unwrap();
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
}

#[test]
fn background_checkbox_off_releases_one_shared_owner_and_preserves_other_game() {
    let mut fixture = GamesFixture::new();
    let app = fixture.service.state().groups[0].programs[0].clone();
    fixture
        .service
        .edit_rule_background(
            &fixture.group,
            &fixture.rules[0],
            app,
            Some(RulePolicy::default()),
        )
        .unwrap();
    fixture.tick_unknown();
    assert_eq!(
        fixture.service.controller.owners(),
        [fixture.owners[1].clone()].into()
    );
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![12]);
    assert!(
        rule_policy(&fixture.service.configured().unwrap(), &fixture.rules[1].0)
            .unwrap()
            .enabled
    );
    fixture.service.undo_background().unwrap();
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
}

#[test]
fn background_legacy_checkbox_split_keeps_the_unedited_running_games_owner() {
    let mut fixture = GamesFixture::legacy_shared();
    let app = fixture.service.state().groups[0].programs[0].clone();
    fixture
        .service
        .edit_rule_background(
            &fixture.group,
            &fixture.rules[0],
            app,
            Some(RulePolicy::default()),
        )
        .unwrap();
    fixture.tick_unknown();
    assert_eq!(
        fixture.service.controller.owners(),
        [fixture.owners[1].clone()].into(),
        "Splitting one legacy checkbox must retain its unedited peer's original owner and baseline"
    );
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![12]);
    assert_eq!(fixture.service.controller.fields()[0].baseline, vec![7]);
    fixture.service.undo_background().unwrap();
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
}

#[test]
fn background_changed_policy_withdraws_its_owner_without_resetting_peer() {
    let mut fixture = GamesFixture::new();
    let mut draft =
        rule_policy(&fixture.service.configured().unwrap(), &fixture.rules[0].0).unwrap();
    draft
        .config
        .excluded_roles
        .push(os_api::background::Role::DeviceCensus);
    let app = fixture.service.state().groups[0].programs[0].clone();
    fixture
        .service
        .edit_rule_background(&fixture.group, &fixture.rules[0], app, Some(draft))
        .unwrap();
    fixture.tick_unknown();
    assert_eq!(
        fixture.service.controller.owners(),
        [fixture.owners[1].clone()].into(),
        "The selector projection must not hide a real module policy change"
    );
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![12]);
    fixture.service.undo_background().unwrap();
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
}

#[test]
fn background_selected_rule_beyond_choice_limit_keeps_ownership_after_reorder() {
    let mut fixture = GamesFixture::new();
    let template = fixture.service.state().groups[0].programs[0].clone();
    fixture
        .service
        .save_base(|state| {
            state.groups[1].programs = (0..257)
                .map(|index| {
                    let mut app = template.clone();
                    app.name = format!("Filler {index}");
                    app.launch_target = crate::app::models::LaunchTarget::Path {
                        dropped_path: format!("C:\\filler-{index}.exe").into(),
                        bin_path: format!("C:\\filler-{index}.exe").into(),
                    };
                    app
                })
                .collect();
            Ok(())
        })
        .unwrap();
    fixture
        .service
        .move_rule_at(&fixture.group, &fixture.rules[0], &fixture.destination, 257)
        .unwrap();
    assert!(
        !game_choices(fixture.service.state(), fixture.service.rules())
            .iter()
            .any(|game| game.rule == fixture.rules[0].0)
    );
    fixture.tick_unknown();
    assert_eq!(fixture.service.controller.owners().len(), 2);
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![12]);
    assert_eq!(fixture.service.controller.fields()[0].baseline, vec![7]);
    fixture
        .service
        .move_rule_at(&fixture.destination, &fixture.rules[0], &fixture.group, 0)
        .unwrap();
    fixture.tick_unknown();
    assert_eq!(fixture.service.controller.owners().len(), 2);
    fixture.service.undo_background().unwrap();
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
}

#[test]
fn background_rule_edit_saves_app_and_policy_atomically_and_preserves_other_rules() {
    use crate::app::models::{AppToRun, CoreGroup, LaunchTarget};
    let (mut service, fail) = service();
    service
        .save_base(|state| {
            state.groups.push(CoreGroup {
                name: "Games".into(),
                cores: vec![0],
                is_hidden: false,
                run_all_button: false,
                programs: ["first", "second"]
                    .into_iter()
                    .map(|name| AppToRun {
                        name: name.into(),
                        launch_target: LaunchTarget::Path {
                            dropped_path: format!("C:\\{name}.exe").into(),
                            bin_path: format!("C:\\{name}.exe").into(),
                        },
                        args: vec![],
                        additional_processes: vec![],
                        manage_descendants: false,
                        autorun: false,
                        priority: os_api::PriorityClass::Normal,
                    })
                    .collect(),
            });
            Ok(())
        })
        .unwrap();
    let group = service.rules().group_id_for_index(0).unwrap();
    let first = service.rules().rule_id_for_index(0, 0).unwrap();
    let second = service.rules().rule_id_for_index(0, 1).unwrap();
    let before = serde_json::to_value(service.state()).unwrap();
    let mut app = service.state().groups[0].programs[0].clone();
    app.name = "Renamed".into();
    let draft = RulePolicy {
        enabled: true,
        ..Default::default()
    };
    fail.store(true, Ordering::Relaxed);
    assert_eq!(
        service.edit_rule_background(&group, &first, app.clone(), Some(draft.clone())),
        Err(SaveError::Storage)
    );
    assert_eq!(serde_json::to_value(service.state()).unwrap(), before);
    fail.store(false, Ordering::Relaxed);
    service
        .edit_rule_background(&group, &first, app, Some(draft))
        .unwrap();
    let saved = service.configured().unwrap();
    assert!(rule_policy(&saved, &first.0).unwrap().enabled);
    assert!(
        snapshot(&service).enabled,
        "per-app policies must appear armed without the retired shared card profile"
    );
    assert!(!rule_policy(&saved, &second.0).unwrap().enabled);
    assert_eq!(service.state().groups[0].programs[0].name, "Renamed");
    assert_eq!(
        service.state().groups[0].programs[0].priority,
        os_api::PriorityClass::Normal
    );
    // A plain API update has no authority to reset the separate per-rule option.
    let app = service.state().groups[0].programs[0].clone();
    service.edit_rule(&group, &first, app).unwrap();
    assert!(
        rule_policy(&service.configured().unwrap(), &first.0)
            .unwrap()
            .enabled
    );
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
    let owner = acquire_preview(&mut service, module.clone(), now);
    (service, module, now, owner)
}
fn acquire_preview(service: &mut HeadlessTuning, module: Arc<Fixture>, now: Instant) -> OwnerKey {
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
    owner
}

#[derive(Default)]
struct RecoveryMemory {
    bytes: Option<Vec<u8>>,
    fail_after_restore: bool,
}
struct Store(Arc<Mutex<RecoveryMemory>>);
impl super::super::persistence::RecoveryStore for Store {
    fn load(&mut self) -> Result<Option<super::super::persistence::RecoveryDocument>, ModuleError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .bytes
            .as_ref()
            .map(|bytes| serde_json::from_slice(bytes).unwrap()))
    }
    fn publish(
        &mut self,
        document: &super::super::persistence::RecoveryDocument,
    ) -> Result<(), ModuleError> {
        let mut memory = self.0.lock().unwrap();
        if memory.fail_after_restore
            && document
                .fields
                .iter()
                .any(|field| field.current == field.baseline && field.attempted.is_none())
        {
            memory.fail_after_restore = false;
            return Err(ModuleError::new(
                ErrorKind::Write,
                "fixture recovery publication",
            ));
        }
        memory.bytes = Some(serde_json::to_vec(document).unwrap());
        Ok(())
    }
}
struct DurablePreview {
    service: HeadlessTuning,
    module: Arc<Fixture>,
    memory: Arc<Mutex<RecoveryMemory>>,
    now: Instant,
    owner: OwnerKey,
}
impl DurablePreview {
    fn new() -> Self {
        let module = Arc::new(Fixture {
            value: Mutex::new(vec![7]),
            exited: AtomicBool::new(false),
        });
        let mut registry = Registry::default();
        registry.register(module.clone()).unwrap();
        let memory = Arc::new(Mutex::new(RecoveryMemory::default()));
        let (mut service, _) = service_with_registry(registry.clone());
        service.controller = super::super::platform::controller::Controller::restore(
            Box::new(Store(memory.clone())),
            "C:/fixture/state.json".into(),
            &registry,
        )
        .unwrap();
        service
            .configure_background(false, Config::default())
            .unwrap();
        let now = Instant::now();
        let owner = acquire_preview(&mut service, module.clone(), now);
        Self {
            service,
            module,
            memory,
            now,
            owner,
        }
    }
    fn reload(mut self) -> Self {
        let registry = self.service.registry().clone();
        let state = self.service.state().clone();
        drop(self.service);
        self.service = super::super::Tuning::from_registry(registry.clone())
            .headless(state, Box::new(Save(Arc::new(AtomicBool::new(false)))));
        self.service.admit_test_modules();
        self.service.controller = super::super::platform::controller::Controller::restore(
            Box::new(Store(self.memory.clone())),
            "C:/fixture/state.json".into(),
            &registry,
        )
        .unwrap();
        self
    }
    fn tick(&mut self, elapsed: u64) {
        tick(
            &mut self.service,
            &ExecutionObservation::Unknown(
                super::super::execution_bridge::UnknownReason::Unavailable,
            ),
            self.now + Duration::from_secs(elapsed),
        );
    }
    fn document(&self) -> super::super::persistence::RecoveryDocument {
        serde_json::from_slice(self.memory.lock().unwrap().bytes.as_ref().unwrap()).unwrap()
    }
}

#[test]
fn background_durable_preview_reload_restores_without_resuming_or_scanning() {
    let fixture = DurablePreview::new();
    let original = fixture.document();
    assert_eq!(original.fields[0].baseline, vec![7]);
    assert!(matches!(
        original.owners[0].owner,
        OwnerKey::SessionTarget {
            source: SessionSource::Preview { .. },
            ..
        }
    ));
    let mut fixture = fixture.reload();
    assert!(fixture.service.controller.needs_recovery(&fixture.owner));
    fixture.tick(0);
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
    assert!(fixture.document().fields.is_empty());
    assert!(fixture.document().owners.is_empty());
    assert!(fixture.service.background.preview.is_none());
    assert!(!fixture.service.background.snapshot.inspected);
    assert!(fixture.service.background.next_scan.is_none());
}

#[test]
fn background_durable_external_drift_survives_restart_and_repeated_undo() {
    let fixture = DurablePreview::new();
    *fixture.module.value.lock().unwrap() = vec![99];
    let mut fixture = fixture.reload();
    fixture.tick(0);
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![99]);
    assert_eq!(
        fixture.service.controller.status(&fixture.owner),
        Some(ModuleStatus::ExternalChange)
    );
    assert_eq!(fixture.document().fields[0].baseline, vec![7]);
    assert_eq!(fixture.document().fields[0].current, vec![12]);
    assert!(fixture.service.undo_background().is_err());
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![99]);
    let mut fixture = fixture.reload();
    fixture.tick(0);
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![99]);
    assert!(fixture.service.controller.owners().contains(&fixture.owner));
    assert!(fixture
        .service
        .background
        .snapshot
        .status
        .contains("Recovery needs attention"));
}

#[test]
fn background_restore_publication_fault_reloads_attempted_original_without_recapture() {
    let mut fixture = DurablePreview::new();
    fixture.memory.lock().unwrap().fail_after_restore = true;
    fixture.tick(600);
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
    assert!(!fixture.service.controller.can_admit());
    assert!(fixture.service.controller.owners().contains(&fixture.owner));
    let retained = fixture.document();
    assert_eq!(retained.fields[0].baseline, vec![7]);
    assert_eq!(retained.fields[0].current, vec![12]);
    assert_eq!(retained.fields[0].attempted, Some(vec![7]));
    let mut fixture = fixture.reload();
    fixture.tick(0);
    assert_eq!(*fixture.module.value.lock().unwrap(), vec![7]);
    assert!(fixture.document().fields.is_empty());
    assert!(fixture.document().owners.is_empty());
    assert!(fixture.service.controller.can_admit());
}

#[test]
fn background_off_and_armed_without_game_never_start_inventory_or_topology_reads() {
    let mut fixture = GamesFixture::new();
    fixture.service.undo_background().unwrap();
    fixture.service.background = Runtime::default();
    for elapsed in [0, 1, 5, 60, 600] {
        tick(
            &mut fixture.service,
            &ExecutionObservation::Unknown(
                super::super::execution_bridge::UnknownReason::Unavailable,
            ),
            fixture.now + Duration::from_secs(elapsed),
        );
    }
    assert!(!fixture.service.background.snapshot.inspected);
    assert!(fixture.service.background.topology_read.is_none());
    assert!(fixture.service.background.next_scan.is_none());
    let app = fixture.service.state().groups[0].programs[0].clone();
    fixture
        .service
        .edit_rule_background(
            &fixture.group,
            &fixture.rules[0],
            app,
            Some(RulePolicy {
                enabled: true,
                ..Default::default()
            }),
        )
        .unwrap();
    let observation = ExecutionObservation::fixture(fixture.service.state(), 1, vec![], facts(99));
    for elapsed in [601, 605, 660, 1200] {
        tick(
            &mut fixture.service,
            &observation,
            fixture.now + Duration::from_secs(elapsed),
        );
    }
    assert!(!fixture.service.background.snapshot.inspected);
    assert!(fixture.service.background.topology_read.is_none());
    assert!(fixture.service.background.next_scan.is_none());
    assert!(fixture
        .service
        .background
        .snapshot
        .status
        .starts_with("Armed"));
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
    let before = ExecutionObservation::fixture(&state, 1, vec![member("old", "r")], facts(99));
    assert_eq!(sources[0].0, game_sources(&config, &before)[0].0,
        "Moving a saved logical rule must not create a second trigger/owner or another session pool");
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
