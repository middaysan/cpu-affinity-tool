use crate::app::models::{CoreInfo, CoreType, CpuCluster, CpuSchema};
use once_cell::sync::Lazy;
use serde::Deserialize;

#[derive(Deserialize)]
struct SchemesRoot {
    schemes: Vec<SchemeConfig>,
}

#[derive(Deserialize)]
struct SchemeConfig {
    #[allow(dead_code)]
    name: String,
    #[serde(rename = "match")]
    match_config: MatchConfig,
    layout: Vec<LayoutEntry>,
}

#[derive(Deserialize)]
struct MatchConfig {
    keywords: Vec<String>,
    total_threads: Option<usize>,
}

#[derive(Deserialize)]
struct LayoutEntry {
    #[serde(rename = "type")]
    entry_type: String,

    #[serde(default = "default_threads_per_core")]
    threads_per_core: usize,

    // For standard group
    group_name: Option<String>,
    label_prefix: Option<String>,
    cores: Option<usize>,

    // For repeat group
    #[serde(default = "default_repeat")]
    repeat: usize,
    group_name_pattern: Option<String>,
    cores_per_group: Option<usize>,
}

fn default_threads_per_core() -> usize {
    1
}
fn default_repeat() -> usize {
    1
}

const PRESETS_JSON: &str = include_str!("../../../assets/cpu_presets.json");

static PRESETS: Lazy<SchemesRoot> = Lazy::new(|| {
    serde_json::from_str(PRESETS_JSON).expect("Failed to parse embedded cpu_presets.json")
});

pub fn get_all_presets_info() -> Vec<(String, Vec<String>, Option<usize>)> {
    let root = &*PRESETS;
    root.schemes
        .iter()
        .map(|s| (s.name.clone(), s.match_config.keywords.clone(), s.match_config.total_threads))
        .collect()
}

pub fn get_preset_for_model(model: &str, total_threads: usize) -> Option<CpuSchema> {
    let model_lower = model.to_lowercase();
    let model_trimmed = model_lower.trim();
    let root = &*PRESETS;

    for scheme in &root.schemes {
        // Match logic
        let threads_match = scheme
            .match_config
            .total_threads
            .is_none_or(|t| t == total_threads);

        let keywords_match = if scheme.match_config.keywords.is_empty() {
            true
        } else {
            scheme
                .match_config
                .keywords
                .iter()
                .all(|kw| model_trimmed.contains(kw.to_lowercase().trim()))
        };

        if threads_match && keywords_match {
            let mut clusters = Vec::new();
            let mut current_thread_idx = 0;

            for entry in &scheme.layout {
                let cores_in_group = entry.cores.or(entry.cores_per_group).unwrap_or(0);

                for r in 0..entry.repeat {
                    let group_name = if let Some(pattern) = &entry.group_name_pattern {
                        pattern.replace("{i}", &r.to_string())
                    } else {
                        entry.group_name.clone().unwrap_or_default()
                    };

                    let mut core_infos = Vec::new();
                    let label_prefix =
                        entry
                            .label_prefix
                            .as_deref()
                            .unwrap_or(match entry.entry_type.as_str() {
                                "performance" | "p_core_no_ht" => "P",
                                "efficient" => "E",
                                "ccd" => "C",
                                _ => "",
                            });

                    for c in 0..cores_in_group {
                        // Labeling
                        let label_index = if entry.group_name_pattern.is_some() {
                            r * cores_in_group + c
                        } else {
                            c
                        };

                        let label = format!("{}{}", label_prefix, label_index);

                        for t in 0..entry.threads_per_core {
                            let core_type = match entry.entry_type.as_str() {
                                "performance" | "ccd" => {
                                    if t == 0 {
                                        CoreType::Performance
                                    } else {
                                        CoreType::HyperThreading
                                    }
                                }
                                "efficient" => CoreType::Efficient,
                                "p_core_no_ht" => CoreType::Performance,
                                _ => CoreType::Other,
                            };

                            core_infos.push(CoreInfo {
                                index: current_thread_idx,
                                core_type,
                                label: label.clone(),
                            });
                            current_thread_idx += 1;
                        }
                    }

                    clusters.push(CpuCluster {
                        name: group_name,
                        cores: core_infos,
                    });
                }
            }

            return Some(CpuSchema {
                model: model.to_string(),
                clusters,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presets_parsing() {
        let info = get_all_presets_info();
        assert!(!info.is_empty(), "Presets should not be empty");
        println!("Loaded {} presets", info.len());
    }

    #[test]
    fn test_intel_i9_matching() {
        let model = "13th Gen Intel(R) Core(TM) i9-13900K";
        let preset = get_preset_for_model(model, 32);
        assert!(preset.is_some(), "Should match i9-13900K");
        let schema = preset.unwrap();
        assert_eq!(schema.clusters.len(), 2);
    }

    #[test]
    fn test_intel_i5_14600_matching() {
        let model = "Intel(R) Core(TM) i5-14600KF";
        let preset = get_preset_for_model(model, 20);
        assert!(preset.is_some(), "Should match i5-14600KF");
        let schema = preset.unwrap();
        assert_eq!(schema.clusters.len(), 2);
        assert_eq!(schema.clusters[0].name, "Performance Cores");
        assert_eq!(schema.clusters[1].name, "Efficient Cores");
        // 6 P-cores * 2 threads = 12 threads in first cluster
        assert_eq!(schema.clusters[0].cores.len(), 12);
        // 8 E-cores * 1 thread = 8 threads in second cluster
        assert_eq!(schema.clusters[1].cores.len(), 8);
    }
}
