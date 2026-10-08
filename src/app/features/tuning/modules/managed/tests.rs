use super::*;
use crate::app::features::tuning::{
    platform::{
        codec::ModuleEnvelope,
        contract::{SchemaVersion, TargetIdentity},
    },
    presentation::ModuleEditor,
};
use os_api::gaming::{
    AccessibilityFeature, AccessibilityState, GuidValue, OwnedPolicy, EXECUTION_SPEED,
};

#[test]
fn managed_editors_validate_exact_versions_without_native_calls() {
    for module in modules() {
        let editor = module.editor();
        let config = editor.default_config();
        assert!(editor.accepts_config(&config));
        let mut future = config.clone();
        future.version = SchemaVersion::new(2).unwrap();
        assert!(!editor.accepts_config(&future));
        let mut unknown = config.clone();
        unknown
            .payload
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), true.into());
        assert!(!editor.accepts_config(&unknown));
        let target = TargetIdentity::Process {
            pid: 1,
            creation_token: 7,
            image_path: "game.exe".into(),
        };
        future.target = Some(target);
        assert!(!editor.accepts_config(&future));
        assert_eq!(
            ModuleEnvelope::from_bytes(&config.to_bytes().unwrap()).unwrap(),
            config
        );
    }
}

#[test]
fn managed_all_process_modules_defer_targets_to_verified_worker() {
    for module in modules().into_iter().filter(|m| {
        m.descriptor()
            .target_scopes
            .contains(&crate::app::features::tuning::platform::contract::TargetScope::Process)
    }) {
        assert!(module.targets().is_empty());
    }
}

#[test]
fn managed_priority_never_accepts_realtime_configuration() {
    let editor = process_priority::ProcessPriority::default();
    let mut config = editor.default_config();
    config.payload = serde_json::json!({"priority":"Realtime"});
    assert!(!editor.accepts_config(&config));
}

#[test]
fn managed_native_tokens_preserve_guid_mouse_policy_and_empty_cpu_assignment() {
    let guid = GuidValue([0xa7; 16]);
    assert_eq!(common::guid(&guid.0).unwrap(), guid);
    assert!(common::guid(&[0; 15]).is_err());
    let tuple = os_api::gaming::MouseParameters([-1, 32767, 2]);
    assert_eq!(common::mouse(&common::mouse_bytes(tuple)).unwrap(), tuple);
    let policy = OwnedPolicy {
        control: EXECUTION_SPEED,
        state: 0,
    };
    assert_eq!(
        common::policy(&common::policy_bytes(policy), EXECUTION_SPEED).unwrap(),
        policy
    );
    assert!(common::policy(
        &common::policy_bytes(OwnedPolicy {
            control: 4,
            state: 0
        }),
        EXECUTION_SPEED
    )
    .is_err());
    assert!(common::cpu_ids(&[]).unwrap().is_empty());
    assert!(common::cpu_ids(&[1, 2]).is_err());
}

#[test]
fn managed_accessibility_skips_already_enabled_features() {
    let candidate = accessibility_hotkeys::changes(
        [
            (
                AccessibilityFeature::Sticky,
                AccessibilityState {
                    enabled: true,
                    hotkey_flags: 12,
                },
            ),
            (
                AccessibilityFeature::Filter,
                AccessibilityState {
                    enabled: false,
                    hotkey_flags: 12,
                },
            ),
            (
                AccessibilityFeature::Toggle,
                AccessibilityState {
                    enabled: false,
                    hotkey_flags: 0,
                },
            ),
        ],
        &accessibility_hotkeys::Config {
            sticky: true,
            filter: true,
            toggle: true,
        },
    );
    assert_eq!(
        candidate,
        vec![
            (AccessibilityFeature::Filter, 0),
            (AccessibilityFeature::Toggle, 0)
        ]
    );
}

#[test]
fn managed_cpu_config_is_ids_not_mask_or_index_and_rejects_duplicates() {
    let editor = cpu_sets::CpuSets::default();
    let mut config = editor.default_config();
    config.payload = serde_json::json!({"ids":[257,512]});
    assert!(editor.accepts_config(&config));
    config.payload = serde_json::json!({"ids":[257,257]});
    assert!(!editor.accepts_config(&config));
    config.payload = serde_json::json!({"mask":3});
    assert!(!editor.accepts_config(&config));
}

#[test]
fn managed_process_field_tokens_are_distinct_and_target_bound() {
    use crate::app::features::tuning::platform::registry::Module;
    let target = TargetIdentity::Process {
        pid: 777,
        creation_token: 19,
        image_path: "game.exe".into(),
    };
    let qos = process_qos::ProcessQos;
    let timer = process_timer_policy::ProcessTimerPolicy;
    let scheduling = std::sync::Arc::new(scheduling::SchedulingHandoff::default());
    scheduling
        .started(
            777,
            19,
            os_api::gaming::ProcessSettingsFields {
                affinity: false,
                priority: false,
            },
        )
        .unwrap();
    let priority = process_priority::ProcessPriority::with_scheduling(scheduling);
    let modules: [&dyn Module; 3] = [&qos, &timer, &priority];
    let mut keys = std::collections::BTreeSet::new();
    for module in modules {
        let config = module.editor().default_config();
        let projected = module.fields(&target, &config).unwrap();
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].key.target, target);
        assert!(keys.insert(projected[0].key.clone()));
        assert!(module.fields(&TargetIdentity::Machine, &config).is_err());
    }
}
