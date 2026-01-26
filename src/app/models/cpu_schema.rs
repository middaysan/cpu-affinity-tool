use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum CoreType {
    Performance,
    Efficient,
    HyperThreading,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreInfo {
    pub index: usize,
    pub core_type: CoreType,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuCluster {
    pub name: String,
    pub cores: Vec<CoreInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuSchema {
    pub model: String,
    pub clusters: Vec<CpuCluster>,
}

impl CpuSchema {
    pub fn get_assigned_cores(&self) -> std::collections::HashSet<usize> {
        self.clusters
            .iter()
            .flat_map(|cluster| cluster.cores.iter().map(|c| c.index))
            .collect()
    }
}
