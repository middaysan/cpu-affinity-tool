//! Reserved composition point: no Linux tweaks or native tuning actions are enabled.
use crate::app::features::system_tweaks::platform::registry::Registry;

pub(super) fn registry() -> Registry {
    Registry::default()
}
