//! Bounded saved profiles. Unknown and legacy recovery documents retain exact raw bytes.
use super::platform::{codec::ModuleEnvelope, contract::*};
use crate::app::shared::ids::{GroupId, RuleId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{value::RawValue, Value};
use std::{collections::BTreeSet, sync::Arc};
pub(crate) const MAX_TUNING_CONFIG_BYTES: usize = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 65536;
#[derive(Clone, Default)]
pub(crate) struct TuningEnvelope(Option<Arc<RawValue>>);
impl std::fmt::Debug for TuningEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningEnvelope")
    }
}
impl Serialize for TuningEnvelope {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.as_deref().serialize(s)
    }
}
impl<'de> Deserialize<'de> for TuningEnvelope {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Box::<RawValue>::deserialize(d).map(|raw| Self(Some(Arc::from(raw))))
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TuningLoad {
    Off,
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
    pub(crate) fn from_config(config: &TuningConfig) -> Result<Self, ()> {
        if !config.valid() {
            return Err(());
        }
        let text = serde_json::to_string(config).map_err(|_| ())?;
        if text.len() > MAX_TUNING_CONFIG_BYTES || bounded_tree(&text).is_err() {
            return Err(());
        }
        RawValue::from_string(text)
            .map(|raw| Self(Some(Arc::from(raw))))
            .map_err(|_| ())
    }
    pub(crate) fn decode(&self) -> TuningLoad {
        let Some(raw) = self.raw() else {
            return TuningLoad::Off;
        };
        if raw.len() > MAX_TUNING_CONFIG_BYTES || bounded_tree(raw).is_err() {
            return TuningLoad::Blocked;
        }
        let Ok(mut value) = serde_json::from_str::<Value>(raw) else {
            return TuningLoad::Blocked;
        };
        let Some(version) = value
            .get("version")
            .and_then(Value::as_u64)
            .filter(|v| *v > 0)
        else {
            return TuningLoad::Blocked;
        };
        if !matches!(version, 1..=3) {
            return TuningLoad::Unknown;
        }
        let allowed = if version == 3 {
            &[
                "version",
                "profiles",
                "associations",
                "next_profile",
                "manual_targets",
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
                "transitions",
                "acknowledgements",
            ][..]
        };
        let Some(object) = value.as_object_mut() else {
            return TuningLoad::Blocked;
        };
        if object.keys().any(|k| !allowed.contains(&k.as_str())) {
            return TuningLoad::Unknown;
        }
        if version != 3 {
            if !object
                .get("lineage")
                .and_then(Value::as_array)
                .is_some_and(|v| {
                    v.len() == 32
                        && v.iter().all(|n| n.as_u64().is_some_and(|n| n <= 255))
                        && v.iter().any(|n| n.as_u64() != Some(0))
                })
            {
                return TuningLoad::Blocked;
            }
            for field in ["activations", "withdrawals"] {
                let Some(items) = object.get(field).and_then(Value::as_array) else {
                    return TuningLoad::Blocked;
                };
                if !items.is_empty() {
                    return TuningLoad::Unknown;
                }
            }
            for field in ["transitions", "acknowledgements"] {
                if object.get(field).is_some_and(|v| !v.is_array()) {
                    return TuningLoad::Blocked;
                }
                if object
                    .get(field)
                    .and_then(Value::as_array)
                    .is_some_and(|v| !v.is_empty())
                {
                    return TuningLoad::Unknown;
                }
            }
            let Some(allocation) = object.get("allocations").and_then(Value::as_object) else {
                return TuningLoad::Blocked;
            };
            if allocation.keys().any(|k| {
                !["next_profile", "next_owner", "next_config_generation"].contains(&k.as_str())
            }) {
                return TuningLoad::Unknown;
            }
            if ["next_profile", "next_owner", "next_config_generation"]
                .iter()
                .any(|k| {
                    !allocation
                        .get(*k)
                        .and_then(Value::as_u64)
                        .is_some_and(|n| n > 0)
                })
            {
                return TuningLoad::Blocked;
            }
            let next = allocation["next_profile"].clone();
            for field in [
                "lineage",
                "allocations",
                "activations",
                "withdrawals",
                "transitions",
                "acknowledgements",
            ] {
                object.remove(field);
            }
            object.insert("version".into(), Value::from(3));
            object.insert("next_profile".into(), next);
        }
        if nested_forward(&value) {
            return TuningLoad::Unknown;
        }
        match serde_json::from_value::<TuningConfig>(value) {
            Ok(config) if config.valid() => TuningLoad::Known(Box::new(config)),
            _ => TuningLoad::Blocked,
        }
    }
}
fn nested_forward(value: &Value) -> bool {
    fn fields(value: &Value, allowed: &[&str]) -> bool {
        value
            .as_object()
            .is_some_and(|object| object.keys().any(|key| !allowed.contains(&key.as_str())))
    }
    let version = |record: &Value| {
        record
            .get("version")
            .and_then(Value::as_u64)
            .is_some_and(|version| version > 1)
    };
    value
        .get("profiles")
        .and_then(Value::as_array)
        .is_some_and(|profiles| {
            profiles.iter().any(|profile| {
                fields(profile, &["version", "id", "name", "modules"])
                    || version(profile)
                    || profile
                        .get("modules")
                        .and_then(Value::as_array)
                        .is_some_and(|modules| {
                            modules.iter().any(|module| {
                                fields(module, &["feature", "enabled", "mode", "config"])
                                    || module.get("config").is_some_and(|config| {
                                        fields(
                                            config,
                                            &["feature", "kind", "version", "target", "payload"],
                                        )
                                    })
                            })
                        })
            })
        })
        || value
            .get("associations")
            .and_then(Value::as_array)
            .is_some_and(|associations| {
                associations.iter().any(|association| {
                    fields(
                        association,
                        &["version", "group", "rule", "profile", "eligibility"],
                    ) || version(association)
                })
            })
        || value
            .get("manual_targets")
            .and_then(Value::as_array)
            .is_some_and(|targets| {
                targets
                    .iter()
                    .any(|target| fields(target, &["profile", "feature", "target"]))
            })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TuningConfig {
    pub version: u32,
    pub profiles: Vec<Profile>,
    pub associations: Vec<Association>,
    pub next_profile: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manual_targets: Vec<ManualTarget>,
}
impl Default for TuningConfig {
    fn default() -> Self {
        Self {
            version: 3,
            profiles: vec![],
            associations: vec![],
            next_profile: 1,
            manual_targets: vec![],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManualTarget {
    pub profile: ProfileId,
    pub feature: FeatureId,
    pub target: TargetIdentity,
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
impl TuningConfig {
    fn valid(&self) -> bool {
        if self.version != 3
            || self.next_profile == 0
            || self.profiles.len() > 64
            || self.associations.len() > 256
            || self.manual_targets.len() > 256
        {
            return false;
        }
        let mut profiles = BTreeSet::new();
        for profile in &self.profiles {
            if profile.version != 1
                || !profiles.insert(profile.id)
                || profile.id.get() >= self.next_profile
                || profile.name.trim().is_empty()
                || profile.name.len() > 128
                || profile.name.chars().any(char::is_control)
                || profile.modules.len() > 64
            {
                return false;
            }
            let mut modules = BTreeSet::new();
            for module in &profile.modules {
                if !modules.insert(&module.feature)
                    || module.feature != module.config.feature
                    || module.config.kind != super::platform::codec::EnvelopeKind::Config
                    || module.config.target.is_some()
                    || module.config.to_bytes().is_err()
                {
                    return false;
                }
            }
        }
        let mut targets = BTreeSet::new();
        if !self.manual_targets.iter().all(|target| {
            target.target.valid()
                && targets.insert((target.profile, &target.feature))
                && self.profiles.iter().any(|profile| {
                    profile.id == target.profile
                        && profile.modules.iter().any(|module| {
                            module.feature == target.feature
                                && module.mode == ActivationMode::Manual
                        })
                })
        }) {
            return false;
        }
        let mut rules = BTreeSet::new();
        self.associations.iter().all(|a| {
            a.version == 1
                && profiles.contains(&a.profile)
                && !a.group.0.is_empty()
                && a.group.0.len() <= 128
                && !a.group.0.chars().any(char::is_control)
                && !a.rule.0.is_empty()
                && a.rule.0.len() <= 128
                && !a.rule.0.chars().any(char::is_control)
                && rules.insert((&a.group.0, &a.rule.0))
        })
    }
}
pub(super) fn bounded_tree(raw: &str) -> Result<(), serde_json::Error> {
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
