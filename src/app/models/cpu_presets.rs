use crate::app::models::{CoreInfo, CoreType, CpuCluster, CpuSchema};
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

fn default_threads_per_core() -> usize { 1 }
fn default_repeat() -> usize { 1 }

const PRESETS_JSON: &str = include_str!("../../../assets/cpu_presets.json");

pub fn get_preset_for_model(model: &str, total_threads: usize) -> Option<CpuSchema> {
    let model_lower = model.to_lowercase();
    let root: SchemesRoot = serde_json::from_str(PRESETS_JSON).ok()?;

    for scheme in root.schemes {
        // Match logic
        let threads_match = scheme
            .match_config
            .total_threads
            .map_or(true, |t| t == total_threads);
        
        let keywords_match = if scheme.match_config.keywords.is_empty() {
            true
        } else {
            scheme
                .match_config
                .keywords
                .iter()
                .all(|kw| model_lower.contains(&kw.to_lowercase()))
        };

        if threads_match && keywords_match {
            let mut clusters = Vec::new();
            let mut current_thread_idx = 0;

            for entry in scheme.layout {
                let cores_in_group = entry.cores.or(entry.cores_per_group).unwrap_or(0);

                for r in 0..entry.repeat {
                    let group_name = if let Some(pattern) = &entry.group_name_pattern {
                        pattern.replace("{i}", &r.to_string())
                    } else {
                        entry.group_name.clone().unwrap_or_default()
                    };

                    let mut core_infos = Vec::new();
                    let label_prefix = entry.label_prefix.as_deref().unwrap_or(match entry
                        .entry_type
                        .as_str()
                    {
                        "performance" => "P",
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
