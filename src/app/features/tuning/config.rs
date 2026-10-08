//! Bounded configuration codec. Raw forward data supplies no execution authority.
use super::platform::{codec::ModuleEnvelope, contract::*};
use crate::app::shared::ids::{GroupId, RuleId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;
use std::sync::Arc;

pub(crate) const MAX_TUNING_CONFIG_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROFILES: usize = 64;
const MAX_MODULES: usize = 64;
const MAX_ASSOCIATIONS: usize = 256;
const MAX_INTENTS: usize = 256;
const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 65_536;

#[derive(Clone, Default)]
pub(crate) struct TuningEnvelope(Option<Arc<RawValue>>);
impl std::fmt::Debug for TuningEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningEnvelope(redacted)")
    }
}
impl Serialize for TuningEnvelope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_deref().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for TuningEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Box::<RawValue>::deserialize(deserializer).map(|raw| Self(Some(Arc::from(raw))))
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TuningLoad {
    Off,
    /// Persistence shape only; module codecs and qualified sources decide admission.
    Known(Box<TuningConfig>),
    Unknown,
    Blocked,
}
impl TuningEnvelope {
    pub(crate) fn is_off(&self) -> bool {
        self.0.is_none()
    }
    pub(crate) fn raw(&self) -> Option<&str> {
        self.0.as_deref().map(RawValue::get)
    }
    pub(crate) fn decode(&self) -> TuningLoad {
        let Some(raw) = self.raw() else {
            return TuningLoad::Off;
        };
        if raw.len() > MAX_TUNING_CONFIG_BYTES || bounded_tree(raw).is_err() {
            return TuningLoad::Blocked;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return TuningLoad::Blocked;
        };
        match forward_shape(&value) {
            Err(()) => TuningLoad::Blocked,
            Ok(true) => TuningLoad::Unknown,
            Ok(false) => match serde_json::from_value::<TuningConfig>(value) {
                Ok(config) if config.valid() => TuningLoad::Known(Box::new(config)),
                _ => TuningLoad::Blocked,
            },
        }
    }
}

/// Count before building typed vectors or payload Values. Unknown content is
/// bounded too; it does not bypass the codec by using a future version.
fn bounded_tree(raw: &str) -> Result<(), serde_json::Error> {
    use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
    struct Seed<'a> {
        remaining: &'a mut usize,
        depth: usize,
    }
    impl<'de> DeserializeSeed<'de> for Seed<'_> {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
            if self.depth > MAX_DEPTH || *self.remaining == 0 {
                return Err(serde::de::Error::custom("tuning tree bounds"));
            }
            *self.remaining -= 1;
            d.deserialize_any(self)
        }
    }
    impl<'de> Visitor<'de> for Seed<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded JSON")
        }
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
            Ok(())
        }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
            Ok(())
        }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
            Ok(())
        }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
            Ok(())
        }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
            Ok(())
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
            Ok(())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
            while a
                .next_element_seed(Seed {
                    remaining: self.remaining,
                    depth: self.depth + 1,
                })?
                .is_some()
            {}
            Ok(())
        }
        fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
            let mut keys = std::collections::BTreeSet::new();
            while let Some(key) = a.next_key::<String>()? {
                if *self.remaining == 0 {
                    return Err(serde::de::Error::custom("tuning tree bounds"));
                }
                if !keys.insert(key) {
                    return Err(serde::de::Error::custom("duplicate tuning field"));
                }
                *self.remaining -= 1;
                a.next_value_seed(Seed {
                    remaining: self.remaining,
                    depth: self.depth + 1,
                })?;
            }
            Ok(())
        }
    }
    let mut remaining = MAX_NODES;
    let mut decoder = serde_json::Deserializer::from_str(raw);
    Seed {
        remaining: &mut remaining,
        depth: 0,
    }
    .deserialize(&mut decoder)?;
    decoder.end()
}
fn fields_forward(v: &serde_json::Value, allowed: &[&str]) -> Result<bool, ()> {
    let object = v.as_object().ok_or(())?;
    Ok(object.keys().any(|key| !allowed.contains(&key.as_str())))
}
fn version_forward(v: &serde_json::Value) -> Result<bool, ()> {
    Ok(v.get("version")
        .and_then(serde_json::Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or(())?
        != 1)
}

/// Validate the understood module envelope independently of payload support.
/// A forward format with no understood version remains opaque and read-only.
fn module_shape(v: &serde_json::Value) -> Result<Option<ModuleEnvelope>, ()> {
    use super::platform::codec::EnvelopeKind;
    let fields = ["feature", "kind", "version", "target", "payload"];
    let forward = fields_forward(v, &fields)?;
    let Some(version) = v.get("version") else {
        return if forward { Ok(None) } else { Err(()) };
    };
    let version = version
        .as_u64()
        .and_then(|version| u32::try_from(version).ok())
        .and_then(|version| SchemaVersion::new(version).ok())
        .ok_or(())?;
    let feature = serde_json::from_value(v.get("feature").ok_or(())?.clone()).map_err(|_| ())?;
    let kind = serde_json::from_value(v.get("kind").ok_or(())?.clone()).map_err(|_| ())?;
    let target = match v.get("target") {
        None | Some(serde_json::Value::Null) => None,
        Some(target) => Some(serde_json::from_value(target.clone()).map_err(|_| ())?),
    };
    let envelope = ModuleEnvelope {
        feature,
        kind,
        version,
        target,
        payload: v.get("payload").ok_or(())?.clone(),
    };
    if envelope.kind != EnvelopeKind::Config || envelope.to_bytes().is_err() {
        return Err(());
    }
    Ok(Some(envelope))
}
fn forward_shape(v: &serde_json::Value) -> Result<bool, ()> {
    let version = v
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .filter(|n| *n > 0)
        .ok_or(())?;
    if !matches!(version, 1 | 2) {
        return Ok(true);
    }
    let v2 = version == 2;
    let mut forward = fields_forward(
        v,
        if v2 {
            &[
                "version",
                "lineage",
                "profiles",
                "associations",
                "allocations",
                "activations",
                "withdrawals",
                "transitions",
                "acknowledgements",
            ][..]
        } else {
            &[
                "version",
                "lineage",
                "profiles",
                "associations",
                "allocations",
                "activations",
                "withdrawals",
            ][..]
        },
    )?;
    // Core known-envelope metadata remains required, including when extensions
    // are preserved. Unsupported nested records cannot be executed.
    let lineage = v
        .get("lineage")
        .and_then(serde_json::Value::as_array)
        .ok_or(())?;
    if lineage.len() != 32
        || lineage.iter().any(|n| n.as_u64().is_none_or(|n| n > 255))
        || lineage.iter().all(|n| n.as_u64() == Some(0))
    {
        return Err(());
    }
    let allocations = v.get("allocations").ok_or(())?;
    let counter = |name| {
        allocations
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or(())
    };
    let next_profile = counter("next_profile")?;
    let next_owner = counter("next_owner")?;
    let next_generation = counter("next_config_generation")?;
    let mut profiles = std::collections::BTreeSet::new();
    let mut associations = std::collections::BTreeSet::new();
    let mut owners = std::collections::BTreeSet::new();
    let mut opaque_profiles = false;
    let mut max_profile = 0;
    let mut max_owner = 0;
    let mut max_generation = 0;
    for (name, limit) in [
        ("profiles", MAX_PROFILES),
        ("associations", MAX_ASSOCIATIONS),
        ("activations", MAX_INTENTS),
        ("withdrawals", MAX_INTENTS),
    ] {
        let list = v
            .get(name)
            .and_then(serde_json::Value::as_array)
            .ok_or(())?;
        if list.len() > limit {
            return Err(());
        }
        for record in list {
            if version_forward(record)? {
                forward = true;
                opaque_profiles |= name == "profiles";
                continue;
            }
            let fields = match name {
                "profiles" => &["version", "id", "name", "modules"][..],
                "associations" => &["version", "group", "rule", "profile", "eligibility"][..],
                _ => &["version", "owner", "binding", "config", "target", "mode"][..],
            };
            forward |= fields_forward(record, fields)?;
            if name == "profiles" {
                let id = record
                    .get("id")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|id| *id > 0)
                    .ok_or(())?;
                if !profiles.insert(id)
                    || !record
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|name| max_text(name, 128))
                {
                    return Err(());
                }
                max_profile = max_profile.max(id);
                let modules = record
                    .get("modules")
                    .and_then(serde_json::Value::as_array)
                    .ok_or(())?;
                if modules.len() > MAX_MODULES {
                    return Err(());
                }
                let mut features = std::collections::BTreeSet::new();
                for module in modules {
                    forward |= fields_forward(module, &["feature", "enabled", "mode", "config"])?;
                    let feature: FeatureId =
                        serde_json::from_value(module.get("feature").ok_or(())?.clone())
                            .map_err(|_| ())?;
                    let _: ActivationMode =
                        serde_json::from_value(module.get("mode").ok_or(())?.clone())
                            .map_err(|_| ())?;
                    if !features.insert(feature.clone())
                        || module
                            .get("enabled")
                            .is_some_and(|enabled| enabled.as_bool().is_none())
                    {
                        return Err(());
                    }
                    let config = module.get("config").ok_or(())?;
                    forward |= fields_forward(
                        config,
                        &["feature", "kind", "version", "target", "payload"],
                    )?;
                    if module_shape(config)?.is_some_and(|config| config.feature != feature) {
                        return Err(());
                    }
                }
            } else if name == "associations" {
                let group = record
                    .get("group")
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| max_text(text, 128))
                    .ok_or(())?;
                let rule = record
                    .get("rule")
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| max_text(text, 128))
                    .ok_or(())?;
                let profile = record
                    .get("profile")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|id| *id > 0)
                    .ok_or(())?;
                let _: ActivationEligibility =
                    serde_json::from_value(record.get("eligibility").ok_or(())?.clone())
                        .map_err(|_| ())?;
                if !associations.insert((group, rule))
                    || (!opaque_profiles && !profiles.contains(&profile))
                {
                    return Err(());
                }
                max_profile = max_profile.max(profile);
            } else {
                let shape = known_intent_shape(record)?;
                let owner = shape.owner.get();
                let configured = shape.configured;
                forward |= shape.forward;
                if !owners.insert(owner)
                    || (name == "activations"
                        && !opaque_profiles
                        && !profiles.contains(&configured.profile.get()))
                {
                    return Err(());
                }
                max_profile = max_profile.max(configured.profile.get());
                max_owner = max_owner.max(owner);
                max_generation = max_generation.max(configured.generation.get());
            }
        }
    }
    let active = v["activations"].as_array().ok_or(())?.len();
    let withdrawn = v["withdrawals"].as_array().ok_or(())?.len();
    let transitions = if v2 {
        v.get("transitions")
            .and_then(serde_json::Value::as_array)
            .ok_or(())?
            .as_slice()
    } else {
        &[]
    };
    if active
        .checked_add(withdrawn)
        .and_then(|n| n.checked_add(transitions.len()))
        .is_none_or(|n| n > MAX_INTENTS)
    {
        return Err(());
    }
    for record in transitions {
        if version_forward(record)? {
            forward = true;
            continue;
        }
        forward |= fields_forward(record, &["version", "owner", "generation", "old", "new"])?;
        let owner = record
            .get("owner")
            .and_then(serde_json::Value::as_u64)
            .filter(|n| *n > 0)
            .ok_or(())?;
        let generation = record
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            .filter(|n| *n > 0)
            .ok_or(())?;
        if !owners.insert(owner) {
            return Err(());
        }
        max_owner = max_owner.max(owner);
        max_generation = max_generation.max(generation);
        let mut tuples = Vec::with_capacity(2);
        for (name, intent) in [
            ("old", record.get("old").ok_or(())?),
            ("new", record.get("new").ok_or(())?),
        ] {
            if version_forward(intent)? {
                forward = true;
                tuples.push(None);
                continue;
            }
            let shape = known_intent_shape(intent)?;
            forward |= shape.forward;
            if shape.owner.get() != owner
                || (name == "new"
                    && (shape.configured.generation.get() != generation
                        || (!opaque_profiles
                            && !profiles.contains(&shape.configured.profile.get()))))
            {
                return Err(());
            }
            max_profile = max_profile.max(shape.configured.profile.get());
            max_owner = max_owner.max(shape.owner.get());
            max_generation = max_generation.max(shape.configured.generation.get());
            tuples.push(Some(shape));
        }
        if let (Some(old), Some(new)) = (tuples[0].take(), tuples[1].take()) {
            // Understood actor/generation metadata validates even when a module
            // shape is opaque; Unknown cannot erase these constraints.
            if !same_actor(&old.binding, &new.binding)
                || old.target != new.target
                || old.mode != new.mode
                || new.configured.generation <= old.configured.generation
            {
                return Err(());
            }
            if let (Some(old), Some(new)) = (old.intent, new.intent) {
                let transition = SavedTransition {
                    version: 1,
                    owner: OwnerId::new(owner).map_err(|_| ())?,
                    generation: ConfigGeneration::new(generation).map_err(|_| ())?,
                    old,
                    new,
                };
                if !transition.valid() {
                    return Err(());
                }
            }
        }
    }
    if v2 && [next_profile, next_owner, next_generation].contains(&u64::MAX) {
        return Err(());
    }
    if v2 {
        let mut ack_owners = std::collections::BTreeSet::new();
        let acknowledgements = match v.get("acknowledgements") {
            None => &[][..],
            Some(value) => value.as_array().ok_or(())?.as_slice(),
        };
        if acknowledgements.len() > MAX_INTENTS {
            return Err(());
        }
        for raw in acknowledgements {
            if version_forward(raw)? {
                forward = true;
                continue;
            }
            forward |= fields_forward(
                raw,
                &["version", "owner", "generation", "journal_revision", "kind"],
            )?;
            // Understood identity/counter metadata is validated even if fields
            // are forward. This wire is bookkeeping, never worker authority.
            let owner = raw
                .get("owner")
                .and_then(serde_json::Value::as_u64)
                .filter(|n| *n > 0)
                .ok_or(())?;
            let generation = raw
                .get("generation")
                .and_then(serde_json::Value::as_u64)
                .filter(|n| *n > 0)
                .ok_or(())?;
            if !ack_owners.insert(owner)
                || raw
                    .get("journal_revision")
                    .and_then(serde_json::Value::as_u64)
                    .is_none_or(|n| n == 0)
            {
                return Err(());
            }
            let kind: AckKind =
                serde_json::from_value(raw.get("kind").ok_or(())?.clone()).map_err(|_| ())?;
            let record = match kind {
                AckKind::Updated => transitions
                    .iter()
                    .find(|t| t.get("owner").and_then(serde_json::Value::as_u64) == Some(owner))
                    .and_then(|t| t.get("new")),
                AckKind::Withdrawn => v
                    .get("withdrawals")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|list| {
                        list.iter().find(|t| {
                            t.get("owner").and_then(serde_json::Value::as_u64) == Some(owner)
                        })
                    }),
            }
            .ok_or(())?;
            if version_forward(record)? {
                forward = true;
                continue;
            }
            if known_intent_shape(record)?.configured.generation.get() != generation {
                return Err(());
            }
        }
    }
    if next_profile <= max_profile || next_owner <= max_owner || next_generation <= max_generation {
        return Err(());
    }
    forward |= fields_forward(
        v.get("allocations").ok_or(())?,
        &["next_profile", "next_owner", "next_config_generation"],
    )?;
    Ok(forward)
}
struct KnownIntentShape {
    forward: bool,
    owner: OwnerId,
    configured: ConfiguredIntent,
    binding: ActivationBinding,
    target: TargetIdentity,
    mode: ActivationMode,
    intent: Option<SavedIntent>,
}
fn known_intent_shape(v: &serde_json::Value) -> Result<KnownIntentShape, ()> {
    let mut forward = fields_forward(
        v,
        &["version", "owner", "binding", "config", "target", "mode"],
    )?;
    let owner = serde_json::from_value(v.get("owner").ok_or(())?.clone()).map_err(|_| ())?;
    let binding: ActivationBinding =
        serde_json::from_value(v.get("binding").ok_or(())?.clone()).map_err(|_| ())?;
    let target = serde_json::from_value(v.get("target").ok_or(())?.clone()).map_err(|_| ())?;
    let mode = serde_json::from_value(v.get("mode").ok_or(())?.clone()).map_err(|_| ())?;
    let configured = binding.configured().ok_or(())?;
    if !binding.permits(mode, &target) {
        return Err(());
    }
    let raw_config = v.get("config").ok_or(())?;
    forward |= fields_forward(
        raw_config,
        &["feature", "kind", "version", "target", "payload"],
    )?;
    let intent = module_shape(raw_config)?.map(|config| SavedIntent {
        version: 1,
        owner,
        binding: binding.clone(),
        config,
        target: target.clone(),
        mode,
    });
    Ok(KnownIntentShape {
        forward,
        owner,
        configured,
        binding,
        target,
        mode,
        intent,
    })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TuningConfig {
    pub version: u32,
    pub lineage: [u8; 32],
    pub profiles: Vec<Profile>,
    pub associations: Vec<Association>,
    pub allocations: Allocations,
    pub activations: Vec<SavedIntent>,
    pub withdrawals: Vec<SavedIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transitions: Option<Vec<SavedTransition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledgements: Option<Vec<SavedAcknowledgement>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AckKind {
    Updated,
    Withdrawn,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedAcknowledgement {
    pub version: u32,
    pub owner: OwnerId,
    pub generation: ConfigGeneration,
    pub journal_revision: u64,
    pub kind: AckKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    pub version: u32,
    pub id: ProfileId,
    pub name: String,
    pub modules: Vec<ModulePreference>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModulePreference {
    pub feature: FeatureId,
    #[serde(default)]
    pub enabled: bool,
    pub mode: ActivationMode,
    pub config: ModuleEnvelope,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Association {
    pub version: u32,
    pub group: GroupId,
    pub rule: RuleId,
    pub profile: ProfileId,
    pub eligibility: ActivationEligibility,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Allocations {
    pub next_profile: u64,
    pub next_owner: u64,
    pub next_config_generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedIntent {
    pub version: u32,
    pub owner: OwnerId,
    pub binding: ActivationBinding,
    pub config: ModuleEnvelope,
    pub target: TargetIdentity,
    pub mode: ActivationMode,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedTransition {
    pub version: u32,
    pub owner: OwnerId,
    pub generation: ConfigGeneration,
    pub old: SavedIntent,
    pub new: SavedIntent,
}
impl SavedTransition {
    fn valid(&self) -> bool {
        let (Some(old), Some(new)) = (self.old.binding.configured(), self.new.binding.configured())
        else {
            return false;
        };
        self.version == 1
            && self.old.version == 1
            && self.new.version == 1
            && self.owner == self.old.owner
            && self.owner == self.new.owner
            && self.generation == new.generation
            && new.generation > old.generation
            && same_actor(&self.old.binding, &self.new.binding)
            && self.old.target == self.new.target
            && self.old.mode == self.new.mode
            && self.old.config.feature == self.new.config.feature
            && self.old.binding.permits(self.old.mode, &self.old.target)
            && self.new.binding.permits(self.new.mode, &self.new.target)
            && [&self.old, &self.new].iter().all(|intent| {
                intent
                    .config
                    .target
                    .as_ref()
                    .is_none_or(|target| target == &intent.target)
            })
    }
}
fn same_actor(old: &ActivationBinding, new: &ActivationBinding) -> bool {
    match (old, new) {
        (ActivationBinding::Manual(old), ActivationBinding::Manual(new)) => {
            old.profile == new.profile
        }
        (ActivationBinding::Game(old), ActivationBinding::Game(new)) => {
            old.configured().profile == new.configured().profile
                && old.rule() == new.rule()
                && old.eligibility() == new.eligibility()
                && old.session() == new.session()
                && old.processes() == new.processes()
        }
        _ => false,
    }
}
impl TuningConfig {
    pub(in crate::app::features::tuning) fn request_matches(&self, intent: &SavedIntent) -> bool {
        let Some(binding) = intent.binding.configured() else {
            return false;
        };
        let preference = self
            .profiles
            .iter()
            .find(|p| p.id == binding.profile)
            .and_then(|p| {
                p.modules
                    .iter()
                    .find(|m| m.feature == intent.config.feature)
            });
        preference.is_some_and(|m| m.enabled && m.mode == intent.mode && m.config == intent.config)
            && intent
                .config
                .target
                .as_ref()
                .is_none_or(|target| target == &intent.target)
            && match &intent.binding {
                ActivationBinding::Manual(_) => true,
                ActivationBinding::Game(game) => self.associations.iter().any(|a| {
                    &a.group == game.rule().group()
                        && &a.rule == game.rule().rule()
                        && a.profile == binding.profile
                        && a.eligibility == game.eligibility()
                }),
                ActivationBinding::Unknown => false,
            }
    }
    fn valid(&self) -> bool {
        use super::platform::codec::EnvelopeKind;
        use std::collections::BTreeSet;
        let text = |s: &str| max_text(s, 128);
        let mut profiles = BTreeSet::new();
        let mut associations = BTreeSet::new();
        let mut owners = BTreeSet::new();
        let mut max_profile = 0;
        let mut max_owner = 0;
        let mut max_generation = 0;
        if (self.version == 1 && (self.transitions.is_some() || self.acknowledgements.is_some()))
            || (self.version == 2 && self.transitions.is_none())
        {
            return false;
        }
        for profile in &self.profiles {
            if !profiles.insert(profile.id) || !text(&profile.name) {
                return false;
            }
            max_profile = max_profile.max(profile.id.get());
            let mut features = BTreeSet::new();
            for module in &profile.modules {
                if !features.insert(&module.feature)
                    || module.feature != module.config.feature
                    || module.config.kind != EnvelopeKind::Config
                    || module.config.to_bytes().is_err()
                {
                    return false;
                }
            }
        }
        for association in &self.associations {
            if !profiles.contains(&association.profile)
                || !text(&association.group.0)
                || !text(&association.rule.0)
                || !associations.insert((&association.group.0, &association.rule.0))
            {
                return false;
            }
        }
        for intent in self.activations.iter().chain(&self.withdrawals) {
            if !owners.insert(intent.owner)
                || !intent.binding.permits(intent.mode, &intent.target)
                || intent.config.kind != EnvelopeKind::Config
                || intent.config.to_bytes().is_err()
            {
                return false;
            }
            let Some(binding) = intent.binding.configured() else {
                return false;
            };
            max_profile = max_profile.max(binding.profile.get());
            max_owner = max_owner.max(intent.owner.get());
            max_generation = max_generation.max(binding.generation.get());
        }
        if let Some(transitions) = &self.transitions {
            for transition in transitions {
                if !owners.insert(transition.owner)
                    || !transition.valid()
                    || !self.request_matches(&transition.new)
                {
                    return false;
                }
                for intent in [&transition.old, &transition.new] {
                    let Some(binding) = intent.binding.configured() else {
                        return false;
                    };
                    max_profile = max_profile.max(binding.profile.get());
                    max_owner = max_owner.max(intent.owner.get());
                    max_generation = max_generation.max(binding.generation.get());
                }
            }
            if !self
                .activations
                .iter()
                .all(|intent| self.request_matches(intent))
            {
                return false;
            }
        }
        self.activations.iter().all(|intent| {
            intent
                .binding
                .configured()
                .is_some_and(|binding| profiles.contains(&binding.profile))
        }) && self.allocations.next_profile > max_profile
            && self.allocations.next_owner > max_owner
            && self.allocations.next_config_generation > max_generation
    }
}
fn max_text(text: &str, limit: usize) -> bool {
    !text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    pub(super) fn known() -> Value {
        json!({"version":1,"lineage":vec![1;32],"profiles":[],"associations":[],"allocations":{"next_profile":1,"next_owner":1,"next_config_generation":1},"activations":[],"withdrawals":[]})
    }
    fn decode(value: Value) -> TuningLoad {
        serde_json::from_value::<TuningEnvelope>(value)
            .unwrap()
            .decode()
    }
    #[test]
    fn p2b_saved_ack_metadata_is_bounded_unique_and_matches_exact_pending_record() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut value = known();
        value["version"] = json!(2);
        value["transitions"] = json!([]);
        value["allocations"] = json!({"next_profile":2,"next_owner":2,"next_config_generation":2});
        value["withdrawals"] = json!([{"version":1,"owner":1,"binding":ActivationBinding::Manual(ConfiguredIntent {profile:ProfileId::new(1).unwrap(),generation:ConfigGeneration::new(1).unwrap()}),"config":module.config_envelope(&ScalarConfig {value:12}).unwrap(),"target":TargetIdentity::Machine(MachineIdentity {id:MachineId::new([1;16]).unwrap()}),"mode":"Manual"}]);
        let ack =
            json!({"version":1,"owner":1,"generation":1,"journal_revision":7,"kind":"Withdrawn"});
        value["acknowledgements"] = json!([ack]);
        assert!(matches!(decode(value.clone()), TuningLoad::Known(_)));
        for (field, bad) in [
            ("generation", json!(2)),
            ("generation", json!(0)),
            ("journal_revision", json!(0)),
            ("kind", json!("Updated")),
            ("owner", json!(2)),
        ] {
            let mut changed = value.clone();
            changed["acknowledgements"][0][field] = bad;
            assert_eq!(decode(changed), TuningLoad::Blocked, "{field}");
        }
        let mut duplicates = value.clone();
        duplicates["acknowledgements"] = json!([ack.clone(), ack.clone()]);
        assert_eq!(decode(duplicates), TuningLoad::Blocked);
        let mut too_many = value.clone();
        too_many["acknowledgements"] = json!(vec![ack; 257]);
        assert_eq!(decode(too_many), TuningLoad::Blocked);
        value["acknowledgements"][0]["future"] = json!(true);
        let raw = serde_json::to_string(&value).unwrap();
        let envelope: TuningEnvelope = serde_json::from_str(&raw).unwrap();
        assert_eq!(envelope.decode(), TuningLoad::Unknown);
        assert_eq!(envelope.raw(), Some(raw.as_str()));
    }
    #[test]
    fn p1_config_off_is_only_absence_and_known_round_trip_retains_lineage() {
        assert_eq!(TuningEnvelope::default().decode(), TuningLoad::Off);
        let raw = serde_json::to_string(&known()).unwrap();
        let envelope: TuningEnvelope = serde_json::from_str(&raw).unwrap();
        let TuningLoad::Known(config) = envelope.decode() else {
            panic!("known envelope");
        };
        assert_eq!(serde_json::to_value(config).unwrap(), known());
        assert_eq!(envelope.raw(), Some(raw.as_str()));
        assert_eq!(decode(Value::Null), TuningLoad::Blocked);
    }
    #[test]
    fn p1_config_unknown_versions_and_fields_preserve_exact_raw_without_execution() {
        let mut valid = serde_json::to_string(&known()).unwrap();
        valid.pop();
        valid.push_str(",\"future\":{\"n\":1e+2}}");
        for raw in ["{\"version\":99,\"future\":1e+2}", valid.as_str()] {
            let envelope: TuningEnvelope = serde_json::from_str(raw).unwrap();
            assert_eq!(envelope.decode(), TuningLoad::Unknown);
            assert_eq!(serde_json::to_string(&envelope).unwrap(), raw);
        }
        let mut v = known();
        v["profiles"] = json!([{"version":2,"future":true}]);
        assert_eq!(decode(v), TuningLoad::Unknown);
    }
    #[test]
    fn p1_config_malformed_or_zero_lineage_never_becomes_off() {
        for value in [json!(true), json!([]), json!({}), json!({"version":1})] {
            assert_eq!(decode(value), TuningLoad::Blocked);
        }
        let mut value = known();
        value["lineage"] = json!(vec![0; 32]);
        assert_eq!(decode(value), TuningLoad::Blocked);
    }
    #[test]
    fn p1_config_aggregate_depth_and_count_bounds_precede_typed_decode() {
        let raw = format!(
            "{{\"version\":99,\"future\":\"{}\"}}",
            "x".repeat(MAX_TUNING_CONFIG_BYTES)
        );
        assert_eq!(
            serde_json::from_str::<TuningEnvelope>(&raw)
                .unwrap()
                .decode(),
            TuningLoad::Blocked
        );
        let mut v = json!(0);
        for _ in 0..34 {
            v = json!([v]);
        }
        let mut root = known();
        root["future"] = v;
        assert_eq!(decode(root), TuningLoad::Blocked);
        let mut root = known();
        root["future"] = json!(vec![0; MAX_NODES]);
        assert_eq!(decode(root), TuningLoad::Blocked);
    }
    fn profile(id: u64) -> Value {
        json!({"version":1,"id":id,"name":"Profile","modules":[]})
    }
    #[test]
    fn p1_config_duplicate_references_and_allocation_rewind_are_blocked() {
        let mut v = known();
        v["profiles"] = json!([profile(1)]);
        v["allocations"]["next_profile"] = json!(2);
        assert!(matches!(decode(v.clone()), TuningLoad::Known(_)));
        let mut duplicate = v.clone();
        duplicate["profiles"] = json!([profile(1), profile(1)]);
        assert_eq!(decode(duplicate), TuningLoad::Blocked);
        let mut rewind = v.clone();
        rewind["allocations"]["next_profile"] = json!(1);
        assert_eq!(decode(rewind), TuningLoad::Blocked);
        let mut dangling = v;
        dangling["associations"] = json!([{"version":1,"group":"group","rule":"rule","profile":2,"eligibility":"Running"}]);
        assert_eq!(decode(dangling), TuningLoad::Blocked);
        let mut excessive = known();
        excessive["profiles"] = json!((1..=65).map(profile).collect::<Vec<_>>());
        assert_eq!(decode(excessive), TuningLoad::Blocked);
    }
    #[test]
    fn p1_config_module_forward_payload_and_profile_extensions_are_preserved() {
        for change in [
            json!({"version":2,"future_profile":1e2}),
            json!({"version":1,"id":1,"name":"P","modules":[],"future_profile":true}),
        ] {
            let mut v = known();
            v["allocations"]["next_profile"] = json!(2);
            v["profiles"] = json!([change]);
            assert_eq!(decode(v), TuningLoad::Unknown);
        }
        let mut v = known();
        v["allocations"]["next_profile"] = json!(2);
        v["profiles"] = json!([{"version":1,"id":1,"name":"P","modules":[{"feature":"future_module","mode":"Manual","config":{"future_version":5,"payload":[1e2]}}]}]);
        assert_eq!(decode(v), TuningLoad::Unknown);
    }

    #[test]
    fn p1_config_module_codec_owns_payload_version_support_after_shape_decode() {
        use super::super::platform::codec::{CodecError, EnvelopeKind, JsonCodec, VersionedCodec};
        let feature = FeatureId::new("fixture_versioned_module").unwrap();
        let codec = JsonCodec::<u32>::new(
            feature.clone(),
            EnvelopeKind::Config,
            SchemaVersion::new(2).unwrap(),
        );
        let encoded = codec.encode(&12, None).unwrap();
        let mut value = known();
        value["allocations"]["next_profile"] = json!(2);
        value["profiles"] = json!([{
            "version": 1, "id": 1, "name": "Profile", "modules": [{
                "feature": feature, "enabled": true, "mode": "Manual", "config": encoded
            }]
        }]);
        let raw = serde_json::to_string(&value).unwrap();
        let envelope: TuningEnvelope = serde_json::from_str(&raw).unwrap();
        let TuningLoad::Known(config) = envelope.decode() else {
            panic!("module schema version belongs to its codec");
        };
        let stored = &config.profiles[0].modules[0].config;
        assert_eq!(codec.decode(stored), Ok(12));
        assert_eq!(serde_json::to_string(&envelope).unwrap(), raw);
        let previous = JsonCodec::<u32>::new(
            feature,
            EnvelopeKind::Config,
            SchemaVersion::new(1).unwrap(),
        );
        assert!(matches!(
            previous.decode(stored),
            Err(CodecError::UnsupportedVersion { .. })
        ));

        value["profiles"][0]["modules"][0]["config"]["payload"] = json!("unknown payload");
        let TuningLoad::Known(config) = decode(value.clone()) else {
            panic!("valid persistence shape does not require payload support");
        };
        assert_eq!(
            codec.decode(&config.profiles[0].modules[0].config),
            Err(CodecError::InvalidData)
        );
        value["profiles"][0]["modules"][0]["config"]["version"] = json!(0);
        assert_eq!(decode(value.clone()), TuningLoad::Blocked);
        value["profiles"][0]["modules"][0]["config"]["version"] = json!(2);
        value["profiles"][0]["modules"][0]["config"]["payload"] = json!("x".repeat(256 * 1024));
        assert_eq!(decode(value), TuningLoad::Blocked);
    }

    #[test]
    fn p1_config_forward_fields_do_not_bypass_understood_metadata_validation() {
        for field in ["next_profile", "next_owner", "next_config_generation"] {
            for invalid in [Some(json!(0)), Some(json!("1")), Some(Value::Null), None] {
                let mut value = known();
                value["future"] = json!(true);
                match invalid {
                    Some(invalid) => value["allocations"][field] = invalid,
                    None => {
                        value["allocations"].as_object_mut().unwrap().remove(field);
                    }
                }
                assert_eq!(decode(value), TuningLoad::Blocked);
            }
        }
        for invalid_profile in [
            json!({"version":1,"id":0,"name":"Profile","modules":[]}),
            json!({"version":1,"id":1,"name":"","modules":[]}),
        ] {
            let mut value = known();
            value["future"] = json!(true);
            value["allocations"]["next_profile"] = json!(2);
            value["profiles"] = json!([invalid_profile]);
            assert_eq!(decode(value), TuningLoad::Blocked);
        }
        let mut rewind = known();
        rewind["future"] = json!(true);
        rewind["profiles"] = json!([profile(1)]);
        assert_eq!(decode(rewind), TuningLoad::Blocked);
        let mut duplicate = known();
        duplicate["future"] = json!(true);
        duplicate["allocations"]["next_profile"] = json!(2);
        duplicate["profiles"] = json!([profile(1), profile(1)]);
        assert_eq!(decode(duplicate), TuningLoad::Blocked);
        let mut dangling = known();
        dangling["future"] = json!(true);
        dangling["associations"] =
            json!([{"version":1,"group":"g","rule":"r","profile":1,"eligibility":"Running"}]);
        assert_eq!(decode(dangling), TuningLoad::Blocked);
        let mut opaque = known();
        opaque["profiles"] = json!([{"version":2,"future_profile":true}]);
        assert_eq!(decode(opaque), TuningLoad::Unknown);
    }

    #[test]
    fn p1_config_known_association_bounds_allocation_with_opaque_profiles() {
        let mut value = known();
        value["profiles"] = json!([{"version":2,"future_profile":true}]);
        value["associations"] =
            json!([{"version":1,"group":"g","rule":"r","profile":999,"eligibility":"Running"}]);
        for next_profile in [1, 998, 999, 1000] {
            value["allocations"]["next_profile"] = json!(next_profile);
            let raw = serde_json::to_string(&value).unwrap();
            let envelope: TuningEnvelope = serde_json::from_str(&raw).unwrap();
            assert_eq!(
                envelope.decode(),
                if next_profile > 999 {
                    TuningLoad::Unknown
                } else {
                    TuningLoad::Blocked
                }
            );
            assert_eq!(envelope.raw(), Some(raw.as_str()));
            assert_eq!(serde_json::to_string(&envelope).unwrap(), raw);
        }
    }

    #[test]
    fn p1_config_duplicate_keys_are_blocked_before_value_collapse() {
        let base = serde_json::to_string(&known()).unwrap();
        let valid_lineage = serde_json::to_string(&vec![1; 32]).unwrap();
        let zero_lineage = serde_json::to_string(&vec![0; 32]).unwrap();
        for raw in [
            base.replace(
                "\"next_profile\":1",
                "\"next_profile\":0,\"next_profile\":1",
            ),
            base.replace("\"version\":1", "\"version\":99,\"version\":1"),
            base.replace(
                &format!("\"lineage\":{valid_lineage}"),
                &format!("\"lineage\":{zero_lineage},\"lineage\":{valid_lineage}"),
            ),
            base.replace(
                "\"profiles\":[]",
                "\"profiles\":[{\"version\":1,\"id\":0,\"id\":1,\"name\":\"P\",\"modules\":[]}]",
            )
            .replace("\"next_profile\":1", "\"next_profile\":2"),
            "{\"version\":99,\"future\":{\"k\":0,\"\\u006b\":1}}".to_owned(),
        ] {
            let envelope: TuningEnvelope = serde_json::from_str(&raw).unwrap();
            assert_eq!(envelope.decode(), TuningLoad::Blocked);
            assert_eq!(envelope.raw(), Some(raw.as_str()));
        }
    }
}
