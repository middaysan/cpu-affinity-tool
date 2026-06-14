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
    /// The application currently being edited, if any.
    pub current_edit: Option<AppToRun>,
    /// Transient logical identity of the rule being edited.
    pub target: Option<RuleEditorTarget>,
    /// Transient result from desktop shortcut creation.
    pub shortcut_result: Option<RuleShortcutResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleShortcutResult {
    Created { filename: String },
    Failed { message: String },
}
