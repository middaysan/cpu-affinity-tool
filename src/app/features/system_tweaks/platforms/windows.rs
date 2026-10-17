//! Windows composition; each tweak declares its version requirements.
use std::sync::Arc;

use crate::app::features::system_tweaks::{
    platform::registry::Registry,
    tweaks::{self, scheduling::SchedulingHandoff},
};

pub(super) fn registry_with_scheduling(scheduling: Arc<SchedulingHandoff>) -> Registry {
    tweaks::registry_with_scheduling(scheduling)
}
