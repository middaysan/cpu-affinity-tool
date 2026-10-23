//! Cached background-session presentation. No native calls or worker ownership.
#![cfg(any(test, all(target_os = "windows", feature = "windows")))]
use super::background_policy;
use super::tweaks::background_workload::{Config, CpuMode};
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
        ui.label("Background CPU placement");
        ui.add_enabled_ui(!pending, |ui| {
            let combo = egui::ComboBox::from_id_salt("background-cpu-mode")
                .selected_text(match config.cpu_mode {
                    CpuMode::PriorityEco => "No core restriction",
                    CpuMode::EfficientCores => "Use efficient cores",
                    CpuMode::ManualPool => "Choose physical cores",
                })
                .show_ui(ui, |ui| {
                    let priority = ui.selectable_value(
                        &mut config.cpu_mode,
                        CpuMode::PriorityEco,
                        "No core restriction",
                    );
                    crate::debug_mcp_control!("rule.background.policy.priority", priority);
                    ui.add_enabled_ui(efficient, |ui| {
                        let efficient = ui.selectable_value(
                            &mut config.cpu_mode,
                            CpuMode::EfficientCores,
                            "Use efficient cores",
                        );
                        crate::debug_mcp_control!("rule.background.policy.efficient", efficient);
                    });
                    ui.add_enabled_ui(!topology.is_empty(), |ui| {
                        let manual = ui.selectable_value(
                            &mut config.cpu_mode,
                            CpuMode::ManualPool,
                            "Choose physical cores",
                        );
                        crate::debug_mcp_control!("rule.background.policy.manual", manual);
                    });
                });
            crate::debug_mcp_control!("rule.background.policy", combo.response);
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
            egui::Label::new("Load the CPU list to choose an optional core restriction.").wrap(),
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
                    &format!("rule.background.core.{group}.{core}"),
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
                egui::Label::new("No cores selected. Background CPU placement stays unrestricted.")
                    .wrap(),
            );
        }
    }
    changed
}

pub(crate) fn draw_rule(
    ui: &mut egui::Ui,
    background: &background_policy::Snapshot,
    draft: &mut background_policy::RulePolicy,
    supported: bool,
    pending: bool,
) -> (bool, bool) {
    let response = ui.add_enabled(
        !pending && (supported || draft.enabled),
        egui::Checkbox::new(
            &mut draft.enabled,
            "Reduce background load while this app runs",
        ),
    );
    crate::debug_mcp_control!("rule.background.enable", response);
    let changed = response.changed();
    response.on_hover_ui(|ui| {
        ui.set_max_width(260.0_f32.min(ui.ctx().content_rect().width() - 16.0).max(80.0));
        ui.add(egui::Label::new(if supported {
            "Temporarily lower the priority and energy use of qualified Windows background tasks while this app is running. Restore this program's changes after the app exits. Your app's priority and affinity stay separate."
        } else {
            "Requires an executable (.exe) launch target. Installed-app activation targets are not supported."
        }).wrap());
    });
    let mut pool_changed = false;
    let mut inspect = false;
    if draft.enabled && supported {
        ui.add(egui::Label::new(egui::RichText::new(
            "Local preview: Windows system tasks remain unchanged until role validation is complete."
        ).small().weak()).wrap());
        let advanced = egui::CollapsingHeader::new("Advanced")
            .id_salt("rule-background-advanced")
            .show(ui, |ui| {
                ui.add(egui::Label::new("Optional CPU placement for background tasks, separate from this app's CPU affinity.").wrap());
                let load = ui.add_enabled(!pending, egui::Button::new("Load CPU list"));
                crate::debug_mcp_control!("rule.background.load_cpus", load);
                inspect = load.clicked();
                pool_changed = pool_controls(ui, background, &mut draft.config, pending);
            });
        crate::debug_mcp_control!("rule.background.advanced", advanced.header_response);
    }
    (changed || pool_changed, inspect)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rule_background_has_one_default_off_draft_and_collapsed_core_options() {
        for width in [240.0, 420.0] {
            let ctx = egui::Context::default();
            let snapshot = background_policy::Snapshot::default();
            let mut draft = background_policy::RulePolicy::default();
            let mut action = (true, true);
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.set_width(width);
                action = draw_rule(ui, &snapshot, &mut draft, true, false);
            });
            output.textures_delta.clear();
            assert_eq!(action, (false, false));
            let text = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                text.contains("Reduce background load while this app runs"),
                "{text}"
            );
            assert!(!text.contains("Advanced"), "{text}");
            assert!(!text.contains("CPU placement"), "{text}");
            assert!(!draft.enabled);
            draft.enabled = true;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.set_width(width);
                draw_rule(ui, &snapshot, &mut draft, true, false);
            });
            output.textures_delta.clear();
            let text = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("Advanced"), "{text}");
            assert!(
                !text.contains("Load CPU list"),
                "advanced controls should start collapsed: {text}"
            );
            assert_eq!(draft.config.cpu_mode, CpuMode::PriorityEco);
        }
    }
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
