//! Pure module-card decisions shared by the presenter and state-level tests.
use super::platform::{
    codec::ModuleEnvelope,
    contract::{Effect, FeatureId, ModuleError, ModuleStatus, ResourceId, TargetIdentity},
};
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
    fn tweak_saved_executable_target(&self, _path: &std::path::Path) -> Option<TargetIdentity> {
        None
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
    pub error: Option<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p5_restart_and_drift_are_distinct_from_effective_active() {
        assert_eq!(status_label(ModuleStatus::Unsupported), "Unavailable");
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
