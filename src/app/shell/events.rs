use crate::app::features::tuning::platform::{
    contract::{FeatureId, TargetScope},
    controller::ShutdownResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    Warning(String),
    Monitor(String),
    RuntimeStateChanged,
    Tuning(TuningDiagnostic),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningOperation {
    Shutdown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningOutcome {
    Completed,
    RecoveryRequired,
    TimedOut,
    Faulted,
}

/// Diagnostic data deliberately excludes configuration, owners and native identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuningDiagnostic {
    pub module: Option<FeatureId>,
    pub operation: TuningOperation,
    pub target_kind: Option<TargetScope>,
    pub outcome: TuningOutcome,
}
impl TuningDiagnostic {
    pub(crate) fn shutdown(outcome: ShutdownResult) -> Self {
        Self {
            module: None,
            operation: TuningOperation::Shutdown,
            target_kind: None,
            outcome: match outcome {
                ShutdownResult::Complete => TuningOutcome::Completed,
                ShutdownResult::RecoveryRequired => TuningOutcome::RecoveryRequired,
                ShutdownResult::TimedOut => TuningOutcome::TimedOut,
                ShutdownResult::Faulted => TuningOutcome::Faulted,
            },
        }
    }
    pub(crate) fn message(&self) -> String {
        let operation = match self.operation {
            TuningOperation::Shutdown => "shutdown",
        };
        let outcome = match self.outcome {
            TuningOutcome::Completed => "worker stopped",
            TuningOutcome::RecoveryRequired => "recovery required",
            TuningOutcome::TimedOut => "cleanup timed out; recovery required",
            TuningOutcome::Faulted => "worker failed; recovery required",
        };
        let module = self
            .module
            .as_ref()
            .map_or(String::new(), |module| format!(" · {}", module.as_str()));
        let target = self
            .target_kind
            .map_or(String::new(), |kind| format!(" · {kind:?}"));
        format!("Modules: {operation} · {outcome}{module}{target}")
    }
}

impl ShellEvent {
    pub fn needs_repaint(&self) -> bool {
        true
    }

    pub fn legacy_log_message(&self) -> Option<(&str, bool)> {
        match self {
            Self::Warning(message) => Some((message.as_str(), true)),
            Self::Monitor(message) => Some((message.as_str(), false)),
            Self::RuntimeStateChanged | Self::Tuning(_) => None,
        }
    }
    pub(crate) fn tuning_log_message(&self) -> Option<String> {
        match self {
            Self::Tuning(event) => Some(event.message()),
            _ => None,
        }
    }
}

#[test]
fn p5_shutdown_diagnostics_distinguish_timeout_and_do_not_claim_restoration() {
    let timeout = TuningDiagnostic::shutdown(ShutdownResult::TimedOut).message();
    let complete = TuningDiagnostic::shutdown(ShutdownResult::Complete).message();
    assert!(timeout.contains("timed out") && timeout.contains("recovery"));
    assert!(!timeout.contains("completed") && !timeout.contains("restored"));
    assert!(complete.contains("worker stopped"));
    assert!(
        !complete.contains("restored"),
        "Manual owners can remain configured after worker exit"
    );
}
