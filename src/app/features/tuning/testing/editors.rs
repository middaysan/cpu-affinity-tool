//! Test-only presentation adapters. No fixture is registered in shipping builds.
use super::{
    scalar::{ScalarConfig, ScalarModule},
    user_setting::{UserConfig, UserModule},
};
use crate::app::features::tuning::{
    platform::{catalog::Module, codec::ModuleEnvelope, contract::FeatureId},
    presentation::{EditorRegistry, ModuleEditor},
};
use eframe::egui;
use std::sync::Arc;

struct ScalarEditor(ScalarModule);
impl ModuleEditor for ScalarEditor {
    fn feature(&self) -> FeatureId {
        self.0.descriptor().id
    }
    fn default_config(&self) -> ModuleEnvelope {
        self.0.config_envelope(&ScalarConfig { value: 12 }).unwrap()
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        self.0
            .config_codec()
            .decode(config)
            .is_ok_and(|value| self.0.validate_config(&value).is_ok())
    }
    fn show(&self, ui: &mut egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        let mut value = self.0.config_codec().decode(config).map_err(|_| ())?;
        let changed = ui
            .horizontal(|ui| {
                ui.label("Value");
                ui.add(egui::DragValue::new(&mut value.value)).changed()
            })
            .inner;
        if changed {
            *config = self
                .0
                .config_codec()
                .encode(&value, config.target.as_ref())
                .map_err(|_| ())?;
        }
        Ok(changed)
    }
}

struct UserEditor(UserModule);
impl ModuleEditor for UserEditor {
    fn feature(&self) -> FeatureId {
        self.0.descriptor().id
    }
    fn default_config(&self) -> ModuleEnvelope {
        self.0
            .config_envelope(&UserConfig {
                value: 12,
                enabled: true,
            })
            .unwrap()
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        self.0
            .config_codec()
            .decode(config)
            .is_ok_and(|value| self.0.validate_config(&value).is_ok())
    }
    fn show(&self, ui: &mut egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        let mut value = self.0.config_codec().decode(config).map_err(|_| ())?;
        let mut changed = ui.checkbox(&mut value.enabled, "Setting enabled").changed();
        changed |= ui
            .horizontal(|ui| {
                ui.label("Value");
                ui.add(egui::DragValue::new(&mut value.value)).changed()
            })
            .inner;
        if changed {
            *config = self
                .0
                .config_codec()
                .encode(&value, config.target.as_ref())
                .map_err(|_| ())?;
        }
        Ok(changed)
    }
}

pub(in crate::app) fn registry(scalar: ScalarModule, user: UserModule) -> EditorRegistry {
    let mut editors = EditorRegistry::default();
    editors.register(Arc::new(ScalarEditor(scalar))).unwrap();
    editors.register(Arc::new(UserEditor(user))).unwrap();
    editors
}

#[test]
fn p5_module_owned_editors_keep_heterogeneous_configs_typed_and_forward_configs_readonly() {
    let scalar = ScalarModule::new(7);
    let user = UserModule::new(None, false);
    let editors = registry(scalar.clone(), user.clone());
    for descriptor in [scalar.descriptor(), user.descriptor()] {
        let editor = editors.get(&descriptor.id).unwrap();
        let config = editor.default_config();
        assert!(editor.accepts_config(&config));
        let mut forward = config.clone();
        forward.version =
            crate::app::features::tuning::platform::contract::SchemaVersion::new(2).unwrap();
        assert!(!editor.accepts_config(&forward));
        let mut invalid = config;
        invalid.payload = serde_json::json!({ "unexpected": true });
        assert!(!editor.accepts_config(&invalid));
    }
    assert_eq!(
        scalar.reads(),
        0,
        "presentation must not inspect native state"
    );
    assert_eq!(scalar.writes(), 0);
    assert_eq!(user.reads(), 0);
    assert_eq!(user.writes(), 0);
}
