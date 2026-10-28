//! Concrete module registration is confined to this composition boundary.
pub(crate) mod diagnostics;
// Independent managed settings. Construction and editors perform no native reads.
pub(crate) mod accessibility_hotkeys;
pub(crate) mod background_capture;
pub(crate) mod background_workload;
pub(crate) mod common;
pub(crate) mod core_parking;
pub(crate) mod cpu_sets;
pub(crate) mod display_refresh;
pub(crate) mod game_mode;
pub(crate) mod gpu_preference;
mod mouse_acceleration;
mod power_mode;
mod power_plan;
mod preference_common;
pub(crate) mod process_priority;
mod process_qos;
mod process_timer_policy;
pub(crate) mod scheduling;
pub(crate) mod windowed_optimizations;

use crate::app::features::system_tweaks::platform::registry::Module;
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
        Arc::new(core_parking::CoreParking),
        Arc::new(mouse_acceleration::MouseAcceleration),
        Arc::new(accessibility_hotkeys::AccessibilityHotkeys),
        Arc::new(process_qos::ProcessQos),
        Arc::new(cpu_sets::CpuSets::with_scheduling(scheduling.clone())),
        Arc::new(background_workload::BackgroundWorkload::with_scheduling(
            scheduling.clone(),
        )),
        Arc::new(process_priority::ProcessPriority::with_scheduling(
            scheduling,
        )),
        Arc::new(process_timer_policy::ProcessTimerPolicy),
        Arc::new(game_mode::GameMode),
        Arc::new(background_capture::BackgroundCapture),
        Arc::new(windowed_optimizations::WindowedOptimizations),
        Arc::new(gpu_preference::GpuPreference),
        Arc::new(display_refresh::DisplayRefresh),
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn registry() -> super::platform::registry::Registry {
    registry_with_scheduling(std::sync::Arc::new(scheduling::SchedulingHandoff::default()))
}

pub(crate) fn registry_with_scheduling(
    scheduling: std::sync::Arc<scheduling::SchedulingHandoff>,
) -> super::platform::registry::Registry {
    let mut registry = super::platform::registry::Registry::default();
    if !super::MODULES_ENABLED {
        return registry;
    }
    for module in modules_with_scheduling(scheduling) {
        registry
            .register(module)
            .expect("static module registrations must be unique and valid");
    }
    diagnostics::register(&mut registry)
        .expect("static diagnostic registrations must be unique and valid");
    registry
}

#[cfg(test)]
mod registry_tests {
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use crate::app::features::system_tweaks::platform::contract::ModuleAccess;
    use crate::app::features::system_tweaks::Tuning;

    #[test]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn modules_default_registry_exposes_every_agreed_module_once_without_native_work() {
        let tuning = Tuning::default();
        let descriptors: Vec<_> = tuning.registry().descriptors().collect();
        assert_eq!(descriptors.len(), 25);
        assert_eq!(
            descriptors
                .iter()
                .filter(|d| d.access == ModuleAccess::Managed)
                .count(),
            15
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

#[cfg(test)]
mod preference_contract_tests {
    use super::common::*;
    use super::preference_common::*;
    use super::*;
    use crate::app::features::system_tweaks::{platform::contract::*, presentation::ModuleEditor};
    use os_api::windows_tweaks::StoredDword;
    fn preference_modules() -> Vec<Arc<dyn Module>> {
        vec![
            Arc::new(game_mode::GameMode),
            Arc::new(background_capture::BackgroundCapture),
            Arc::new(windowed_optimizations::WindowedOptimizations),
            Arc::new(gpu_preference::GpuPreference),
            Arc::new(display_refresh::DisplayRefresh),
        ]
    }

    #[test]
    fn durable_account_targets_share_one_identity_across_logons() {
        let expected = TargetIdentity::Account {
            sid: "S-1-5-21-1".into(),
        };
        let modules: Vec<Arc<dyn Module>> = vec![
            Arc::new(game_mode::GameMode),
            Arc::new(background_capture::BackgroundCapture),
            Arc::new(windowed_optimizations::WindowedOptimizations),
            Arc::new(power_plan::PowerPlan::default()),
            Arc::new(power_mode::PowerMode),
        ];
        for module in modules {
            for logon in [77, 78] {
                let legacy = TargetIdentity::User {
                    sid: "S-1-5-21-1".into(),
                    logon,
                    session: None,
                };
                assert_eq!(
                    module.canonical_target(&legacy).unwrap(),
                    expected,
                    "{} must retain its first account baseline",
                    module.descriptor().id.as_str()
                );
            }
            assert_eq!(module.canonical_target(&expected).unwrap(), expected);
            assert_ne!(
                module
                    .canonical_target(&TargetIdentity::User {
                        sid: "S-1-5-21-2".into(),
                        logon: 78,
                        session: None,
                    })
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn durable_account_identity_requires_canonical_bounded_sid() {
        for sid in ["S-1-5-21-1", "S-1-5-21-4294967295-0-1-1001"] {
            let account = TargetIdentity::Account { sid: sid.into() };
            assert!(account.valid(), "{sid}");
            assert_eq!(account.scope(), TargetScope::User);
            assert_eq!(canonical_account_target(&account).unwrap(), account);
        }
        for sid in [
            "S-1-",
            "S-2-5-21-1",
            "S-1-5",
            "S-1-5-21-x",
            "S-1-5-21-01",
            "S-1-5-21-4294967296",
            "S-1-281474976710656-1",
            "S-1-5-21-1\0",
            "s-1-5-21-1",
            "S-1-5--1",
            "S-1-5-+1",
            "S-1-5-21-1 ",
        ] {
            let account = TargetIdentity::Account { sid: sid.into() };
            assert!(!account.valid(), "{sid}");
            assert!(canonical_account_target(&account).is_err(), "{sid}");
            assert!(
                canonical_account_target(&TargetIdentity::User {
                    sid: sid.into(),
                    logon: 77,
                    session: None,
                })
                .is_err(),
                "{sid}"
            );
        }
        let excessive = TargetIdentity::Account {
            sid: format!("S-1-5{}", "-1".repeat(16)),
        };
        assert!(!excessive.valid());
        assert!(canonical_account_target(&excessive).is_err());
        let missing_logon = TargetIdentity::User {
            sid: "S-1-5-21-1".into(),
            logon: 0,
            session: None,
        };
        assert!(!missing_logon.valid());
        assert!(canonical_account_target(&missing_logon).is_err());
        assert!(canonical_account_target(&TargetIdentity::Machine).is_err());
    }

    #[test]
    fn durable_account_fields_accept_stable_targets_without_native_work() {
        let target = TargetIdentity::Account {
            sid: "S-1-5-21-1".into(),
        };
        let modules: Vec<Arc<dyn Module>> = vec![
            Arc::new(game_mode::GameMode),
            Arc::new(background_capture::BackgroundCapture),
            Arc::new(windowed_optimizations::WindowedOptimizations),
            Arc::new(power_mode::PowerMode),
        ];
        for module in modules {
            let fields = module
                .fields(&target, &module.editor().default_config())
                .unwrap();
            assert!(!fields.is_empty());
            assert!(fields.iter().all(|field| field.key.target == target));
        }
    }

    #[test]
    fn durable_account_mapping_preserves_exact_session_and_process_targets() {
        let old = TargetIdentity::User {
            sid: "S-1-5-21-1".into(),
            logon: 77,
            session: None,
        };
        let current = TargetIdentity::User {
            sid: "S-1-5-21-1".into(),
            logon: 78,
            session: None,
        };
        let modules: Vec<Arc<dyn Module>> = vec![
            Arc::new(mouse_acceleration::MouseAcceleration),
            Arc::new(accessibility_hotkeys::AccessibilityHotkeys),
        ];
        for module in modules {
            assert_eq!(module.canonical_target(&old).unwrap(), old);
            assert_eq!(module.canonical_target(&current).unwrap(), current);
            assert!(module
                .fields(
                    &TargetIdentity::Account {
                        sid: "S-1-5-21-1".into(),
                    },
                    &module.editor().default_config()
                )
                .is_err());
        }
        let process = TargetIdentity::Process {
            pid: 42,
            creation_token: 123,
            image_path: r"C:\game.exe".into(),
        };
        assert_eq!(
            process_qos::ProcessQos.canonical_target(&process).unwrap(),
            process
        );
        assert!(canonical_account_target(&process).is_err());
        let display = TargetIdentity::Device {
            identity: "display:".to_owned()
                + &serde_json::to_string(&os_api::windows_tweaks::DisplayTarget {
                    user_sid: "S-1-5-21-1".into(),
                    user_logon: 77,
                    device_name: "display1".into(),
                    adapter_id: "adapter1".into(),
                    monitor_id: "monitor1".into(),
                    user_session: None,
                })
                .unwrap(),
        };
        assert_eq!(
            display_refresh::DisplayRefresh
                .canonical_target(&display)
                .unwrap(),
            display
        );
        assert!(canonical_account_target(&display).is_err());
    }

    #[test]
    fn windows_tweaks_preserve_saved_feature_field_and_token_contracts() {
        let target = TargetIdentity::User {
            sid: "S-1-5-21-1".into(),
            logon: 77,
            session: None,
        };
        for (module, id, field_name, desired) in [
            (
                Arc::new(game_mode::GameMode) as Arc<dyn Module>,
                "game_mode",
                "game_mode_preference",
                token(&StoredDword::Value(1)),
            ),
            (
                Arc::new(background_capture::BackgroundCapture) as Arc<dyn Module>,
                "background_capture",
                "background_capture_preference",
                token(&StoredDword::Value(0)),
            ),
            (
                Arc::new(windowed_optimizations::WindowedOptimizations) as Arc<dyn Module>,
                "windowed_optimizations",
                "windowed_optimization_preference",
                token(&Some("1".to_owned())),
            ),
        ] {
            assert_eq!(module.descriptor().id, feature(id));
            let config = module.editor().default_config();
            assert_eq!(
                config.payload,
                serde_json::json!({"recommended":true,"refresh":null})
            );
            let fields = module.fields(&target, &config).unwrap();
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].key.field, resource(field_name));
            assert_eq!(fields[0].desired, desired);
        }
    }

    #[test]
    fn windows_tweaks_preserve_gpu_and_display_target_configuration_contracts() {
        use os_api::windows_tweaks::{DisplayMode, DisplayTarget, GpuTarget};
        let gpu = gpu_preference::GpuPreference;
        let gpu_target = TargetIdentity::Device {
            identity: "gpu_exe:".to_owned()
                + &serde_json::to_string(&GpuTarget {
                    user_sid: "S-1-5-21-1".into(),
                    user_logon: 77,
                    executable: r"C:\game.exe".into(),
                })
                .unwrap(),
        };
        let gpu_fields = gpu.fields(&gpu_target, &gpu.default_config()).unwrap();
        assert_eq!(gpu.descriptor().id, feature("gpu_preference"));
        assert_eq!(gpu_fields[0].key.field, resource("gpu_preference_field"));
        assert_eq!(gpu_fields[0].key.target, gpu_target);
        assert_eq!(gpu_fields[0].desired, token(&Some("2".to_owned())));

        let display = display_refresh::DisplayRefresh;
        let display_target = TargetIdentity::Device {
            identity: "display:".to_owned()
                + &serde_json::to_string(&DisplayTarget {
                    user_sid: "S-1-5-21-1".into(),
                    user_logon: 77,
                    device_name: "display1".into(),
                    adapter_id: "adapter1".into(),
                    monitor_id: "monitor1".into(),
                    user_session: None,
                })
                .unwrap(),
        };
        let mode = DisplayMode {
            width: 1920,
            height: 1080,
            bits_per_pixel: 32,
            frequency: 144,
            flags: 0,
            orientation: 0,
            fixed_output: 0,
        };
        let config = envelope(
            "display_refresh",
            &Config {
                recommended: true,
                refresh: Some(mode),
            },
        );
        assert_eq!(display.descriptor().id, feature("display_refresh"));
        assert!(display.accepts_config(&config));
        // Configuration is valid, but a synthetic monitor is not authority to
        // change a real display or establish a restorable native baseline.
        assert!(display.fields(&display_target, &config).is_err());
        assert!(display
            .fields(&display_target, &display.default_config())
            .is_err());
        for module in preference_modules()
            .into_iter()
            .filter(|module| module.descriptor().id != feature("display_refresh"))
        {
            let mut config = module.editor().default_config();
            config.payload["refresh"] = serde_json::to_value(mode).unwrap();
            assert!(!module.editor().accepts_config(&config));
        }
    }

    #[test]
    fn windows_tweaks_register_manual_owned_fields_without_native_work() {
        let modules = preference_modules();
        assert_eq!(modules.len(), 5);
        for module in modules {
            assert_eq!(
                module.descriptor().activation_modes,
                vec![ActivationMode::Manual]
            );
            assert!(module.editor().tweak().is_some());
            assert!(module
                .editor()
                .accepts_config(&module.editor().default_config()));
            assert!(module
                .fields(&TargetIdentity::Machine, &module.editor().default_config())
                .is_err());
        }
    }

    #[test]
    fn windows_tweaks_baseline_text_distinguishes_absence_from_off() {
        let module = game_mode::GameMode;
        assert_eq!(
            module
                .format_baseline(&[(
                    resource("game_mode_preference"),
                    token(&StoredDword::Absent)
                )])
                .as_deref(),
            Some("No explicit preference")
        );
        assert_eq!(
            module
                .format_baseline(&[(
                    resource("game_mode_preference"),
                    token(&StoredDword::Value(0))
                )])
                .as_deref(),
            Some("Off")
        );
    }

    #[test]
    fn windows_tweaks_config_rejects_unknown_fields_and_wrong_feature() {
        let module = windowed_optimizations::WindowedOptimizations;
        let mut config = module.default_config();
        config.payload = serde_json::json!({"refresh":null,"unknown":true});
        assert!(!module.accepts_config(&config));
        config = module.default_config();
        config.feature = feature("different_feature");
        assert!(!module.accepts_config(&config));
    }

    #[test]
    fn windows_tweaks_former_readonly_configuration_cannot_become_managed_intent() {
        for module in preference_modules() {
            let mut legacy = module.editor().default_config();
            legacy.payload = serde_json::json!({});
            assert!(!module.editor().accepts_config(&legacy));
        }
    }

    #[test]
    fn windows_tweaks_device_descriptors_expose_semantic_display_and_gpu_scopes() {
        let display = display_refresh::DisplayRefresh.descriptor();
        assert!(display.target_scopes.contains(&TargetScope::PnpDevice));
        assert!(display.target_scopes.contains(&TargetScope::Display));
        let gpu = gpu_preference::GpuPreference.descriptor();
        assert!(gpu.target_scopes.contains(&TargetScope::PnpDevice));
        assert!(gpu.target_scopes.contains(&TargetScope::Gpu));
    }
}
