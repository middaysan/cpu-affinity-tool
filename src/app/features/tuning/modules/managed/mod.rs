//! Independent managed settings. Construction and editors perform no native reads.
pub(crate) mod accessibility_hotkeys;
mod common;
pub(crate) mod cpu_sets;
mod mouse_acceleration;
mod power_mode;
mod power_plan;
pub(crate) mod process_priority;
mod process_qos;
mod process_timer_policy;
pub(crate) mod scheduling;

use crate::app::features::tuning::platform::registry::Module;
use std::sync::Arc;

#[cfg(test)]
pub(crate) fn modules() -> Vec<Arc<dyn Module>> {
    modules_with_scheduling(Arc::new(scheduling::SchedulingHandoff::default()))
}
pub(crate) fn modules_with_scheduling(
    scheduling: Arc<scheduling::SchedulingHandoff>,
) -> Vec<Arc<dyn Module>> {
    vec![
        Arc::new(power_plan::PowerPlan::default()),
        Arc::new(power_mode::PowerMode),
        Arc::new(mouse_acceleration::MouseAcceleration),
        Arc::new(accessibility_hotkeys::AccessibilityHotkeys),
        Arc::new(process_qos::ProcessQos),
        Arc::new(cpu_sets::CpuSets::with_scheduling(scheduling.clone())),
        Arc::new(process_priority::ProcessPriority::with_scheduling(
            scheduling,
        )),
        Arc::new(process_timer_policy::ProcessTimerPolicy),
    ]
}

#[cfg(test)]
mod tests;
