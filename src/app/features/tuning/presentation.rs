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
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CardControls {
    pub configure: bool,
    pub enable: bool,
    pub disable: bool,
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
        enable: editable && mode_supported && matches!(capability, Capability::Available),
        disable: editable && saved_enabled,
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

/// Each module decodes and edits its own typed configuration. Shell presenters
/// dispatch through this adapter and never interpret payloads or feature IDs.
pub(crate) trait ModuleEditor: Send + Sync {
    fn feature(&self) -> FeatureId;
    fn default_config(&self) -> ModuleEnvelope;
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool;
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()>;
}

#[derive(Default, Clone)]
pub(crate) struct EditorRegistry {
    editors: Vec<Arc<dyn ModuleEditor>>,
}
impl EditorRegistry {
    #[cfg(test)]
    pub(crate) fn register(&mut self, editor: Arc<dyn ModuleEditor>) -> Result<(), ()> {
        let feature = editor.feature();
        let config = editor.default_config();
        if self.editors.len() >= 64
            || self.get(&feature).is_some()
            || config.feature != feature
            || config.to_bytes().is_err()
            || !editor.accepts_config(&config)
        {
            return Err(());
        }
        self.editors.push(editor);
        Ok(())
    }
    pub(crate) fn get(&self, feature: &FeatureId) -> Option<&dyn ModuleEditor> {
        self.editors
            .iter()
            .find(|editor| editor.feature() == *feature)
            .map(Arc::as_ref)
    }
}

pub(crate) struct EditorDraft {
    pub profile: ProfileId,
    pub expected: Option<ModulePreference>,
    pub candidate: ModulePreference,
}

#[derive(Default)]
pub(crate) struct TuningUiSession {
    pub profile_name: String,
    pub selected_profile: Option<ProfileId>,
    pub editor: Option<EditorDraft>,
    pub error: Option<&'static str>,
    pub association: Option<AssociationDraft>,
    pub manual_target: Option<(ProfileId, FeatureId, TargetIdentity)>,
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
                disable: false
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
                disable: true
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
                    disable: false
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
