use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WaitCondition {
    NewSample,
    SettingsApplied,
    CollectionReleased,
    UiSettled,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WaitOptions {
    pub condition: WaitCondition,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub after_seconds: Option<f64>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub history_seconds: Option<u32>,
}
fn timeout() -> u64 {
    3000
}
#[derive(Default, Serialize)]
pub(crate) struct WaitObservation {
    pub completed_seconds: Option<f64>,
    pub enabled: bool,
    pub history_seconds: u32,
    pub settings_committed: bool,
    pub collection_released: bool,
    pub ui_settled: bool,
}
impl WaitOptions {
    pub(super) fn validate(&self) -> Result<(), String> {
        if !(1..=6000).contains(&self.timeout_ms)
            || self
                .after_seconds
                .is_some_and(|n| !n.is_finite() || n < 0.0)
            || self
                .history_seconds
                .is_some_and(|n| !matches!(n, 30 | 300 | 1800))
        {
            return Err("Invalid wait duration, sample time or history duration".into());
        }
        let valid = match self.condition {
            WaitCondition::NewSample => self.enabled.is_none() && self.history_seconds.is_none(),
            WaitCondition::SettingsApplied => {
                self.after_seconds.is_none()
                    && (self.enabled.is_some() || self.history_seconds.is_some())
            }
            _ => {
                self.after_seconds.is_none()
                    && self.enabled.is_none()
                    && self.history_seconds.is_none()
            }
        };
        if !valid {
            return Err("Wait arguments do not match this condition".into());
        }
        Ok(())
    }
    pub(super) fn satisfied(&self, state: &WaitObservation) -> bool {
        match self.condition {
            WaitCondition::NewSample => state
                .completed_seconds
                .is_some_and(|n| self.after_seconds.is_none_or(|after| n > after)),
            WaitCondition::SettingsApplied => {
                state.settings_committed
                    && self.enabled.is_none_or(|v| v == state.enabled)
                    && self
                        .history_seconds
                        .is_none_or(|v| v == state.history_seconds)
            }
            WaitCondition::CollectionReleased => state.collection_released,
            WaitCondition::UiSettled => state.ui_settled,
        }
    }
}
