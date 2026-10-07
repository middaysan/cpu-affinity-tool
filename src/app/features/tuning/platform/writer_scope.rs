//! One worker owns store, input capture and native scope evidence throughout
//! recovery, command admission, durable intent and completion.
use super::super::{
    catalog::PreparedOperation,
    configured_intent::{persistent_machine_resources, ConfiguredSnapshot, IntentDecision},
    contract::*,
    coordinator::{Command, ExecutionControl, WriteIntent},
    journal::{JournalError, JournalOwner, JournalSource},
    recovery::{OwnerAttachment, RecoveryPlan, RecoveryResolver, ResourceLifetime},
};
use super::*;

pub(in crate::app::features::tuning::platform) struct ScopedWriter {
    pub(super) session: DurableSession<Box<dyn JournalStore>>,
    catalog: Catalog,
    machine: MachineIdentity,
    source: Arc<dyn ConfiguredIntentSource>,
    scope: ScopeEvidence,
    captured: Option<Captured>,
}
struct Captured {
    configured: ConfiguredSnapshot,
    before: CoordinatorSnapshot,
}

impl ScopeEvidence {
    fn verify(&mut self, _expected: &MachineIdentity) -> Result<(), GateError> {
        match self {
            Self::Unavailable => Err(GateError::Rejected("resource scope is unavailable")),
            #[cfg(windows)]
            Self::Native(original) => {
                if &original.machine != _expected {
                    return Err(GateError::Rejected("native machine scope mismatch"));
                }
                let actual = os_api::OS::tuning_machine_identity().map_err(|error| {
                    GateError::Durability(JournalError::Io(native_error(error)))
                })?;
                let actual = native_machine(actual.source(), actual.bytes())
                    .map_err(GateError::Durability)?;
                if &actual == _expected {
                    Ok(())
                } else {
                    Err(GateError::Rejected("machine scope mismatch"))
                }
            }
            #[cfg(test)]
            Self::Fixture { machine, .. } => {
                if &machine()? == _expected {
                    Ok(())
                } else {
                    Err(GateError::Rejected("machine scope mismatch"))
                }
            }
        }
    }
    fn fixture_lifetime_enabled(&self) -> bool {
        match self {
            #[cfg(test)]
            Self::Fixture {
                lifetime: Some(_), ..
            } => true,
            _ => false,
        }
    }
    fn supports(
        &self,
        descriptor: &ModuleDescriptor,
        target: &TargetIdentity,
        keys: &[ResourceKey],
        machine: &MachineIdentity,
    ) -> bool {
        if self.fixture_lifetime_enabled() {
            // Deliberately fixture-only lifecycle domain; this never creates
            // native boot/game authority or enters the canonical constructor.
            return !keys.is_empty()
                && keys.iter().all(|key| {
                    descriptor.resources.iter().any(|declared| {
                        declared.field == key.field
                            && target.resource_target(declared.scope).as_ref() == Some(&key.target)
                    })
                });
        }
        persistent_machine_resources(descriptor, target, keys, machine)
    }
}
impl ScopedWriter {
    fn check_operation(&self, operation: &PreparedOperation) -> Result<(), GateError> {
        let keys = operation
            .resources()
            .iter()
            .map(|claim| claim.key.clone())
            .collect::<Vec<_>>();
        if self.scope.supports(
            operation.descriptor(),
            operation.target(),
            &keys,
            &self.machine,
        ) {
            Ok(())
        } else {
            Err(GateError::Rejected("resource lifetime is unsupported"))
        }
    }
    fn check_source(
        &self,
        snapshot: &super::super::codec::ModuleEnvelope,
        config: Option<&super::super::codec::ModuleEnvelope>,
    ) -> Result<(), GateError> {
        let operation = self
            .catalog
            .recover_source(snapshot, config)
            .map_err(|_| GateError::Rejected("source hydration is unverified"))?;
        self.check_operation(&operation)
    }
    fn decision(
        &self,
        input: &ConfiguredSnapshot,
        owner: &super::super::coordinator::OwnerView,
    ) -> IntentDecision {
        let Some(config) = owner.config.as_deref() else {
            return IntentDecision::Unknown;
        };
        if !self.scope.fixture_lifetime_enabled()
            && !matches!(
                (&owner.binding, owner.mode),
                (ActivationBinding::Manual(_), ActivationMode::Manual)
            )
        {
            return IntentDecision::Unknown;
        }
        input.decision(
            owner.owner,
            &owner.binding,
            config,
            &owner.target,
            owner.mode,
        )
    }
    fn check_ledger(
        &self,
        input: &ConfiguredSnapshot,
        snapshot: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        for owner in &snapshot.owners {
            if self.decision(input, owner) == IntentDecision::Unknown {
                return Err(GateError::Rejected("configured owner is unknown"));
            }
            self.check_source(&owner.snapshot, owner.config.as_deref())?;
        }
        // The original adapter remains the restorer across cross-module sharing.
        // A later writer cannot confer its lifetime declaration on that source.
        for resource in &snapshot.resources {
            self.check_source(
                &resource.baseline_snapshot,
                resource.baseline_config.as_deref(),
            )?;
            self.check_source(
                &resource.current_snapshot,
                resource.current_config.as_deref(),
            )?;
        }
        Ok(())
    }
    pub(super) fn capture(&mut self, before: &CoordinatorSnapshot) -> Result<(), GateError> {
        if matches!(self.session.load(), JournalLoad::Blocked(_)) {
            return Err(GateError::Durability(JournalError::Blocked));
        }
        self.captured = None;
        let configured = self.source.snapshot();
        if matches!(configured, ConfiguredSnapshot::Unavailable) {
            return Err(GateError::Rejected("configured input is unavailable"));
        }
        self.scope.verify(&self.machine)?;
        self.check_ledger(&configured, before)?;
        self.captured = Some(Captured {
            configured,
            before: before.clone(),
        });
        Ok(())
    }
    pub(super) fn admit_command(
        &mut self,
        command: &Command,
        before: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        self.capture(before)?;
        let captured = self
            .captured
            .as_ref()
            .ok_or(GateError::Rejected("configured input is unavailable"))?;
        match command {
            Command::Acquire {
                owner,
                binding,
                config,
                target,
                mode,
            } => {
                if !self.scope.fixture_lifetime_enabled()
                    && !matches!(
                        (binding, mode),
                        (ActivationBinding::Manual(_), ActivationMode::Manual)
                    )
                {
                    return Err(GateError::Rejected("activation scope is unsupported"));
                }
                if captured
                    .configured
                    .decision(*owner, binding, config, target, *mode)
                    != IntentDecision::Enabled
                {
                    return Err(GateError::Rejected("configured request is unverified"));
                }
                self.catalog
                    .validate_config(config)
                    .map_err(|_| GateError::Rejected("configuration codec is unverified"))?;
                let descriptor = self
                    .catalog
                    .descriptors()
                    .find(|descriptor| descriptor.id == config.feature)
                    .ok_or(GateError::Rejected("module is unknown"))?;
                let keys = descriptor
                    .resources
                    .iter()
                    .filter_map(|resource| {
                        target
                            .resource_target(resource.scope)
                            .map(|target| ResourceKey {
                                target,
                                field: resource.field.clone(),
                            })
                    })
                    .collect::<Vec<_>>();
                if !self
                    .scope
                    .supports(descriptor, target, &keys, &self.machine)
                {
                    return Err(GateError::Rejected("resource lifetime is unsupported"));
                }
                Ok(())
            }
            Command::Release { owner } => {
                let recorded = before
                    .owners
                    .iter()
                    .find(|entry| entry.owner == *owner)
                    .ok_or(GateError::Rejected("configured release has no owner"))?;
                if self.decision(&captured.configured, recorded) == IntentDecision::Disabled {
                    Ok(())
                } else {
                    Err(GateError::Rejected("configured release is unverified"))
                }
            }
            Command::Retire { .. } => {
                Err(GateError::Rejected("resource retirement is unsupported"))
            }
            Command::Observe { .. } => Ok(()),
        }
    }
    pub(super) fn check_current(&mut self) -> Result<(), GateError> {
        let checked = (|| {
            if matches!(self.session.load(), JournalLoad::Blocked(_)) {
                return Err(GateError::Durability(JournalError::Blocked));
            }
            let captured = self
                .captured
                .as_ref()
                .ok_or(GateError::Rejected("configured capture is absent"))?;
            if self.source.snapshot() != captured.configured {
                return Err(GateError::Rejected("configured input changed"));
            }
            self.scope.verify(&self.machine)?;
            self.check_ledger(&captured.configured, &captured.before)
        })();
        if let Err(error) = &checked {
            let reason = match error {
                GateError::Durability(reason) => reason.clone(),
                _ => JournalError::InvalidEvidence("configured or resource authority changed"),
            };
            self.session.block(reason);
        }
        checked
    }
    pub(super) fn check_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.check_current()?;
        self.check_operation(intent.operation)?;
        let captured = self
            .captured
            .as_ref()
            .ok_or(GateError::Rejected("configured capture is absent"))?;
        self.check_ledger(&captured.configured, &intent.ownership)
    }
    pub(super) fn commit(
        &mut self,
        stamp: JournalStamp,
        phase: JournalPhase,
        snapshot: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        self.check_current()?;
        let captured = self
            .captured
            .as_ref()
            .ok_or(GateError::Rejected("configured capture is absent"))?;
        self.check_ledger(&captured.configured, snapshot)?;
        self.session
            .commit(stamp, phase, snapshot)
            .map_err(GateError::Durability)
    }
}
struct Resolver<'a> {
    scope: &'a mut ScopeEvidence,
    configured: &'a ConfiguredSnapshot,
}
impl RecoveryResolver for Resolver<'_> {
    fn owner(&mut self, owner: &JournalOwner, source: &JournalSource) -> OwnerAttachment {
        let Some(config) = source.config.as_ref() else {
            return OwnerAttachment::Unknown;
        };
        let Some(target) = source.snapshot.target.as_ref() else {
            return OwnerAttachment::Unknown;
        };
        if self
            .configured
            .decision(owner.owner, &owner.binding, config, target, owner.mode)
            == IntentDecision::Unknown
        {
            return OwnerAttachment::Unknown;
        }
        match self.scope {
            #[cfg(test)]
            ScopeEvidence::Fixture {
                lifetime: Some(resolver),
                ..
            } => resolver.owner(owner, source),
            _ => OwnerAttachment::Unknown,
        }
    }
    fn resource(&mut self, key: &ResourceKey) -> ResourceLifetime {
        match self.scope {
            #[cfg(test)]
            ScopeEvidence::Fixture {
                lifetime: Some(resolver),
                ..
            } => resolver.resource(key),
            _ if matches!(key.target, TargetIdentity::Machine(_)) => ResourceLifetime::VerifiedLive,
            _ => ResourceLifetime::Unknown,
        }
    }
}
struct BootstrapControl<'a>(&'a mut ScopedWriter);
impl ExecutionControl for BootstrapControl<'_> {
    fn cancellation_requested(&self) -> bool {
        false
    }
    fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.0.check_write(intent)?;
        let step =
            u32::try_from(intent.step).map_err(|_| GateError::Rejected("invalid step index"))?;
        self.0.commit(
            JournalStamp::Recovery,
            JournalPhase::BeforeWrite {
                step,
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )?;
        Ok(())
    }
    fn before_execute(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
        self.0.check_current()
    }
    fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        let step =
            u32::try_from(intent.step).map_err(|_| GateError::Rejected("invalid step index"))?;
        self.0.commit(
            JournalStamp::Recovery,
            JournalPhase::VerifiedWrite {
                step,
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )
    }
    fn commit_transition(&mut self, snapshot: &CoordinatorSnapshot) -> Result<(), GateError> {
        self.0
            .commit(JournalStamp::Recovery, JournalPhase::Recovery, snapshot)
    }
    fn publish(&mut self, _: CoordinatorSnapshot) {}
}
pub(super) fn bootstrap(
    catalog: Catalog,
    machine: MachineIdentity,
    store: Box<dyn JournalStore>,
    source: Arc<dyn ConfiguredIntentSource>,
    scope: ScopeEvidence,
) -> (Coordinator, WorkerAuthority) {
    let session = DurableSession::bootstrap(store, machine.clone());
    let mut writer = ScopedWriter {
        session,
        catalog: catalog.clone(),
        machine,
        source,
        scope,
        captured: None,
    };
    let coordinator = match writer.session.load().clone() {
        JournalLoad::Known(document) => match Coordinator::recover(catalog.clone(), &document) {
            Ok(mut recovered) => {
                let result = (|| {
                    writer.capture(&recovered.snapshot())?;
                    let captured = writer
                        .captured
                        .as_ref()
                        .ok_or(GateError::Rejected("configured capture is absent"))?;
                    let configured = captured.configured.clone();
                    let mut resolver = Resolver {
                        scope: &mut writer.scope,
                        configured: &configured,
                    };
                    let plan = RecoveryPlan::resolve_configured(
                        &document,
                        &mut resolver,
                        &mut |owner, source| {
                            let Some(config) = source.config.as_ref() else {
                                return IntentDecision::Unknown;
                            };
                            let Some(target) = source.snapshot.target.as_ref() else {
                                return IntentDecision::Unknown;
                            };
                            configured.decision(
                                owner.owner,
                                &owner.binding,
                                config,
                                target,
                                owner.mode,
                            )
                        },
                    );
                    if plan.blocked() {
                        return Err(GateError::Rejected("recovery lifetime is unverified"));
                    }
                    let result =
                        recovered.apply_recovery(&plan, &mut BootstrapControl(&mut writer));
                    if result.error.is_some()
                        || !matches!(
                            result.outcome,
                            super::super::coordinator::Outcome::Unchanged
                                | super::super::coordinator::Outcome::Released
                        )
                    {
                        return Err(GateError::Rejected("durable recovery did not complete"));
                    }
                    Ok(())
                })();
                if result.is_err() && !matches!(writer.session.load(), JournalLoad::Blocked(_)) {
                    writer.session.block(JournalError::InvalidEvidence(
                        "configured recovery is unverified",
                    ));
                }
                recovered
            }
            Err(_) => {
                writer.session.block(JournalError::InvalidEvidence(
                    "journal source hydration failed",
                ));
                Coordinator::new(catalog)
            }
        },
        JournalLoad::Clean | JournalLoad::Blocked(_) => Coordinator::new(catalog),
    };
    // Keep the same worker-local session/lease even after a blocked bootstrap.
    (coordinator, WorkerAuthority::Scoped(Box::new(writer)))
}
