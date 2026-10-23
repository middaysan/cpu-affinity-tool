//! One per-app checkbox. No native calls, CPU picker or worker ownership.
#![cfg(any(test, all(target_os = "windows", feature = "windows")))]
use super::background_policy;
use eframe::egui;

pub(crate) fn draw_rule(
    ui: &mut egui::Ui,
    draft: &mut background_policy::RulePolicy,
    supported: bool,
    pending: bool,
) -> bool {
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
    if draft.enabled && supported {
        ui.add(egui::Label::new(egui::RichText::new(
            "Local preview: Windows system tasks remain unchanged until role validation is complete."
        ).small().weak()).wrap());
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_checkbox_has_no_advanced_or_core_controls_even_when_enabled() {
        for width in [240.0, 420.0] {
            let ctx = egui::Context::default();
            let mut draft = background_policy::RulePolicy::default();
            let mut action = true;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.set_width(width);
                action = draw_rule(ui, &mut draft, true, false);
            });
            output.textures_delta.clear();
            assert!(!action);
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
                draw_rule(ui, &mut draft, true, false);
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
            assert!(!text.contains("Advanced"), "{text}");
            assert!(
                !text.contains("Load CPU list"),
                "advanced controls should start collapsed: {text}"
            );
            assert_eq!(
                draft.config.cpu_mode,
                super::super::tweaks::background_workload::CpuMode::PriorityEco
            );
        }
    }
}
