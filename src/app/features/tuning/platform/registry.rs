//! One registration supplies the descriptor, codec/editor, observation and field operations.
use super::{
    codec::{EnvelopeKind, ModuleEnvelope},
    contract::*,
};
use crate::app::features::tuning::presentation::{ModuleAction, ModuleEditor};
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Debug)]
pub struct FieldChange {
    pub key: ResourceKey,
    pub desired: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct ModuleObservation {
    pub capability: Capability,
    pub summary: ObservationSummary,
    pub effect: Effect,
}
pub trait Module: Send + Sync {
    fn descriptor(&self) -> ModuleDescriptor;
    fn editor(&self) -> &dyn ModuleEditor;
    /// Process targets are supplied by the worker from verified tracked instances.
    fn targets(&self) -> Vec<TargetIdentity>;
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation;
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError>;
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError>;
    /// Compare and write on the same validated native handle. Empty bytes may encode absence.
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError>;
    fn action(&self, action: ModuleAction) -> Result<(), ModuleError> {
        match action {
            ModuleAction::Refresh => Ok(()),
            _ => Err(ModuleError::new(ErrorKind::Unsupported, "action")),
        }
    }
}
#[derive(Clone, Default)]
pub struct Registry {
    modules: BTreeMap<FeatureId, Arc<dyn Module>>,
}
impl Registry {
    pub fn register(&mut self, module: Arc<dyn Module>) -> Result<(), ModuleError> {
        let descriptor = module.descriptor();
        let editor = module.editor();
        let config = editor.default_config();
        if self.modules.len() >= 64
            || self.modules.contains_key(&descriptor.id)
            || editor.feature() != descriptor.id
            || config.feature != descriptor.id
            || config.kind != EnvelopeKind::Config
            || config.version != descriptor.config_version
            || config.to_bytes().is_err()
            || !editor.accepts_config(&config)
        {
            return Err(ModuleError::new(ErrorKind::InvalidConfig, "register"));
        }
        self.modules.insert(descriptor.id, module);
        Ok(())
    }
    pub fn get(&self, id: &FeatureId) -> Option<&Arc<dyn Module>> {
        self.modules.get(id)
    }
    pub fn descriptors(&self) -> impl Iterator<Item = ModuleDescriptor> + '_ {
        self.modules.values().map(|m| m.descriptor())
    }
    pub(crate) fn editor(&self, id: &FeatureId) -> Option<&dyn ModuleEditor> {
        self.get(id).map(|m| m.editor())
    }
    pub fn validate_config(&self, config: &ModuleEnvelope) -> Result<(), ModuleError> {
        let module = self
            .get(&config.feature)
            .ok_or_else(|| ModuleError::new(ErrorKind::Unsupported, "config"))?;
        if config.kind != EnvelopeKind::Config
            || config.version != module.descriptor().config_version
            || config.to_bytes().is_err()
            || !module.editor().accepts_config(config)
        {
            return Err(ModuleError::new(ErrorKind::InvalidConfig, "config"));
        }
        Ok(())
    }
}
