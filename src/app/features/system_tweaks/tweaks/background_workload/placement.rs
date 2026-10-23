//! Session-stable pool preparation. Failure skips only optional placement.
use super::{Config, CpuMode};
use os_api::background::{
    efficient_pool, validate_pool, PlacementCpu, PlacementSkip, PlacementWitness,
};
pub(crate) fn resolve_pool(
    config: &Config,
    topology: &[PlacementCpu],
) -> Result<Config, PlacementSkip> {
    if config.cpu_mode == CpuMode::PriorityEco {
        return Ok(config.clone());
    }
    if topology.len() > 512 {
        return Err(PlacementSkip::InvalidTopology);
    }
    let mut resolved = config.clone();
    resolved.manual_ids = match config.cpu_mode {
        CpuMode::PriorityEco => unreachable!(),
        CpuMode::EfficientCores => efficient_pool(topology)?,
        CpuMode::ManualPool => {
            if config.manual_topology != topology {
                return Err(PlacementSkip::InvalidTopology);
            }
            validate_pool(&config.manual_ids, topology)?;
            config.manual_ids.clone()
        }
    };
    resolved.manual_ids.sort_unstable();
    resolved.manual_topology = topology.to_vec();
    Ok(resolved)
}
pub(crate) fn placement_plan(
    config: &Config,
    topology: &[PlacementCpu],
    baseline: &[u32],
) -> Result<Option<PlacementWitness>, PlacementSkip> {
    if config.cpu_mode == CpuMode::PriorityEco {
        return Ok(None);
    }
    if topology != config.manual_topology {
        return Err(PlacementSkip::InvalidTopology);
    }
    let witness = PlacementWitness {
        topology: topology.to_vec(),
        baseline: baseline.to_vec(),
        desired: config.manual_ids.clone(),
    };
    if !witness.valid() {
        return Err(PlacementSkip::ExistingRestriction);
    }
    Ok(Some(witness))
}
