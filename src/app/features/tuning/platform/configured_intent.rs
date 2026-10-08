//! Independent configured input. Recorded journal ownership is never a producer.
use super::{codec::ModuleEnvelope, contract::*};
use serde::Serialize;
use std::sync::Arc;

pub(crate) const MAX_CONFIGURED_OWNERS: usize = 256;
const MAX_CONFIGURED_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn saved_configuration(bytes: &[u8]) -> Option<super::super::config::TuningConfig> {
    if bytes.len() > 16 * 1024 * 1024 {
        return None;
    }
    let state: crate::app::models::AppStateStorage = serde_json::from_slice(bytes).ok()?;
    if state.version != 11 || state.save_blocked() {
        return None;
    }
    match state.tuning.decode() {
        super::super::config::TuningLoad::Known(config) => Some(config),
        _ => None,
    }
}

/// A parsed wire origin never constructs this evidence. Only sealed native
/// receipt bytes from the selected storage seam can supply it.
#[cfg(windows)]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NativeConfigurationProof {
    receipt: os_api::tuning::TuningConfigReceipt,
    selected_path: std::path::PathBuf,
    origin: ConfigurationOrigin,
    tuning_raw: Arc<str>,
}
#[cfg(windows)]
impl std::fmt::Debug for NativeConfigurationProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConfigurationProof(redacted)")
    }
}
#[cfg(windows)]
impl NativeConfigurationProof {
    fn saved_bytes(&self) -> &[u8] {
        self.receipt.bytes()
    }
    pub(crate) fn from_receipt(
        selected_path: &std::path::Path,
        receipt: os_api::tuning::TuningConfigReceipt,
    ) -> Option<Self> {
        let config = saved_configuration(receipt.bytes())?;
        let state: crate::app::models::AppStateStorage =
            serde_json::from_slice(receipt.bytes()).ok()?;
        let origin = ConfigurationOrigin::from_source(
            config.lineage,
            UserSid::new(receipt.principal_sid()).ok()?,
            receipt.volume_guid().to_vec(),
            receipt.volume_serial(),
            *receipt.directory_id(),
            receipt.leaf(),
        )
        .ok()?;
        Some(Self {
            tuning_raw: Arc::from(state.tuning.raw()?),
            receipt,
            selected_path: selected_path.into(),
            origin,
        })
    }
    pub(crate) fn origin(&self) -> &ConfigurationOrigin {
        &self.origin
    }
    pub(crate) fn matches_tuning(&self, tuning: &super::super::config::TuningEnvelope) -> bool {
        saved_tuning_matches(&self.tuning_raw, tuning)
    }
    pub(super) fn open_guard(
        &self,
    ) -> Result<os_api::tuning::TuningConfigSource, os_api::tuning::TuningNativeError> {
        let mut source = os_api::tuning::TuningConfigSource::open(&self.selected_path)?;
        self.verify_guard(&mut source)?;
        Ok(source)
    }
    pub(super) fn verify_guard(
        &self,
        source: &mut os_api::tuning::TuningConfigSource,
    ) -> Result<(), os_api::tuning::TuningNativeError> {
        if source.read()?.as_ref() == Some(&self.receipt) {
            Ok(())
        } else {
            Err(os_api::tuning::TuningNativeError::UnsafeStore(
                "configured source revision changed",
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum ConfiguredDecision {
    Enabled,
    Disabled,
}
fn saved_tuning_matches(saved: &str, current: &super::super::config::TuningEnvelope) -> bool {
    current.raw() == Some(saved)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntentDecision {
    Enabled,
    Disabled,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ConfiguredEntry {
    pub owner: OwnerId,
    pub binding: ActivationBinding,
    pub config: ModuleEnvelope,
    pub target: TargetIdentity,
    pub mode: ActivationMode,
    pub decision: ConfiguredDecision,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ConfiguredSet {
    origin: ConfigurationOrigin,
    #[cfg(windows)]
    #[serde(skip)]
    native: Option<NativeConfigurationProof>,
    revision: u64,
    entries: Vec<ConfiguredEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    continuations: Vec<ConfiguredEntry>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ConfiguredSnapshot {
    #[default]
    Unavailable,
    Complete(Arc<ConfiguredSet>),
}
pub(crate) trait ConfiguredIntentSource: Send + Sync {
    fn snapshot(&self) -> ConfiguredSnapshot;
}
pub(crate) struct UnavailableConfiguredIntent;
impl ConfiguredIntentSource for UnavailableConfiguredIntent {
    fn snapshot(&self) -> ConfiguredSnapshot {
        ConfiguredSnapshot::Unavailable
    }
}
impl ConfiguredSnapshot {
    pub(crate) fn continuation_for_origin(
        &self,
        expected: &ConfigurationOrigin,
        owner: OwnerId,
        binding: &ActivationBinding,
        config: &ModuleEnvelope,
        target: &TargetIdentity,
        mode: ActivationMode,
    ) -> IntentDecision {
        let Self::Complete(set) = self else {
            return IntentDecision::Unknown;
        };
        if &set.origin == expected
            && set.continuations.iter().any(|entry| {
                entry.owner == owner
                    && &entry.binding == binding
                    && &entry.config == config
                    && &entry.target == target
                    && entry.mode == mode
            })
        {
            IntentDecision::Enabled
        } else {
            IntentDecision::Unknown
        }
    }
    #[cfg(test)]
    pub(crate) fn complete(
        revision: u64,
        entries: Vec<ConfiguredEntry>,
    ) -> Result<Self, &'static str> {
        Self::build_set(
            ConfigurationOrigin::fixture(),
            revision,
            entries,
            vec![],
            #[cfg(windows)]
            None,
        )
    }
    fn build_set(
        origin: ConfigurationOrigin,
        revision: u64,
        mut entries: Vec<ConfiguredEntry>,
        mut continuations: Vec<ConfiguredEntry>,
        #[cfg(windows)] native: Option<NativeConfigurationProof>,
    ) -> Result<Self, &'static str> {
        use super::codec::EnvelopeKind;
        if revision == 0 || entries.len() > MAX_CONFIGURED_OWNERS {
            return Err("invalid configured snapshot bounds");
        }
        let mut owners = std::collections::BTreeSet::new();
        for entry in &entries {
            if !owners.insert(entry.owner)
                || entry.binding.configured().is_none()
                || !entry.binding.permits(entry.mode, &entry.target)
                || entry.config.kind != EnvelopeKind::Config
            {
                return Err("invalid configured entry");
            }
            entry
                .config
                .to_bytes()
                .map_err(|_| "invalid configured envelope")?;
        }
        entries.sort_by_key(|entry| entry.owner);
        let mut old_owners = std::collections::BTreeSet::new();
        for old in &continuations {
            if !old_owners.insert(old.owner)
                || old.decision != ConfiguredDecision::Enabled
                || !entries.iter().any(|entry| {
                    entry.owner == old.owner && entry.decision == ConfiguredDecision::Enabled
                })
            {
                return Err("invalid configured continuation");
            }
            old.config
                .to_bytes()
                .map_err(|_| "invalid configured continuation")?;
        }
        continuations.sort_by_key(|entry| entry.owner);
        let set = ConfiguredSet {
            origin,
            #[cfg(windows)]
            native,
            revision,
            entries,
            continuations,
        };
        struct Budget(usize);
        impl std::io::Write for Budget {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self
                    .0
                    .checked_add(bytes.len())
                    .filter(|count| *count <= MAX_CONFIGURED_BYTES)
                    .ok_or_else(|| std::io::Error::other("configured snapshot bytes exceeded"))?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        // Measurement only: do not allocate a second copy or persist producer
        // input. All dynamic metadata, not just config envelopes, is bounded.
        serde_json::to_writer(&mut Budget(0), &set)
            .map_err(|_| "configured snapshot bytes exceeded")?;
        Ok(Self::Complete(Arc::new(set)))
    }
    #[cfg(windows)]
    pub(in crate::app::features::tuning) fn from_saved_native(
        proof: NativeConfigurationProof,
        catalog: &super::catalog::Catalog,
    ) -> Self {
        // No caller tuple/config/origin is accepted: all derive from the same
        // sealed receipt bytes that created the private proof.
        Self::saved_set(
            proof.saved_bytes(),
            proof.origin().clone(),
            catalog,
            Some(proof.clone()),
        )
        .unwrap_or_default()
    }
    #[cfg(test)]
    pub(in crate::app::features::tuning) fn from_saved_fixture(
        bytes: &[u8],
        origin: ConfigurationOrigin,
        catalog: &super::catalog::Catalog,
    ) -> Self {
        Self::saved_set(
            bytes,
            origin,
            catalog,
            #[cfg(windows)]
            None,
        )
        .unwrap_or_default()
    }
    fn saved_set(
        bytes: &[u8],
        origin: ConfigurationOrigin,
        catalog: &super::catalog::Catalog,
        #[cfg(windows)] native: Option<NativeConfigurationProof>,
    ) -> Option<Self> {
        let config = saved_configuration(bytes)?;
        if !origin.matches_lineage(&config.lineage)
            || !config
                .activations
                .iter()
                .all(|entry| config.request_matches(entry))
        {
            return None;
        }
        let entry = |intent: &super::super::config::SavedIntent, decision| ConfiguredEntry {
            owner: intent.owner,
            binding: intent.binding.clone(),
            config: intent.config.clone(),
            target: intent.target.clone(),
            mode: intent.mode,
            decision,
        };
        let mut entries = config
            .activations
            .iter()
            .map(|intent| entry(intent, ConfiguredDecision::Enabled))
            .chain(
                config
                    .withdrawals
                    .iter()
                    .map(|intent| entry(intent, ConfiguredDecision::Disabled)),
            )
            .collect::<Vec<_>>();
        let mut continuations = Vec::new();
        for transition in config.transitions.as_deref().unwrap_or_default() {
            if !config.request_matches(&transition.new) {
                return None;
            }
            entries.push(entry(&transition.new, ConfiguredDecision::Enabled));
            continuations.push(entry(&transition.old, ConfiguredDecision::Enabled));
        }
        // Unknown inactive preferences are retained but are not execution input.
        // Every relevant tuple must validate; none is silently discarded.
        for intent in entries.iter().chain(&continuations) {
            if intent
                .config
                .target
                .as_ref()
                .is_some_and(|target| target != &intent.target)
                || catalog.validate_config(&intent.config).is_err()
            {
                return None;
            }
        }
        Self::build_set(
            origin,
            config.allocations.next_config_generation,
            entries,
            continuations,
            #[cfg(windows)]
            native,
        )
        .ok()
    }
    fn decision(
        &self,
        owner: OwnerId,
        binding: &ActivationBinding,
        config: &ModuleEnvelope,
        target: &TargetIdentity,
        mode: ActivationMode,
    ) -> IntentDecision {
        let Self::Complete(set) = self else {
            return IntentDecision::Unknown;
        };
        match set.entries.iter().find(|entry| {
            entry.owner == owner
                && &entry.binding == binding
                && &entry.config == config
                && &entry.target == target
                && entry.mode == mode
        }) {
            Some(entry) => match entry.decision {
                ConfiguredDecision::Enabled => IntentDecision::Enabled,
                ConfiguredDecision::Disabled => IntentDecision::Disabled,
            },
            None => IntentDecision::Unknown,
        }
    }
    pub(crate) fn decision_for_origin(
        &self,
        expected: &ConfigurationOrigin,
        owner: OwnerId,
        binding: &ActivationBinding,
        config: &ModuleEnvelope,
        target: &TargetIdentity,
        mode: ActivationMode,
    ) -> IntentDecision {
        if self.origin() != Some(expected) {
            return IntentDecision::Unknown;
        }
        self.decision(owner, binding, config, target, mode)
    }
    pub(crate) fn origin(&self) -> Option<&ConfigurationOrigin> {
        match self {
            Self::Complete(set) => Some(&set.origin),
            Self::Unavailable => None,
        }
    }
    #[cfg(windows)]
    pub(super) fn native_proof(&self) -> Option<&NativeConfigurationProof> {
        match self {
            Self::Complete(set) => set.native.as_ref(),
            Self::Unavailable => None,
        }
    }
    #[cfg(test)]
    pub(crate) fn complete_fixture(
        origin: ConfigurationOrigin,
        revision: u64,
        entries: Vec<ConfiguredEntry>,
    ) -> Result<Self, &'static str> {
        Self::build_set(
            origin,
            revision,
            entries,
            vec![],
            #[cfg(windows)]
            None,
        )
    }
}

/// The declaration is generic and independent of module ID. Hydration/catalog
/// checks remain responsible for codec and exact source/claim mapping.
pub(super) fn persistent_machine_resources(
    descriptor: &ModuleDescriptor,
    target: &TargetIdentity,
    keys: &[ResourceKey],
    machine: &MachineIdentity,
) -> bool {
    target == &TargetIdentity::Machine(machine.clone())
        && descriptor.access == ModuleAccess::Managed
        && descriptor
            .activation_modes
            .contains(&ActivationMode::Manual)
        && !keys.is_empty()
        && keys.len() <= 64
        && keys.iter().collect::<std::collections::BTreeSet<_>>().len() == keys.len()
        && keys.iter().all(|key| {
            key.target == *target
                && descriptor.resources.iter().any(|declared| {
                    declared.field == key.field
                        && declared.scope == TargetScope::Machine
                        && declared.lifetime == ResourceLifetimeRequirement::PersistentMachine
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::features::tuning::testing::scalar::{self, ScalarConfig, ScalarModule};
    fn machine() -> MachineIdentity {
        MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        }
    }
    fn entry(decision: ConfiguredDecision) -> ConfiguredEntry {
        let module = ScalarModule::new(7);
        ConfiguredEntry {
            owner: OwnerId::new(1).unwrap(),
            binding: ActivationBinding::Manual(ConfiguredIntent {
                profile: ProfileId::new(1).unwrap(),
                generation: ConfigGeneration::new(1).unwrap(),
            }),
            config: module.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
            target: TargetIdentity::Machine(machine()),
            mode: ActivationMode::Manual,
            decision,
        }
    }
    fn decide(snapshot: &ConfiguredSnapshot, e: &ConfiguredEntry) -> IntentDecision {
        snapshot.decision(e.owner, &e.binding, &e.config, &e.target, e.mode)
    }
    #[test]
    fn p2a_configured_budget_counts_both_old_and_new_metadata() {
        let mut e = entry(ConfiguredDecision::Enabled);
        e.config.payload = serde_json::json!("x".repeat(200_000));
        let entries = (1..=11)
            .map(|id| {
                let mut e = e.clone();
                e.owner = OwnerId::new(id).unwrap();
                e
            })
            .collect::<Vec<_>>();
        assert!(ConfiguredSnapshot::build_set(
            ConfigurationOrigin::fixture(),
            2,
            entries.clone(),
            vec![],
            #[cfg(windows)]
            None
        )
        .is_ok());
        assert!(ConfiguredSnapshot::build_set(
            ConfigurationOrigin::fixture(),
            2,
            entries.clone(),
            entries,
            #[cfg(windows)]
            None
        )
        .is_err());
    }
    #[test]
    fn p1b_configured_origin_precedes_identical_numeric_owner_tuple() {
        let e = entry(ConfiguredDecision::Enabled);
        let snapshot = ConfiguredSnapshot::complete(1, vec![e.clone()]).unwrap();
        let origin = ConfigurationOrigin::fixture();
        for field in [
            "lineage",
            "principal",
            "volume_guid",
            "volume_serial",
            "directory",
        ] {
            assert_eq!(
                snapshot.decision_for_origin(
                    &origin.fixture_changed(field),
                    e.owner,
                    &e.binding,
                    &e.config,
                    &e.target,
                    e.mode
                ),
                IntentDecision::Unknown,
                "{field}"
            );
        }
    }
    #[test]
    fn configured_consumer_exact_enabled_and_disabled_are_distinct_from_unknown() {
        for (decision, expected) in [
            (ConfiguredDecision::Enabled, IntentDecision::Enabled),
            (ConfiguredDecision::Disabled, IntentDecision::Disabled),
        ] {
            let e = entry(decision);
            let snapshot = ConfiguredSnapshot::complete(1, vec![e.clone()]).unwrap();
            assert_eq!(decide(&snapshot, &e), expected);
            assert_eq!(
                decide(&ConfiguredSnapshot::Unavailable, &e),
                IntentDecision::Unknown
            );
            assert_eq!(
                decide(&ConfiguredSnapshot::complete(2, vec![]).unwrap(), &e),
                IntentDecision::Unknown
            );
        }
    }
    #[test]
    fn p1b_saved_configuration_raw_cannot_authorize_an_in_memory_draft_or_byte_disagreement() {
        let saved = r#"{"version":1,"lineage":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"profiles":[],"associations":[],"allocations":{"next_profile":1,"next_owner":1,"next_config_generation":1},"activations":[],"withdrawals":[]}"#;
        let parsed: super::super::super::config::TuningEnvelope =
            serde_json::from_str(saved).unwrap();
        assert!(saved_tuning_matches(saved, &parsed));
        let mut draft: serde_json::Value = serde_json::from_str(saved).unwrap();
        draft["allocations"]["next_owner"] = serde_json::json!(2);
        let draft = serde_json::from_value(draft).unwrap();
        assert!(!saved_tuning_matches(saved, &draft));
        assert!(!saved_tuning_matches(saved, &Default::default()));
        let whitespace = format!(" {saved} ");
        assert!(!saved_tuning_matches(&whitespace, &parsed));
    }
    #[test]
    fn configured_consumer_changed_tuple_never_becomes_disabled_evidence() {
        let e = entry(ConfiguredDecision::Disabled);
        let snapshot = ConfiguredSnapshot::complete(1, vec![e.clone()]).unwrap();
        let mut variants = Vec::new();
        let mut v = e.clone();
        v.owner = OwnerId::new(2).unwrap();
        variants.push(v);
        let mut v = e.clone();
        v.binding = ActivationBinding::Manual(ConfiguredIntent {
            profile: ProfileId::new(1).unwrap(),
            generation: ConfigGeneration::new(2).unwrap(),
        });
        variants.push(v);
        let mut v = e.clone();
        v.config.payload = serde_json::json!({"value":13});
        variants.push(v);
        let mut v = e.clone();
        v.target = TargetIdentity::Machine(MachineIdentity {
            id: MachineId::new([2; 16]).unwrap(),
        });
        variants.push(v);
        let mut v = e.clone();
        v.mode = ActivationMode::Session;
        variants.push(v);
        for v in variants {
            assert_eq!(decide(&snapshot, &v), IntentDecision::Unknown);
        }
    }
    #[test]
    fn configured_consumer_rejects_partial_duplicate_and_unbounded_input() {
        let e = entry(ConfiguredDecision::Enabled);
        assert!(ConfiguredSnapshot::complete(0, vec![e.clone()]).is_err());
        assert!(ConfiguredSnapshot::complete(1, vec![e.clone(), e.clone()]).is_err());
        assert!(
            ConfiguredSnapshot::complete(1, vec![e.clone(); MAX_CONFIGURED_OWNERS + 1]).is_err()
        );
        let mut malformed = e.clone();
        malformed.config.kind = super::super::codec::EnvelopeKind::Snapshot;
        assert!(ConfiguredSnapshot::complete(1, vec![malformed]).is_err());
        let mut missing = e;
        missing.binding = ActivationBinding::Unknown;
        assert!(ConfiguredSnapshot::complete(1, vec![missing]).is_err());
    }
    #[test]
    fn configured_consumer_snapshot_is_immutable_and_revision_is_part_of_change_detection() {
        let e = entry(ConfiguredDecision::Enabled);
        let old = ConfiguredSnapshot::complete(1, vec![e.clone()]).unwrap();
        let mut producer_entries = vec![e.clone()];
        let captured = ConfiguredSnapshot::complete(1, producer_entries.clone()).unwrap();
        producer_entries[0].decision = ConfiguredDecision::Disabled;
        assert_eq!(captured, old);
        assert_ne!(captured, ConfiguredSnapshot::complete(2, vec![e]).unwrap());
        assert_ne!(
            captured,
            ConfiguredSnapshot::complete(1, producer_entries).unwrap()
        );
    }
    #[test]
    fn configured_consumer_aggregate_budget_includes_binding_and_target_metadata() {
        use crate::app::shared::ids::{GroupId, RuleId};
        let mut e = entry(ConfiguredDecision::Enabled);
        let session = SessionIdentity {
            user: UserIdentity {
                machine: machine(),
                sid: UserSid::new("S-1-5-21-1").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(1).unwrap(),
            boot: BootId::new([1; 16]).unwrap(),
        };
        let image = ImagePath::new(format!("C:\\{}.exe", "a".repeat(30000))).unwrap();
        let processes = (1..=2)
            .map(|pid| ProcessIdentity::new(session.clone(), pid, 1, image.clone()).unwrap())
            .collect();
        e.binding = ActivationBinding::Game(
            GameActivation::new(
                e.binding.configured().unwrap(),
                RuleActivation::new(GroupId("g".into()), RuleId("r".into())).unwrap(),
                ActivationEligibility::Running,
                session,
                processes,
            )
            .unwrap(),
        );
        e.mode = ActivationMode::Session;
        let entries = (1..=70)
            .map(|id| {
                let mut e = e.clone();
                e.owner = OwnerId::new(id).unwrap();
                e
            })
            .collect();
        assert!(
            ConfiguredSnapshot::complete(1, entries).is_err(),
            "complete metadata exceeds aggregate input budget"
        );
    }
    #[test]
    fn configured_consumer_machine_projection_does_not_supply_persistent_lifetime() {
        let mut descriptor = scalar::descriptor();
        let target = TargetIdentity::Machine(machine());
        let key = ResourceKey {
            field: descriptor.resources[0].field.clone(),
            target: target.clone(),
        };
        assert!(!persistent_machine_resources(
            &descriptor,
            &target,
            std::slice::from_ref(&key),
            &machine()
        ));
        descriptor.resources[0].lifetime = ResourceLifetimeRequirement::PersistentMachine;
        assert!(persistent_machine_resources(
            &descriptor,
            &target,
            std::slice::from_ref(&key),
            &machine()
        ));
        let mut foreign = key.clone();
        foreign.field = ResourceId::new("foreign").unwrap();
        assert!(!persistent_machine_resources(
            &descriptor,
            &target,
            &[foreign],
            &machine()
        ));
        assert!(!persistent_machine_resources(
            &descriptor,
            &target,
            &[key],
            &MachineIdentity {
                id: MachineId::new([2; 16]).unwrap()
            }
        ));
    }
    #[test]
    fn configured_consumer_lifetime_is_not_a_second_resource_identity() {
        let mut descriptor = scalar::descriptor();
        let mut alias = descriptor.resources[0].clone();
        alias.lifetime = ResourceLifetimeRequirement::PersistentMachine;
        descriptor.resources.push(alias);
        let mut catalog = super::super::catalog::Catalog::default();
        assert!(
            catalog
                .register(ScalarModule::with_descriptor(descriptor))
                .is_err(),
            "one resource cannot carry contradictory declarations"
        );
    }
}
