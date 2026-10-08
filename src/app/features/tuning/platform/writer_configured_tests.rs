//! Fixture-domain tests of the production consumer, not native scope authority.
use super::super::{configured_intent::*, contract::*, coordinator::*, journal::*};
use super::*;
use crate::app::features::tuning::testing::scalar::{self, ScalarConfig, ScalarModule};
use std::sync::Mutex;

#[derive(Clone)]
struct Input(Arc<Mutex<ConfiguredSnapshot>>);
impl ConfiguredIntentSource for Input {
    fn snapshot(&self) -> ConfiguredSnapshot {
        self.0.lock().unwrap().clone()
    }
}
#[derive(Default)]
struct Memory {
    bytes: Option<Vec<u8>>,
    publishes: usize,
    after_publish: Option<Arc<dyn Fn() + Send + Sync>>,
}
struct Store(Arc<Mutex<Memory>>);
impl JournalStore for Store {
    fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .bytes
            .clone()
            .map_or(StoredJournal::Absent, StoredJournal::Bytes))
    }
    fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
        let hook = {
            let mut memory = self.0.lock().unwrap();
            memory.bytes = Some(bytes.to_vec());
            memory.publishes += 1;
            memory.after_publish.clone()
        };
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    }
    fn validate_authority(&mut self) -> Result<(), JournalIoError> {
        Ok(())
    }
}
struct Control<'a>(&'a mut WorkerAuthority);
impl ExecutionControl for Control<'_> {
    fn before_command(
        &mut self,
        command: &Command,
        snapshot: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        self.0.admit_command(command, snapshot)
    }
    fn cancellation_requested(&self) -> bool {
        false
    }
    fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.0.check_write(intent)?;
        self.0.commit(
            JournalStamp::Recovery,
            JournalPhase::BeforeWrite {
                step: intent.step as u32,
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )?;
        Ok(())
    }
    fn before_execute(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
        self.0.after_intent()
    }
    fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.0.commit(
            JournalStamp::Recovery,
            JournalPhase::VerifiedWrite {
                step: intent.step as u32,
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )
    }
    fn commit_transition(&mut self, snapshot: &CoordinatorSnapshot) -> Result<(), GateError> {
        self.0
            .commit(JournalStamp::Recovery, JournalPhase::Transition, snapshot)
    }
    fn publish(&mut self, _: CoordinatorSnapshot) {}
}
struct FixtureControl;
impl ExecutionControl for FixtureControl {
    fn cancellation_requested(&self) -> bool {
        false
    }
    fn before_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
        Ok(())
    }
    fn after_verified_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
        Ok(())
    }
    fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
        Ok(())
    }
    fn publish(&mut self, _: CoordinatorSnapshot) {}
}
fn machine() -> MachineIdentity {
    MachineIdentity {
        id: MachineId::new([1; 16]).unwrap(),
    }
}
fn module(persistent: bool) -> ScalarModule {
    let mut descriptor = scalar::descriptor();
    if persistent {
        descriptor.resources[0].lifetime = ResourceLifetimeRequirement::PersistentMachine;
    }
    ScalarModule::with_descriptor(descriptor)
}
fn catalog(module: &ScalarModule) -> Catalog {
    let mut catalog = Catalog::default();
    catalog.register(module.clone()).unwrap();
    catalog
}
fn entry(module: &ScalarModule, owner: u64, decision: ConfiguredDecision) -> ConfiguredEntry {
    ConfiguredEntry {
        owner: OwnerId::new(owner).unwrap(),
        binding: ActivationBinding::Manual(ConfiguredIntent {
            profile: ProfileId::new(owner).unwrap(),
            generation: ConfigGeneration::new(1).unwrap(),
        }),
        config: module.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
        target: TargetIdentity::Machine(machine()),
        mode: ActivationMode::Manual,
        decision,
    }
}
fn acquire(entry: &ConfiguredEntry) -> Command {
    Command::Acquire {
        owner: entry.owner,
        binding: entry.binding.clone(),
        config: entry.config.clone(),
        target: entry.target.clone(),
        mode: entry.mode,
    }
}
fn input(entries: Vec<ConfiguredEntry>) -> Input {
    Input(Arc::new(Mutex::new(
        ConfiguredSnapshot::complete(1, entries).unwrap(),
    )))
}
fn record() -> JournalDocument {
    let module = module(true);
    let mut coordinator = Coordinator::new(catalog(&module));
    assert_eq!(
        coordinator
            .handle(
                acquire(&entry(&module, 1, ConfiguredDecision::Enabled)),
                &mut FixtureControl
            )
            .outcome,
        Outcome::Applied(Effect::Immediate)
    );
    JournalDocument::capture(
        machine(),
        3,
        JournalStamp::Recovery,
        JournalPhase::Recovery,
        &coordinator.snapshot(),
    )
    .unwrap()
}
fn memory(document: Option<&JournalDocument>) -> Arc<Mutex<Memory>> {
    Arc::new(Mutex::new(Memory {
        bytes: document.map(|doc| doc.to_bytes().unwrap()),
        ..Default::default()
    }))
}
fn loaded(memory: &Arc<Mutex<Memory>>) -> JournalDocument {
    let JournalLoad::Known(document) =
        JournalDocument::load(memory.lock().unwrap().bytes.clone().unwrap(), &machine())
    else {
        panic!("known retained fixture journal required");
    };
    document
}
fn open(
    module: &ScalarModule,
    memory: &Arc<Mutex<Memory>>,
    input: Input,
) -> (Coordinator, WorkerAuthority) {
    bootstrap_configured(
        catalog(module),
        machine(),
        Box::new(Store(memory.clone())),
        Arc::new(input),
        ScopeEvidence::fixture(|| Ok(machine())),
    )
}

#[test]
fn p4_saved_membership_recovery_keeps_original_source_at_both_crash_boundaries() {
    use super::super::recovery::{OwnerAttachment, RecoveryResolver, ResourceLifetime};
    use crate::app::{
        features::{
            rules::RulesContext,
            tuning::{config::TuningLoad, configured_source::SavedConfiguredSource},
        },
        models::{AppStateStorage, AppToRun, CoreGroup},
    };
    struct Actor(Vec<ProcessIdentity>);
    impl RecoveryResolver for Actor {
        fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            let ActivationBinding::Game(game) = &owner.binding else {
                return OwnerAttachment::Unknown;
            };
            let live: Vec<_> = game
                .processes()
                .iter()
                .filter(|p| self.0.contains(p))
                .cloned()
                .collect();
            if live.is_empty() {
                OwnerAttachment::Unknown
            } else {
                OwnerAttachment::VerifiedLive {
                    owner: owner.owner,
                    binding: game.clone(),
                    live_processes: live,
                }
            }
        }
        fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
            ResourceLifetime::VerifiedLive
        }
    }
    for durable_new in [false, true] {
        let module = module(false);
        let mut state = AppStateStorage::test_fixture(|state| {
            state.groups = vec![CoreGroup {
                name: "G".into(),
                cores: vec![],
                programs: vec![AppToRun::new_installed(
                    "fixture".into(),
                    "fixture".into(),
                    os_api::PriorityClass::Normal,
                    false,
                )],
                is_hidden: false,
                run_all_button: false,
            }]
        });
        let rules = RulesContext::from_storage(&state);
        state.mark_ready_for_current_schema_save(rules.to_persisted_identities());
        let session = SessionIdentity {
            user: UserIdentity {
                machine: machine(),
                sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(17).unwrap(),
            boot: BootId::new([2; 16]).unwrap(),
        };
        let old_process = ProcessIdentity::new(
            session.clone(),
            101,
            123,
            ImagePath::new("C:\\fixture\\game.exe").unwrap(),
        )
        .unwrap();
        let child = ProcessIdentity::new(
            session.clone(),
            102,
            124,
            ImagePath::new("C:\\fixture\\child.exe").unwrap(),
        )
        .unwrap();
        let old_game = GameActivation::new(
            ConfiguredIntent {
                profile: ProfileId::new(1).unwrap(),
                generation: ConfigGeneration::new(1).unwrap(),
            },
            RuleActivation::new(
                rules.group_id_for_index(0).unwrap(),
                rules.rule_id_for_index(0, 0).unwrap(),
            )
            .unwrap(),
            ActivationEligibility::Running,
            session,
            vec![old_process.clone()],
        )
        .unwrap();
        let new_game = old_game
            .with_membership(vec![old_process.clone(), child.clone()])
            .unwrap();
        let config = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let old = crate::app::features::tuning::config::SavedIntent {
            version: 1,
            owner: OwnerId::new(1).unwrap(),
            binding: ActivationBinding::Game(old_game.clone()),
            config: config.clone(),
            target: TargetIdentity::Machine(machine()),
            mode: ActivationMode::Session,
        };
        let mut new = old.clone();
        new.binding = ActivationBinding::Game(new_game.clone());
        state.tuning = serde_json::from_value(serde_json::json!({ "version":2,"lineage":vec![1;32],"profiles":[{"version":1,"id":1,"name":"P","modules":[{"feature":config.feature,"enabled":true,"mode":"Session","config":config}]}],"associations":[{"version":1,"group":old_game.rule().group(),"rule":old_game.rule().rule(),"profile":1,"eligibility":"Running"}],"allocations":{"next_profile":2,"next_owner":2,"next_config_generation":2},"activations":[],"withdrawals":[],"transitions":[{"version":2,"owner":1,"generation":1,"old":old,"new":new}]})).unwrap();
        assert!(matches!(state.tuning.decode(), TuningLoad::Known(_)));
        // These are independently saved and reread ordinary fixture bytes.
        let path = std::path::PathBuf::from("target").join(format!(
            "p4-membership-state-{}-{durable_new}.json",
            std::process::id()
        ));
        let bytes = serde_json::to_vec(&state).unwrap();
        let mut saved_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        use std::io::Write;
        saved_file.write_all(&bytes).unwrap();
        saved_file.sync_all().unwrap();
        drop(saved_file);
        let saved: Arc<[u8]> = Arc::from(std::fs::read(&path).unwrap());
        let mut coordinator = Coordinator::new(catalog(&module));
        assert_eq!(
            coordinator
                .handle(
                    Command::Acquire {
                        owner: old.owner,
                        binding: old.binding.clone(),
                        config: old.config.clone(),
                        target: old.target.clone(),
                        mode: old.mode
                    },
                    &mut FixtureControl
                )
                .outcome,
            Outcome::Applied(Effect::Immediate)
        );
        if durable_new {
            assert_eq!(
                coordinator
                    .handle(
                        Command::UpdateMembership {
                            owner: old.owner,
                            binding: new.binding.clone()
                        },
                        &mut FixtureControl
                    )
                    .outcome,
                Outcome::Unchanged
            );
        }
        let origin = ConfigurationOrigin::fixture();
        let mut original = JournalDocument::capture(
            machine(),
            3,
            JournalStamp::Recovery,
            JournalPhase::Transition,
            &coordinator.snapshot(),
        )
        .unwrap();
        original.origin = Some(origin.clone());
        let memory = memory(Some(&original));
        let read_back = saved.clone();
        let mut scope = ScopeEvidence::fixture_saved(
            origin.clone(),
            || Ok(machine()),
            saved.clone(),
            move || Ok(read_back.clone()),
        );
        if let ScopeEvidence::Fixture { lifetime, .. } = &mut scope {
            *lifetime = Some(Box::new(Actor(vec![old_process, child])));
        }
        let writes = module.writes();
        let (recovered, authority) = bootstrap_configured(
            catalog(&module),
            machine(),
            Box::new(Store(memory.clone())),
            Arc::new(SavedConfiguredSource::from_fixture(
                &saved,
                origin,
                &catalog(&module),
            )),
            scope,
        );
        assert!(
            !matches!(authority.status(), WriterStatus::Blocked(_)),
            "saved membership recovery must attach without recapture"
        );
        let recovered = recovered.snapshot();
        assert_eq!(recovered.owners[0].binding, new.binding);
        assert_eq!(recovered.resources[0].baseline.bytes(), 7i32.to_le_bytes());
        let retained = loaded(&memory);
        assert_eq!(retained.origin, original.origin);
        assert_eq!(retained.sources, original.sources);
        assert_eq!(
            retained.resources[0].baseline,
            original.resources[0].baseline
        );
        assert_eq!(
            retained.resources[0].confirmed,
            original.resources[0].confirmed
        );
        assert_eq!(module.value(), 12);
        assert_eq!(module.writes(), writes);
        assert_eq!(std::fs::read(&path).unwrap(), saved.as_ref());
        drop(authority);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());
    }
}

#[cfg(all(windows, feature = "tuning-test-controls"))]
#[test]
fn p1b_fixture_cleanup_recaptures_after_idle_and_preserves_original_baseline() {
    let original = record();
    let module = module(true);
    module.external_change(12);
    let memory = memory(Some(&original));
    let (mut coordinator, mut authority) = open(
        &module,
        &memory,
        input(vec![entry(&module, 1, ConfiguredDecision::Enabled)]),
    );
    assert!(!matches!(authority.status(), WriterStatus::Blocked(_)));
    let attached = loaded(&memory);
    assert_eq!(attached.sources, original.sources);
    assert_eq!(
        coordinator.snapshot().resources[0].baseline.bytes(),
        7i32.to_le_bytes()
    );
    assert_eq!(module.value(), 12);
    assert_eq!(module.writes(), 0, "bootstrap reattachment is read only");
    // The plan is explicitly fixture-domain positive expiry. It supplies no
    // native process proof; the actual actor test retains its SAME real guard.
    let expired = super::super::recovery::RecoveryPlan {
        release: vec![OwnerId::new(1).unwrap()],
        ..Default::default()
    };
    let result = super::super::native_recovery_tests::apply_fixture_recovery(
        &mut coordinator,
        &mut authority,
        &expired,
    )
    .unwrap();
    assert_eq!(
        result.outcome,
        Outcome::Released,
        "fresh capture at the next cleanup operation is required: {:?}",
        result.error
    );
    assert!(result.error.is_none());
    assert_eq!(module.value(), 7);
    assert_eq!(
        module.writes(),
        1,
        "restore the retained first baseline once"
    );
    let released = loaded(&memory);
    assert!(released.owners.is_empty() && released.resources.is_empty());
    assert!(released.revision > attached.revision);
    assert!(!matches!(authority.status(), WriterStatus::Blocked(_)));
    // A completed callback must release its capture again at the terminal
    // boundary, while the same durable authority and original store survive.
    assert_eq!(
        authority.after_intent(),
        Err(GateError::Rejected("configured capture is absent"))
    );
    assert_eq!(loaded(&memory), released);
    assert_eq!(module.writes(), 1);
}

#[test]
fn p1b_foreign_original_origin_blocks_before_hydration_scope_resolver_or_owners() {
    for field in [
        "lineage",
        "principal",
        "volume_guid",
        "volume_serial",
        "directory",
    ] {
        let document = record();
        let module = module(true);
        module.external_change(12);
        let memory = memory(Some(&document));
        let raw = memory.lock().unwrap().bytes.clone();
        let expected = ConfigurationOrigin::fixture().fixture_changed(field);
        let e = entry(&module, 1, ConfiguredDecision::Disabled);
        let source = Input(Arc::new(Mutex::new(
            ConfiguredSnapshot::complete_fixture(expected.clone(), 1, vec![e]).unwrap(),
        )));
        let scope_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = scope_calls.clone();
        let (coordinator, authority) = bootstrap_configured(
            catalog(&module),
            machine(),
            Box::new(Store(memory.clone())),
            Arc::new(source),
            ScopeEvidence::fixture_origin(expected, move || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(machine())
            }),
        );
        assert!(matches!(
            authority.status(),
            WriterStatus::Blocked(JournalError::InvalidEvidence(
                "configuration origin mismatch or unavailable"
            ))
        ));
        assert!(
            coordinator.snapshot().resources.is_empty() && coordinator.snapshot().owners.is_empty()
        );
        assert_eq!(scope_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(module.reads(), 0);
        assert_eq!(module.writes(), 0);
        assert_eq!(memory.lock().unwrap().bytes, raw);
        assert_eq!(memory.lock().unwrap().publishes, 0);
    }
}

#[test]
fn p1b_matching_original_origin_with_unavailable_producer_cannot_publish_or_mutate() {
    let document = record();
    let module = module(true);
    module.external_change(12);
    let memory = memory(Some(&document));
    let raw = memory.lock().unwrap().bytes.clone();
    let (_, authority) = open(
        &module,
        &memory,
        Input(Arc::new(Mutex::new(ConfiguredSnapshot::Unavailable))),
    );
    assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
    assert_eq!(module.reads(), 0);
    assert_eq!(module.writes(), 0);
    assert_eq!(memory.lock().unwrap().publishes, 0);
    assert_eq!(memory.lock().unwrap().bytes, raw);
}

#[cfg(windows)]
#[test]
fn p1b_fixture_origin_cannot_enter_native_scope_even_with_identical_owner_tuple() {
    let module = module(true);
    let e = entry(&module, 1, ConfiguredDecision::Enabled);
    let memory = memory(None);
    let (mut coordinator, mut authority) = bootstrap_configured(
        catalog(&module),
        machine(),
        Box::new(Store(memory.clone())),
        Arc::new(input(vec![e.clone()])),
        ScopeEvidence::Native(Box::new(NativeScope {
            machine: machine(),
            configuration: None,
            guard: None,
        })),
    );
    let result = coordinator.handle(acquire(&e), &mut Control(&mut authority));
    assert_eq!(result.outcome, Outcome::Rejected);
    assert_eq!(module.reads(), 0);
    assert_eq!(module.writes(), 0);
    assert_eq!(memory.lock().unwrap().publishes, 0);
}

#[test]
fn production_bootstrap_applies_authoritative_manual_attachment_without_recapturing_baseline() {
    let document = record();
    let module = module(true);
    module.external_change(12);
    let memory = memory(Some(&document));
    let (coordinator, authority) = open(
        &module,
        &memory,
        input(vec![entry(&module, 1, ConfiguredDecision::Enabled)]),
    );
    assert!(!matches!(authority.status(), WriterStatus::Blocked(_)));
    assert_eq!(module.value(), 12);
    assert_eq!(module.writes(), 0);
    assert!(
        module.reads() > 0,
        "actual bootstrap must apply the plan and verify current state"
    );
    assert_eq!(
        coordinator.snapshot().resources[0].baseline,
        document.resources[0].baseline
    );
    assert!(
        memory.lock().unwrap().publishes > 0,
        "attachment is a durable zero-write transition"
    );
}
#[test]
fn production_bootstrap_exact_disabled_intent_durably_restores_original_baseline() {
    let document = record();
    let module = module(true);
    module.external_change(12);
    let memory = memory(Some(&document));
    let (coordinator, authority) = open(
        &module,
        &memory,
        input(vec![entry(&module, 1, ConfiguredDecision::Disabled)]),
    );
    assert!(!matches!(authority.status(), WriterStatus::Blocked(_)));
    assert_eq!(module.value(), 7);
    assert_eq!(module.writes(), 1);
    assert!(coordinator.snapshot().owners.is_empty());
    let durable = loaded(&memory);
    assert!(durable.owners.is_empty());
    assert!(durable.resources.is_empty());
}
#[test]
fn production_bootstrap_unknown_or_changed_config_preserves_raw_and_never_observes_target() {
    let document = record();
    for variant in 0..4 {
        let module = module(variant != 3);
        module.external_change(12);
        let mut e = entry(&module, 1, ConfiguredDecision::Disabled);
        if variant == 1 {
            e.config.payload = serde_json::json!({"value":13});
        }
        if variant == 2 {
            e.binding = ActivationBinding::Manual(ConfiguredIntent {
                profile: ProfileId::new(1).unwrap(),
                generation: ConfigGeneration::new(2).unwrap(),
            });
        }
        let source = if variant == 0 {
            Input(Arc::new(Mutex::new(ConfiguredSnapshot::Unavailable)))
        } else {
            input(vec![e])
        };
        let memory = memory(Some(&document));
        let raw = memory.lock().unwrap().bytes.clone();
        let (coordinator, authority) = open(&module, &memory, source);
        assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
        assert_eq!(module.reads(), 0);
        assert_eq!(module.writes(), 0);
        assert_eq!(module.value(), 12);
        assert_eq!(
            coordinator.snapshot().resources[0].baseline,
            document.resources[0].baseline
        );
        assert_eq!(memory.lock().unwrap().bytes, raw);
        assert_eq!(memory.lock().unwrap().publishes, 0);
    }
}
#[test]
fn production_command_unavailable_input_blocks_before_prepare_but_observe_remains_available() {
    let module = module(true);
    let e = entry(&module, 1, ConfiguredDecision::Enabled);
    let memory = memory(None);
    let (mut coordinator, mut authority) = open(
        &module,
        &memory,
        Input(Arc::new(Mutex::new(ConfiguredSnapshot::Unavailable))),
    );
    let result = coordinator.handle(acquire(&e), &mut Control(&mut authority));
    assert_eq!(result.outcome, Outcome::Rejected);
    assert_eq!(module.reads(), 0);
    assert_eq!(module.writes(), 0);
    assert_eq!(memory.lock().unwrap().publishes, 0);
    assert!(matches!(
        coordinator
            .handle(
                Command::Observe {
                    feature: e.config.feature,
                    target: e.target
                },
                &mut Control(&mut authority)
            )
            .outcome,
        Outcome::Observed(_)
    ));
}
#[test]
fn production_command_persistent_manual_supports_acquire_share_and_explicit_disabled_release() {
    let module = module(true);
    let first = entry(&module, 1, ConfiguredDecision::Enabled);
    let second = entry(&module, 2, ConfiguredDecision::Enabled);
    let source = input(vec![first.clone(), second.clone()]);
    let memory = memory(None);
    let (mut coordinator, mut authority) = open(&module, &memory, source.clone());
    assert_eq!(
        coordinator
            .handle(acquire(&first), &mut Control(&mut authority))
            .outcome,
        Outcome::Applied(Effect::Immediate)
    );
    assert_eq!(
        coordinator
            .handle(acquire(&second), &mut Control(&mut authority))
            .outcome,
        Outcome::Shared
    );
    assert_eq!(module.writes(), 1);
    assert_eq!(module.value(), 12);
    let mut disabled_first = first.clone();
    disabled_first.decision = ConfiguredDecision::Disabled;
    let mut disabled_second = second.clone();
    disabled_second.decision = ConfiguredDecision::Disabled;
    *source.0.lock().unwrap() =
        ConfiguredSnapshot::complete(2, vec![disabled_first, disabled_second]).unwrap();
    assert_eq!(
        coordinator
            .handle(
                Command::Release { owner: first.owner },
                &mut Control(&mut authority)
            )
            .outcome,
        Outcome::Released
    );
    assert_eq!(module.writes(), 1);
    assert_eq!(
        coordinator
            .handle(
                Command::Release {
                    owner: second.owner
                },
                &mut Control(&mut authority)
            )
            .outcome,
        Outcome::Released
    );
    assert_eq!(module.value(), 7);
    assert_eq!(module.writes(), 2);
}
#[test]
fn production_command_machine_projection_and_retire_never_bypass_lifetime_declaration() {
    let module = module(false);
    let e = entry(&module, 1, ConfiguredDecision::Enabled);
    let memory = memory(None);
    let (mut coordinator, mut authority) = open(&module, &memory, input(vec![e.clone()]));
    assert_eq!(
        coordinator
            .handle(acquire(&e), &mut Control(&mut authority))
            .outcome,
        Outcome::Rejected
    );
    assert_eq!(module.reads(), 0);
    assert_eq!(module.writes(), 0);
    assert_eq!(memory.lock().unwrap().publishes, 0);
    let result = coordinator.handle(
        Command::Retire { target: e.target },
        &mut Control(&mut authority),
    );
    assert!(result.error.is_some());
    assert_eq!(memory.lock().unwrap().publishes, 0);
}
#[test]
fn production_command_rechecks_complete_configured_input_after_durable_intent_before_step() {
    let module = module(true);
    let e = entry(&module, 1, ConfiguredDecision::Enabled);
    let source = input(vec![e.clone()]);
    let memory = memory(None);
    let changed = source.clone();
    let mut disabled = e.clone();
    disabled.decision = ConfiguredDecision::Disabled;
    memory.lock().unwrap().after_publish = Some(Arc::new(move || {
        *changed.0.lock().unwrap() =
            ConfiguredSnapshot::complete(2, vec![disabled.clone()]).unwrap();
    }));
    let (mut coordinator, mut authority) = open(&module, &memory, source);
    let result = coordinator.handle(acquire(&e), &mut Control(&mut authority));
    assert!(result.error.is_some());
    assert_eq!(module.value(), 7);
    assert_eq!(module.writes(), 0);
    assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
    assert_eq!(
        result.snapshot.resources.len(),
        1,
        "published intent must remain in the worker ledger when pre-step authority changes"
    );
    assert_eq!(
        result.snapshot.resources[0].baseline.bytes(),
        7i32.to_le_bytes()
    );
    let retained = loaded(&memory);
    assert!(matches!(retained.phase, JournalPhase::BeforeWrite { .. }));
    assert_eq!(retained.resources.len(), 1);
}
#[test]
fn production_command_rechecks_actual_scope_after_intent_and_retains_evidence_on_unknown() {
    let module = module(true);
    let e = entry(&module, 1, ConfiguredDecision::Enabled);
    let source = input(vec![e.clone()]);
    let memory = memory(None);
    let observed = Arc::new(Mutex::new(machine()));
    let change = observed.clone();
    memory.lock().unwrap().after_publish = Some(Arc::new(move || {
        *change.lock().unwrap() = MachineIdentity {
            id: MachineId::new([2; 16]).unwrap(),
        };
    }));
    let (mut coordinator, mut authority) = bootstrap_configured(
        catalog(&module),
        machine(),
        Box::new(Store(memory.clone())),
        Arc::new(source),
        ScopeEvidence::fixture(move || Ok(observed.lock().unwrap().clone())),
    );
    let result = coordinator.handle(acquire(&e), &mut Control(&mut authority));
    assert!(result.error.is_some());
    assert_eq!(module.value(), 7);
    assert_eq!(module.writes(), 0);
    assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
    let retained = loaded(&memory);
    assert!(matches!(retained.phase, JournalPhase::BeforeWrite { .. }));
}
#[test]
fn production_command_missing_equal_owner_cannot_acquire_via_zero_write_sharing() {
    let module = module(true);
    let first = entry(&module, 1, ConfiguredDecision::Enabled);
    let second = entry(&module, 2, ConfiguredDecision::Enabled);
    let memory = memory(None);
    let (mut coordinator, mut authority) = open(&module, &memory, input(vec![first.clone()]));
    assert_eq!(
        coordinator
            .handle(acquire(&first), &mut Control(&mut authority))
            .outcome,
        Outcome::Applied(Effect::Immediate)
    );
    let reads = module.reads();
    let publications = memory.lock().unwrap().publishes;
    assert_eq!(
        coordinator
            .handle(acquire(&second), &mut Control(&mut authority))
            .outcome,
        Outcome::Rejected
    );
    assert_eq!(coordinator.snapshot().owners.len(), 1);
    assert_eq!(module.reads(), reads);
    assert_eq!(module.writes(), 1);
    assert_eq!(memory.lock().unwrap().publishes, publications);
}
#[test]
fn production_command_unknown_after_native_write_retains_intent_without_blind_compensation() {
    let module = module(true);
    let e = entry(&module, 1, ConfiguredDecision::Enabled);
    let source = input(vec![e.clone()]);
    let memory = memory(None);
    let changed = source.clone();
    module.set_execute_hook(Arc::new(move || {
        *changed.0.lock().unwrap() = ConfiguredSnapshot::Unavailable;
    }));
    let (mut coordinator, mut authority) = open(&module, &memory, source);
    let result = coordinator.handle(acquire(&e), &mut Control(&mut authority));
    assert_eq!(result.outcome, Outcome::RecoveryRequired);
    assert_eq!(module.value(), 12);
    assert_eq!(module.writes(), 1);
    assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
    assert!(matches!(
        loaded(&memory).phase,
        JournalPhase::BeforeWrite { .. }
    ));
}
#[test]
fn production_bootstrap_later_writer_cannot_confer_persistence_on_original_restorer() {
    let original = module(false);
    let mut descriptor = scalar::descriptor();
    descriptor.id = FeatureId::new("other_scalar").unwrap();
    descriptor.resources[0].lifetime = ResourceLifetimeRequirement::PersistentMachine;
    let current = ScalarModule::with_descriptor(descriptor.clone());
    current.external_change(12);
    let mut both = Catalog::default();
    both.register(original.clone()).unwrap();
    both.register(current.clone()).unwrap();
    let mut coordinator = Coordinator::new(both);
    coordinator.handle(
        acquire(&entry(&original, 1, ConfiguredDecision::Enabled)),
        &mut FixtureControl,
    );
    assert_eq!(
        coordinator
            .handle(
                acquire(&entry(&current, 2, ConfiguredDecision::Enabled)),
                &mut FixtureControl
            )
            .outcome,
        Outcome::Shared
    );
    coordinator.handle(
        Command::Release {
            owner: OwnerId::new(1).unwrap(),
        },
        &mut FixtureControl,
    );
    let document = JournalDocument::capture(
        machine(),
        4,
        JournalStamp::Recovery,
        JournalPhase::Recovery,
        &coordinator.snapshot(),
    )
    .unwrap();
    let old = module(false);
    old.external_change(12);
    let new = ScalarModule::with_descriptor(descriptor);
    new.external_change(12);
    let mut both = Catalog::default();
    both.register(old.clone()).unwrap();
    both.register(new.clone()).unwrap();
    let memory = memory(Some(&document));
    let raw = memory.lock().unwrap().bytes.clone();
    let (_, authority) = bootstrap_configured(
        both,
        machine(),
        Box::new(Store(memory.clone())),
        Arc::new(input(vec![entry(&new, 2, ConfiguredDecision::Disabled)])),
        ScopeEvidence::fixture(|| Ok(machine())),
    );
    assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
    assert_eq!(old.reads() + new.reads(), 0);
    assert_eq!(old.writes() + new.writes(), 0);
    assert_eq!(memory.lock().unwrap().bytes, raw);
    assert_eq!(memory.lock().unwrap().publishes, 0);
}
#[test]
fn production_bootstrap_evidenced_non_desired_state_is_not_successful_recovery() {
    let document = record();
    let module = module(true);
    let memory = memory(Some(&document));
    let (mut coordinator, mut authority) = open(
        &module,
        &memory,
        input(vec![entry(&module, 1, ConfiguredDecision::Enabled)]),
    );
    assert_eq!(module.value(), 7);
    assert_eq!(module.writes(), 0);
    assert!(
        matches!(authority.status(), WriterStatus::Blocked(_)),
        "RecoveryRequired without an error is not readiness"
    );
    let retained = loaded(&memory);
    assert_eq!(retained.sources, document.sources);
    assert_eq!(
        retained.resources[0].baseline,
        document.resources[0].baseline
    );
    assert_eq!(
        retained.resources[0].confirmed,
        document.resources[0].confirmed
    );
    assert_eq!(
        retained.resources[0].attempted,
        document.resources[0].attempted
    );
    assert_eq!(
        coordinator
            .handle(
                acquire(&entry(&module, 1, ConfiguredDecision::Enabled)),
                &mut Control(&mut authority)
            )
            .outcome,
        Outcome::Rejected
    );
    assert_eq!(module.writes(), 0);
}
#[test]
fn production_command_and_bootstrap_reject_game_binding_even_with_machine_persistence() {
    use crate::app::shared::ids::{GroupId, RuleId};
    let module = module(true);
    let mut e = entry(&module, 1, ConfiguredDecision::Enabled);
    let session = SessionIdentity {
        user: UserIdentity {
            machine: machine(),
            sid: UserSid::new("S-1-5-21-1").unwrap(),
        },
        session_id: 1,
        logon: LogonId::new(1).unwrap(),
        boot: BootId::new([1; 16]).unwrap(),
    };
    let process = ProcessIdentity::new(
        session.clone(),
        1,
        1,
        ImagePath::new("C:\\Fixture\\actor.exe").unwrap(),
    )
    .unwrap();
    e.binding = ActivationBinding::Game(
        GameActivation::new(
            e.binding.configured().unwrap(),
            RuleActivation::new(GroupId("g".into()), RuleId("r".into())).unwrap(),
            ActivationEligibility::Running,
            session,
            vec![process],
        )
        .unwrap(),
    );
    e.mode = ActivationMode::Session;
    let memory = memory(None);
    let (mut coordinator, mut authority) = open(&module, &memory, input(vec![e.clone()]));
    assert_eq!(
        coordinator
            .handle(acquire(&e), &mut Control(&mut authority))
            .outcome,
        Outcome::Rejected
    );
    assert_eq!(module.reads(), 0);
    assert_eq!(module.writes(), 0);
    assert_eq!(memory.lock().unwrap().publishes, 0);
    let mut document = record();
    document.owners[0].binding = e.binding.clone();
    document.owners[0].mode = e.mode;
    let old = self::memory(Some(&document));
    let raw = old.lock().unwrap().bytes.clone();
    let (_, authority) = open(&module, &old, input(vec![e]));
    assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
    assert_eq!(module.reads(), 0);
    assert_eq!(old.lock().unwrap().bytes, raw);
}
