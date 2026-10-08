//! JSON exists only at this bounded persistence boundary, never in runtime plans.
use super::contract::{FeatureId, SchemaVersion, TargetIdentity};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::marker::PhantomData;

const MAX_BYTES: usize = 256 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_ITEMS: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvelopeKind {
    Config,
    Snapshot,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleEnvelope {
    pub feature: FeatureId,
    pub kind: EnvelopeKind,
    pub version: SchemaVersion,
    pub target: Option<TargetIdentity>,
    pub payload: Value,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    TooLarge,
    TooDeep,
    TooManyItems,
    InvalidData,
    WrongFeature,
    WrongKind,
    WrongTarget,
    UnsupportedVersion {
        expected: SchemaVersion,
        actual: SchemaVersion,
    },
}

fn check_tree(value: &Value, depth: usize, remaining: &mut usize) -> Result<(), CodecError> {
    if depth > MAX_DEPTH {
        return Err(CodecError::TooDeep);
    }
    *remaining = remaining.checked_sub(1).ok_or(CodecError::TooManyItems)?;
    match value {
        Value::Array(values) => {
            for child in values {
                check_tree(child, depth + 1, remaining)?;
            }
        }
        Value::Object(values) => {
            for child in values.values() {
                check_tree(child, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}
impl ModuleEnvelope {
    #[cfg(test)]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() > MAX_BYTES {
            return Err(CodecError::TooLarge);
        }
        let envelope: Self = serde_json::from_slice(bytes).map_err(|_| CodecError::InvalidData)?;
        envelope.check()?;
        Ok(envelope)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, CodecError> {
        self.check()?;
        let bytes = serde_json::to_vec(self).map_err(|_| CodecError::InvalidData)?;
        if bytes.len() > MAX_BYTES {
            return Err(CodecError::TooLarge);
        }
        Ok(bytes)
    }
    fn check(&self) -> Result<(), CodecError> {
        if (self.kind == EnvelopeKind::Config) != self.target.is_none() {
            return Err(CodecError::WrongTarget);
        }
        let mut remaining = MAX_ITEMS;
        check_tree(&self.payload, 0, &mut remaining)
    }
}

pub trait VersionedCodec<T>: Send + Sync {
    fn encode(
        &self,
        value: &T,
        target: Option<&TargetIdentity>,
    ) -> Result<ModuleEnvelope, CodecError>;
    fn decode(&self, envelope: &ModuleEnvelope) -> Result<T, CodecError>;
}

pub struct JsonCodec<T> {
    feature: FeatureId,
    kind: EnvelopeKind,
    version: SchemaVersion,
    marker: PhantomData<fn() -> T>,
}
impl<T> JsonCodec<T> {
    pub fn new(feature: FeatureId, kind: EnvelopeKind, version: SchemaVersion) -> Self {
        Self {
            feature,
            kind,
            version,
            marker: PhantomData,
        }
    }
}
impl<T: Serialize + DeserializeOwned> VersionedCodec<T> for JsonCodec<T> {
    fn encode(
        &self,
        value: &T,
        target: Option<&TargetIdentity>,
    ) -> Result<ModuleEnvelope, CodecError> {
        let envelope = ModuleEnvelope {
            feature: self.feature.clone(),
            kind: self.kind,
            version: self.version,
            target: target.cloned(),
            payload: serde_json::to_value(value).map_err(|_| CodecError::InvalidData)?,
        };
        envelope.to_bytes()?;
        Ok(envelope)
    }
    fn decode(&self, envelope: &ModuleEnvelope) -> Result<T, CodecError> {
        envelope.to_bytes()?;
        if envelope.feature != self.feature {
            return Err(CodecError::WrongFeature);
        }
        if envelope.kind != self.kind {
            return Err(CodecError::WrongKind);
        }
        if envelope.version != self.version {
            return Err(CodecError::UnsupportedVersion {
                expected: self.version,
                actual: envelope.version,
            });
        }
        serde_json::from_value(envelope.payload.clone()).map_err(|_| CodecError::InvalidData)
    }
}
