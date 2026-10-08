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
    #[cfg(any(windows, test))]
    staged_native: bool,
}
struct Captured {
    configured: ConfiguredSnapshot,
    before: CoordinatorSnapshot,
}
/// Only this worker-local writer creates positive terminal corroboration.
/// Serialized acknowledgements are never accepted as this token.
pub(in crate::app::features::tuning) struct VerifiedAcknowledgements {
    records: Vec<crate::app::features::tuning::config::SavedAcknowledgement>,
}
impl VerifiedAcknowledgements {
    pub(in crate::app::features::tuning) fn records(
        &self,
    ) -> &[crate::app::features::tuning::config::SavedAcknowledgement] {
        &self.records
    }
}

impl ScopeEvidence {
    fn origin(&self) -> Option<&ConfigurationOrigin> {
        match self {
            Self::Unavailable => None,
            #[cfg(windows)]
            Self::Native(native) => native.configuration.as_ref().map(|proof| proof.origin()),
            #[cfg(test)]
            Self::Fixture { origin, .. } => Some(origin),
        }
    }
    fn begin_configuration_guard(&mut self) -> Result<(), GateError> {
        match self {
            Self::Unavailable => Err(GateError::Rejected("configuration origin is unavailable")),
            #[cfg(windows)]
            Self::Native(native) => {
                let proof = native.configuration.as_ref().ok_or(GateError::Rejected(
                    "native configuration proof is unavailable",
                ))?;
                if native.guard.is_none() {
                    native.guard = Some(proof.open_guard().map_err(|error| {
                        GateError::Durability(JournalError::Io(native_error(error)))
                    })?);
                }
                Ok(())
            }
            #[cfg(test)]
            Self::Fixture { .. } => Ok(()),
        }
    }
    fn finish_configuration_guard(&mut self) {
        #[cfg(windows)]
        if let Self::Native(native) = self {
            native.guard = None;
        }
    }
    fn configured_origin_matches(&self, configured: &ConfiguredSnapshot) -> bool {
        if self.origin().is_none() || self.origin() != configured.origin() {
            return false;
        }
        match self {
            #[cfg(windows)]
            Self::Native(native) => native.configuration.as_ref() == configured.native_proof(),
            #[cfg(test)]
            Self::Fixture { .. } => true,
            _ => false,
        }
    }
    fn verify(&mut self, _expected: &MachineIdentity) -> Result<(), GateError> {
        match self {
            Self::Unavailable => Err(GateError::Rejected("resource scope is unavailable")),
            #[cfg(windows)]
            Self::Native(original) => {
                let proof = original.configuration.as_ref().ok_or(GateError::Rejected(
                    "native configuration proof is unavailable",
                ))?;
                let guard = original
                    .guard
                    .as_mut()
                    .ok_or(GateError::Rejected("native configuration guard is absent"))?;
                proof.verify_guard(guard).map_err(|error| {
                    GateError::Durability(JournalError::Io(native_error(error)))
                })?;
                Ok(())
            }
            #[cfg(test)]
            Self::Fixture { revision, .. } => {
                if let Some(revision) = revision {
                    if (revision.observe)()? != revision.bytes {
                        return Err(GateError::Rejected("fixture saved revision changed"));
                    }
                }
                Ok(())
            }
        }?;
        self.verify_machine(_expected)
    }
    fn verify_machine(&mut self, expected: &MachineIdentity) -> Result<(), GateError> {
        #[cfg(not(any(windows, test)))]
        let _ = expected;
        match self {
            Self::Unavailable => Err(GateError::Rejected("resource scope is unavailable")),
            #[cfg(windows)]
            Self::Native(original) => {
                if &original.machine != expected {
                    return Err(GateError::Rejected("native machine scope mismatch"));
                }
                let actual = os_api::OS::tuning_machine_identity().map_err(|error| {
                    GateError::Durability(JournalError::Io(native_error(error)))
                })?;
                let actual = native_machine(actual.source(), actual.bytes())
                    .map_err(GateError::Durability)?;
                if &actual == expected {
                    Ok(())
                } else {
                    Err(GateError::Rejected("machine scope mismatch"))
                }
            }
            #[cfg(test)]
            Self::Fixture { machine, .. } => {
                if &machine()? == expected {
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
    #[cfg(test)]
    pub(super) fn stage_configuration_for_test(&mut self) {
        self.staged_native = true;
    }
    pub(super) fn configuration_staged(&self) -> bool {
        #[cfg(any(windows, test))]
        {
            self.staged_native
        }
        #[cfg(not(any(windows, test)))]
        {
            false
        }
    }
    pub(super) fn reserve_configuration(
        &mut self,
        before: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        let checked = (|| {
            if matches!(self.session.load(), JournalLoad::Blocked(_)) {
                return Err(GateError::Durability(JournalError::Blocked));
            }
            if self.scope.origin().is_none() || self.scope.origin() != self.session.origin() {
                return Err(GateError::Rejected(
                    "original configuration origin is unavailable",
                ));
            }
            let configured = self.source.snapshot();
            if configured != ConfiguredSnapshot::Unavailable {
                if !self.scope.configured_origin_matches(&configured) {
                    return Err(GateError::Rejected("original configured input changed"));
                }
                self.check_ledger(&configured, before)?;
            } else if !before.owners.is_empty() || !before.resources.is_empty() {
                return Err(GateError::Rejected(
                    "retained configured input is unavailable",
                ));
            }
            self.scope.begin_configuration_guard()?;
            self.scope.verify(&self.machine)
        })();
        self.finish_operation();
        checked
    }
    pub(super) fn saved_acknowledgements(
        &mut self,
        before: &CoordinatorSnapshot,
    ) -> Result<VerifiedAcknowledgements, GateError> {
        self.reserve_configuration(before)?;
        let checked = (|| {
            self.scope.begin_configuration_guard()?;
            self.scope.verify(&self.machine)?;
            let configured = self.source.snapshot();
            let records = match self.session.load() {
                JournalLoad::Known(document)
                    if matches!(
                        document.phase,
                        JournalPhase::Transition | JournalPhase::Recovery
                    ) =>
                {
                    let origin = self
                        .session
                        .origin()
                        .ok_or(GateError::Rejected("ack original origin absent"))?;
                    let measured = super::super::journal::JournalDocument::capture_bound(
                        self.machine.clone(),
                        origin.clone(),
                        document.revision,
                        document.stamp,
                        document.phase.clone(),
                        before,
                    )
                    .map_err(GateError::Durability)?;
                    if &measured != document {
                        return Err(GateError::Rejected("ack durable ledger differs"));
                    }
                    // Observe each current typed writer once for its exact owned
                    // fields. No snapshot/baseline is recaptured or write attempted.
                    let mut groups = std::collections::BTreeMap::new();
                    for resource in &before.resources {
                        let target = resource
                            .current_snapshot
                            .target
                            .as_ref()
                            .ok_or(GateError::Rejected("ack writer target absent"))?;
                        groups
                            .entry((&resource.current_source, target))
                            .or_insert_with(Vec::new)
                            .push(resource);
                    }
                    for ((feature, target), resources) in groups {
                        let keys = resources.iter().map(|r| r.key.clone()).collect::<Vec<_>>();
                        let observed = self
                            .catalog
                            .observe_resources(feature, target, &keys)
                            .map_err(|_| {
                                GateError::Rejected("ack resource observation unavailable")
                            })?;
                        if resources.iter().any(|resource| {
                            !observed.iter().any(|state| {
                                state.key == resource.key
                                    && state.value == resource.confirmed
                                    && state.value == resource.desired
                            })
                        }) {
                            return Err(GateError::Rejected("ack current resource changed"));
                        }
                    }
                    configured.terminal_acknowledgements(origin, before, document.revision)
                }
                _ => Vec::new(),
            };
            self.scope.verify(&self.machine)?;
            if self.source.snapshot() != configured {
                return Err(GateError::Rejected("ack saved source changed"));
            }
            Ok(VerifiedAcknowledgements { records })
        })();
        self.finish_operation();
        checked
    }
    pub(super) fn install_saved(
        &mut self,
        revision: &SavedRevision,
        before: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        #[cfg(not(any(windows, test)))]
        let _ = before;
        #[cfg(any(windows, test))]
        use crate::app::features::tuning::configured_source::SavedConfiguredSource;
        match revision {
            SavedRevision::Unavailable => {
                return Err(GateError::Rejected("saved input is unavailable"))
            }
            #[cfg(windows)]
            SavedRevision::Native(proof) => {
                let proof = proof.as_ref();
                if !matches!(self.scope, ScopeEvidence::Native(_))
                    || self.scope.origin() != Some(proof.origin())
                    || self.session.origin() != Some(proof.origin())
                {
                    return Err(GateError::Rejected("saved original origin mismatch"));
                }
                let guard = proof.open_guard().map_err(|error| {
                    GateError::Durability(JournalError::Io(native_error(error)))
                })?;
                self.scope.verify_machine(&self.machine)?;
                let source = SavedConfiguredSource::from_native(proof.clone(), &self.catalog);
                let configured = source.snapshot();
                if configured == ConfiguredSnapshot::Unavailable
                    || configured.native_proof() != Some(proof)
                {
                    return Err(GateError::Rejected("saved configured input is unavailable"));
                }
                self.check_ledger(&configured, before)?;
                let ScopeEvidence::Native(native) = &mut self.scope else {
                    unreachable!()
                };
                native.configuration = Some(proof.clone());
                native.guard = Some(guard);
                self.source = Arc::new(source);
                // B3 must teach both admission and recovery old/new roles before
                // a saved producer can enable native mutations. This closes
                // actual capture independently of factory/source availability.
                self.staged_native = true;
            }
            #[cfg(test)]
            SavedRevision::Fixture { bytes, origin } => {
                if !matches!(self.scope, ScopeEvidence::Fixture { .. })
                    || self.scope.origin() != Some(origin)
                    || self.session.origin() != Some(origin)
                {
                    return Err(GateError::Rejected(
                        "saved fixture original origin mismatch",
                    ));
                }
                let ScopeEvidence::Fixture { revision, .. } = &mut self.scope else {
                    unreachable!()
                };
                if let Some(revision) = revision {
                    if (revision.observe)()?.as_ref() != bytes.as_ref() {
                        return Err(GateError::Rejected("fixture handoff revision mismatch"));
                    }
                }
                let source =
                    SavedConfiguredSource::from_fixture(bytes, origin.clone(), &self.catalog);
                let configured = source.snapshot();
                if configured == ConfiguredSnapshot::Unavailable {
                    return Err(GateError::Rejected("saved configured input is unavailable"));
                }
                self.scope.verify_machine(&self.machine)?;
                self.check_ledger(&configured, before)?;
                let ScopeEvidence::Fixture { revision, .. } = &mut self.scope else {
                    unreachable!()
                };
                if let Some(revision) = revision {
                    revision.bytes = Arc::clone(bytes);
                }
                self.source = Arc::new(source);
            }
        }
        self.finish_operation();
        Ok(())
    }
    pub(super) fn finish_operation(&mut self) {
        self.captured = None;
        self.scope.finish_configuration_guard();
    }
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
        let Some(origin) = self.session.origin() else {
            return IntentDecision::Unknown;
        };
        input.ledger_decision_for_origin(
            origin,
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
        if self.configuration_staged() {
            return Err(GateError::MutationDisabled);
        }
        if matches!(self.session.load(), JournalLoad::Blocked(_)) {
            return Err(GateError::Durability(JournalError::Blocked));
        }
        self.captured = None;
        let configured = self.source.snapshot();
        if matches!(configured, ConfiguredSnapshot::Unavailable) {
            return Err(GateError::Rejected("configured input is unavailable"));
        }
        if !self.scope.configured_origin_matches(&configured)
            || configured.origin() != self.session.origin()
        {
            return Err(GateError::Rejected("configured origin is unverified"));
        }
        self.scope.begin_configuration_guard()?;
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
                if captured.configured.decision_for_origin(
                    self.session
                        .origin()
                        .ok_or(GateError::Rejected("configuration origin is unavailable"))?,
                    *owner,
                    binding,
                    config,
                    target,
                    *mode,
                ) != IntentDecision::Enabled
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
                if !before.owners.iter().any(|entry| entry.owner == *owner) {
                    let entry = captured
                        .configured
                        .disabled_owner_for_origin(
                            self.session
                                .origin()
                                .ok_or(GateError::Rejected("withdrawal origin absent"))?,
                            *owner,
                        )
                        .ok_or(GateError::Rejected("unacquired withdrawal is unknown"))?;
                    if !self.scope.fixture_lifetime_enabled()
                        && !matches!(
                            (&entry.binding, entry.mode),
                            (ActivationBinding::Manual(_), ActivationMode::Manual)
                        )
                    {
                        return Err(GateError::Rejected("withdrawal scope unavailable"));
                    }
                    self.catalog
                        .validate_config(&entry.config)
                        .map_err(|_| GateError::Rejected("withdrawal codec unavailable"))?;
                    let descriptor = self
                        .catalog
                        .descriptors()
                        .find(|d| d.id == entry.config.feature)
                        .ok_or(GateError::Rejected("withdrawal module unavailable"))?;
                    let keys = descriptor
                        .resources
                        .iter()
                        .filter_map(|r| {
                            entry
                                .target
                                .resource_target(r.scope)
                                .map(|target| ResourceKey {
                                    target,
                                    field: r.field.clone(),
                                })
                        })
                        .collect::<Vec<_>>();
                    if !self
                        .scope
                        .supports(descriptor, &entry.target, &keys, &self.machine)
                        || !entry.binding.permits(entry.mode, &entry.target)
                    {
                        return Err(GateError::Rejected("withdrawal lifetime unavailable"));
                    }
                    return Ok(());
                }
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
            if !self.scope.configured_origin_matches(&captured.configured)
                || captured.configured.origin() != self.session.origin()
            {
                return Err(GateError::Rejected("configured origin changed"));
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
    origin: &'a ConfigurationOrigin,
}
impl RecoveryResolver for Resolver<'_> {
    fn owner(&mut self, owner: &JournalOwner, source: &JournalSource) -> OwnerAttachment {
        let Some(config) = source.config.as_ref() else {
            return OwnerAttachment::Unknown;
        };
        let Some(target) = source.snapshot.target.as_ref() else {
            return OwnerAttachment::Unknown;
        };
        if self.configured.ledger_decision_for_origin(
            self.origin,
            owner.owner,
            &owner.binding,
            config,
            target,
            owner.mode,
        ) == IntentDecision::Unknown
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
    fn before_pending_update(
        &mut self,
        command: &Command,
        before: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        self.0.admit_command(command, before)?;
        let Command::Acquire {
            owner,
            binding,
            config,
            target,
            mode,
        } = command
        else {
            return Err(GateError::MutationDisabled);
        };
        let recorded = before
            .owners
            .iter()
            .find(|entry| entry.owner == *owner)
            .ok_or(GateError::Rejected("pending update has no retained owner"))?;
        let captured = self
            .0
            .captured
            .as_ref()
            .ok_or(GateError::Rejected("pending update capture is absent"))?;
        let origin = self
            .0
            .session
            .origin()
            .ok_or(GateError::Rejected("pending update origin is absent"))?;
        let requested = captured
            .configured
            .pending_update_for_origin(origin, recorded)
            .ok_or(GateError::Rejected("independent pending update is absent"))?;
        if &requested.binding == binding
            && &requested.config == config
            && &requested.target == target
            && requested.mode == *mode
        {
            Ok(())
        } else {
            Err(GateError::Rejected("pending update request differs"))
        }
    }
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
    let expected_origin = scope.origin().cloned();
    let session = DurableSession::bootstrap_bound(store, machine.clone(), expected_origin);
    let mut writer = ScopedWriter {
        session,
        catalog: catalog.clone(),
        machine,
        source,
        scope,
        captured: None,
        #[cfg(any(windows, test))]
        staged_native: false,
    };
    let coordinator = match writer.session.load().clone() {
        JournalLoad::Known(document) => {
            match writer.scope.begin_configuration_guard().and_then(|_| {
                Coordinator::recover(catalog.clone(), &document)
                    .map_err(|_| GateError::Rejected("journal source hydration failed"))
            }) {
                Ok(mut recovered) => {
                    let result = (|| {
                        writer.capture(&recovered.snapshot())?;
                        let captured = writer
                            .captured
                            .as_ref()
                            .ok_or(GateError::Rejected("configured capture is absent"))?;
                        let configured = captured.configured.clone();
                        let origin =
                            writer.session.origin().cloned().ok_or(GateError::Rejected(
                                "configuration origin is unavailable",
                            ))?;
                        let mut resolver = Resolver {
                            scope: &mut writer.scope,
                            configured: &configured,
                            origin: &origin,
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
                                configured.ledger_decision_for_origin(
                                    &origin,
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
                        if result.error.is_some() {
                            return Err(GateError::Rejected("durable recovery did not complete"));
                        }
                        let requests = recovered
                            .snapshot()
                            .owners
                            .iter()
                            .filter_map(|recorded| {
                                configured
                                    .pending_update_for_origin(&origin, recorded)
                                    .cloned()
                            })
                            .collect::<Vec<_>>();
                        for request in requests {
                            let snapshot = recovered.snapshot();
                            let completed = snapshot.owners.iter().find(|owner| owner.owner == request.owner)
                                .is_some_and(|owner| owner.binding == request.binding && owner.config.as_deref() == Some(&request.config)
                                && owner.resources.iter().all(|key| snapshot.resources.iter().find(|resource| &resource.key == key)
                                    .is_some_and(|resource| resource.confirmed == resource.desired && resource.possible.is_empty()
                                        && matches!(resource.status, ModuleStatus::VerifiedActive(_) | ModuleStatus::StoredRestartRequired))));
                            if !completed {
                                let result = recovered.resume_pending_update(
                                    &request,
                                    &mut BootstrapControl(&mut writer),
                                );
                                if result.error.is_some() {
                                    return Err(GateError::Rejected(
                                        "pending update did not complete",
                                    ));
                                }
                            }
                        }
                        if recovered.snapshot().resources.iter().any(|resource| {
                            !matches!(
                                resource.status,
                                ModuleStatus::VerifiedActive(_)
                                    | ModuleStatus::StoredRestartRequired
                            )
                        }) {
                            return Err(GateError::Rejected("durable recovery remains incomplete"));
                        }
                        Ok(())
                    })();
                    if result.is_err() && !matches!(writer.session.load(), JournalLoad::Blocked(_))
                    {
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
            }
        }
        JournalLoad::Clean | JournalLoad::Blocked(_) => Coordinator::new(catalog),
    };
    writer.finish_operation();
    // Keep the same worker-local session/lease even after a blocked bootstrap.
    (coordinator, WorkerAuthority::Scoped(Box::new(writer)))
}
