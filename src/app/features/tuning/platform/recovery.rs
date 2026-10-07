//! Native lifetime evidence is supplied by an authoritative producer, never
//! inferred from identity shape, a changed continuity nonce, or access denial.
use super::{
    contract::*,
    journal::{JournalDocument, JournalError, JournalOwner, JournalSource},
};
use std::collections::BTreeSet;

// Lifetime belongs to one physical incarnation. A different image/provenance
// description cannot turn the same exact process into an independently dead one.
fn incarnation(process: &ProcessIdentity) -> (SessionIdentity, u32, u64) {
    (
        process.session.clone(),
        process.pid(),
        process.creation_token(),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerAttachment {
    /// A complete producer proves the original actor and current recorded
    /// eligibility. Foreground bindings require actual OS foreground proof;
    /// this subset contains eligible original incarnations, never new PID shapes.
    VerifiedLive {
        owner: OwnerId,
        binding: GameActivation,
        live_processes: Vec<ProcessIdentity>,
    },
    PositivelyExpired {
        owner: OwnerId,
        binding: GameActivation,
    },
    VerifiedIneligible {
        owner: OwnerId,
        binding: GameActivation,
        reason: OwnerIneligibility,
    },
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerIneligibility {
    ForegroundLost,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceLifetime {
    VerifiedLive,
    PositivelyExpired,
    Unknown,
}
pub(super) trait RecoveryResolver {
    /// VerifiedLive must prove the original game binding. A live logon/session
    /// or an opaque owner number alone cannot establish game attachment. Positive
    /// expiry proves the recorded eligible actor ended; rule deletion, access
    /// failure and unknown foreground state cannot supply that evidence.
    fn owner(&mut self, owner: &JournalOwner, source: &JournalSource) -> OwnerAttachment;
    fn resource(&mut self, key: &ResourceKey) -> ResourceLifetime;
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryPlan {
    pub reattach: Vec<OwnerId>,
    pub release: Vec<OwnerId>,
    pub retire: Vec<TargetIdentity>,
    pub unknown_owners: Vec<OwnerId>,
    pub unknown_resources: Vec<ResourceKey>,
    pub error: Option<JournalError>,
}
impl RecoveryPlan {
    pub fn blocked(&self) -> bool {
        self.error.is_some()
            || !self.unknown_owners.is_empty()
            || !self.unknown_resources.is_empty()
    }
    #[cfg(test)]
    pub(super) fn resolve(document: &JournalDocument, resolver: &mut dyn RecoveryResolver) -> Self {
        // Explicit legacy fixture policy. Production uses independent configured
        // input through resolve_configured; recorded Manual shape is no authority.
        Self::resolve_configured(document, resolver, &mut |_, _| {
            super::configured_intent::IntentDecision::Enabled
        })
    }
    pub(super) fn resolve_configured(
        document: &JournalDocument,
        resolver: &mut dyn RecoveryResolver,
        manual: &mut dyn FnMut(
            &JournalOwner,
            &JournalSource,
        ) -> super::configured_intent::IntentDecision,
    ) -> Self {
        let mut plan = Self::default();
        if let Err(error) = document.validate() {
            plan.error = Some(error);
            return plan;
        }
        let mut retired = BTreeSet::new();
        let mut live = BTreeSet::new();
        for resource in &document.resources {
            match resolver.resource(&resource.key) {
                ResourceLifetime::VerifiedLive => {
                    live.insert(resource.key.target.clone());
                }
                ResourceLifetime::PositivelyExpired
                    if matches!(
                        resource.key.target,
                        TargetIdentity::Process(_) | TargetIdentity::Session(_)
                    ) =>
                {
                    retired.insert(resource.key.target.clone());
                }
                _ => plan.unknown_resources.push(resource.key.clone()),
            }
        }
        // Actual-resource lifetime evidence must agree on physical incarnation,
        // independently of image aliases, manual ownership or game projections.
        // Session retirement likewise cannot contradict a verified live member.
        for target in live.clone() {
            if retired.iter().any(|ended| match (&target, ended) {
                (TargetIdentity::Process(process), TargetIdentity::Process(ended)) => {
                    incarnation(process) == incarnation(ended)
                }
                (TargetIdentity::Process(process), TargetIdentity::Session(session)) => {
                    process.session == *session
                }
                _ => target == *ended,
            }) {
                plan.unknown_resources.extend(
                    document
                        .resources
                        .iter()
                        .filter(|resource| resource.key.target == target)
                        .map(|resource| resource.key.clone()),
                );
            }
        }
        let mut live_incarnations = live
            .iter()
            .filter_map(|target| match target {
                TargetIdentity::Process(process) => Some(incarnation(process)),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let mut expired_actors = Vec::new();
        for owner in &document.owners {
            let source = &document.sources[owner.source.0 as usize];
            let game = match &owner.binding {
                ActivationBinding::Manual(_) => {
                    match manual(owner, source) {
                        super::configured_intent::IntentDecision::Enabled => {
                            if document
                                .resources
                                .iter()
                                .any(|resource| resource.owners.contains(&owner.owner))
                            {
                                plan.reattach.push(owner.owner);
                            } else {
                                plan.release.push(owner.owner);
                            }
                        }
                        super::configured_intent::IntentDecision::Disabled => {
                            plan.release.push(owner.owner)
                        }
                        super::configured_intent::IntentDecision::Unknown => {
                            plan.unknown_owners.push(owner.owner)
                        }
                    }
                    continue;
                }
                ActivationBinding::Game(game) => game,
                ActivationBinding::Unknown => {
                    plan.unknown_owners.push(owner.owner);
                    continue;
                }
            };
            let exact = |candidate_owner: OwnerId, candidate: &GameActivation| {
                candidate_owner == owner.owner && candidate == game
            };
            match resolver.owner(owner, source) {
                OwnerAttachment::VerifiedLive {
                    owner: candidate_owner,
                    binding,
                    live_processes,
                } if exact(candidate_owner, &binding) => {
                    let mut pids = BTreeSet::new();
                    let valid = !live_processes.is_empty()
                        && live_processes.len() <= 64
                        && live_processes.iter().all(|process| {
                            game.processes().contains(process) && pids.insert(process.pid())
                        });
                    let conflicts = valid
                        && retired.iter().any(|ended| match ended {
                            TargetIdentity::Session(session) => game.session() == session,
                            TargetIdentity::Process(process) => live_processes
                                .iter()
                                .any(|live| incarnation(live) == incarnation(process)),
                            _ => false,
                        });
                    if valid && !conflicts {
                        live_incarnations.extend(live_processes.iter().map(incarnation));
                        plan.reattach.push(owner.owner);
                    } else {
                        plan.unknown_owners.push(owner.owner);
                    }
                }
                OwnerAttachment::PositivelyExpired {
                    owner: candidate_owner,
                    binding,
                } if exact(candidate_owner, &binding) => {
                    expired_actors.push((owner.owner, game.clone()));
                    plan.release.push(owner.owner);
                }
                OwnerAttachment::VerifiedIneligible {
                    owner: candidate_owner,
                    binding,
                    reason: OwnerIneligibility::ForegroundLost,
                } if exact(candidate_owner, &binding)
                    && game.eligibility() == ActivationEligibility::Foreground =>
                {
                    // Current foreground intent ended; the background process
                    // and projected Machine/User setting remain independently live.
                    plan.release.push(owner.owner);
                }
                _ => plan.unknown_owners.push(owner.owner),
            }
        }
        // Inspect validated attachment evidence only after all owners. Order
        // cannot hide a live process behind another owner's projected resource.
        for (owner, game) in expired_actors {
            if game
                .processes()
                .iter()
                .any(|process| live_incarnations.contains(&incarnation(process)))
            {
                plan.unknown_owners.push(owner);
            }
        }
        plan.retire = retired.into_iter().collect();
        if plan.blocked() {
            plan.reattach.clear();
            plan.release.clear();
            plan.retire.clear();
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        catalog::Catalog,
        coordinator::{
            Command, Coordinator, CoordinatorSnapshot, ExecutionControl, GateError, Outcome,
            WriteIntent,
        },
        journal::{JournalPhase, JournalStamp},
    };
    use super::*;
    use crate::app::features::tuning::testing::{
        scalar::{ScalarConfig, ScalarModule},
        user_setting::{UserConfig, UserModule},
        value_setting::{ValueConfig, ValueModule},
    };
    struct Control;
    impl ExecutionControl for Control {
        fn cancellation_requested(&self) -> bool {
            false
        }
        fn before_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
            Ok(())
        }
        fn after_verified_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
            Ok(())
        }
        fn publish(&mut self, _: CoordinatorSnapshot) {}
        fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
            Ok(())
        }
    }
    fn machine() -> MachineIdentity {
        MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        }
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
    fn id(n: u64) -> OwnerId {
        OwnerId::new(n).unwrap()
    }
    fn manual_binding() -> ActivationBinding {
        ActivationBinding::Manual(ConfiguredIntent {
            profile: ProfileId::new(1).unwrap(),
            generation: ConfigGeneration::new(1).unwrap(),
        })
    }
    fn game_binding(
        rule: u64,
        generation: u64,
        eligibility: ActivationEligibility,
        pids: &[u32],
    ) -> ActivationBinding {
        use crate::app::shared::ids::{GroupId, RuleId};
        ActivationBinding::Game(
            GameActivation::new(
                ConfiguredIntent {
                    profile: ProfileId::new(rule).unwrap(),
                    generation: ConfigGeneration::new(generation).unwrap(),
                },
                RuleActivation::new(GroupId("group-0".into()), RuleId(format!("rule-{rule}")))
                    .unwrap(),
                eligibility,
                session(),
                pids.iter()
                    .map(|pid| {
                        ProcessIdentity::new(
                            session(),
                            *pid,
                            123,
                            ImagePath::new("C:\\Games\\game.exe").unwrap(),
                        )
                        .unwrap()
                    })
                    .collect(),
            )
            .unwrap(),
        )
    }
    fn live_attachment(owner: &JournalOwner) -> OwnerAttachment {
        match &owner.binding {
            ActivationBinding::Game(binding) => OwnerAttachment::VerifiedLive {
                owner: owner.owner,
                binding: binding.clone(),
                live_processes: binding.processes().to_vec(),
            },
            _ => OwnerAttachment::Unknown,
        }
    }
    fn expired_attachment(owner: &JournalOwner) -> OwnerAttachment {
        match &owner.binding {
            ActivationBinding::Game(binding) => OwnerAttachment::PositivelyExpired {
                owner: owner.owner,
                binding: binding.clone(),
            },
            _ => OwnerAttachment::Unknown,
        }
    }
    fn journal(coordinator: &Coordinator) -> JournalDocument {
        JournalDocument::capture(
            machine(),
            1,
            JournalStamp::Recovery,
            JournalPhase::Recovery,
            &coordinator.snapshot(),
        )
        .unwrap()
    }
    #[test]
    fn hydration_never_observes_or_recaptures_first_baseline() {
        let scalar = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog);
        coordinator.handle(
            Command::Acquire {
                binding: manual_binding(),
                owner: id(1),
                config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                target: TargetIdentity::Machine(machine()),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        );
        let document = journal(&coordinator);
        let baseline = document.resources[0].baseline.clone();
        let reads = scalar.reads();
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let recovered = Coordinator::recover(catalog, &document);
        assert!(
            recovered.is_ok(),
            "journal sources must hydrate independently from native observations"
        );
        let mut recovered = recovered.unwrap();
        assert_eq!(scalar.reads(), reads);
        assert_eq!(recovered.snapshot().resources[0].baseline, baseline);
        let released = recovered.handle(Command::Release { owner: id(1) }, &mut Control);
        assert_eq!(released.outcome, Outcome::Released);
        assert_eq!(scalar.value(), 7);
    }
    struct Unknown;
    impl RecoveryResolver for Unknown {
        fn owner(&mut self, _: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            OwnerAttachment::Unknown
        }
        fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
            ResourceLifetime::Unknown
        }
    }
    #[test]
    fn unknown_native_evidence_blocks_without_becoming_expiration_or_reattachment() {
        let scalar = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog);
        coordinator.handle(
            Command::Acquire {
                binding: ActivationBinding::Unknown,
                owner: id(1),
                config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                target: TargetIdentity::Machine(machine()),
                mode: ActivationMode::Session,
            },
            &mut Control,
        );
        let plan = RecoveryPlan::resolve(&journal(&coordinator), &mut Unknown);
        assert!(
            plan.blocked(),
            "query denial and uncertain boot evidence must stay Unknown"
        );
        assert!(plan.reattach.is_empty());
        assert!(plan.release.is_empty());
        assert!(plan.retire.is_empty());
    }
    fn scalar_document() -> (JournalDocument, ScalarModule) {
        let scalar = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog);
        coordinator.handle(
            Command::Acquire {
                binding: manual_binding(),
                owner: id(1),
                config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                target: TargetIdentity::Machine(machine()),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        );
        (journal(&coordinator), scalar)
    }
    fn activation_document(
        scope: TargetScope,
        eligibility: ActivationEligibility,
        pids: &[u32],
    ) -> (JournalDocument, ScalarModule, Catalog) {
        let scalar = if scope == TargetScope::Machine {
            ScalarModule::new(7)
        } else {
            scoped(scope, "test_bound_projection")
        };
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog.clone());
        let binding = game_binding(1, 1, eligibility, pids);
        let target = if scope == TargetScope::Machine {
            TargetIdentity::Machine(machine())
        } else {
            let ActivationBinding::Game(game) = &binding else {
                unreachable!()
            };
            TargetIdentity::Process(game.processes()[0].clone())
        };
        let result = coordinator.handle(
            Command::Acquire {
                owner: id(1),
                binding,
                config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                target,
                mode: ActivationMode::Session,
            },
            &mut Control,
        );
        assert!(result.error.is_none());
        (journal(&coordinator), scalar, catalog)
    }
    struct Evidence(OwnerAttachment);
    impl RecoveryResolver for Evidence {
        fn owner(&mut self, _: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            self.0.clone()
        }
        fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
            ResourceLifetime::VerifiedLive
        }
    }
    #[test]
    fn activation_binding_missing_manual_or_game_provenance_never_invents_attachment() {
        use super::super::journal::JournalLoad;
        let (document, _) = scalar_document();
        let mut wire = serde_json::to_value(document).unwrap();
        wire["owners"][0].as_object_mut().unwrap().remove("binding");
        let JournalLoad::Known(document) =
            JournalDocument::load(serde_json::to_vec(&wire).unwrap(), &machine())
        else {
            panic!("additive legacy binding remains known evidence");
        };
        assert_eq!(document.owners[0].binding, ActivationBinding::Unknown);
        assert!(
            RecoveryPlan::resolve(&document, &mut Live).blocked(),
            "manual mode alone cannot invent configured ownership"
        );
        let (mut document, _, _) =
            activation_document(TargetScope::Machine, ActivationEligibility::Running, &[42]);
        let live = live_attachment(&document.owners[0]);
        document.owners[0].binding = ActivationBinding::Unknown;
        assert!(
            RecoveryPlan::resolve(&document, &mut Evidence(live)).blocked(),
            "shape-valid positive output cannot fill missing durable game binding"
        );
    }
    #[test]
    fn activation_binding_two_games_share_machine_and_user_resources_without_owner_alias() {
        for scope in [TargetScope::Machine, TargetScope::User] {
            let (mut document, scalar, catalog) =
                activation_document(scope, ActivationEligibility::Running, &[42]);
            let mut coordinator = Coordinator::recover(catalog.clone(), &document).unwrap();
            let binding = game_binding(2, 1, ActivationEligibility::Running, &[43]);
            let target = if scope == TargetScope::Machine {
                TargetIdentity::Machine(machine())
            } else {
                let ActivationBinding::Game(game) = &binding else {
                    unreachable!()
                };
                TargetIdentity::Process(game.processes()[0].clone())
            };
            assert_eq!(
                coordinator
                    .handle(
                        Command::Acquire {
                            owner: id(2),
                            binding,
                            config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                            target,
                            mode: ActivationMode::Session
                        },
                        &mut Control
                    )
                    .outcome,
                Outcome::Shared
            );
            document = journal(&coordinator);
            struct Selective;
            impl RecoveryResolver for Selective {
                fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
                    if owner.owner == id(1) {
                        live_attachment(owner)
                    } else {
                        expired_attachment(owner)
                    }
                }
                fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
                    ResourceLifetime::VerifiedLive
                }
            }
            let plan = RecoveryPlan::resolve(&document, &mut Selective);
            assert!(!plan.blocked());
            assert_eq!(plan.reattach, vec![id(1)]);
            assert_eq!(plan.release, vec![id(2)]);
            assert!(plan.retire.is_empty());
            let mut recovered = Coordinator::recover(catalog, &document).unwrap();
            let result = recovered.apply_recovery(&plan, &mut Control);
            assert_eq!(result.outcome, Outcome::Unchanged);
            assert_eq!(scalar.value(), 12);
            assert_eq!(result.snapshot.owners.len(), 1);
            assert_eq!(
                result.snapshot.owners[0].binding,
                document.owners[0].binding
            );
            assert_eq!(
                recovered
                    .handle(Command::Release { owner: id(1) }, &mut Control)
                    .outcome,
                Outcome::Released
            );
            assert_eq!(scalar.value(), 7);
        }
    }
    #[test]
    fn activation_binding_cross_owner_live_incarnation_blocks_projected_expiry_in_both_orders() {
        for (scope, alias) in [
            (TargetScope::Machine, false),
            (TargetScope::User, false),
            (TargetScope::Machine, true),
            (TargetScope::User, true),
        ] {
            let (mut document, scalar, catalog) =
                activation_document(scope, ActivationEligibility::Running, &[42]);
            let mut coordinator = Coordinator::recover(catalog.clone(), &document).unwrap();
            let mut binding = game_binding(2, 1, ActivationEligibility::Running, &[42]);
            if alias {
                let ActivationBinding::Game(game) = binding else {
                    unreachable!()
                };
                let process = ProcessIdentity::new(
                    session(),
                    42,
                    123,
                    ImagePath::new("C:\\Games\\alias.exe").unwrap(),
                )
                .unwrap();
                binding = ActivationBinding::Game(
                    GameActivation::new(
                        game.configured(),
                        game.rule().clone(),
                        game.eligibility(),
                        game.session().clone(),
                        vec![process],
                    )
                    .unwrap(),
                );
            }
            let target = if scope == TargetScope::Machine {
                TargetIdentity::Machine(machine())
            } else {
                let ActivationBinding::Game(game) = &binding else {
                    unreachable!()
                };
                TargetIdentity::Process(game.processes()[0].clone())
            };
            assert_eq!(
                coordinator
                    .handle(
                        Command::Acquire {
                            owner: id(2),
                            binding,
                            config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                            target,
                            mode: ActivationMode::Session
                        },
                        &mut Control
                    )
                    .outcome,
                Outcome::Shared
            );
            document = journal(&coordinator);
            struct ContradictoryAttachments {
                live_owner: OwnerId,
            }
            impl RecoveryResolver for ContradictoryAttachments {
                fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
                    if owner.owner == self.live_owner {
                        live_attachment(owner)
                    } else {
                        expired_attachment(owner)
                    }
                }
                fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
                    ResourceLifetime::VerifiedLive
                }
            }
            let reads = scalar.reads();
            let writes = scalar.writes();
            for live_owner in [id(1), id(2)] {
                let plan =
                    RecoveryPlan::resolve(&document, &mut ContradictoryAttachments { live_owner });
                assert!(
                    plan.blocked(),
                    "an actual live recorded process contradicts another actor's complete expiry even with projected resources"
                );
                assert!(
                    plan.reattach.is_empty() && plan.release.is_empty() && plan.retire.is_empty()
                );
                let mut recovered = Coordinator::recover(catalog.clone(), &document).unwrap();
                let before = recovered.snapshot();
                assert_eq!(
                    recovered.apply_recovery(&plan, &mut Control).outcome,
                    Outcome::RecoveryRequired
                );
                assert_eq!(recovered.snapshot(), before);
            }
            assert_eq!(scalar.reads(), reads);
            assert_eq!(scalar.writes(), writes);
        }
    }
    #[test]
    fn activation_binding_surviving_original_process_reattaches_without_new_pid_substitution() {
        let (document, _, _) = activation_document(
            TargetScope::Machine,
            ActivationEligibility::Running,
            &[42, 43],
        );
        let ActivationBinding::Game(binding) = &document.owners[0].binding else {
            unreachable!()
        };
        let evidence = OwnerAttachment::VerifiedLive {
            owner: id(1),
            binding: binding.clone(),
            live_processes: vec![binding.processes()[1].clone()],
        };
        let plan = RecoveryPlan::resolve(&document, &mut Evidence(evidence));
        assert!(!plan.blocked());
        assert_eq!(plan.reattach, vec![id(1)]);
        let alien = ProcessIdentity::new(
            session(),
            44,
            123,
            ImagePath::new("C:\\Games\\game.exe").unwrap(),
        )
        .unwrap();
        for live_processes in [
            vec![],
            vec![alien],
            vec![
                binding.processes()[0].clone(),
                binding.processes()[0].clone(),
            ],
        ] {
            assert!(RecoveryPlan::resolve(
                &document,
                &mut Evidence(OwnerAttachment::VerifiedLive {
                    owner: id(1),
                    binding: binding.clone(),
                    live_processes
                })
            )
            .blocked());
        }
    }
    #[test]
    fn activation_binding_physical_incarnation_aliases_cannot_hide_resource_lifetime_contradictions(
    ) {
        let (mut document, _, mut catalog) =
            activation_document(TargetScope::Machine, ActivationEligibility::Running, &[42]);
        let process_module = scoped(TargetScope::Process, "test_process_alias");
        catalog.register(process_module.clone()).unwrap();
        let mut coordinator = Coordinator::recover(catalog, &document).unwrap();
        let alias = ProcessIdentity::new(
            session(),
            42,
            123,
            ImagePath::new("C:\\Games\\alias.exe").unwrap(),
        )
        .unwrap();
        assert!(coordinator
            .handle(
                Command::Acquire {
                    owner: id(2),
                    binding: manual_binding(),
                    config: process_module
                        .config_envelope(&ScalarConfig { value: 12 })
                        .unwrap(),
                    target: TargetIdentity::Process(alias),
                    mode: ActivationMode::Manual
                },
                &mut Control
            )
            .error
            .is_none());
        document = journal(&coordinator);
        struct Contradiction {
            process_live: bool,
        }
        impl RecoveryResolver for Contradiction {
            fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
                if self.process_live {
                    expired_attachment(owner)
                } else {
                    live_attachment(owner)
                }
            }
            fn resource(&mut self, key: &ResourceKey) -> ResourceLifetime {
                if key.target.scope() == TargetScope::Process && !self.process_live {
                    ResourceLifetime::PositivelyExpired
                } else {
                    ResourceLifetime::VerifiedLive
                }
            }
        }
        for process_live in [false, true] {
            let plan = RecoveryPlan::resolve(&document, &mut Contradiction { process_live });
            assert!(
                plan.blocked(),
                "image aliases cannot turn one physical process incarnation into independent native lifetimes"
            );
            assert!(plan.reattach.is_empty() && plan.release.is_empty() && plan.retire.is_empty());
        }
    }
    #[test]
    fn activation_binding_positive_answers_cannot_transfer_owner_rule_profile_config_or_incarnation(
    ) {
        let (document, scalar, catalog) =
            activation_document(TargetScope::Machine, ActivationEligibility::Running, &[42]);
        let ActivationBinding::Game(exact) = &document.owners[0].binding else {
            unreachable!()
        };
        let mut wrong = vec![(id(2), exact.clone())];
        for binding in [
            game_binding(2, 1, ActivationEligibility::Running, &[42]),
            game_binding(1, 2, ActivationEligibility::Running, &[42]),
            game_binding(1, 1, ActivationEligibility::Foreground, &[42]),
            game_binding(1, 1, ActivationEligibility::Running, &[43]),
        ] {
            let ActivationBinding::Game(binding) = binding else {
                unreachable!()
            };
            wrong.push((id(1), binding));
        }
        let mut recovered = Coordinator::recover(catalog, &document).unwrap();
        let before = recovered.snapshot();
        let reads = scalar.reads();
        let writes = scalar.writes();
        for (owner, binding) in wrong {
            for answer in [
                OwnerAttachment::VerifiedLive {
                    owner,
                    live_processes: binding.processes().to_vec(),
                    binding: binding.clone(),
                },
                OwnerAttachment::PositivelyExpired {
                    owner,
                    binding: binding.clone(),
                },
            ] {
                let plan = RecoveryPlan::resolve(&document, &mut Evidence(answer));
                assert!(plan.blocked());
                assert_eq!(
                    recovered.apply_recovery(&plan, &mut Control).outcome,
                    Outcome::RecoveryRequired
                );
                assert_eq!(recovered.snapshot(), before);
            }
        }
        assert_eq!(scalar.reads(), reads);
        assert_eq!(scalar.writes(), writes);
    }
    #[test]
    fn activation_binding_foreground_loss_releases_intent_without_expiring_live_resources() {
        let (document, scalar, catalog) = activation_document(
            TargetScope::Machine,
            ActivationEligibility::Foreground,
            &[42],
        );
        let ActivationBinding::Game(binding) = &document.owners[0].binding else {
            unreachable!()
        };
        let plan = RecoveryPlan::resolve(
            &document,
            &mut Evidence(OwnerAttachment::VerifiedIneligible {
                owner: id(1),
                binding: binding.clone(),
                reason: OwnerIneligibility::ForegroundLost,
            }),
        );
        assert!(!plan.blocked());
        assert_eq!(plan.release, vec![id(1)]);
        assert!(plan.retire.is_empty());
        let mut recovered = Coordinator::recover(catalog, &document).unwrap();
        assert_eq!(
            recovered.apply_recovery(&plan, &mut Control).outcome,
            Outcome::Released
        );
        assert_eq!(scalar.value(), 7);
        let (document, _, _) =
            activation_document(TargetScope::Machine, ActivationEligibility::Running, &[42]);
        let ActivationBinding::Game(binding) = &document.owners[0].binding else {
            unreachable!()
        };
        assert!(RecoveryPlan::resolve(
            &document,
            &mut Evidence(OwnerAttachment::VerifiedIneligible {
                owner: id(1),
                binding: binding.clone(),
                reason: OwnerIneligibility::ForegroundLost
            })
        )
        .blocked());
        assert!(
            RecoveryPlan::resolve(&document, &mut Evidence(OwnerAttachment::Unknown)).blocked()
        );
    }
    struct Live;
    impl RecoveryResolver for Live {
        fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            live_attachment(owner)
        }
        fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
            ResourceLifetime::VerifiedLive
        }
    }
    #[test]
    fn decoded_source_config_cannot_disagree_with_its_typed_snapshot_request() {
        let (mut document, scalar) = scalar_document();
        let reads = scalar.reads();
        document.sources[0].config.as_mut().unwrap().payload = serde_json::json!({"value":25});
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        assert!(
            Coordinator::recover(catalog, &document).is_err(),
            "source config and snapshot must describe the same typed request"
        );
        assert_eq!(scalar.reads(), reads);
    }
    #[test]
    fn restart_recovery_restores_first_adapter_a_after_later_adapter_b_edit() {
        let user = UserModule::new(None, true);
        let value = ValueModule::new(user.clone());
        let make_catalog = || {
            let mut catalog = Catalog::default();
            catalog.register(user.clone()).unwrap();
            catalog.register(value.clone()).unwrap();
            catalog
        };
        let mut coordinator = Coordinator::new(make_catalog());
        let target = TargetIdentity::Session(session());
        coordinator.handle(
            Command::Acquire {
                binding: manual_binding(),
                owner: id(1),
                config: user
                    .config_envelope(&UserConfig {
                        value: 9,
                        enabled: false,
                    })
                    .unwrap(),
                target: target.clone(),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        );
        coordinator.handle(
            Command::Acquire {
                binding: manual_binding(),
                owner: id(2),
                config: value.config_envelope(&ValueConfig { value: 9 }).unwrap(),
                target: target.clone(),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        );
        coordinator.handle(Command::Release { owner: id(1) }, &mut Control);
        coordinator.handle(
            Command::Acquire {
                binding: ActivationBinding::Manual(ConfiguredIntent {
                    profile: ProfileId::new(1).unwrap(),
                    generation: ConfigGeneration::new(2).unwrap(),
                }),
                owner: id(2),
                config: value.config_envelope(&ValueConfig { value: 13 }).unwrap(),
                target,
                mode: ActivationMode::Manual,
            },
            &mut Control,
        );
        let document = journal(&coordinator);
        assert_ne!(
            document.resources[0].original,
            document.resources[0].current
        );
        let bytes = document.to_bytes().unwrap();
        let super::super::journal::JournalLoad::Known(document) =
            JournalDocument::load(bytes, &machine())
        else {
            panic!("known codec");
        };
        let mut recovered = Coordinator::recover(make_catalog(), &document).unwrap();
        let plan = RecoveryPlan::resolve(&document, &mut Live);
        let result = recovered.apply_recovery(&plan, &mut Control);
        assert_eq!(result.outcome, Outcome::Unchanged);
        assert_eq!(user.value(), Some(13));
        let result = recovered.handle(Command::Release { owner: id(2) }, &mut Control);
        assert_eq!(result.outcome, Outcome::Released);
        assert_eq!(user.value(), None);
        assert!(user.enabled());
        assert_eq!(result.restoration_effect, Some(Effect::RestartRequired));
    }
    #[test]
    fn interrupted_last_release_is_cleanup_intent_even_for_a_manual_owner() {
        let (mut document, _) = scalar_document();
        document.resources[0].owners.clear();
        document.resources[0].status = ModuleStatus::Restoring;
        document.owners[0].status = ModuleStatus::Restoring;
        let plan = RecoveryPlan::resolve(&document, &mut Live);
        assert_eq!(
            plan.release,
            vec![id(1)],
            "persisted release intent cannot reattach a disabled manual owner"
        );
        assert!(plan.reattach.is_empty());
    }
    fn process() -> TargetIdentity {
        TargetIdentity::Process(
            ProcessIdentity::new(
                session(),
                42,
                123,
                ImagePath::new("C:\\Games\\game.exe").unwrap(),
            )
            .unwrap(),
        )
    }
    fn scoped(scope: TargetScope, name: &str) -> ScalarModule {
        let mut descriptor = crate::app::features::tuning::testing::scalar::descriptor();
        descriptor.id = FeatureId::new(name).unwrap();
        descriptor.target_scopes = vec![TargetScope::Process, TargetScope::Session];
        descriptor.resources[0].scope = scope;
        ScalarModule::with_descriptor(descriptor)
    }
    struct Contradictory;
    impl RecoveryResolver for Contradictory {
        fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            live_attachment(owner)
        }
        fn resource(&mut self, key: &ResourceKey) -> ResourceLifetime {
            if key.target.scope() == TargetScope::Session {
                ResourceLifetime::PositivelyExpired
            } else {
                ResourceLifetime::VerifiedLive
            }
        }
    }
    #[test]
    fn actual_process_resources_with_image_aliases_cannot_have_contradictory_lifetimes() {
        let scalar = scoped(TargetScope::Process, "test_actual_process_alias");
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog.clone());
        let original = process();
        let alias = TargetIdentity::Process(
            ProcessIdentity::new(
                session(),
                42,
                123,
                ImagePath::new("C:\\Games\\alias.exe").unwrap(),
            )
            .unwrap(),
        );
        for (owner, target) in [(id(1), original.clone()), (id(2), alias.clone())] {
            assert!(coordinator
                .handle(
                    Command::Acquire {
                        owner,
                        binding: manual_binding(),
                        config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                        target,
                        mode: ActivationMode::Manual,
                    },
                    &mut Control,
                )
                .error
                .is_none());
        }
        let document = journal(&coordinator);
        assert_eq!(document.resources.len(), 2);
        struct ResourceEvidence(TargetIdentity);
        impl RecoveryResolver for ResourceEvidence {
            fn owner(&mut self, _: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
                panic!("configured manual ownership does not query game attachment");
            }
            fn resource(&mut self, key: &ResourceKey) -> ResourceLifetime {
                if key.target == self.0 {
                    ResourceLifetime::VerifiedLive
                } else {
                    ResourceLifetime::PositivelyExpired
                }
            }
        }
        let reads = scalar.reads();
        let writes = scalar.writes();
        for live in [original, alias] {
            let plan = RecoveryPlan::resolve(&document, &mut ResourceEvidence(live));
            assert!(
                plan.blocked(),
                "image aliases describe one physical process lifetime"
            );
            assert!(plan.reattach.is_empty() && plan.release.is_empty() && plan.retire.is_empty());
            let mut recovered = Coordinator::recover(catalog.clone(), &document).unwrap();
            let before = recovered.snapshot();
            assert_eq!(
                recovered.apply_recovery(&plan, &mut Control).outcome,
                Outcome::RecoveryRequired
            );
            assert_eq!(recovered.snapshot(), before);
        }
        assert_eq!(scalar.reads(), reads);
        assert_eq!(scalar.writes(), writes);
    }
    #[test]
    fn expired_session_cannot_retire_an_independently_live_process_resource() {
        let session_module = scoped(TargetScope::Session, "test_session");
        let process_module = scoped(TargetScope::Process, "test_process");
        let mut catalog = Catalog::default();
        catalog.register(session_module.clone()).unwrap();
        catalog.register(process_module.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog);
        for (owner, module) in [(id(1), session_module), (id(2), process_module)] {
            coordinator.handle(
                Command::Acquire {
                    binding: manual_binding(),
                    owner,
                    config: module.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                    target: process(),
                    mode: ActivationMode::Manual,
                },
                &mut Control,
            );
        }
        let plan = RecoveryPlan::resolve(&journal(&coordinator), &mut Contradictory);
        assert!(
            plan.blocked(),
            "per-resource expiration cannot become a blanket session-end attestation"
        );
        assert!(plan.retire.is_empty());
    }
    struct JournalControl {
        revision: u64,
        documents: Vec<JournalDocument>,
    }
    #[test]
    fn activation_binding_zero_write_generation_update_is_durable_without_new_original() {
        let scalar = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog.clone());
        let mut control = JournalControl {
            revision: 0,
            documents: vec![],
        };
        for generation in [1, 2] {
            let before = control.documents.len();
            let writes = scalar.writes();
            let result = coordinator.handle(
                Command::Acquire {
                    owner: id(1),
                    binding: game_binding(1, generation, ActivationEligibility::Running, &[42]),
                    config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                    target: TargetIdentity::Machine(machine()),
                    mode: ActivationMode::Session,
                },
                &mut control,
            );
            assert!(result.error.is_none());
            if generation == 2 {
                assert_eq!(result.outcome, Outcome::Unchanged);
                assert_eq!(scalar.writes(), writes);
                assert_eq!(control.documents.len(), before + 1);
            }
            let document = control.documents.last().unwrap();
            assert_eq!(document.resources[0].baseline.bytes(), 7i32.to_le_bytes());
            assert_eq!(
                document.owners[0]
                    .binding
                    .configured()
                    .unwrap()
                    .generation
                    .get(),
                generation
            );
        }
        let document = control.documents.last().unwrap();
        let reads = scalar.reads();
        let mut recovered = Coordinator::recover(catalog, document).unwrap();
        assert_eq!(scalar.reads(), reads);
        assert_eq!(
            recovered
                .handle(Command::Release { owner: id(1) }, &mut Control)
                .outcome,
            Outcome::Released
        );
        assert_eq!(scalar.value(), 7);
    }
    impl JournalControl {
        fn commit(
            &mut self,
            snapshot: &CoordinatorSnapshot,
            phase: JournalPhase,
        ) -> Result<(), GateError> {
            self.revision += 1;
            let document = JournalDocument::capture(
                machine(),
                self.revision,
                JournalStamp::Recovery,
                phase,
                snapshot,
            )
            .map_err(GateError::Durability)?;
            document.to_bytes().map_err(GateError::Durability)?;
            self.documents.push(document);
            Ok(())
        }
    }
    impl ExecutionControl for JournalControl {
        fn cancellation_requested(&self) -> bool {
            false
        }
        fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
            self.commit(
                &intent.ownership,
                JournalPhase::BeforeWrite {
                    step: intent.step as u32,
                    effect: intent.operation.effect(),
                },
            )
        }
        fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
            self.commit(
                &intent.ownership,
                JournalPhase::VerifiedWrite {
                    step: intent.step as u32,
                    effect: intent.operation.effect(),
                },
            )
        }
        fn publish(&mut self, _: CoordinatorSnapshot) {}
        fn commit_transition(&mut self, snapshot: &CoordinatorSnapshot) -> Result<(), GateError> {
            self.commit(snapshot, JournalPhase::Transition)
        }
    }
    #[test]
    fn every_multifield_restore_intent_has_valid_owner_resource_references() {
        let user = UserModule::new(None, true);
        let mut catalog = Catalog::default();
        catalog.register(user.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog);
        let mut control = JournalControl {
            revision: 0,
            documents: vec![],
        };
        let applied = coordinator.handle(
            Command::Acquire {
                binding: manual_binding(),
                owner: id(1),
                config: user
                    .config_envelope(&UserConfig {
                        value: 9,
                        enabled: false,
                    })
                    .unwrap(),
                target: TargetIdentity::Session(session()),
                mode: ActivationMode::Manual,
            },
            &mut control,
        );
        assert_eq!(applied.outcome, Outcome::Applied(Effect::RestartRequired));
        let result = coordinator.handle(Command::Release { owner: id(1) }, &mut control);
        assert_eq!(
            result.outcome,
            Outcome::Released,
            "the second field restore must publish a coherent owner reference set"
        );
        assert!(result.error.is_none());
        assert_eq!(user.value(), None);
        assert!(user.enabled());
        assert!(result.snapshot.resources.is_empty());
        assert!(control
            .documents
            .iter()
            .all(|document| document.validate().is_ok()));
    }
    #[test]
    fn catalog_survives_failed_hydration_and_observation_stays_available() {
        let (mut document, scalar) = scalar_document();
        document.sources[0].snapshot.version = SchemaVersion::new(99).unwrap();
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let reads = scalar.reads();
        assert!(Coordinator::recover(catalog.clone(), &document).is_err());
        assert_eq!(scalar.reads(), reads);
        let mut coordinator = Coordinator::new(catalog);
        let result = coordinator.handle(
            Command::Observe {
                feature: scalar
                    .config_envelope(&ScalarConfig { value: 12 })
                    .unwrap()
                    .feature,
                target: TargetIdentity::Machine(machine()),
            },
            &mut Control,
        );
        assert!(matches!(result.outcome, Outcome::Observed(_)));
        assert_eq!(scalar.reads(), reads + 1);
    }
    #[test]
    fn interruption_before_native_after_native_and_after_verification_keeps_exact_evidence() {
        for (phase, native_value, expected) in [
            (
                JournalPhase::BeforeWrite {
                    step: 0,
                    effect: Effect::Immediate,
                },
                7,
                Outcome::RecoveryRequired,
            ),
            (
                JournalPhase::BeforeWrite {
                    step: 0,
                    effect: Effect::Immediate,
                },
                12,
                Outcome::Unchanged,
            ),
            (
                JournalPhase::VerifiedWrite {
                    step: 0,
                    effect: Effect::Immediate,
                },
                12,
                Outcome::Unchanged,
            ),
        ] {
            let scalar = ScalarModule::new(7);
            let mut catalog = Catalog::default();
            catalog.register(scalar.clone()).unwrap();
            let mut coordinator = Coordinator::new(catalog.clone());
            let mut control = JournalControl {
                revision: 0,
                documents: vec![],
            };
            coordinator.handle(
                Command::Acquire {
                    binding: manual_binding(),
                    owner: id(1),
                    config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                    target: TargetIdentity::Machine(machine()),
                    mode: ActivationMode::Manual,
                },
                &mut control,
            );
            let document = control
                .documents
                .iter()
                .find(|document| document.phase == phase)
                .unwrap()
                .clone();
            scalar.external_change(native_value);
            let writes = scalar.writes();
            let mut recovered = Coordinator::recover(catalog, &document).unwrap();
            let plan = RecoveryPlan::resolve(&document, &mut Live);
            let result = recovered.apply_recovery(&plan, &mut Control);
            assert_eq!(result.outcome, expected);
            assert_eq!(
                scalar.writes(),
                writes,
                "reattachment never retries an interrupted apply without a new explicit request"
            );
            assert_eq!(
                result.snapshot.resources[0].baseline,
                document.resources[0].baseline
            );
            let result = recovered.handle(Command::Release { owner: id(1) }, &mut Control);
            assert_eq!(result.outcome, Outcome::Released);
            assert_eq!(scalar.value(), 7);
        }
    }
    struct Expired;
    impl RecoveryResolver for Expired {
        fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            expired_attachment(owner)
        }
        fn resource(&mut self, key: &ResourceKey) -> ResourceLifetime {
            if matches!(
                key.target,
                TargetIdentity::Process(_) | TargetIdentity::Session(_)
            ) {
                ResourceLifetime::PositivelyExpired
            } else {
                ResourceLifetime::VerifiedLive
            }
        }
    }
    #[test]
    fn ended_activation_restores_projected_machine_but_retires_actual_dead_resources() {
        for scope in [
            TargetScope::Machine,
            TargetScope::Process,
            TargetScope::Session,
        ] {
            let scalar = scoped(scope, "test_scoped");
            let mut catalog = Catalog::default();
            catalog.register(scalar.clone()).unwrap();
            let mut coordinator = Coordinator::new(catalog.clone());
            coordinator.handle(
                Command::Acquire {
                    binding: game_binding(1, 1, ActivationEligibility::Running, &[42]),
                    owner: id(1),
                    config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                    target: process(),
                    mode: ActivationMode::Session,
                },
                &mut Control,
            );
            let document = journal(&coordinator);
            let plan = RecoveryPlan::resolve(&document, &mut Expired);
            assert!(!plan.blocked());
            let writes = scalar.writes();
            let reads = scalar.reads();
            if scope != TargetScope::Machine {
                scalar.set_capability(Capability::Denied("dead native process".into()));
            }
            let mut recovered = Coordinator::recover(catalog, &document).unwrap();
            assert_eq!(scalar.reads(), reads);
            let result = recovered.apply_recovery(&plan, &mut Control);
            assert_eq!(result.outcome, Outcome::Released);
            assert!(result.snapshot.resources.is_empty());
            assert!(result.snapshot.owners.is_empty());
            if scope == TargetScope::Machine {
                assert_eq!(scalar.value(), 7);
                assert_eq!(scalar.writes(), writes + 1);
            } else {
                assert_eq!(scalar.value(), 12);
                assert_eq!(scalar.writes(), writes);
                assert_eq!(scalar.reads(), reads);
            }
        }
    }
    #[test]
    fn recovery_never_promotes_unknown_drift_to_confirmed_or_attempted_state() {
        let (document, scalar) = scalar_document();
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        scalar.external_change(99);
        let writes = scalar.writes();
        let mut recovered = Coordinator::recover(catalog, &document).unwrap();
        let plan = RecoveryPlan::resolve(&document, &mut Live);
        let result = recovered.apply_recovery(&plan, &mut Control);
        assert_eq!(result.outcome, Outcome::RecoveryRequired);
        assert_eq!(
            result.snapshot.resources[0].status,
            ModuleStatus::ExternalChange
        );
        assert_eq!(
            result.snapshot.resources[0].confirmed,
            document.resources[0].confirmed
        );
        assert!(result.snapshot.resources[0].possible.is_empty());
        let result = recovered.handle(Command::Release { owner: id(1) }, &mut Control);
        assert_eq!(result.outcome, Outcome::RecoveryRequired);
        assert_eq!(scalar.value(), 99);
        assert_eq!(scalar.writes(), writes);
    }
    struct WrongAttachment;
    impl RecoveryResolver for WrongAttachment {
        fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            let ActivationBinding::Game(binding) =
                game_binding(2, 1, ActivationEligibility::Running, &[43])
            else {
                unreachable!()
            };
            OwnerAttachment::VerifiedLive {
                owner: owner.owner,
                live_processes: binding.processes().to_vec(),
                binding,
            }
        }
        fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
            ResourceLifetime::VerifiedLive
        }
    }
    #[test]
    fn wrong_activation_and_unknown_actual_lifetime_do_not_dispatch_cleanup() {
        let (mut document, scalar) = scalar_document();
        document.owners[0].mode = ActivationMode::Session;
        document.owners[0].binding = game_binding(1, 1, ActivationEligibility::Running, &[42]);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let mut recovered = Coordinator::recover(catalog, &document).unwrap();
        let before = recovered.snapshot();
        let reads = scalar.reads();
        let writes = scalar.writes();
        for plan in [
            RecoveryPlan::resolve(&document, &mut WrongAttachment),
            RecoveryPlan::resolve(&document, &mut Unknown),
        ] {
            assert!(plan.blocked());
            assert_eq!(
                recovered.apply_recovery(&plan, &mut Control).outcome,
                Outcome::RecoveryRequired
            );
            assert_eq!(recovered.snapshot(), before);
        }
        assert_eq!(scalar.reads(), reads);
        assert_eq!(scalar.writes(), writes);
    }
    #[test]
    fn read_only_observation_and_later_apply_keep_pending_restoration_restart() {
        let user = UserModule::new(None, true);
        let scalar = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(user.clone()).unwrap();
        catalog.register(scalar.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog);
        coordinator.handle(
            Command::Acquire {
                binding: manual_binding(),
                owner: id(1),
                config: user
                    .config_envelope(&UserConfig {
                        value: 9,
                        enabled: false,
                    })
                    .unwrap(),
                target: TargetIdentity::Session(session()),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        );
        let released = coordinator.handle(Command::Release { owner: id(1) }, &mut Control);
        assert_eq!(released.restoration_effect, Some(Effect::RestartRequired));
        let feature = scalar
            .config_envelope(&ScalarConfig { value: 12 })
            .unwrap()
            .feature;
        let observed = coordinator.handle(
            Command::Observe {
                feature,
                target: TargetIdentity::Machine(machine()),
            },
            &mut Control,
        );
        assert_eq!(
            observed.snapshot.restoration_effect,
            Some(Effect::RestartRequired),
            "Observe has no durable transition and cannot clear pending recovery metadata"
        );
        let applied = coordinator.handle(
            Command::Acquire {
                binding: manual_binding(),
                owner: id(2),
                config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                target: TargetIdentity::Machine(machine()),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        );
        assert_eq!(
            applied.snapshot.restoration_effect,
            Some(Effect::RestartRequired)
        );
    }
}
