use super::common::*;
use crate::app::features::tuning::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
};
use os_api::gaming::{self, CpuSet};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
const ID: &str = "cpu_sets";
pub(crate) const FIELD: &str = "process_default_cpu_sets";
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    ids: Vec<u32>,
}
impl Config {
    fn valid(&self) -> bool {
        self.ids.len() <= 16384
            && self
                .ids
                .iter()
                .enumerate()
                .all(|(index, id)| !self.ids[..index].contains(id))
    }
}
#[derive(Default)]
pub(crate) struct CpuSets {
    topology: RwLock<Vec<CpuSet>>,
    scheduling: Arc<super::scheduling::SchedulingHandoff>,
}
impl CpuSets {
    pub(crate) fn with_scheduling(scheduling: Arc<super::scheduling::SchedulingHandoff>) -> Self {
        Self {
            topology: Default::default(),
            scheduling,
        }
    }
}
pub(crate) fn configured_ids(config: &ModuleEnvelope) -> Result<Vec<u32>, ModuleError> {
    let config: Config = decode(ID, config)?;
    if !config.valid() {
        return Err(error(ErrorKind::InvalidConfig, "CPU Set configuration"));
    }
    let mut ids = config.ids;
    ids.sort_unstable();
    Ok(ids)
}
impl ModuleEditor for CpuSets {
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        envelope(ID, &Config::default())
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        configured_ids(config).is_ok()
    }
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        let mut value: Config = decode(ID, config).map_err(|_| ())?;
        if !value.valid() {
            return Err(());
        }
        let before = value.ids.clone();
        if ui.button("Let Windows choose CPUs").clicked() {
            value.ids.clear();
        }
        eframe::egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                if let Ok(topology) = self.topology.read() {
                    if topology.is_empty() {
                        ui.label("Refresh to load the available CPUs.");
                    }
                    for cpu in topology.iter() {
                        let mut selected = value.ids.contains(&cpu.id);
                        let permitted = !cpu.allocated || cpu.allocated_to_target;
                        let label = format!(
                            "CPU {} · group {} · core {} · efficiency class {} · ID {}{}{}",
                            cpu.logical_processor,
                            cpu.group,
                            cpu.core,
                            cpu.efficiency_class,
                            cpu.id,
                            if cpu.parked { " · parked" } else { "" },
                            if !permitted {
                                " · used by another app"
                            } else {
                                ""
                            }
                        );
                        if ui
                            .add_enabled(
                                permitted,
                                eframe::egui::Checkbox::new(&mut selected, label),
                            )
                            .changed()
                        {
                            if selected {
                                value.ids.push(cpu.id);
                            } else {
                                value.ids.retain(|id| *id != cpu.id);
                            }
                            value.ids.sort_unstable();
                        }
                    }
                    let unavailable: Vec<_> = value
                        .ids
                        .iter()
                        .filter(|id| !topology.iter().any(|cpu| cpu.id == **id))
                        .collect();
                    if !unavailable.is_empty() {
                        ui.label(format!(
                            "These saved CPU choices are unavailable: {unavailable:?}"
                        ));
                    }
                }
            });
        ui.small("Select the CPUs this game may use, or let Windows choose. The game's own settings and existing affinity may limit this choice.");
        let changed = before != value.ids;
        if changed {
            *config = envelope(ID, &value);
        }
        Ok(changed)
    }
    fn summary(&self, config: &ModuleEnvelope) -> Option<String> {
        configured_ids(config).ok().map(|ids| {
            if ids.is_empty() {
                "Windows chooses CPUs".into()
            } else {
                format!("{} CPUs selected", ids.len())
            }
        })
    }
}
impl Module for CpuSets {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Game CPU selection",
            "Choose which CPUs are available to games linked to this profile.",
            TargetScope::Process,
            automatic_modes(),
        )
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        vec![]
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        observation((|| {
            if matches!(target, TargetIdentity::Machine) {
                let topology = gaming::cpu_topology(None).map_err(|e| native("read", e))?;
                let count = topology.len();
                *self
                    .topology
                    .write()
                    .map_err(|_| error(ErrorKind::Read, "CPU metadata"))? = topology;
                return Ok(format!(
                    "{count} CPUs available. Start a linked game to inspect its current selection."
                ));
            }
            let process = process(target)?;
            let topology = gaming::cpu_topology(Some(&process)).map_err(|e| native("read", e))?;
            let ids = gaming::read_cpu_sets(&process).map_err(|e| native("read", e))?;
            let count = topology.len();
            *self
                .topology
                .write()
                .map_err(|_| error(ErrorKind::Read, "CPU metadata"))? = topology;
            Ok(if ids.is_empty() {
                format!("Windows default assignment; {count} actual CPU Sets")
            } else {
                format!("Default IDs: {ids:?}; {count} actual CPU Sets")
            })
        })())
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        let process = process(target)?;
        let ids = configured_ids(config)?;
        self.scheduling
            .baseline(process.pid, process.creation_token, true)?;
        gaming::validate_cpu_set_selection(
            &ids,
            &gaming::cpu_topology(Some(&process)).map_err(|e| native("read", e))?,
        )
        .map_err(|e| native("CPU Set selection", e))?;
        Ok(vec![field(target, FIELD, cpu_bytes(&ids))])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        gaming::read_cpu_sets(&process(&key.target)?)
            .map(|ids| cpu_bytes(&ids))
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        gaming::write_cpu_sets(
            &process(&key.target)?,
            &cpu_ids(expected)?,
            &cpu_ids(desired)?,
        )
        .map_err(|e| native("write", e))
    }
}
