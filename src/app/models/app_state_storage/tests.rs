use super::{state_path, storage_io, AppStateStorage, CURRENT_APP_STATE_VERSION};
use crate::app::models::app_to_run::{AppToRun, LaunchTarget};
use crate::app::models::core_group::CoreGroup;
use crate::app::models::cpu_presets::get_preset_for_model;
use crate::app::models::cpu_schema::{CoreInfo, CoreType, CpuCluster, CpuSchema};
use crate::app::models::meta::{effective_cpu_model, effective_total_threads};
use os_api::PriorityClass;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn with_temp_state_path(test_name: &str, test_fn: impl FnOnce(&Path)) {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let temp_dir = std::env::temp_dir().join(format!(
        "cpu_affinity_tool_{test_name}_{}_{}",
        std::process::id(),
        unique_suffix
    ));

    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    fs::create_dir_all(&temp_dir).unwrap();

    let state_path = temp_dir.join(state_path::STATE_FILE_NAME);
    test_fn(&state_path);

    let _ = fs::remove_dir_all(&temp_dir);
}

fn sample_state() -> AppStateStorage {
    AppStateStorage {
        version: CURRENT_APP_STATE_VERSION,
        groups: vec![CoreGroup {
            name: "Games".to_string(),
            cores: vec![0, 1],
            programs: vec![AppToRun::new_path(
                PathBuf::from(r"C:\Sample.lnk"),
                vec!["--fullscreen".to_string()],
                PathBuf::from(r"C:\Sample.exe"),
                PriorityClass::Normal,
                true,
            )],
            is_hidden: false,
            run_all_button: true,
        }],
        cpu_schema: CpuSchema {
            model: "Sample CPU".to_string(),
            clusters: Vec::new(),
        },
        theme_index: 2,
        process_monitoring_enabled: true,
    }
}

fn current_schema_state() -> AppStateStorage {
    AppStateStorage {
        version: CURRENT_APP_STATE_VERSION,
        groups: Vec::new(),
        cpu_schema: CpuSchema {
            model: effective_cpu_model(),
            clusters: vec![CpuCluster {
                name: "Pinned".to_string(),
                cores: vec![CoreInfo {
                    index: 0,
                    core_type: CoreType::Performance,
                    label: "P0".to_string(),
                }],
            }],
        },
        theme_index: 1,
        process_monitoring_enabled: false,
    }
}

fn expected_migrated_cpu_schema(clusters: Vec<Vec<usize>>) -> CpuSchema {
    let cpu_model = effective_cpu_model();
    let total_threads = effective_total_threads();

    if let Some(preset) = get_preset_for_model(&cpu_model, total_threads) {
        preset
    } else {
        CpuSchema {
            model: cpu_model,
            clusters: super::migrations::build_generic_clusters(clusters),
        }
    }
}

#[test]
fn test_backup_rotation() {
    with_temp_state_path("backup_rotation", |state_path| {
        // 1. First backup
        fs::write(state_path, "original").unwrap();
        storage_io::backup_state_file(state_path);
        assert!(!state_path.exists());
        assert!(state_path.with_file_name("state.json.old").exists());
        assert_eq!(
            fs::read_to_string(state_path.with_file_name("state.json.old")).unwrap(),
            "original"
        );

        // 2. Second backup (should be .old1)
        fs::write(state_path, "second").unwrap();
        storage_io::backup_state_file(state_path);
        assert!(!state_path.exists());
        assert!(state_path.with_file_name("state.json.old1").exists());
        assert_eq!(
            fs::read_to_string(state_path.with_file_name("state.json.old1")).unwrap(),
            "second"
        );

        // 3. Third backup (should be .old2)
        fs::write(state_path, "third").unwrap();
        storage_io::backup_state_file(state_path);
        assert!(!state_path.exists());
        assert!(state_path.with_file_name("state.json.old2").exists());
        assert_eq!(
            fs::read_to_string(state_path.with_file_name("state.json.old2")).unwrap(),
            "third"
        );
    });
}

#[test]
fn test_load_v5_state_keeps_current_schema_without_rewrite() {
    with_temp_state_path("v4_current", |state_path| {
        let serialized = serde_json::to_string_pretty(&current_schema_state()).unwrap();
        fs::write(state_path, &serialized).unwrap();

        let loaded = AppStateStorage::load_from_path(state_path);
        let persisted = fs::read_to_string(state_path).unwrap();

        assert_eq!(loaded.version, CURRENT_APP_STATE_VERSION);
        assert_eq!(persisted, serialized);
    });
}

#[test]
fn test_load_v5_generic_state_refreshes_when_cpu_model_is_known() {
    with_temp_state_path("v4_generic", |state_path| {
        let generic = AppStateStorage {
            version: CURRENT_APP_STATE_VERSION,
            groups: Vec::new(),
            cpu_schema: CpuSchema {
                model: "Generic CPU".to_string(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
        };

        let before = serde_json::to_string_pretty(&generic).unwrap();
        fs::write(state_path, &before).unwrap();

        let loaded = AppStateStorage::load_from_path(state_path);
        let after = fs::read_to_string(state_path).unwrap();

        if effective_cpu_model().is_empty() {
            assert_eq!(after, before);
            assert_eq!(loaded.cpu_schema.model, "Generic CPU");
        } else {
            assert_ne!(after, before);
            assert_ne!(loaded.cpu_schema.model, "Generic CPU");
        }

        assert!(!state_path.with_file_name("state.json.old").exists());
    });
}

#[test]
fn test_load_v3_state_migrates_and_persists_v5() {
    with_temp_state_path("v3_migration", |state_path| {
        let mut value = serde_json::to_value(sample_state()).unwrap();
        value["version"] = json!(3);

        value
            .get_mut("groups")
            .and_then(Value::as_array_mut)
            .and_then(|groups| groups.get_mut(0))
            .and_then(|group| group.get_mut("programs"))
            .and_then(Value::as_array_mut)
            .and_then(|programs| programs.get_mut(0))
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove("additional_processes");

        fs::write(state_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = AppStateStorage::load_from_path(state_path);
        assert_eq!(loaded.version, CURRENT_APP_STATE_VERSION);
        assert!(loaded.groups[0].programs[0].additional_processes.is_empty());
        assert!(!state_path.with_file_name("state.json.old").exists());

        let persisted: Value =
            serde_json::from_str(&fs::read_to_string(state_path).unwrap()).unwrap();
        assert_eq!(persisted["version"], json!(CURRENT_APP_STATE_VERSION));
        assert_eq!(
            persisted["groups"][0]["programs"][0]["additional_processes"],
            json!([])
        );
    });
}

#[test]
fn test_load_v4_state_migrates_path_targets_losslessly() {
    with_temp_state_path("v4_to_v5_path_targets", |state_path| {
        let legacy_v4 = json!({
            "version": 4,
            "groups": [{
                "name": "Games",
                "cores": [0, 1],
                "programs": [{
                    "name": "Sample",
                    "dropped_path": r"C:\Sample.lnk",
                    "args": ["--fullscreen"],
                    "bin_path": r"C:\Sample.exe",
                    "additional_processes": ["sample_helper.exe"],
                    "autorun": true,
                    "priority": "Normal"
                }],
                "is_hidden": false,
                "run_all_button": true
            }],
            "cpu_schema": {
                "model": "Generic CPU",
                "clusters": []
            },
            "theme_index": 2,
            "process_monitoring_enabled": true
        });

        fs::write(
            state_path,
            serde_json::to_string_pretty(&legacy_v4).unwrap(),
        )
        .unwrap();

        let loaded = AppStateStorage::load_from_path(state_path);
        assert_eq!(loaded.version, CURRENT_APP_STATE_VERSION);
        assert!(matches!(
            loaded.groups[0].programs[0].launch_target,
            LaunchTarget::Path { .. }
        ));
        assert_eq!(
            loaded.groups[0].programs[0].bin_path(),
            Some(PathBuf::from(r"C:\Sample.exe").as_path())
        );
        assert_eq!(
            loaded.groups[0].programs[0].dropped_path(),
            Some(PathBuf::from(r"C:\Sample.lnk").as_path())
        );
        assert_eq!(
            loaded.groups[0].programs[0].additional_processes,
            vec!["sample_helper.exe".to_string()]
        );

        let persisted: Value =
            serde_json::from_str(&fs::read_to_string(state_path).unwrap()).unwrap();
        assert_eq!(persisted["version"], json!(CURRENT_APP_STATE_VERSION));
        assert!(persisted["groups"][0]["programs"][0]["launch_target"].is_object());
    });
}

#[test]
fn test_build_generic_clusters_preserves_order_and_labels() {
    let clusters = super::migrations::build_generic_clusters(vec![vec![0, 2], vec![1]]);

    assert_eq!(clusters.len(), 2);
    assert_eq!(clusters[0].name, "Cluster 1");
    assert_eq!(clusters[1].name, "Cluster 2");
    assert_eq!(clusters[0].cores[0].index, 0);
    assert_eq!(clusters[0].cores[0].label, "Core 0");
    assert_eq!(clusters[0].cores[0].core_type, CoreType::Other);
    assert_eq!(clusters[0].cores[1].index, 2);
    assert_eq!(clusters[0].cores[1].label, "Core 2");
    assert_eq!(clusters[1].cores[0].index, 1);
    assert_eq!(clusters[1].cores[0].label, "Core 1");
}

#[test]
fn test_load_v2_state_migrates_and_keeps_backup() {
    with_temp_state_path("v2_migration", |state_path| {
        let legacy_v2 = json!({
            "version": 2,
            "_version": 2,
            "groups": [{
                "name": "Games",
                "cores": [0, 1],
                "programs": [],
                "is_hidden": false,
                "run_all_button": true
            }],
            "clusters": [[0, 1], [2, 3]],
            "theme_index": 1,
            "process_monitoring_enabled": true
        });

        let original = serde_json::to_string_pretty(&legacy_v2).unwrap();
        fs::write(state_path, &original).unwrap();

        let loaded = AppStateStorage::load_from_path(state_path);
        let expected_cpu_schema = expected_migrated_cpu_schema(vec![vec![0, 1], vec![2, 3]]);
        assert_eq!(loaded.version, CURRENT_APP_STATE_VERSION);
        assert_eq!(loaded.theme_index, 1);
        assert!(loaded.process_monitoring_enabled);
        assert_eq!(
            serde_json::to_value(&loaded.cpu_schema).unwrap(),
            serde_json::to_value(&expected_cpu_schema).unwrap()
        );
        assert!(state_path.with_file_name("state.json.old").exists());
        assert_eq!(
            fs::read_to_string(state_path.with_file_name("state.json.old")).unwrap(),
            original
        );

        let persisted: Value =
            serde_json::from_str(&fs::read_to_string(state_path).unwrap()).unwrap();
        assert_eq!(persisted["version"], json!(CURRENT_APP_STATE_VERSION));
        assert_eq!(persisted["theme_index"], json!(1));
        assert_eq!(persisted["process_monitoring_enabled"], json!(true));
        assert_eq!(
            persisted["cpu_schema"],
            serde_json::to_value(expected_cpu_schema).unwrap()
        );
    });
}

#[test]
fn test_load_legacy_state_defaults_monitor_flag_and_keeps_backup() {
    with_temp_state_path("legacy_migration", |state_path| {
        let legacy = json!({
            "groups": [{
                "name": "Games",
                "cores": [0, 1],
                "programs": [],
                "is_hidden": false,
                "run_all_button": false
            }],
            "clusters": [[0, 1], [2, 3]],
            "theme_index": 2
        });

        let original = serde_json::to_string_pretty(&legacy).unwrap();
        fs::write(state_path, &original).unwrap();

        let loaded = AppStateStorage::load_from_path(state_path);
        let expected_cpu_schema = expected_migrated_cpu_schema(vec![vec![0, 1], vec![2, 3]]);
        assert_eq!(loaded.version, CURRENT_APP_STATE_VERSION);
        assert_eq!(loaded.theme_index, 2);
        assert!(!loaded.process_monitoring_enabled);
        assert_eq!(
            serde_json::to_value(&loaded.cpu_schema).unwrap(),
            serde_json::to_value(&expected_cpu_schema).unwrap()
        );
        assert!(state_path.with_file_name("state.json.old").exists());
        assert_eq!(
            fs::read_to_string(state_path.with_file_name("state.json.old")).unwrap(),
            original
        );

        let persisted: Value =
            serde_json::from_str(&fs::read_to_string(state_path).unwrap()).unwrap();
        assert_eq!(persisted["version"], json!(CURRENT_APP_STATE_VERSION));
        assert_eq!(persisted["theme_index"], json!(2));
        assert_eq!(persisted["process_monitoring_enabled"], json!(false));
        assert_eq!(
            persisted["cpu_schema"],
            serde_json::to_value(expected_cpu_schema).unwrap()
        );
    });
}

#[test]
fn test_invalid_state_file_is_backed_up_and_replaced_with_default() {
    with_temp_state_path("invalid_state", |state_path| {
        fs::write(state_path, "{not valid json").unwrap();

        let loaded = AppStateStorage::load_from_path(state_path);
        assert_eq!(loaded.version, CURRENT_APP_STATE_VERSION);
        assert!(loaded.groups.is_empty());
        assert!(state_path.with_file_name("state.json.old").exists());
        assert_eq!(
            fs::read_to_string(state_path.with_file_name("state.json.old")).unwrap(),
            "{not valid json"
        );

        let persisted: AppStateStorage =
            serde_json::from_str(&fs::read_to_string(state_path).unwrap()).unwrap();
        assert_eq!(persisted.version, CURRENT_APP_STATE_VERSION);
        assert!(persisted.groups.is_empty());
    });
}
