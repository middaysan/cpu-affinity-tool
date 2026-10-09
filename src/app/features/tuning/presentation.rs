//! Pure module-card decisions shared by the presenter and state-level tests.
use super::{
    config::{Association, ModulePreference},
    platform::{
        codec::ModuleEnvelope,
        contract::{
            ActivationEligibility, ActivationMode, Capability, Effect, FeatureId, ModuleAccess,
            ModuleStatus, ProfileId, TargetIdentity, TargetScope,
        },
    },
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CardControls {
    pub configure: bool,
    pub enable: bool,
    pub disable: bool,
    pub apply: bool,
}

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

pub(crate) fn mode_label(mode: ActivationMode) -> &'static str {
    match mode {
        ActivationMode::Manual => "Manual",
        ActivationMode::Session => "While the game is running",
        ActivationMode::ForegroundSession => "While the game is in the foreground",
    }
}

pub(crate) fn scope_label(scope: TargetScope) -> &'static str {
    match scope {
        TargetScope::Machine => "Computer",
        TargetScope::User => "User",
        TargetScope::Session => "Session",
        TargetScope::Process => "Process",
        TargetScope::PnpDevice => "Device",
        TargetScope::Gpu => "GPU",
        TargetScope::Display => "Display",
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum ModuleSection {
    #[default]
    Performance,
    Input,
    Power,
    Windows,
    Diagnostics,
}
impl ModuleSection {
    pub const ALL: [Self; 5] = [
        Self::Performance,
        Self::Input,
        Self::Power,
        Self::Windows,
        Self::Diagnostics,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Performance => "Game performance",
            Self::Input => "Mouse & keyboard",
            Self::Power => "Power settings",
            Self::Windows => "Windows settings",
            Self::Diagnostics => "Check your PC",
        }
    }
}
/// Each module decodes and edits its own typed configuration. Shell presenters
/// dispatch through this adapter and never interpret payloads or feature IDs.
pub(crate) trait ModuleEditor: Send + Sync {
    fn feature(&self) -> FeatureId;
    fn default_config(&self) -> ModuleEnvelope;
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool;
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()>;
    fn section(&self) -> ModuleSection {
        ModuleSection::Performance
    }
    fn summary(&self, _config: &ModuleEnvelope) -> Option<String> {
        None
    }
    fn actions(&self, _ui: &mut eframe::egui::Ui) -> Option<ModuleAction> {
        None
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ModuleAction {
    Refresh,
    OpenSettings,
    Import(std::path::PathBuf),
}

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

pub(crate) fn preference_with_enabled(
    saved: Option<&ModulePreference>,
    default: ModulePreference,
    enabled: bool,
) -> ModulePreference {
    let mut preference = saved.cloned().unwrap_or(default);
    preference.enabled = enabled;
    preference
}

pub(crate) struct EditorDraft {
    pub profile: ProfileId,
    pub expected: Option<ModulePreference>,
    pub candidate: ModulePreference,
    pub pending_save_revision: Option<u64>,
}

#[derive(Default)]
pub(crate) struct TuningUiSession {
    pub profile_name: String,
    pub selected_profile: Option<ProfileId>,
    pub editor: Option<EditorDraft>,
    pub error: Option<&'static str>,
    pub association: Option<AssociationDraft>,
    pub manual_target: Option<(ProfileId, FeatureId, TargetIdentity)>,
    pub section: ModuleSection,
    pub show_new_profile: bool,
    pub creating_revision: Option<u64>,
    pub selected_rule: Option<(
        crate::app::shared::ids::GroupId,
        crate::app::shared::ids::RuleId,
    )>,
    pub rule_eligibility: Option<ActivationEligibility>,
}

pub(crate) struct AssociationDraft {
    pub target: crate::app::shell::sessions::RuleEditorTarget,
    pub expected: Option<Association>,
    pub profile: Option<ProfileId>,
    pub eligibility: ActivationEligibility,
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
        let _ = ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
            save_allowed = edit_configuration(&InvalidEditor, ui, &mut draft);
        });
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
