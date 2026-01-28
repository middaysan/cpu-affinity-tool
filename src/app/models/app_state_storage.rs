use crate::app::models::core_group::CoreGroup;
use crate::app::models::cpu_presets::get_preset_for_model;
use crate::app::models::cpu_schema::{CoreInfo, CoreType, CpuCluster, CpuSchema};
use crate::app::models::meta::{TEST_CPU_MODEL, TEST_TOTAL_THREADS};
use os_api::OS;
use serde::{Deserialize, Serialize};

/// Current version of the application state schema
pub const CURRENT_APP_STATE_VERSION: u32 = 3;

impl AppStateStorage {
    /// Helper to get the CPU model, respecting test overrides.
    pub fn get_effective_cpu_model() -> String {
        #[allow(clippy::const_is_empty)]
        if !TEST_CPU_MODEL.is_empty() {
            TEST_CPU_MODEL.to_string()
        } else {
            OS::get_cpu_model()
        }
    }

    /// Helper to get the total threads, respecting test overrides.
    pub fn get_effective_total_threads() -> usize {
        #[allow(clippy::absurd_extreme_comparisons)]
        if TEST_TOTAL_THREADS > 0 {
            TEST_TOTAL_THREADS
        } else {
            num_cpus::get()
        }
    }
}

/// Storage for persistent application state that can be serialized to and deserialized from JSON.
/// This structure is responsible for saving and loading the application state between sessions.
#[derive(Serialize, Deserialize, Clone)]
pub struct AppStateStorage {
    /// Version of the application state schema
    /// Used for migrations between different versions
    pub version: u32,
    /// List of core groups defined by the user
    pub groups: Vec<CoreGroup>,
    /// CPU schema configuration
    pub cpu_schema: CpuSchema,
    /// Index of the currently selected UI theme (0: default, 1: light, 2: dark)
    pub theme_index: usize,
    /// Flag indicating whether process monitoring is enabled
    #[serde(default)]
    pub process_monitoring_enabled: bool,
}

impl AppStateStorage {
    /// Loads the application state from a JSON file.
    ///
    /// Attempts to read the state from a file named "state.json" located in the same directory
    /// as the executable. If the file doesn't exist or can't be parsed, it creates a default state
    /// with empty groups and clusters, and theme_index set to 0.
    ///
    /// # Returns
    ///
    /// An `AppStateStorage` instance is either loaded from the file or created with default values.
    pub fn load_state() -> AppStateStorage {
        let path = std::env::current_exe()
            .map(|mut p| {
                p.set_file_name("state.json");
                p
            })
            .unwrap_or_else(|_| "state.json".into());

        std::fs::read_to_string(&path)
            .ok()
            .and_then(|data| {
                // Try to parse with version check
                #[derive(Deserialize)]
                struct VersionCheck {
                    pub version: Option<u32>,
                }

                let v_check: VersionCheck = serde_json::from_str(&data).ok()?;

                match v_check.version {
                    Some(3) => {
                        let mut state: AppStateStorage = serde_json::from_str(&data).ok()?;
                        // Always try to refresh schema if it looks generic, to catch new presets
                        let cpu_model = Self::get_effective_cpu_model();
                        let total_threads = Self::get_effective_total_threads();

                        if state.cpu_schema.model == "Generic CPU"
                            || state.cpu_schema.clusters.is_empty()
                            || {
                                #[allow(clippy::const_is_empty)]
                                !TEST_CPU_MODEL.is_empty()
                            }
                            || (state.cpu_schema.model != cpu_model && !cpu_model.is_empty())
                        {
                            if let Some(preset) = get_preset_for_model(&cpu_model, total_threads) {
                                state.cpu_schema = preset;
                                state.save_state();
                            } else {
                                // If no preset found but we have a custom model name, at least update the name
                                if state.cpu_schema.model != cpu_model {
                                    state.cpu_schema.model = cpu_model;
                                    state.save_state();
                                }
                            }
                        }
                        Some(state)
                    }
                    Some(2) => {
                        #[derive(Deserialize)]
                        struct V2AppStateStorage {
                            pub _version: u32,
                            pub groups: Vec<CoreGroup>,
                            pub clusters: Vec<Vec<usize>>,
                            pub theme_index: usize,
                            pub process_monitoring_enabled: bool,
                        }

                        let v2: V2AppStateStorage = serde_json::from_str(&data).ok()?;
                        let mut schema_clusters = Vec::new();
                        for (i, cluster_cores) in v2.clusters.into_iter().enumerate() {
                            let cores = cluster_cores
                                .into_iter()
                                .map(|ci| CoreInfo {
                                    index: ci,
                                    core_type: CoreType::Other,
                                    label: format!("Core {ci}"),
                                })
                                .collect();
                            schema_clusters.push(CpuCluster {
                                name: format!("Cluster {}", i + 1),
                                cores,
                            });
                        }

                        let mut migrated = AppStateStorage {
                            version: CURRENT_APP_STATE_VERSION,
                            groups: v2.groups,
                            cpu_schema: CpuSchema {
                                model: "Generic CPU".to_string(),
                                clusters: schema_clusters,
                            },
                            theme_index: v2.theme_index,
                            process_monitoring_enabled: v2.process_monitoring_enabled,
                        };

                        // Try to get a better schema
                        let cpu_model = Self::get_effective_cpu_model();
                        let num_threads = Self::get_effective_total_threads();
                        if let Some(preset) = get_preset_for_model(&cpu_model, num_threads) {
                            migrated.cpu_schema = preset;
                        } else if migrated.cpu_schema.clusters.is_empty()
                            || migrated.cpu_schema.model == "Generic CPU"
                        {
                            migrated.cpu_schema.model = cpu_model;
                        }

                        migrated.save_state();
                        Some(migrated)
                    }
                    _ => {
                        // Legacy or V1
                        #[derive(Deserialize)]
                        struct LegacyAppStateStorage {
                            pub groups: Vec<CoreGroup>,
                            pub clusters: Vec<Vec<usize>>,
                            pub theme_index: usize,
                        }

                        let legacy: LegacyAppStateStorage = serde_json::from_str(&data).ok()?;
                        let mut schema_clusters = Vec::new();
                        for (i, cluster_cores) in legacy.clusters.into_iter().enumerate() {
                            let cores = cluster_cores
                                .into_iter()
                                .map(|ci| CoreInfo {
                                    index: ci,
                                    core_type: CoreType::Other,
                                    label: format!("Core {ci}"),
                                })
                                .collect();
                            schema_clusters.push(CpuCluster {
                                name: format!("Cluster {}", i + 1),
                                cores,
                            });
                        }

                        let mut migrated = AppStateStorage {
                            version: CURRENT_APP_STATE_VERSION,
                            groups: legacy.groups,
                            cpu_schema: CpuSchema {
                                model: "Generic CPU".to_string(),
                                clusters: schema_clusters,
                            },
                            theme_index: legacy.theme_index,
                            process_monitoring_enabled: false,
                        };

                        // Try to get a better schema
                        let cpu_model = Self::get_effective_cpu_model();
                        let total_threads = Self::get_effective_total_threads();
                        if let Some(preset) = get_preset_for_model(&cpu_model, total_threads) {
                            migrated.cpu_schema = preset;
                        } else if migrated.cpu_schema.clusters.is_empty()
                            || migrated.cpu_schema.model == "Generic CPU"
                        {
                            migrated.cpu_schema.model = cpu_model;
                        }

                        migrated.save_state();
                        Some(migrated)
                    }
                }
            })
            .unwrap_or_else(|| {
                // Create a new default state with the current version
                let cpu_model = Self::get_effective_cpu_model();
                let total_threads = Self::get_effective_total_threads();
                let cpu_schema =
                    get_preset_for_model(&cpu_model, total_threads).unwrap_or(CpuSchema {
                        model: cpu_model,
                        clusters: Vec::new(),
                    });

                let default_state = AppStateStorage {
                    version: CURRENT_APP_STATE_VERSION,
                    groups: Vec::new(),
                    cpu_schema,
                    theme_index: 0,
                    process_monitoring_enabled: false,
                };

                // Save the default state to disk
                default_state.save_state();

                default_state
            })
    }

    fn save_to_path(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Saves the current application state to a JSON file.
    ///
    /// Serializes the current state to JSON and writes it to a file named "state.json"
    /// in the current directory. If serialization or writing fails, the error is silently ignored.
    pub fn save_state(&self) {
        let path = std::env::current_exe()
            .map(|mut p| {
                p.set_file_name("state.json");
                p
            })
            .unwrap_or_else(|_| "state.json".into());
        let _ = self.save_to_path(&path);
    }
}
