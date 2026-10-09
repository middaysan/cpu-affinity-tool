//! Concrete module registration is confined to this composition boundary.
pub(crate) mod diagnostics;
pub(crate) mod managed;

pub(crate) fn registry() -> super::platform::registry::Registry {
    registry_with_scheduling(std::sync::Arc::new(
        managed::scheduling::SchedulingHandoff::default(),
    ))
}

pub(crate) fn registry_with_scheduling(
    scheduling: std::sync::Arc<managed::scheduling::SchedulingHandoff>,
) -> super::platform::registry::Registry {
    let mut registry = super::platform::registry::Registry::default();
    if !super::MODULES_ENABLED {
        return registry;
    }
    for module in managed::modules_with_scheduling(scheduling) {
        registry
            .register(module)
            .expect("static module registrations must be unique and valid");
    }
    diagnostics::register(&mut registry)
        .expect("static diagnostic registrations must be unique and valid");
    registry
}

#[cfg(test)]
mod tests {
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use crate::app::features::tuning::platform::contract::ModuleAccess;
    use crate::app::features::tuning::Tuning;

    #[test]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn modules_default_registry_exposes_every_agreed_module_once_without_native_work() {
        let tuning = Tuning::default();
        let descriptors: Vec<_> = tuning.registry().descriptors().collect();
        assert_eq!(descriptors.len(), 18);
        assert_eq!(
            descriptors
                .iter()
                .filter(|d| d.access == ModuleAccess::Managed)
                .count(),
            8
        );
        assert_eq!(
            descriptors
                .iter()
                .filter(|d| d.access == ModuleAccess::ReadOnly)
                .count(),
            10
        );
        let mut ids: Vec<_> = descriptors.iter().map(|d| d.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), descriptors.len());
    }

    #[test]
    #[cfg(not(all(target_os = "windows", feature = "windows")))]
    fn linux_modules_registry_is_empty() {
        assert!(Tuning::default().registry().descriptors().next().is_none());
    }
}
