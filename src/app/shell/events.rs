#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    Warning(String),
    Monitor(String),
    RuntimeStateChanged,
}

impl ShellEvent {
    pub fn needs_repaint(&self) -> bool {
        true
    }

    pub fn legacy_log_message(&self) -> Option<(&str, bool)> {
        match self {
            Self::Warning(message) => Some((message.as_str(), true)),
            Self::Monitor(message) => Some((message.as_str(), false)),
            Self::RuntimeStateChanged => None,
        }
    }
}
