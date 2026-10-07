//! Native lifetime evidence is supplied by an authoritative producer, never
//! inferred from identity shape, a changed continuity nonce, or access denial.
use super::{
    contract::*,
    journal::{JournalDocument, JournalError, JournalOwner, JournalSource},
};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerAttachment {
    VerifiedLive { activation: TargetIdentity },
    PositivelyExpired,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceLifetime {
    VerifiedLive,
    PositivelyExpired,
    Unknown,
}
pub(super) trait RecoveryResolver {
    /// VerifiedLive must prove the original game binding. A live logon/session
    /// or an opaque owner number alone cannot establish game attachment.
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
    pub(super) fn resolve(document: &JournalDocument, resolver: &mut dyn RecoveryResolver) -> Self {
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
        // `retire(Session)` is an authoritative whole-incarnation action. A
        // positive result for just one setting cannot contradict a separately
        // verified live process in that same incarnation.
        for target in live {
            if retired.iter().any(|ended| match (&target, ended) {
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
        for owner in &document.owners {
            let source = &document.sources[owner.source.0 as usize];
            if !document
                .resources
                .iter()
                .any(|resource| resource.owners.contains(&owner.owner))
            {
                plan.release.push(owner.owner);
                continue;
            }
            if owner.mode == ActivationMode::Manual {
                plan.reattach.push(owner.owner);
                continue;
            }
            match resolver.owner(owner, source) {
                OwnerAttachment::VerifiedLive { activation }
                    if source.snapshot.target.as_ref() == Some(&activation) =>
                {
                    let conflicts = retired.iter().any(|ended| match (&activation, ended) {
                        (TargetIdentity::Process(process), TargetIdentity::Session(session)) => {
                            process.session == *session
                        }
                        _ => activation == *ended,
                    });
                    if conflicts {
                        plan.unknown_owners.push(owner.owner);
                    } else {
                        plan.reattach.push(owner.owner);
                    }
                }
                OwnerAttachment::PositivelyExpired => plan.release.push(owner.owner),
                _ => plan.unknown_owners.push(owner.owner),
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
                owner: id(1),
                config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                target: TargetIdentity::Machine(machine()),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        );
        (journal(&coordinator), scalar)
    }
    struct Live;
    impl RecoveryResolver for Live {
        fn owner(&mut self, _: &JournalOwner, source: &JournalSource) -> OwnerAttachment {
            OwnerAttachment::VerifiedLive {
                activation: source.snapshot.target.clone().unwrap(),
            }
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
        fn owner(&mut self, _: &JournalOwner, source: &JournalSource) -> OwnerAttachment {
            OwnerAttachment::VerifiedLive {
                activation: source.snapshot.target.clone().unwrap(),
            }
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
        fn owner(&mut self, _: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            OwnerAttachment::PositivelyExpired
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
        fn owner(&mut self, _: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
            OwnerAttachment::VerifiedLive {
                activation: TargetIdentity::Session(session()),
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
