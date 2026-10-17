//! One registration supplies the descriptor, codec/editor, observation and field operations.
use super::{
    codec::{EnvelopeKind, ModuleEnvelope},
    contract::*,
};
use crate::app::features::system_tweaks::presentation::{ModuleAction, ModuleEditor};
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
    /// Stable resource identity; session and process targets remain exact by default.
    fn canonical_target(&self, target: &TargetIdentity) -> Result<TargetIdentity, ModuleError> {
        Ok(target.clone())
    }
    /// True only after native evidence proves this temporary resource's session ended.
    /// Unavailable context or lifetime reads must remain errors, not expiry.
    fn target_has_expired(&self, _target: &TargetIdentity) -> Result<bool, ModuleError> {
        Ok(false)
    }
    fn accepts_target(&self, target: &TargetIdentity) -> bool {
        let Ok(target) = self.canonical_target(target) else {
            return false;
        };
        self.targets().iter().any(|candidate| {
            self.canonical_target(candidate)
                .is_ok_and(|candidate| candidate == target)
        })
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation;
    /// One compact card refresh; adapters can reuse one native read for both results.
    fn observe_tweak(
        &self,
        target: &TargetIdentity,
        saved: Option<&ModuleEnvelope>,
    ) -> (ModuleObservation, Result<bool, ModuleError>) {
        (
            self.observe(target),
            self.editor().tweak_ready_for_config(target, saved),
        )
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError>;
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError>;
    /// Compare and write on the same validated native handle. Empty bytes may encode absence.
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError>;
    /// Some stored settings need a guarded runtime refresh even when bytes already match.
    /// Called only for retained ownership, before recovery/Undo evidence is retired.
    fn confirm_applied(&self, _key: &ResourceKey, _expected: &[u8]) -> Result<(), ModuleError> {
        Ok(())
    }
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
