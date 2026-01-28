use crate::app::models::{CoreInfo, CoreType, CpuCluster, CpuSchema};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

#[derive(Deserialize)]
struct SchemesRoot {
    schemes: Vec<SchemeConfig>,
}

#[derive(Deserialize)]
struct SchemeConfig {
    name: String,
    #[serde(rename = "rules")]
    match_rules: Vec<MatchRule>,
    layout: Vec<LayoutEntry>,
}

#[derive(Deserialize)]
struct MatchRule {
    #[serde(default)]
    regexes: Vec<String>,
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
    let mut info = Vec::new();
    for s in &root.schemes {
        for rule in &s.match_rules {
            info.push((s.name.clone(), rule.regexes.clone(), rule.total_threads));
        }
    }
    info
}

pub fn get_preset_for_model(model: &str, total_threads: usize) -> Option<CpuSchema> {
    let root = &*PRESETS;

    for scheme in &root.schemes {
        let mut matched = false;

        for rule in &scheme.match_rules {
            let threads_match = rule.total_threads.is_none_or(|t| t == total_threads);

            let regex_match = if rule.regexes.is_empty() {
                true
            } else {
                rule.regexes.iter().any(|re_str| {
                    if let Ok(re) = Regex::new(re_str) {
                        re.is_match(model)
                    } else {
                        false
                    }
                })
            };

            if threads_match && regex_match {
                matched = true;
                break;
            }
        }

        if matched {
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

    #[test]
    fn test_amd_ryzen_9_matching() {
        let model = "AMD Ryzen 9 7950X3D 16-Core Processor";
        let preset = get_preset_for_model(model, 32);
        assert!(preset.is_some(), "Should match Ryzen 9 7950X3D");
        let schema = preset.unwrap();
        assert_eq!(schema.clusters.len(), 2);
        assert_eq!(schema.clusters[0].name, "CCD 0");
        assert_eq!(schema.clusters[1].name, "CCD 1");
    }

    #[test]
    fn test_intel_ultra_matching() {
        let model = "Intel(R) Core(TM) Ultra 9 285K";
        let preset = get_preset_for_model(model, 24);
        assert!(preset.is_some(), "Should match Ultra 9 285K");
        let schema = preset.unwrap();
        assert_eq!(schema.clusters.len(), 2);
        assert_eq!(
            schema.clusters[0].cores[0].core_type,
            crate::app::models::CoreType::Performance
        );
        // Arrow Lake doesn't have HT on P-cores
        assert_eq!(schema.clusters[0].cores.len(), 8);
    }
}
