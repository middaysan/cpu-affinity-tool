/// A bounded, read-only explanation; it grants no process membership or writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SteamDetectionIssue {
    Installation(String),
    AmbiguousProcesses,
    HelperOnly,
    NotDetected,
}

impl SteamDetectionIssue {
    pub(crate) fn reason(&self) -> &str {
        match self {
            Self::Installation(message) => message,
            Self::AmbiguousProcesses => {
                "The main game process could not be identified unambiguously."
            }
            Self::HelperOnly => "The possible main game process is listed as a helper.",
            Self::NotDetected => "The main game process has not been detected yet.",
        }
    }

    pub(crate) fn guidance(&self) -> &'static str {
        "Start the game in Steam, then choose its main process (for example, dota2.exe) and save. This links the game executable to this rule. Optional helper names do not select the main process."
    }
}
