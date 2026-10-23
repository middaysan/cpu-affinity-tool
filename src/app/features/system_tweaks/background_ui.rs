//! Cached background-session presentation. No native calls or worker ownership.
use super::tweaks::background_workload::{Config, CpuMode};
use super::{
    background_policy,
    session_reducer::{TuningRequest, TuningSnapshot},
    tweaks::background_workload::GameRule,
};
use eframe::egui;

fn toggle_core(
    config: &mut Config,
    topology: &[os_api::background::PlacementCpu],
    group: u16,
    core: u32,
    selected: bool,
) -> bool {
    let members: Vec<_> = topology
        .iter()
        .filter(|cpu| cpu.group == group && cpu.physical_core == core)
        .collect();
    if members.is_empty() || (selected && members.iter().any(|cpu| !cpu.allowed || cpu.reserved)) {
        return false;
    }
    if config.manual_topology != topology {
        config.manual_ids.clear();
    }
    config.manual_topology = topology.to_vec();
    config
        .manual_ids
        .retain(|id| !members.iter().any(|cpu| cpu.id == *id));
    if selected {
        config.manual_ids.extend(members.iter().map(|cpu| cpu.id));
    }
    config.manual_ids.sort_unstable();
    true
}
fn pool_controls(
    ui: &mut egui::Ui,
    background: &background_policy::Snapshot,
    config: &mut Config,
    pending: bool,
) -> bool {
    let topology = background.topology.as_slice();
    let efficient = os_api::background::efficient_pool(topology).is_ok();
    let before = config.cpu_mode;
    ui.horizontal_wrapped(|ui| {
        ui.label("CPU policy");
        ui.add_enabled_ui(!pending, |ui| {
            let combo = egui::ComboBox::from_id_salt("background-cpu-mode")
                .selected_text(match config.cpu_mode {
                    CpuMode::PriorityEco => "Priority + EcoQoS",
                    CpuMode::EfficientCores => "Prefer efficient cores",
                    CpuMode::ManualPool => "Manual whole-core pool",
                })
                .show_ui(ui, |ui| {
                    let priority = ui.selectable_value(
                        &mut config.cpu_mode,
                        CpuMode::PriorityEco,
                        "Priority + EcoQoS",
                    );
                    crate::debug_mcp_control!("tweaks.background.policy.priority", priority);
                    ui.add_enabled_ui(efficient, |ui| {
                        let efficient = ui.selectable_value(
                            &mut config.cpu_mode,
                            CpuMode::EfficientCores,
                            "Prefer efficient cores",
                        );
                        crate::debug_mcp_control!("tweaks.background.policy.efficient", efficient);
                    });
                    ui.add_enabled_ui(!topology.is_empty(), |ui| {
                        let manual = ui.selectable_value(
                            &mut config.cpu_mode,
                            CpuMode::ManualPool,
                            "Manual whole-core pool",
                        );
                        crate::debug_mcp_control!("tweaks.background.policy.manual", manual);
                    });
                });
            crate::debug_mcp_control!("tweaks.background.policy", combo.response);
        });
    });
    let mut changed = before != config.cpu_mode;
    if changed {
        config.manual_ids.clear();
        config.manual_topology = if config.cpu_mode == CpuMode::ManualPool {
            topology.to_vec()
        } else {
            vec![]
        };
    }
    if let Some(error) = &background.topology_error {
        ui.add(egui::Label::new(format!("CPU pool unavailable: {error}")).wrap());
    } else if topology.is_empty() {
        ui.add(
            egui::Label::new(
                "Check applicability to read physical-core topology and choose an optional pool.",
            )
            .wrap(),
        );
    }
    if config.cpu_mode == CpuMode::ManualPool && !topology.is_empty() {
        ui.label("Choose complete physical cores (all SMT threads stay together)");
        let mut cores = std::collections::BTreeMap::<_, Vec<_>>::new();
        for cpu in topology {
            cores
                .entry((cpu.group, cpu.physical_core))
                .or_default()
                .push(cpu);
        }
        ui.horizontal_wrapped(|ui| {
            for ((group, core), cpus) in cores {
                let mut selected = config.manual_topology == topology
                    && cpus.iter().all(|cpu| config.manual_ids.contains(&cpu.id));
                let available = cpus.iter().all(|cpu| cpu.allowed && !cpu.reserved);
                let response = ui.add_enabled(
                    !pending && available,
                    egui::Checkbox::new(&mut selected, format!("G{group} · core {core}")),
                );
                crate::debug_mcp_control!(
                    &format!("tweaks.background.core.{group}.{core}"),
                    response
                );
                if response.changed() {
                    changed |= toggle_core(config, topology, group, core, selected);
                }
                response.on_hover_ui(|ui| {
                    ui.set_max_width(
                        260.0_f32
                            .min(ui.ctx().content_rect().width() - 16.0)
                            .max(80.0),
                    );
                    let logicals = cpus
                        .iter()
                        .map(|cpu| cpu.logical.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.label(format!(
                        "Logical CPUs {logicals} · efficiency class {} · cache {} · NUMA {}",
                        cpus[0].efficiency_class, cpus[0].cache, cpus[0].numa
                    ));
                    if !available {
                        ui.label("Reserved or allocated to another process; unavailable.");
                    }
                });
            }
        });
        if config.manual_ids.is_empty() {
            ui.add(
                egui::Label::new("No pool selected. Only priority and EcoQoS can apply.").wrap(),
            );
        }
    }
    if let Some(status) = &background.placement_status {
        ui.add(egui::Label::new(status).wrap());
    }
    changed
}

pub(crate) fn draw(
    ui: &mut egui::Ui,
    snapshot: &TuningSnapshot,
    pending: bool,
) -> Option<TuningRequest> {
    let background = &snapshot.background;
    let mut config = background.config.clone();
    let mut enabled = background.enabled;
    let mut changed = false;
    let mut request = None;
    let colors = crate::app::shell::presenters::shared_elements::palette(ui);
    egui::Frame::group(ui.style()).inner_margin(8.0).show(ui,|ui|{
        ui.label(egui::RichText::new("Reduce background impact").strong());
        ui.add(egui::Label::new(egui::RichText::new("Temporarily lower background CPU priority and use EcoQoS while your selected games run. Undo restores this app's changes.").size(10.5).color(colors.text_muted)).wrap());
        ui.add(egui::Label::new(egui::RichText::new(&background.status).size(10.5)).wrap());
        let response=ui.add_enabled(!pending && !config.games.is_empty(),egui::Checkbox::new(&mut enabled,"Enable for selected games"));
        crate::debug_mcp_control!("tweaks.background.enable",response);
        changed|=response.changed();
        let choices=background_policy::game_choices(&snapshot.state,&snapshot.rules);
        let games=egui::CollapsingHeader::new(format!("Selected games ({})",config.games.len())).id_salt("background-games").default_open(config.games.is_empty()).show(ui,|ui|{
            if choices.is_empty(){ ui.add(egui::Label::new("Add a game's EXE in Affinity groups first. Store activation targets are not supported here.").wrap()); }
            for choice in &choices {
                let mut selected=config.games.iter().any(|game|game.rule==choice.rule);
                let response=ui.add_enabled(!pending && (selected || config.games.len()<64),egui::Checkbox::new(&mut selected,&choice.label));
                crate::debug_mcp_control!(&format!("tweaks.background.game.{}",choice.rule),response);
                if response.changed(){
                    config.games.retain(|game|game.rule!=choice.rule);
                    if selected { config.games.push(GameRule{group:choice.group.clone(),rule:choice.rule.clone()}); }
                    changed=true;
                }
            }
            let missing:Vec<_>=config.games.iter().filter(|game|!choices.iter().any(|choice|choice.rule==game.rule)).cloned().collect();
            for game in missing {
                let mut selected=true;
                if ui.add_enabled(!pending,egui::Checkbox::new(&mut selected,format!("Unavailable saved game ({})",game.rule))).changed(){config.games.retain(|saved|saved.rule!=game.rule);changed=true;}
            }
            ui.add(egui::Label::new(egui::RichText::new("Runs until the game exits, including while you Alt-Tab. The game itself is never a background target.").size(10.0).color(colors.text_muted)).wrap());
        });
        crate::debug_mcp_control!("tweaks.background.games",games.header_response);
        ui.add_space(3.0);
        changed|=pool_controls(ui,background,&mut config,pending);
        ui.horizontal_wrapped(|ui|{
            let check=ui.add_enabled(!pending,egui::Button::new("Check applicability"));
            crate::debug_mcp_control!("tweaks.background.inspect",check);
            if check.clicked(){request=Some(TuningRequest::InspectBackground);}
            let preview=ui.add_enabled(!pending && background.preview_until.is_none(),egui::Button::new("Preview for 10 min"));
            crate::debug_mcp_control!("tweaks.background.preview",preview);
            if preview.clicked(){request=Some(TuningRequest::PreviewBackground);}
            let undo=ui.add_enabled(!pending && (enabled || background.preview_until.is_some() || background.changed_targets>0),egui::Button::new("Undo now"));
            crate::debug_mcp_control!("tweaks.background.undo",undo);
            if undo.clicked(){request=Some(TuningRequest::UndoBackground);}
        });
        ui.add(egui::Label::new(egui::RichText::new("Local preview · System roles remain unchanged until controlled Windows VM validation is complete.").size(10.0).color(colors.warning.fg)).wrap());
        if background.preview_until.is_some(){ui.add(egui::Label::new("Preview stops automatically after 10 minutes. Closing this app also restores session changes.").wrap());}
        let details=egui::CollapsingHeader::new("Background roles and details").id_salt("background-details").show(ui,|ui|{
            ui.add(egui::Label::new("Signed images and verified service/task roles are checked separately from permission to change them. Access denied and unqualified roles are skipped.").wrap());
            for role in [os_api::background::Role::Appraiser,os_api::background::Role::DeviceCensus,os_api::background::Role::FrameworkOptimizer,os_api::background::Role::DiagnosticsTelemetry] {
                let mut include=!config.excluded_roles.contains(&role);
                if ui.add_enabled(!pending,egui::Checkbox::new(&mut include,role.label())).changed(){config.excluded_roles.retain(|saved|*saved!=role);if !include{config.excluded_roles.push(role);}changed=true;}
            }
            ui.separator();
            if !background.inspected {ui.label("Use Check applicability to inspect running background work.");}
            else if background.candidates.is_empty(){ui.label("No catalogued background tasks are running. Nothing changed.");}
            else {
                for row in &background.candidates {
                    ui.label(egui::RichText::new(format!("{} · PID {}",row.role,row.pid)).strong());
                    ui.add(egui::Label::new(egui::RichText::new(&row.status).size(10.0).color(colors.text_muted)).wrap());
                }
            }
            ui.add_space(3.0);
            if !background.events.is_empty(){
                egui::CollapsingHeader::new("Recent session events").id_salt("background-events").show(ui,|ui|{
                    for event in background.events.iter().rev(){ui.add(egui::Label::new(egui::RichText::new(event).size(10.0)).wrap());}
                });
            }
            ui.add(egui::Label::new(egui::RichText::new("This does not stop services, block updates, cap CPU usage or change download bandwidth. Use Windows or the download app's settings for network limits. A performance gain is not guaranteed.").size(10.0).color(colors.text_muted)).wrap());
        });
        crate::debug_mcp_control!("tweaks.background.details",details.header_response);
    });
    if changed {
        if config.games.is_empty() {
            enabled = false;
        }
        request = Some(TuningRequest::ConfigureBackground {
            enabled,
            config: Box::new(config),
        });
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_pool_selects_whole_physical_core_and_refuses_unavailable_sibling() {
        let mut topology: Vec<_> = (0..4)
            .map(|index| os_api::background::PlacementCpu {
                id: index,
                group: 1,
                logical: index as u8,
                physical_core: index / 2,
                efficiency_class: 0,
                numa: 1,
                cache: 2,
                allowed: true,
                reserved: false,
            })
            .collect();
        let mut config = super::super::tweaks::background_workload::Config::default();
        assert!(toggle_core(&mut config, &topology, 1, 0, true));
        assert_eq!(config.manual_ids, vec![0, 1]);
        assert!(toggle_core(&mut config, &topology, 1, 0, false));
        assert!(config.manual_ids.is_empty());
        topology[1].reserved = true;
        assert!(!toggle_core(&mut config, &topology, 1, 0, true));
        assert!(config.manual_ids.is_empty());
    }
}
