use crate::app::models::AppToRun;
use crate::app::shared::ids::{GroupId, RuleId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShortcutCreationRole {
    Primary,
    #[cfg_attr(not(feature = "windows"), allow(dead_code))]
    NonPrimary,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleEditorTarget {
    pub group_id: GroupId,
    pub rule_id: RuleId,
}

/// State for editing an application that will be run with a specific CPU affinity.
pub struct RuleEditorSession {
    pub(crate) process_names: crate::app::features::rules::process_names::ProcessNames,
    /// The application currently being edited, if any.
    pub current_edit: Option<AppToRun>,
    pub(crate) background:
        Option<crate::app::features::system_tweaks::background_policy::RulePolicy>,
    pub(crate) background_error: Option<String>,
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub(crate) background_baseline: Option<(
        RuleId,
        crate::app::features::system_tweaks::config::TuningEnvelope,
        Option<crate::app::features::system_tweaks::background_policy::RulePolicy>,
    )>,
    /// Transient logical identity of the rule being edited.
    pub target: Option<RuleEditorTarget>,
    /// Transient result from desktop shortcut creation.
    pub shortcut_result: Option<RuleShortcutResult>,
}

#[cfg(any(test, all(target_os = "windows", feature = "windows")))]
impl RuleEditorSession {
    pub(crate) fn background_is_dirty(
        &mut self,
        saved: &crate::app::features::system_tweaks::config::TuningEnvelope,
    ) -> bool {
        use crate::app::features::system_tweaks::{background_policy, config::TuningLoad};
        let (Some(target), Some(draft)) = (&self.target, &self.background) else {
            return false;
        };
        if self
            .background_baseline
            .as_ref()
            .is_none_or(|(rule, previous, _)| {
                rule != &target.rule_id || !previous.same_document(saved)
            })
        {
            let baseline = match saved.decode() {
                TuningLoad::Off => Some(background_policy::RulePolicy::default()),
                TuningLoad::Known(config) => {
                    background_policy::rule_policy(&config, &target.rule_id.0).ok()
                }
                _ => None,
            };
            self.background_baseline = Some((target.rule_id.clone(), saved.clone(), baseline));
        }
        self.background_baseline
            .as_ref()
            .and_then(|(_, _, policy)| policy.as_ref())
            != Some(draft)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleShortcutResult {
    Created { filename: String },
    Failed { message: String },
}
