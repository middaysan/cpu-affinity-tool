//! Pure module-card decisions shared by the presenter and state-level tests.
use super::platform::{
    codec::ModuleEnvelope,
    contract::{Effect, FeatureId, ModuleError, ModuleStatus, ResourceId, TargetIdentity},
};
#[cfg(test)]
use super::{
    config::ModulePreference,
    platform::contract::{ActivationMode, Capability, ModuleAccess, ProfileId},
};

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CardControls {
    pub configure: bool,
    pub enable: bool,
    pub disable: bool,
    pub apply: bool,
}

#[cfg(test)]
pub(crate) fn card_controls(
    access: ModuleAccess,
    capability: &Capability,
    config_supported: bool,
    saved_enabled: bool,
    mode_supported: bool,
    busy: bool,
) -> CardControls {
    let editable = access == ModuleAccess::Managed && config_supported && !busy;
    CardControls {
        configure: editable,
        enable: editable && mode_supported,
        disable: editable && saved_enabled,
        apply: editable && mode_supported && matches!(capability, Capability::Available),
    }
}

pub(crate) fn status_label(status: ModuleStatus) -> &'static str {
    match status {
        ModuleStatus::SessionUnverified => "Session could not be verified",
        ModuleStatus::Unmanaged => "Unmanaged",
        ModuleStatus::Unsupported => "Unavailable",
        ModuleStatus::PermissionDenied => "Permission denied",
        ModuleStatus::Applying => "Applying",
        ModuleStatus::VerifiedActive(Effect::Immediate) => "Verified active",
        ModuleStatus::VerifiedActive(Effect::GameRestartRequired) => {
            "Saved · game restart required"
        }
        ModuleStatus::VerifiedActive(Effect::RestartRequired)
        | ModuleStatus::StoredRestartRequired => "Saved · restart required",
        ModuleStatus::PartialFailure => "Partially applied · check recovery",
        ModuleStatus::Restoring => "Restoring original state",
        ModuleStatus::ExternalChange => "Changed outside this app",
        ModuleStatus::RecoveryRequired => "Recovery required",
    }
}

/// Each module validates its typed configuration and supplies its own tweak
/// metadata. The compact presenter does not decode native payloads.
pub(crate) trait ModuleEditor: Send + Sync {
    fn platform_support(&self) -> super::platforms::PlatformSupport {
        super::platforms::WIN10_11
    }
    fn availability(&self) -> super::platforms::TweakAvailability {
        self.platform_support()
            .for_platform(os_api::platform::current_platform())
    }
    fn tweak(&self) -> Option<TweakSpec> {
        None
    }
    fn tweak_config(&self, _target: &TargetIdentity) -> Result<ModuleEnvelope, ModuleError> {
        Ok(self.default_config())
    }
    /// True only when the read-back value already matches the recommended setting.
    fn tweak_ready(&self, _target: &TargetIdentity) -> Result<bool, ModuleError> {
        Ok(false)
    }
    /// Optional module-owned controls. Opening them performs no native work.
    fn tweak_options(&self, _saved: Option<&ModuleEnvelope>) -> Option<ModuleEnvelope> {
        None
    }
    fn show_tweak_options(&self, _ui: &mut eframe::egui::Ui, _config: &mut ModuleEnvelope) {}
    /// Resolve native targets on the worker before persisting a new manual intent.
    fn prepare_tweak_options(
        &self,
        _target: &TargetIdentity,
        _config: &ModuleEnvelope,
    ) -> Result<ModuleEnvelope, ModuleError> {
        Err(ModuleError::new(
            super::platform::contract::ErrorKind::InvalidConfig,
            "tweak options",
        ))
    }
    fn tweak_ready_for_config(
        &self,
        target: &TargetIdentity,
        _saved: Option<&ModuleEnvelope>,
    ) -> Result<bool, ModuleError> {
        self.tweak_ready(target)
    }
    fn tweak_summary(&self, _config: &ModuleEnvelope) -> Option<String> {
        None
    }
    /// Describe the value actually read back, without implying a measured hardware effect.
    fn tweak_verified_label(&self) -> &'static str {
        "Current value verified"
    }
    fn selects_saved_exe(&self) -> bool {
        false
    }
    fn qualify_tweak_target(&self, target: &TargetIdentity) -> Result<TargetIdentity, ModuleError> {
        Ok(target.clone())
    }
    fn tweak_executable(&self, _target: &TargetIdentity) -> Option<std::path::PathBuf> {
        None
    }
    fn format_baseline(&self, _fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        None
    }
    fn feature(&self) -> FeatureId;
    fn default_config(&self) -> ModuleEnvelope;
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool;
    /// Legacy editor fixtures still characterize rejected configuration edits.
    #[cfg(test)]
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TweakSection {
    Games,
    Comfort,
    Hardware,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct TweakSpec {
    pub order: u8,
    pub section: TweakSection,
    pub title: &'static str,
    pub scope: &'static str,
    pub apply_label: &'static str,
    pub help: &'static str,
}

#[derive(Clone, Debug)]
pub(crate) enum ModuleAction {
    // Kept for headless diagnostic actions; the compact page has no old toolbar.
    #[cfg_attr(not(test), allow(dead_code))]
    Refresh,
    OpenSettings,
    #[cfg_attr(not(test), allow(dead_code))]
    Import(std::path::PathBuf),
}

#[cfg(test)]
pub(crate) fn edit_configuration(
    editor: &dyn ModuleEditor,
    ui: &mut eframe::egui::Ui,
    config: &mut ModuleEnvelope,
) -> bool {
    let previous = config.clone();
    if editor.show(ui, config).is_ok() && editor.accepts_config(config) {
        true
    } else {
        *config = previous;
        false
    }
}

#[cfg(test)]
pub(crate) fn preference_with_enabled(
    saved: Option<&ModulePreference>,
    default: ModulePreference,
    enabled: bool,
) -> ModulePreference {
    let mut preference = saved.cloned().unwrap_or(default);
    preference.enabled = enabled;
    preference
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum TuningPageSection {
    #[default]
    System,
    Hardware,
    PcChecks,
}

#[derive(Default)]
pub(crate) struct TuningUiSession {
    pub selected_section: TuningPageSection,
    pub tweak_help: Option<(&'static str, String)>,
    pub confirm_undo_all: bool,
    pub tweak_target: Option<TargetIdentity>,
    pub tweak_options: Option<(FeatureId, TargetIdentity, ModuleEnvelope)>,
    #[cfg(test)]
    pub selected_profile: Option<ProfileId>,
    pub error: Option<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggle_default() -> ModulePreference {
        ModulePreference {
            feature: FeatureId::new("toggle_test").unwrap(),
            enabled: false,
            mode: ActivationMode::ForegroundSession,
            config: ModuleEnvelope {
                feature: FeatureId::new("toggle_test").unwrap(),
                kind: super::super::platform::codec::EnvelopeKind::Config,
                version: super::super::platform::contract::SchemaVersion::new(1).unwrap(),
                target: None,
                payload: serde_json::json!({"value": 7}),
            },
        }
    }

    #[test]
    fn simple_ui_first_enable_uses_module_defaults_and_supported_trigger() {
        let default = toggle_default();
        let actual = preference_with_enabled(None, default.clone(), true);
        assert!(actual.enabled);
        assert_eq!(actual.config, default.config);
        assert_eq!(actual.mode, ActivationMode::ForegroundSession);
        assert!(
            !default.enabled,
            "registration itself never enables management"
        );
    }

    #[test]
    fn simple_ui_disable_keeps_saved_parameters_and_trigger() {
        let mut saved = toggle_default();
        saved.enabled = true;
        saved.mode = ActivationMode::Session;
        saved.config.payload = serde_json::json!({"value": 12});
        let actual = preference_with_enabled(Some(&saved), toggle_default(), false);
        assert!(!actual.enabled);
        assert_eq!(actual.config, saved.config);
        assert_eq!(actual.mode, ActivationMode::Session);
        assert_eq!(actual.feature, saved.feature);
    }

    #[test]
    fn simple_ui_editor_failure_blocks_save_and_retains_original_draft() {
        struct InvalidEditor;
        impl ModuleEditor for InvalidEditor {
            fn feature(&self) -> FeatureId {
                toggle_default().feature
            }
            fn default_config(&self) -> ModuleEnvelope {
                toggle_default().config
            }
            fn accepts_config(&self, _: &ModuleEnvelope) -> bool {
                true
            }
            fn show(
                &self,
                _: &mut eframe::egui::Ui,
                config: &mut ModuleEnvelope,
            ) -> Result<bool, ()> {
                config.payload = serde_json::json!({"broken": true});
                Err(())
            }
        }
        let mut draft = toggle_default().config;
        let original = draft.clone();
        let ctx = eframe::egui::Context::default();
        let mut save_allowed = true;
        let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
            save_allowed = edit_configuration(&InvalidEditor, ui, &mut draft);
        });
        output.textures_delta.clear();
        assert!(!save_allowed);
        assert_eq!(draft, original);
    }

    #[test]
    fn simple_ui_process_intent_can_be_enabled_before_the_game_is_running() {
        let controls = card_controls(
            ModuleAccess::Managed,
            &Capability::Unsupported("Waiting for a verified process".into()),
            true,
            false,
            true,
            false,
        );
        assert!(controls.configure);
        assert!(
            controls.enable,
            "saving an enabled trigger must not require its current target"
        );
        assert!(
            !controls.apply,
            "a saved trigger is not permission to apply to an unverified target"
        );
    }

    #[test]
    fn p5_available_managed_card_allows_saved_configuration() {
        assert_eq!(
            card_controls(
                ModuleAccess::Managed,
                &Capability::Available,
                true,
                false,
                true,
                false
            ),
            CardControls {
                configure: true,
                enable: true,
                disable: false,
                apply: true
            }
        );
    }

    #[test]
    fn p5_unavailable_card_can_withdraw_saved_intent_without_claiming_active() {
        assert_eq!(
            card_controls(
                ModuleAccess::Managed,
                &Capability::Unsupported("Scope unavailable".into()),
                true,
                true,
                false,
                false
            ),
            CardControls {
                configure: true,
                enable: false,
                disable: true,
                apply: false
            }
        );
        assert_eq!(status_label(ModuleStatus::Unsupported), "Unavailable");
    }

    #[test]
    fn p5_pending_unknown_and_readonly_cards_are_not_editable() {
        for (access, known, busy) in [
            (ModuleAccess::Managed, true, true),
            (ModuleAccess::Managed, false, false),
            (ModuleAccess::ReadOnly, true, false),
        ] {
            assert_eq!(
                card_controls(access, &Capability::Available, known, true, true, busy),
                CardControls {
                    configure: false,
                    enable: false,
                    disable: false,
                    apply: false
                }
            );
        }
    }

    #[test]
    fn p5_restart_and_drift_are_distinct_from_effective_active() {
        assert_eq!(
            status_label(ModuleStatus::VerifiedActive(Effect::RestartRequired)),
            "Saved · restart required"
        );
        assert_eq!(
            status_label(ModuleStatus::StoredRestartRequired),
            "Saved · restart required"
        );
        assert_eq!(
            status_label(ModuleStatus::ExternalChange),
            "Changed outside this app"
        );
        assert_eq!(
            status_label(ModuleStatus::RecoveryRequired),
            "Recovery required"
        );
    }
}
