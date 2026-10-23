#[cfg(test)]
use crate::app::features::system_tweaks::{
    config::ModulePreference, platform::contract::ProfileId,
};
use crate::app::{
    features::system_tweaks::{
        platform::contract::{Capability, Effect, ModuleStatus, TargetScope},
        presentation::{status_label, ModuleAction, TuningPageSection, TweakSection},
        PcCheck, SaveError, TuningConfigurationState, TuningOperationState, TuningRequest,
        TweakCard,
    },
    runtime::AppState,
    shell::presenters::shared_elements::{
        close_button, content_frame, glass_frame, paint_focus_ring, palette, row_fill,
        toned_sized_button, ToneRole,
    },
};
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const MEMORY_INTEGRITY_HELP: &str = "Opens Windows Security > Device security > Core isolation. Find Memory integrity on that page; its label is translated in localized Windows. This shortcut opens the page only, without focusing or highlighting the switch.\n\nMemory integrity protects the Windows kernel. Turning it off reduces protection; a performance gain is not guaranteed. Choose the setting yourself in Windows and restart only if Windows asks you to.\n\nThis app does not read or change this setting and does not save an Undo baseline. To restore your choice, return to the same Windows page. If the shortcut is unavailable, open Windows Security > Device security > Core isolation details manually.";

pub(crate) fn save_error_message(error: SaveError) -> &'static str {
    match error {
        SaveError::Storage => "Could not save. Your previous settings are retained.",
        SaveError::Invalid => "Settings changed or are unavailable. Refresh and try again.",
        SaveError::Pending => "Another operation is pending. Wait for it to finish.",
        SaveError::Recovering => "Recovery needs attention before more changes can be made.",
    }
}
pub(crate) fn submit(app: &mut AppState, request: TuningRequest) -> bool {
    app.ui.tuning.error = app.request_tuning(request).err().map(save_error_message);
    app.ui.tuning.error.is_none()
}
fn busy(operation: &TuningOperationState) -> bool {
    matches!(operation, TuningOperationState::Pending { .. })
}
#[cfg(test)]
fn current_preference(
    app: &AppState,
    profile: ProfileId,
    descriptor: &crate::app::features::system_tweaks::platform::contract::ModuleDescriptor,
) -> Option<ModulePreference> {
    app.tuning_snapshot()
        .profiles
        .iter()
        .find(|p| p.id == profile)
        .and_then(|p| p.modules.iter().find(|m| m.feature == descriptor.id))
        .map(|m| ModulePreference {
            feature: m.feature.clone(),
            enabled: m.enabled,
            mode: m.mode,
            config: m.config.clone(),
        })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TweakActionKind {
    Managed,
    Settings,
    Guide,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompactAction {
    Apply,
    Applied,
    AlreadyConfigured,
    Unavailable,
    Working,
    Settings,
    Help,
    Review,
}
struct TweakControls {
    action: CompactAction,
    apply: bool,
    undo: bool,
}
fn primary_action_enabled(controls: &TweakControls, platform_available: bool) -> bool {
    platform_available
        && (controls.apply
            || matches!(
                controls.action,
                CompactAction::Settings
                    | CompactAction::Help
                    | CompactAction::Unavailable
                    | CompactAction::Review
            ))
}
fn tweak_controls(
    status: Option<ModuleStatus>,
    capability: &Capability,
    ready: bool,
    owned: bool,
    pending: bool,
    kind: TweakActionKind,
) -> TweakControls {
    let (action, apply, undo) = if pending {
        (CompactAction::Working, false, false)
    } else if kind == TweakActionKind::Settings {
        (CompactAction::Settings, false, false)
    } else if kind == TweakActionKind::Guide {
        (CompactAction::Help, false, false)
    } else if matches!(
        status,
        Some(ModuleStatus::VerifiedActive(_) | ModuleStatus::StoredRestartRequired)
    ) {
        (
            if owned {
                CompactAction::Applied
            } else {
                CompactAction::AlreadyConfigured
            },
            false,
            owned,
        )
    } else if owned {
        (CompactAction::Review, false, true)
    } else if ready && matches!(capability, Capability::Available) {
        (CompactAction::Apply, true, false)
    } else {
        (CompactAction::Unavailable, false, false)
    };
    TweakControls {
        action,
        apply,
        undo,
    }
}

pub(crate) fn draw_system_tweaks(app: &mut AppState, root_ui: &mut egui::Ui) {
    if !app.modules_enabled() {
        return;
    }
    let snapshot = app.tuning_snapshot();
    let editable = matches!(
        snapshot.configuration,
        TuningConfigurationState::Off | TuningConfigurationState::Known
    );
    let pending = busy(&snapshot.operation);
    egui::CentralPanel::default()
        .frame(content_frame(root_ui))
        .show(root_ui, |ui| {
            ui.set_width(ui.available_width().min(400.0));
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("System Tweaks").size(19.0).strong());
                help_button(app, ui, "System Tweaks", "Apply individual settings when they suit your PC. Opening this page changes nothing.\n\nThis app reads the current value before a change and retains it for Undo. It only restores fields that still match its own verified change. Later changes by Windows or other programs are preserved; an uncertain or skipped restore is shown.\n\nManual changes remain after closing the app. Windows decides when Game Mode and graphics preferences affect a game. A stored preference is not proof of higher FPS or lower latency. CPU affinity and process priority remain in Affinity groups.\n\nHardware guides and Check PC provide information. They do not apply a performance bundle.");
            });
            ui.add_space(3.0);
            draw_section_selector(app, ui);
            ui.add_space(3.0);
            draw_operation_state(app, ui);
            if !editable {
                ui.colored_label(palette(ui).warning.fg, "Saved settings are retained and read-only because this version cannot safely understand them.");
            }
            if let Some(effect) = snapshot.restoration_effect {
                let partial = snapshot.operation_error.is_some()
                    || !matches!(snapshot.operation, TuningOperationState::Idle);
                let message = match (partial, effect) {
                    (false, Effect::Immediate) => "Original settings restored and verified.",
                    (false, Effect::GameRestartRequired) => "Original settings restored. Restart the game to use them.",
                    (false, Effect::RestartRequired) => "Original settings restored. Restart Windows to use them.",
                    (true, Effect::Immediate) => "Some original settings restored. Remaining changes need attention.",
                    (true, Effect::GameRestartRequired) => "Some original settings restored. Restart the game to use them. Remaining changes need attention.",
                    (true, Effect::RestartRequired) => "Some original settings restored. Restart Windows to use them. Remaining changes need attention.",
                };
                let color = if partial { palette(ui).warning.fg } else { palette(ui).success.fg };
                ui.add(egui::Label::new(egui::RichText::new(message).size(10.5).color(color)).wrap());
            }
            ui.horizontal_wrapped(|ui| {
                let response = ui.add_enabled(!pending && editable && snapshot.has_tweak_changes, egui::Button::new(egui::RichText::new("Undo my changes").size(10.5)));
                crate::debug_mcp_control!("tweaks.undo_all", response);
                if response.clicked() {
                    app.ui.tuning.confirm_undo_all = true;
                }
                ui.label(egui::RichText::new("CPU controls: Affinity groups").size(10.0).color(palette(ui).text_muted));
            });
            ui.separator();
            let selected = app.ui.tuning.selected_section;
            let scroll = egui::ScrollArea::vertical()
                .id_salt(("system-tweaks-content", selected))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    match selected {
                        TuningPageSection::System => {
                            ui.add(egui::Label::new(egui::RichText::new("Manual settings · stay changed until you undo them").size(10.0).color(palette(ui).text_muted)).wrap());
                            if snapshot.catalog.is_empty() { ui.label("No modules are available in this build."); }
                            else if snapshot.tweak_cards.is_empty() { ui.label("Reading current Windows settings…"); }
                            ui.add_space(4.0);
                            section_heading(ui, "FOR GAMES", None);
                            draw_section(app, ui, TweakSection::Games, pending || !editable);
                            draw_memory_integrity_guide(app, ui, pending);
                            ui.add_space(8.0);
                            section_heading(ui, "FOR COMFORT", Some("No FPS increase"));
                            draw_section(app, ui, TweakSection::Comfort, pending || !editable);
                            draw_previous_settings(app, ui, pending || !editable);
                            ui.add_space(8.0);
                            section_heading(ui,"WHILE GAMES RUN",Some("Temporary sessions"));
                            if let Some(request)=crate::app::features::system_tweaks::background_ui::draw(ui,&snapshot,pending || !editable) { submit(app,request); }
                        }
                        TuningPageSection::Hardware => draw_hardware(app, ui, pending || !editable),
                        TuningPageSection::PcChecks => draw_pc_checks(app, ui, pending),
                    }
                });
            #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
            crate::app::features::diagnostics::debug_mcp::rect_control(ui.ctx(), "tweaks.body", scroll.inner_rect, true, serde_json::Value::Null);
            #[cfg(not(all(target_os = "windows", debug_assertions, feature = "debug-mcp")))]
            let _ = scroll;
        });
    draw_dialogs(app, root_ui.ctx());
}

fn draw_section_selector(app: &mut AppState, ui: &mut egui::Ui) {
    let track_width = ui.available_width().min(300.0);
    let sections = [
        (
            TuningPageSection::System,
            "System",
            "System settings: games and comfort",
        ),
        (
            TuningPageSection::Hardware,
            "GPU / network",
            "Graphics preference and network guides",
        ),
        (
            TuningPageSection::PcChecks,
            "Check PC",
            "Read-only hardware information. Choose Check PC inside to run the check.",
        ),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(((ui.available_width() - track_width) * 0.5).max(0.0));
        egui::Frame::NONE
            .fill(row_fill(ui))
            .stroke(ui.visuals().widgets.inactive.bg_stroke)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(2))
            .show(ui, |ui| {
                ui.set_width((track_width - 4.0).max(1.0));
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().button_padding.x = 3.0;
                ui.horizontal(|ui| {
                    let width = ui.available_width() / 3.0;
                    for (section, title, help) in sections {
                        let selected = app.ui.tuning.selected_section == section;
                        let label = egui::RichText::new(title).size(10.0);
                        let button =
                            egui::Button::new(if selected { label.strong() } else { label })
                                .selected(selected);
                        let response = if selected {
                            toned_sized_button(ui, [width, 20.0], button, ToneRole::Selected)
                        } else {
                            let response = ui.add_sized(
                                [width, 20.0],
                                button
                                    .fill(egui::Color32::TRANSPARENT)
                                    .stroke(egui::Stroke::NONE),
                            );
                            paint_focus_ring(ui, &response);
                            response
                        };
                        crate::debug_mcp_control!(
                            format!(
                                "tweaks.section.{}",
                                match section {
                                    TuningPageSection::System => "system",
                                    TuningPageSection::Hardware => "hardware",
                                    TuningPageSection::PcChecks => "checks",
                                }
                            ),
                            response
                        );
                        if response.on_hover_text(help).clicked() {
                            app.ui.tuning.selected_section = section;
                        }
                    }
                });
            });
    });
}

fn draw_hardware(app: &mut AppState, ui: &mut egui::Ui, pending: bool) {
    section_heading(ui, "GRAPHICS PREFERENCE", None);
    draw_section(app, ui, TweakSection::Hardware, pending);
    ui.add_space(8.0);
    section_heading(ui, "GUIDES", None);
    glass_frame(ui)
        .inner_margin(egui::Margin::same(0))
        .show(ui, |ui| {
            for (index, guide) in
                crate::app::features::system_tweaks::pc_checks::hardware_guidance()
                    .into_iter()
                    .enumerate()
            {
                if index != 0 {
                    row_separator(ui);
                }
                ui.push_id(guide.id, |ui| {
                    ui.horizontal(|ui| {
                        let width =
                            (ui.available_width() - 112.0 - ui.spacing().item_spacing.x).max(0.0);
                        ui.add_sized(
                            [width, 25.0],
                            egui::Label::new(egui::RichText::new(guide.title).size(11.0)).wrap(),
                        )
                        .on_hover_text(&guide.summary);
                        let response = ui.add_sized(
                            [112.0, 25.0],
                            egui::Button::new(egui::RichText::new("Read guide").size(10.5)),
                        );
                        crate::debug_mcp_control!(format!("tweaks.guide.{}", guide.id), response);
                        if response.clicked() {
                            app.ui.tuning.tweak_help = Some((guide.title, guide.help.to_owned()));
                        }
                    });
                });
            }
        });
}

fn draw_pc_checks(app: &mut AppState, ui: &mut egui::Ui, pending: bool) {
    let snapshot = app.tuning_snapshot();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new("Read-only hardware information")
                .size(10.5)
                .color(palette(ui).text_secondary),
        );
        let response = ui.add_enabled(
            !pending,
            egui::Button::new(egui::RichText::new("Check PC").size(10.5)),
        );
        crate::debug_mcp_control!("tweaks.check_pc", response);
        if response.clicked() {
            submit(app, TuningRequest::RefreshPcChecks);
        }
    });
    if snapshot.pc_checks.is_empty() {
        ui.add(egui::Label::new("Use Check PC to read your hardware information.").wrap());
    } else {
        glass_frame(ui)
            .inner_margin(egui::Margin::same(0))
            .show(ui, |ui| {
                for (index, check) in snapshot.pc_checks.iter().enumerate() {
                    if index != 0 {
                        row_separator(ui);
                    }
                    ui.push_id(check.id, |ui| draw_pc_check(app, ui, check));
                }
            });
    }
}

fn draw_memory_integrity_guide(app: &mut AppState, ui: &mut egui::Ui, pending: bool) {
    glass_frame(ui)
        .inner_margin(egui::Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 112.0 - ui.spacing().item_spacing.x).max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 25.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(width);
                        ui.label(egui::RichText::new("Memory integrity").size(11.0).strong());
                        help_button(app, ui, "Memory integrity", MEMORY_INTEGRITY_HELP);
                    },
                );
                if ui
                    .add_enabled(
                        !pending,
                        egui::Button::new(egui::RichText::new("Open settings").size(10.5))
                            .min_size(egui::vec2(112.0, 25.0)),
                    )
                    .clicked()
                {
                    submit(app, TuningRequest::OpenCoreIsolation);
                }
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Turning this off reduces protection · Undo in Windows")
                        .size(9.5)
                        .color(palette(ui).text_secondary),
                )
                .wrap(),
            );
        });
}
fn draw_operation_state(app: &mut AppState, ui: &mut egui::Ui) {
    let snapshot = app.tuning_snapshot();
    match snapshot.operation {
        TuningOperationState::Pending { .. } => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small("Working…");
            });
        }
        TuningOperationState::Refused { error, .. }
            if error != SaveError::Recovering || snapshot.operation_error.is_none() =>
        {
            ui.colored_label(palette(ui).danger.fg, save_error_message(error));
        }
        TuningOperationState::Recovering => {
            ui.colored_label(
                palette(ui).warning.fg,
                "A previous change needs attention. Applying settings is paused.",
            );
        }
        _ => {}
    }
    if let Some(error) = app.ui.tuning.error {
        ui.colored_label(palette(ui).danger.fg, error);
    }
    if let Some(error) = &snapshot.operation_error {
        ui.colored_label(
            palette(ui).danger.fg,
            "Could not complete this action. Open Error details for more information.",
        );
        egui::CollapsingHeader::new("Error details").show(ui, |ui| draw_operation_error(ui, error));
    }
}
fn section_heading(ui: &mut egui::Ui, title: &str, note: Option<&str>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(10.0)
                .strong()
                .color(palette(ui).text_secondary),
        );
        if let Some(note) = note {
            ui.label(
                egui::RichText::new(note)
                    .size(10.0)
                    .color(palette(ui).text_muted),
            );
        }
    });
}
fn draw_section(app: &mut AppState, ui: &mut egui::Ui, section: TweakSection, pending: bool) {
    let snapshot = app.tuning_snapshot();
    let mut cards: Vec<_> = snapshot
        .tweak_cards
        .iter()
        .filter(|card| card.spec.section == section)
        .collect();
    cards.sort_by_key(|card| card.spec.order);
    if cards.is_empty() {
        return;
    }
    glass_frame(ui)
        .inner_margin(egui::Margin::same(0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            for (index, card) in cards.into_iter().enumerate() {
                if index != 0 {
                    row_separator(ui);
                }
                ui.push_id(card.feature.as_str(), |ui| {
                    draw_tweak(app, ui, card, pending)
                });
            }
        });
}
fn row_separator(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.separator();
    ui.spacing_mut().item_spacing.y = 5.0;
}
fn current_value_text<'a>(capability: &Capability, summary: Option<&'a str>) -> &'a str {
    if matches!(capability, Capability::Denied(_)) {
        "Current setting unavailable"
    } else {
        summary.map_or("Not read yet", |text| {
            text.lines().next().unwrap_or("No value available")
        })
    }
}
fn unavailable_text(capability: &Capability) -> Option<&str> {
    match capability {
        Capability::Denied(_) => Some("Windows denied access. Open details for the reason."),
        Capability::Unsupported(reason) => Some(reason),
        _ => None,
    }
}
fn draw_tweak(app: &mut AppState, ui: &mut egui::Ui, card: &TweakCard, pending: bool) {
    let platform_available = card.availability.is_available();
    let kind = if card.spec.apply_label.eq_ignore_ascii_case("Open settings") {
        TweakActionKind::Settings
    } else if card.spec.apply_label.eq_ignore_ascii_case("Read guide")
        || card.spec.apply_label.eq_ignore_ascii_case("More")
    {
        TweakActionKind::Guide
    } else {
        TweakActionKind::Managed
    };
    let controls = tweak_controls(
        card.status,
        &card.capability,
        card.ready,
        card.owned,
        pending,
        kind,
    );
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let title = egui::RichText::new(card.spec.title).size(12.0).strong();
                ui.label(if platform_available {
                    title
                } else {
                    title.weak()
                });
                help_button(app, ui, card.spec.title, &tweak_help_text(card));
                let draft = app
                    .tuning_editors()
                    .editor(&card.feature)
                    .and_then(|editor| editor.tweak_options(card.config.as_ref()));
                if let Some(draft) = draft {
                    let available = platform_available
                        && !pending
                        && !card.owned
                        && card.target.is_some()
                        && matches!(card.capability, Capability::Available);
                    if ui
                        .add_enabled(available, egui::Button::new("Settings…").small())
                        .on_hover_text(if card.owned {
                            "Undo this tweak before changing its settings."
                        } else {
                            "Choose a CPU frequency ceiling and high-clock preference."
                        })
                        .clicked()
                    {
                        app.ui.tuning.tweak_options =
                            Some((card.feature.clone(), card.target.clone().unwrap(), draft));
                    }
                }
            });
            ui.label(
                egui::RichText::new(card.spec.scope)
                    .size(10.0)
                    .color(palette(ui).text_muted),
            );
            if app
                .tuning_editors()
                .editor(&card.feature)
                .is_some_and(|editor| editor.selects_saved_exe())
            {
                ui.add_enabled_ui(platform_available, |ui| {
                    draw_saved_exe_picker(app, ui, card, pending)
                });
            }
            ui.horizontal_top(|ui| {
                let current_width =
                    (ui.available_width() - 112.0 - ui.spacing().item_spacing.x).max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(current_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        // Reserve the whole text column even when its value is short.
                        ui.set_width(current_width);
                        let current = current_value_text(
                            &card.capability,
                            card.observed.as_ref().map(|o| o.summary.as_str()),
                        );
                        let response = ui.add(
                            egui::Label::new(
                                egui::RichText::new(current)
                                    .size(11.0)
                                    .color(palette(ui).text_primary),
                            )
                            .wrap(),
                        );
                        if let Some(observed) = &card.observed {
                            response.on_hover_text(&observed.summary);
                        }
                        if let Some(summary) = card.config.as_ref().and_then(|config| {
                            app.tuning_editors()
                                .editor(&card.feature)?
                                .tweak_summary(config)
                        }) {
                            let summary = format!(
                                "{}: {summary}",
                                if card.owned { "Requested" } else { "Saved" }
                            );
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(summary)
                                        .size(10.0)
                                        .color(palette(ui).text_muted),
                                )
                                .wrap(),
                            );
                        }
                        if card.owned {
                            if let Some(baseline) = &card.baseline {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(format!(
                                            "Previous: {}",
                                            baseline.lines().next().unwrap_or("Saved")
                                        ))
                                        .size(10.0)
                                        .color(palette(ui).text_muted),
                                    )
                                    .wrap(),
                                )
                                .on_hover_text(baseline);
                            } else {
                                ui.small("Previous value retained for recovery");
                            }
                        }
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(112.0, 0.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let label = if controls.undo {
                            "Undo"
                        } else {
                            match controls.action {
                                CompactAction::Apply => card.spec.apply_label,
                                CompactAction::Applied => "Applied",
                                CompactAction::AlreadyConfigured => "Already configured",
                                CompactAction::Unavailable => {
                                    if platform_available {
                                        "Why unavailable?"
                                    } else {
                                        "Unavailable"
                                    }
                                }
                                CompactAction::Working => "Working…",
                                CompactAction::Settings => "Open settings",
                                CompactAction::Help => "Read guide",
                                CompactAction::Review => "Check status",
                            }
                        };
                        let enabled =
                            controls.undo || primary_action_enabled(&controls, platform_available);
                        let mut button = egui::Button::new(egui::RichText::new(label).size(10.5));
                        if controls.apply {
                            button = button
                                .fill(palette(ui).primary.fill)
                                .stroke(egui::Stroke::new(1.0, palette(ui).primary.border));
                        }
                        let response =
                            ui.add_enabled(enabled, button.min_size(egui::vec2(112.0, 25.0)));
                        crate::debug_mcp_control!(
                            format!("tweaks.action.{}", card.feature.as_str()),
                            response
                        );
                        if response.clicked() {
                            if controls.undo {
                                submit(
                                    app,
                                    TuningRequest::UndoTweak {
                                        feature: card.feature.clone(),
                                    },
                                );
                            } else {
                                match controls.action {
                                    CompactAction::Apply => {
                                        if let Some(target) = card.target.clone() {
                                            let options = app
                                                .tuning_editors()
                                                .editor(&card.feature)
                                                .and_then(|editor| {
                                                    editor.tweak_options(card.config.as_ref())
                                                });
                                            submit(
                                                app,
                                                match options {
                                                    Some(config) => {
                                                        TuningRequest::ApplyConfiguredTweak {
                                                            feature: card.feature.clone(),
                                                            target,
                                                            config,
                                                        }
                                                    }
                                                    None => TuningRequest::ApplyTweak {
                                                        feature: card.feature.clone(),
                                                        target,
                                                    },
                                                },
                                            );
                                        }
                                    }
                                    CompactAction::Settings => {
                                        submit(
                                            app,
                                            TuningRequest::ModuleAction {
                                                feature: card.feature.clone(),
                                                action: ModuleAction::OpenSettings,
                                            },
                                        );
                                    }
                                    _ => {
                                        app.ui.tuning.tweak_help =
                                            Some((card.spec.title, tweak_help_text(card)));
                                    }
                                }
                            }
                        }
                    },
                );
            });
            if let Some(status) = card.status.filter(|s| *s != ModuleStatus::Unmanaged) {
                let color = match status {
                    ModuleStatus::VerifiedActive(Effect::Immediate) => palette(ui).success.fg,
                    ModuleStatus::PartialFailure
                    | ModuleStatus::PermissionDenied
                    | ModuleStatus::RecoveryRequired => palette(ui).danger.fg,
                    _ => palette(ui).warning.fg,
                };
                let label = if status == ModuleStatus::VerifiedActive(Effect::Immediate) {
                    app.tuning_editors()
                        .editor(&card.feature)
                        .map_or("Current value verified", |editor| {
                            editor.tweak_verified_label()
                        })
                } else {
                    status_label(status)
                };
                ui.label(egui::RichText::new(label).size(10.0).color(color));
            }
            if let Some(reason) = unavailable_text(&card.capability) {
                ui.add(
                    egui::Label::new(egui::RichText::new(reason).size(10.0).color(
                        if platform_available {
                            palette(ui).warning.fg
                        } else {
                            palette(ui).text_muted
                        },
                    ))
                    .wrap(),
                );
            } else if card.observed.is_none() && kind == TweakActionKind::Managed {
                ui.label(
                    egui::RichText::new(
                        "The current value has not been verified. No change is available.",
                    )
                    .size(10.0)
                    .color(palette(ui).warning.fg),
                );
            }
        });
}
fn tweak_help_text(card: &TweakCard) -> String {
    let mut text = card.spec.help.to_owned();
    if let Capability::Unsupported(reason) | Capability::Denied(reason) = &card.capability {
        text.push_str(&format!("\n\nCurrent availability\n{reason}"));
    }
    if let Some(observed) = &card.observed {
        text.push_str(&format!("\n\nCurrent observation\n{}", observed.summary));
    }
    if let Some(baseline) = &card.baseline {
        text.push_str(&format!("\n\nSaved previous value\n{baseline}"));
    }
    text
}
fn saved_exe_choices(state: &crate::app::models::AppStateStorage) -> Vec<(String, PathBuf)> {
    let mut choices = Vec::new();
    for group in &state.groups {
        for rule in &group.programs {
            let Some(path) = rule.bin_path().filter(|path| {
                path.is_absolute()
                    && path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            }) else {
                continue;
            };
            if path.to_str().is_none() {
                continue;
            }
            if !choices.iter().any(|(_, existing)| existing == path) {
                choices.push((
                    format!("{} · {}", group.name, rule.name),
                    path.to_path_buf(),
                ));
            }
        }
    }
    choices
}
fn draw_saved_exe_picker(app: &mut AppState, ui: &mut egui::Ui, card: &TweakCard, pending: bool) {
    let snapshot = app.tuning_snapshot();
    let choices = saved_exe_choices(&snapshot.state);
    if choices.is_empty() {
        ui.small("Add an EXE rule in Affinity groups to choose a game.");
        return;
    }
    let original = card.target.as_ref().and_then(|target| {
        app.tuning_editors()
            .editor(&card.feature)
            .and_then(|editor| editor.tweak_executable(target))
    });
    let mut selected = original.clone();
    let name = choices
        .iter()
        .find(|(_, path)| Some(path) == selected.as_ref())
        .map_or("Choose a saved game EXE", |(name, _)| name.as_str());
    ui.add_enabled_ui(!pending, |ui| {
        let combo = egui::ComboBox::from_id_salt("tweak-saved-exe")
            .selected_text(name)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (index, (name, path)) in choices.iter().enumerate() {
                    let response = ui.selectable_value(&mut selected, Some(path.clone()), name);
                    crate::debug_mcp_control!(format!("tweaks.gpu_exe.{index}"), response);
                    let _ = index;
                }
            });
        crate::debug_mcp_control!("tweaks.gpu_exe", combo.response);
    });
    if selected != original {
        if let Some(path) = selected {
            let Some(target) = app
                .tuning_editors()
                .get(&card.feature)
                .and_then(|module| module.editor().tweak_saved_executable_target(&path))
            else {
                return;
            };
            if submit(
                app,
                TuningRequest::SelectTweakTarget {
                    feature: card.feature.clone(),
                    target: target.clone(),
                },
            ) {
                app.ui.tuning.tweak_target = Some(target);
            }
        }
    }
}
fn draw_pc_check(app: &mut AppState, ui: &mut egui::Ui, check: &PcCheck) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(check.title).size(11.0).strong());
                help_button(app, ui, check.title, check.help);
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&check.summary)
                        .size(10.5)
                        .color(palette(ui).text_secondary),
                )
                .wrap(),
            );
        });
}
fn draw_previous_settings(app: &mut AppState, ui: &mut egui::Ui, pending: bool) {
    let snapshot = app.tuning_snapshot();
    if !snapshot
        .profiles
        .iter()
        .any(|profile| profile.modules.iter().any(|card| card.owned))
    {
        return;
    }
    ui.add_space(8.0);
    ui.label("Changes from an earlier version");
    for profile in &snapshot.profiles {
        for card in profile.modules.iter().filter(|card| card.owned) {
            ui.push_id((profile.id.get(), &card.feature), |ui| {
                glass_frame(ui).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    let action_width = 112.0_f32.min(ui.available_width() * 0.42);
                    let text_width =
                        (ui.available_width() - action_width - ui.spacing().item_spacing.x)
                            .max(1.0);
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(text_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let title = ui.add(
                                    egui::Label::new(egui::RichText::new(&card.title).strong())
                                        .wrap(),
                                );
                                if let Some(descriptor) =
                                    snapshot.catalog.iter().find(|d| d.id == card.feature)
                                {
                                    title.on_hover_text(descriptor.description);
                                }
                                if let Some(status) = card.status {
                                    ui.small(status_label(status));
                                }
                            },
                        );
                        let response = ui
                            .add_enabled_ui(!pending, |ui| {
                                ui.add_sized([action_width, 26.0], egui::Button::new("Undo"))
                            })
                            .inner;
                        crate::debug_mcp_control!(
                            format!(
                                "tweaks.previous.{}.{}",
                                profile.id.get(),
                                card.feature.as_str()
                            ),
                            response
                        );
                        if response.clicked() {
                            submit(
                                app,
                                TuningRequest::UndoPreviousModule {
                                    profile: profile.id,
                                    feature: card.feature.clone(),
                                },
                            );
                        }
                    });
                });
            });
        }
    }
}
fn help_button(app: &mut AppState, ui: &mut egui::Ui, title: &'static str, help: &str) {
    let response = ui
        .add(
            egui::Button::new(
                egui::RichText::new("?")
                    .size(10.0)
                    .color(palette(ui).text_muted),
            )
            .corner_radius(9.0)
            .min_size(egui::vec2(17.0, 17.0)),
        )
        .on_hover_text(help.split("\n\n").next().unwrap_or(help));
    crate::debug_mcp_control!(format!("tweaks.help.{title}"), response);
    if response.clicked() {
        app.ui.tuning.tweak_help = Some((title, help.to_owned()));
    }
}
fn draw_dialogs(app: &mut AppState, context: &egui::Context) {
    let snapshot = app.tuning_snapshot();
    let width = (context.content_rect().width() - 48.0).clamp(160.0, 360.0);
    if let Some(deadline) = snapshot.display_confirmation {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .div_ceil(1000);
        context.request_repaint_after(Duration::from_millis(250));
        let mut keep = false;
        let mut cancel = false;
        let modal = crate::app::shell::presenters::shared_elements::modal(egui::Id::new("tweak-display-confirmation")).show(context, |ui| {
            ui.set_width(width);
            ui.heading("Keep this display mode?");
            ui.label(format!("Check that the picture is clear. The previous mode returns automatically in {remaining} seconds unless you keep it."));
            ui.horizontal_wrapped(|ui| {
                let response = ui.button("Keep this mode");
                crate::debug_mcp_control!("tweaks.display.keep", response);
                keep = response.clicked();
                let response = ui.button("Return previous mode");
                crate::debug_mcp_control!("tweaks.display.revert", response);
                cancel = response.clicked();
            });
        });
        if keep {
            submit(app, TuningRequest::ConfirmDisplayTweak);
        } else if cancel || modal.should_close() {
            if let Some(card) = snapshot.tweak_cards.iter().find(|card| {
                snapshot.catalog.iter().any(|descriptor| {
                    descriptor.id == card.feature
                        && descriptor.target_scopes.contains(&TargetScope::Display)
                })
            }) {
                submit(
                    app,
                    TuningRequest::UndoTweak {
                        feature: card.feature.clone(),
                    },
                );
            }
        }
        return;
    }
    if app.ui.tuning.confirm_undo_all {
        let mut cancel = false;
        let mut confirm = false;
        let modal = crate::app::shell::presenters::shared_elements::modal(egui::Id::new("tweak-undo-all")).show(context, |ui| {
            ui.set_width(width);
            ui.heading("Undo your System Tweaks?");
            ui.label("Return the settings changed here to their saved previous values. Later changes made outside this app are preserved. Any skipped restore is shown.");
            ui.horizontal_wrapped(|ui| {
                let response = ui.button("Cancel");
                crate::debug_mcp_control!("tweaks.undo_all.cancel", response);
                cancel = response.clicked();
                let response = ui.add_enabled(!busy(&snapshot.operation), egui::Button::new("Undo my changes"));
                crate::debug_mcp_control!("tweaks.undo_all.confirm", response);
                confirm = response.clicked();
            });
        });
        if cancel || modal.should_close() {
            app.ui.tuning.confirm_undo_all = false;
        }
        if confirm && submit(app, TuningRequest::UndoAllTweaks) {
            app.ui.tuning.confirm_undo_all = false;
        }
        return;
    }
    if let Some((feature, target, mut config)) = app.ui.tuning.tweak_options.clone() {
        let mut cancel = false;
        let mut apply = false;
        let available = snapshot.tweak_cards.iter().any(|card| {
            card.feature == feature
                && card.target.as_ref() == Some(&target)
                && !card.owned
                && card.availability.is_available()
                && matches!(card.capability, Capability::Available)
        }) && !busy(&snapshot.operation);
        let modal =
            crate::app::shell::presenters::shared_elements::modal(egui::Id::new("tweak-options"))
                .show(context, |ui| {
                    ui.set_width(width);
                    ui.heading("AC power settings");
                    if let Some(editor) = app.tuning_editors().editor(&feature) {
                        editor.show_tweak_options(ui, &mut config);
                        ui.separator();
                        ui.horizontal(|ui| {
                            cancel = ui.button("Cancel").clicked();
                            apply = ui
                                .add_enabled(
                                    available && editor.accepts_config(&config),
                                    egui::Button::new("Apply"),
                                )
                                .clicked();
                        });
                    }
                });
        app.ui.tuning.tweak_options = Some((feature.clone(), target.clone(), config.clone()));
        if cancel || modal.should_close() {
            app.ui.tuning.tweak_options = None;
        }
        if apply
            && submit(
                app,
                TuningRequest::ApplyConfiguredTweak {
                    feature,
                    target,
                    config,
                },
            )
        {
            app.ui.tuning.tweak_options = None;
        }
        return;
    }
    if let Some((title, help)) = app.ui.tuning.tweak_help.clone() {
        let mut close = false;
        let modal =
            crate::app::shell::presenters::shared_elements::modal(egui::Id::new("tweak-help"))
                .show(context, |ui| {
                    ui.set_width(width);
                    ui.label(egui::RichText::new(title).size(16.0).strong());
                    egui::ScrollArea::vertical()
                        .id_salt("tweak-help-text")
                        .max_height((context.content_rect().height() - 180.0).clamp(40.0, 420.0))
                        .show(ui, |ui| {
                            for paragraph in help.split("\n\n") {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(paragraph).size(12.0))
                                        .wrap(),
                                );
                                ui.add_space(6.0);
                            }
                        });
                    let response = close_button(ui);
                    crate::debug_mcp_control!("tweaks.help.close", response);
                    close = response.clicked();
                });
        if close || modal.should_close() {
            app.ui.tuning.tweak_help = None;
        }
    }
}
#[cfg(test)]
fn draw_observation(ui: &mut egui::Ui, summary: &str) {
    egui::ScrollArea::vertical()
        .id_salt(ui.id().with("observation"))
        .max_height(180.0)
        .show(ui, |ui| {
            ui.label(format!("Observed: {summary}"));
        });
}
fn draw_operation_error(
    ui: &mut egui::Ui,
    error: &crate::app::features::system_tweaks::platform::contract::ModuleError,
) {
    let mut message = format!("{}: {:?}", error.operation, error.kind);
    if let Some(detail) = error.detail() {
        message.push_str(&format!(" — {detail}"));
    }
    if let Some(native) = &error.native {
        message.push_str(&format!(" ({}: {})", native.domain.as_str(), native.code));
    }
    egui::ScrollArea::vertical()
        .id_salt(ui.id().with("operation-error-details"))
        .max_height(96.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(message).color(palette(ui).danger.fg)).wrap(),
            );
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn background_card_explains_local_qualification_and_has_explicit_preview_and_undo() {
        let state = std::sync::Arc::new(std::sync::RwLock::new(
            crate::app::models::AppStateStorage::test_fixture(|_| {}),
        ));
        let mut app = empty_modules_app(state);
        let text = rendered_text(&mut app);
        assert!(text.contains("Reduce background impact"), "{text}");
        assert!(text.contains("Check applicability"), "{text}");
        assert!(text.contains("Preview for 10 min"), "{text}");
        assert!(text.contains("System roles remain unchanged"), "{text}");
        finish(&mut app);
    }
    use super::*;
    use crate::app::features::system_tweaks::platform::contract::ActivationMode;
    use crate::app::features::system_tweaks::platform::contract::TargetIdentity;
    use crate::app::models::AppStateStorage;
    use std::{
        sync::{Arc, RwLock},
        time::{Duration, Instant},
    };

    fn rendered_tweak_layout(current: &str, owned: bool) -> (egui::Rect, egui::Rect, String) {
        use crate::app::features::system_tweaks::{
            platform::contract::{FeatureId, ObservationSummary},
            platforms::TweakAvailability,
            presentation::TweakSpec,
        };
        let mut app =
            empty_modules_app(Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))));
        let card = TweakCard {
            config: None,
            feature: FeatureId::new("layout_setting").unwrap(),
            spec: TweakSpec {
                order: 0,
                section: TweakSection::Games,
                title: "Game Mode",
                scope: "Your Windows account",
                apply_label: "Turn on",
                help: "Test help",
            },
            target: Some(TargetIdentity::Machine),
            observed: Some(ObservationSummary {
                summary: current.into(),
                effect: Effect::Immediate,
            }),
            status: owned.then_some(ModuleStatus::VerifiedActive(Effect::Immediate)),
            ready: !owned,
            owned,
            baseline: owned.then(|| "No explicit preference".into()),
            capability: Capability::Available,
            availability: TweakAvailability::Available,
        };
        let mut output = egui::Context::default().run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| draw_tweak(&mut app, ui, &card, false),
        );
        output.textures_delta.clear();
        let buttons: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if (rect.rect.width() - 112.0).abs() < 0.1
                        && (rect.rect.height() - 25.0).abs() < 0.1 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .collect();
        let values: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                )),
                _ => None,
            })
            .collect();
        finish(&mut app);
        assert_eq!(
            buttons.len(),
            1,
            "Each row must contain exactly one action button"
        );
        let value = values.iter().find(|(text, _)| text == current).unwrap().1;
        let text = values
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
            .join("\n");
        (buttons[0], value, text)
    }

    #[test]
    fn compact_layout_actions_align_and_long_values_wrap_before_buttons() {
        let (short_button, short_value, _) = rendered_tweak_layout("Off", false);
        let (long_button, long_value, _) = rendered_tweak_layout(
            "No explicit preference · stored preference with additional setting details",
            false,
        );
        assert_eq!(short_button.left(), long_button.left());
        assert_eq!(short_button.right(), long_button.right());
        assert!(short_button.right() <= 400.0);
        assert!(long_value.right() < long_button.left());
        assert!(long_value.height() > short_value.height());
    }

    #[test]
    fn compact_layout_undo_replaces_apply_in_the_same_slot() {
        let (apply, _, _) = rendered_tweak_layout("Off", false);
        let (undo, _, text) = rendered_tweak_layout("On", true);
        assert_eq!(apply, undo);
        assert!(text.contains("Undo"));
        assert!(!text
            .lines()
            .any(|line| matches!(line, "Applied" | "Turn on")));
        assert!(text.contains("Previous: No explicit preference"));
    }

    #[test]
    fn compact_tweaks_permission_failure_uses_plain_copy_and_keeps_the_reason() {
        let reason = "Denied { operation: Caller and Explorer SID/session/logon differ, code: 5 }";
        let capability = Capability::Denied(reason.into());
        assert_eq!(
            current_value_text(&capability, Some(reason)),
            "Current setting unavailable"
        );
        assert_eq!(
            unavailable_text(&capability),
            Some("Windows denied access. Open details for the reason.")
        );
        assert_eq!(
            capability,
            Capability::Denied(reason.into()),
            "Native evidence is retained for help"
        );
        assert_eq!(
            current_value_text(&Capability::Available, Some("144 Hz\nMore info")),
            "144 Hz"
        );
        assert_eq!(
            unavailable_text(&Capability::Unsupported("Choose a saved EXE".into())),
            Some("Choose a saved EXE")
        );
    }
    #[test]
    fn compact_tweaks_unsupported_platform_disables_main_button_but_retains_undo() {
        let controls = tweak_controls(
            Some(ModuleStatus::ExternalChange),
            &Capability::Unsupported("Requires Windows 11".into()),
            false,
            true,
            false,
            TweakActionKind::Managed,
        );
        assert!(!primary_action_enabled(&controls, false));
        assert!(controls.undo);
    }

    #[test]
    fn compact_tweaks_unknown_value_never_enables_apply() {
        let controls = tweak_controls(
            None,
            &Capability::Available,
            false,
            false,
            false,
            TweakActionKind::Managed,
        );
        assert!(!controls.apply);
        assert!(!controls.undo);
        assert_eq!(controls.action, CompactAction::Unavailable);
    }

    #[test]
    fn compact_tweaks_already_configured_does_not_claim_ownership() {
        let controls = tweak_controls(
            Some(ModuleStatus::VerifiedActive(
                crate::app::features::system_tweaks::platform::contract::Effect::Immediate,
            )),
            &Capability::Available,
            false,
            false,
            false,
            TweakActionKind::Managed,
        );
        assert_eq!(controls.action, CompactAction::AlreadyConfigured);
        assert!(!controls.apply && !controls.undo);
    }

    #[test]
    fn compact_tweaks_external_change_retains_undo_without_reapplying() {
        let controls = tweak_controls(
            Some(ModuleStatus::ExternalChange),
            &Capability::Available,
            true,
            true,
            false,
            TweakActionKind::Managed,
        );
        assert_eq!(controls.action, CompactAction::Review);
        assert!(!controls.apply);
        assert!(controls.undo);
    }

    #[test]
    fn compact_tweaks_guidance_never_has_an_apply_or_undo_action() {
        let controls = tweak_controls(
            None,
            &Capability::Available,
            true,
            false,
            false,
            TweakActionKind::Guide,
        );
        assert_eq!(controls.action, CompactAction::Help);
        assert!(!controls.apply && !controls.undo);
    }

    #[test]
    fn compact_tweaks_busy_operations_disable_competing_changes() {
        let controls = tweak_controls(
            Some(ModuleStatus::VerifiedActive(
                crate::app::features::system_tweaks::platform::contract::Effect::GameRestartRequired,
            )),
            &Capability::Available,
            true,
            true,
            true,
            TweakActionKind::Managed,
        );
        assert_eq!(controls.action, CompactAction::Working);
        assert!(!controls.apply && !controls.undo);
    }

    struct UiSave(Arc<std::sync::atomic::AtomicBool>);
    impl crate::app::features::system_tweaks::StatePersistence for UiSave {
        fn save(
            &mut self,
            _: &mut AppStateStorage,
        ) -> Result<crate::app::features::system_tweaks::service::SavedState, SaveError> {
            if self.0.load(std::sync::atomic::Ordering::SeqCst) {
                Err(SaveError::Storage)
            } else {
                Ok(crate::app::features::system_tweaks::service::SavedState::Unavailable)
            }
        }
    }
    fn ui_fixture() -> (
        AppState,
        Arc<crate::app::features::system_tweaks::simple_tests::Setting>,
        Arc<std::sync::atomic::AtomicBool>,
        ProfileId,
    ) {
        use crate::app::features::system_tweaks::{
            platform::registry::Registry, Tuning, TuningRuntime,
        };
        let state = AppStateStorage::test_fixture(|_| {});
        let setting =
            crate::app::features::system_tweaks::simple_tests::Setting::new("ui_setting", false);
        let mut registry = Registry::default();
        registry.register(setting.clone()).unwrap();
        let mut production = Registry::default();
        crate::app::features::system_tweaks::tweaks::diagnostics::register(&mut production)
            .unwrap();
        for id in ["gpu_preference_guide", "measurements"] {
            registry
                .register(
                    production
                        .get(
                            &crate::app::features::system_tweaks::platform::contract::FeatureId::new(id)
                                .unwrap(),
                        )
                        .unwrap()
                        .clone(),
                )
                .unwrap();
        }
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut service = Tuning::from_registry(registry.clone())
            .headless(state.clone(), Box::new(UiSave(fail.clone())));
        service.admit_test_modules();
        let runtime = TuningRuntime::start(
            service,
            Arc::new(tokio::sync::RwLock::new(Default::default())),
        );
        let mut app = AppState::new_for_test(Arc::new(RwLock::new(state)), 4);
        app.replace_tuning_for_test(runtime, registry);
        app.request_tuning(TuningRequest::CreateProfile {
            name: "Gaming".into(),
        })
        .unwrap();
        wait_saved(&mut app);
        let profile = app.tuning_snapshot().profiles[0].id;
        app.ui.tuning.selected_profile = Some(profile);
        (app, setting, fail, profile)
    }
    fn wait_saved(app: &mut AppState) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(
            app.tuning_snapshot().operation,
            TuningOperationState::Pending { .. }
        ) {
            assert!(Instant::now() < deadline);
            app.poll_tuning();
            std::thread::sleep(Duration::from_millis(2));
        }
        app.poll_tuning();
    }

    fn feedback_fixture(
        settings: &[(&str, bool, Effect)],
    ) -> (
        AppState,
        Vec<Arc<crate::app::features::system_tweaks::simple_tests::Setting>>,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        use crate::app::features::system_tweaks::{
            platform::registry::{Module, Registry},
            simple_tests::Setting,
            Tuning, TuningRuntime,
        };
        let settings: Vec<_> = settings
            .iter()
            .map(|(name, user, effect)| Setting::with_effect(name, *user, *effect))
            .collect();
        let mut registry = Registry::default();
        for setting in &settings {
            registry.register(setting.clone()).unwrap();
        }
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut service = Tuning::from_registry(registry.clone()).headless(
            AppStateStorage::test_fixture(|_| {}),
            Box::new(UiSave(fail.clone())),
        );
        service.admit_test_modules();
        for setting in &settings {
            let feature = setting.descriptor().id;
            let profile = service
                .create_profile(
                    "Windows Tweaks [manual v1]".into(),
                    vec![ModulePreference {
                        feature: feature.clone(),
                        enabled: true,
                        mode: ActivationMode::Manual,
                        config: setting.config(12),
                    }],
                )
                .unwrap();
            service
                .activate_manual(profile, feature, setting.targets()[0].clone())
                .unwrap();
        }
        let state = service.state().clone();
        let runtime = TuningRuntime::start(
            service,
            Arc::new(tokio::sync::RwLock::new(Default::default())),
        );
        let mut app = AppState::new_for_test(Arc::new(RwLock::new(state)), 4);
        app.replace_tuning_for_test(runtime, registry);
        (app, settings, fail)
    }

    fn feedback_undo(
        app: &mut AppState,
        setting: &crate::app::features::system_tweaks::simple_tests::Setting,
    ) {
        use crate::app::features::system_tweaks::platform::registry::Module;
        app.request_tuning(TuningRequest::UndoTweak {
            feature: setting.descriptor().id,
        })
        .unwrap();
        wait_saved(app);
    }

    fn feedback_drift(
        setting: &crate::app::features::system_tweaks::simple_tests::Setting,
        value: u8,
    ) {
        use crate::app::features::system_tweaks::platform::contract::ResourceId;
        setting
            .fields
            .lock()
            .unwrap()
            .insert(ResourceId::new("value").unwrap(), vec![value]);
    }

    #[test]
    fn operation_feedback_successful_undo_retry_clears_the_previous_error() {
        let (mut app, settings, _) =
            feedback_fixture(&[("simple_tweak", false, Effect::Immediate)]);
        feedback_drift(&settings[0], 20);
        feedback_undo(&mut app, &settings[0]);
        assert!(app.tuning_snapshot().operation_error.is_some());
        assert_eq!(settings[0].value(), vec![20]);
        feedback_drift(&settings[0], 12);
        feedback_undo(&mut app, &settings[0]);
        let snapshot = app.tuning_snapshot();
        let text = rendered_text(&mut app);
        finish(&mut app);
        assert_eq!(snapshot.operation, TuningOperationState::Idle);
        assert!(!snapshot.has_tweak_changes);
        assert_eq!(settings[0].value(), vec![7]);
        assert!(
            snapshot.operation_error.is_none(),
            "the successful retry resolved the original failure"
        );
        assert!(!text.contains("Could not complete this action"));
    }

    #[test]
    fn operation_feedback_storage_failure_is_visible_beside_retained_controller_error() {
        use crate::app::features::system_tweaks::platform::contract::ErrorKind;
        let (mut app, settings, fail) =
            feedback_fixture(&[("simple_tweak", false, Effect::Immediate)]);
        feedback_drift(&settings[0], 20);
        feedback_undo(&mut app, &settings[0]);
        fail.store(true, std::sync::atomic::Ordering::SeqCst);
        feedback_undo(&mut app, &settings[0]);
        let snapshot = app.tuning_snapshot();
        let text = rendered_text(&mut app);
        finish(&mut app);
        assert!(matches!(
            snapshot.operation,
            TuningOperationState::Refused {
                error: SaveError::Storage,
                ..
            }
        ));
        assert_eq!(
            snapshot.operation_error.as_ref().unwrap().kind,
            ErrorKind::ExternalChange
        );
        assert!(snapshot.has_tweak_changes);
        assert!(
            text.contains("Could not save"),
            "the current save failure must not be hidden by retained native evidence: {text}"
        );
        assert!(text.contains("Error details"));
    }

    #[test]
    fn operation_feedback_new_request_keeps_unresolved_controller_error() {
        use crate::app::features::system_tweaks::platform::{
            contract::ErrorKind, registry::Module,
        };
        let (mut app, settings, _) =
            feedback_fixture(&[("simple_tweak", false, Effect::Immediate)]);
        feedback_drift(&settings[0], 20);
        feedback_undo(&mut app, &settings[0]);
        app.request_tuning(TuningRequest::ModuleAction {
            feature: settings[0].descriptor().id,
            action: ModuleAction::Refresh,
        })
        .unwrap();
        wait_saved(&mut app);
        let snapshot = app.tuning_snapshot();
        finish(&mut app);
        assert_eq!(snapshot.operation, TuningOperationState::Idle);
        assert_eq!(
            snapshot.operation_error.as_ref().unwrap().kind,
            ErrorKind::ExternalChange
        );
        assert!(snapshot.has_tweak_changes);
        assert_eq!(settings[0].value(), vec![20]);
    }

    #[test]
    fn operation_feedback_new_apply_clears_the_previous_restore_success() {
        use crate::app::features::system_tweaks::platform::registry::Module;
        let (mut app, settings, _) =
            feedback_fixture(&[("simple_tweak", false, Effect::Immediate)]);
        feedback_undo(&mut app, &settings[0]);
        assert_eq!(
            app.tuning_snapshot().restoration_effect,
            Some(Effect::Immediate)
        );
        app.request_tuning(TuningRequest::ApplyTweak {
            feature: settings[0].descriptor().id,
            target: TargetIdentity::Machine,
        })
        .unwrap();
        wait_saved(&mut app);
        let snapshot = app.tuning_snapshot();
        let text = rendered_text(&mut app);
        finish(&mut app);
        assert!(snapshot.has_tweak_changes);
        assert_eq!(settings[0].value(), vec![12]);
        assert_eq!(
            snapshot.restoration_effect, None,
            "a previous Undo is not the result of this Apply"
        );
        assert!(!text.contains("Original settings restored"));
    }

    #[test]
    fn operation_feedback_stale_request_clears_the_previous_restore_success() {
        let (mut app, settings, _) =
            feedback_fixture(&[("simple_tweak", false, Effect::Immediate)]);
        feedback_undo(&mut app, &settings[0]);
        let restored = app.tuning_snapshot();
        assert_eq!(restored.restoration_effect, Some(Effect::Immediate));
        app.request_tuning(TuningRequest::SaveBase {
            candidate: Box::new((*restored.state).clone()),
            expected_revision: 0,
        })
        .unwrap();
        wait_saved(&mut app);
        let snapshot = app.tuning_snapshot();
        finish(&mut app);
        assert!(matches!(
            snapshot.operation,
            TuningOperationState::Refused {
                error: SaveError::Invalid,
                ..
            }
        ));
        assert_eq!(snapshot.restoration_effect, None);
    }

    #[test]
    fn operation_feedback_partial_undo_keeps_restart_notice_without_claiming_full_restore() {
        let (mut app, settings, _) = feedback_fixture(&[
            ("feedback_restart", true, Effect::GameRestartRequired),
            ("feedback_drift", false, Effect::Immediate),
        ]);
        feedback_drift(&settings[1], 20);
        app.request_tuning(TuningRequest::UndoAllTweaks).unwrap();
        wait_saved(&mut app);
        let snapshot = app.tuning_snapshot();
        let text = rendered_text(&mut app);
        finish(&mut app);
        assert_eq!(settings[0].value(), vec![7]);
        assert_eq!(settings[1].value(), vec![20]);
        assert!(snapshot.has_tweak_changes);
        assert_eq!(
            snapshot.restoration_effect,
            Some(Effect::GameRestartRequired)
        );
        assert!(
            text.contains("Some original settings restored"),
            "partial Undo needs qualified feedback: {text}"
        );
        assert!(text.contains("Restart the game"));
        assert!(!text.contains("Original settings restored."));
    }

    #[test]
    fn operation_feedback_complete_undo_preserves_the_strongest_restart_requirement() {
        for effect in [Effect::GameRestartRequired, Effect::RestartRequired] {
            let (mut app, settings, _) = feedback_fixture(&[
                ("feedback_restart", true, effect),
                ("feedback_immediate", false, Effect::Immediate),
            ]);
            app.request_tuning(TuningRequest::UndoAllTweaks).unwrap();
            wait_saved(&mut app);
            let snapshot = app.tuning_snapshot();
            let text = rendered_text(&mut app);
            finish(&mut app);
            assert_eq!(snapshot.operation, TuningOperationState::Idle);
            assert!(!snapshot.has_tweak_changes);
            assert!(settings.iter().all(|setting| setting.value() == vec![7]));
            assert_eq!(
                snapshot.restoration_effect,
                Some(effect),
                "an Immediate field restored last must not discard an earlier restart requirement"
            );
            assert!(text.contains(match effect {
                Effect::GameRestartRequired => "Restart the game",
                _ => "Restart Windows",
            }));
            assert!(!text.contains("Some original settings restored"));
        }
    }

    #[test]
    fn modules_ui_legacy_preference_save_publishes_without_stale_warning() {
        use crate::app::features::system_tweaks::platform::registry::Module;
        let (mut app, setting, _, profile) = ui_fixture();
        let preference = ModulePreference {
            feature: setting.descriptor().id,
            enabled: true,
            mode: ActivationMode::Session,
            config: setting.config(12),
        };
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference.clone()),
            expected: None,
        })
        .unwrap();
        wait_saved(&mut app);
        let text = rendered_text(&mut app);
        assert_eq!(
            current_preference(&app, profile, &setting.descriptor()),
            Some(preference)
        );
        assert!(!text.contains("Saved settings changed"));
        finish(&mut app);
    }
    #[test]
    fn modules_ui_saved_preference_is_visible_when_restoration_needs_attention() {
        use crate::app::features::system_tweaks::platform::contract::{
            ResourceId, ResourceKey, TargetIdentity,
        };
        use crate::app::features::system_tweaks::platform::registry::Module;
        let (mut app, setting, _, profile) = ui_fixture();
        let original = ModulePreference {
            feature: setting.descriptor().id,
            enabled: true,
            mode: ActivationMode::Manual,
            config: setting.config(12),
        };
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(original.clone()),
            expected: None,
        })
        .unwrap();
        wait_saved(&mut app);
        app.request_tuning(TuningRequest::ActivateManual {
            profile,
            feature: original.feature.clone(),
            target: TargetIdentity::Machine,
        })
        .unwrap();
        wait_saved(&mut app);
        let key = ResourceKey {
            target: TargetIdentity::Machine,
            field: ResourceId::new("value").unwrap(),
        };
        assert_eq!(setting.read(&key).unwrap(), vec![12]);
        setting.write(&key, &[12], &[20]).unwrap();
        let candidate = ModulePreference {
            config: setting.config(13),
            ..original.clone()
        };
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(candidate.clone()),
            expected: Some(original),
        })
        .unwrap();
        wait_saved(&mut app);
        assert!(matches!(
            app.tuning_snapshot().operation,
            TuningOperationState::Refused {
                error: SaveError::Recovering,
                ..
            }
        ));
        assert_eq!(
            current_preference(&app, profile, &setting.descriptor()),
            Some(candidate)
        );
        let text = rendered_text(&mut app);
        assert!(
            text.contains("Could not complete this action")
                || text.contains("Recovery needs attention")
        );
        assert_eq!(
            setting.read(&key).unwrap(),
            vec![20],
            "foreign changes must be preserved"
        );
        finish(&mut app);
    }
    #[test]
    fn modules_ui_failed_legacy_save_preserves_saved_preference_and_allows_retry() {
        use crate::app::features::system_tweaks::platform::registry::Module;
        let (mut app, setting, fail, profile) = ui_fixture();
        let preference = ModulePreference {
            feature: setting.descriptor().id,
            enabled: true,
            mode: ActivationMode::Session,
            config: setting.config(13),
        };
        fail.store(true, std::sync::atomic::Ordering::SeqCst);
        let before = current_preference(&app, profile, &setting.descriptor());
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference.clone()),
            expected: None,
        })
        .unwrap();
        wait_saved(&mut app);
        rendered_text(&mut app);
        assert_eq!(
            current_preference(&app, profile, &setting.descriptor()),
            before
        );
        assert!(matches!(
            app.tuning_snapshot().operation,
            TuningOperationState::Refused {
                error: SaveError::Storage,
                ..
            }
        ));
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference.clone()),
            expected: before,
        })
        .unwrap();
        wait_saved(&mut app);
        assert_eq!(
            current_preference(&app, profile, &setting.descriptor()),
            Some(preference)
        );
        finish(&mut app);
    }
    #[test]
    fn modules_ui_import_error_does_not_claim_saved_settings_or_activation_broke() {
        use crate::app::features::system_tweaks::{
            platform::contract::FeatureId, presentation::ModuleAction,
        };
        let (mut app, _, _, profile) = ui_fixture();
        app.request_tuning(TuningRequest::RemoveProfile { profile })
            .unwrap();
        wait_saved(&mut app);
        assert!(app.tuning_snapshot().profiles.is_empty());
        let revision = app.tuning_snapshot().revision;
        let missing = std::env::temp_dir().join(format!(
            "modules-missing-{}.csv",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        app.request_tuning(TuningRequest::ModuleAction {
            feature: FeatureId::new("measurements").unwrap(),
            action: ModuleAction::Import(missing),
        })
        .unwrap();
        wait_saved(&mut app);
        assert_eq!(app.tuning_snapshot().revision, revision);
        assert_eq!(
            app.tuning_snapshot()
                .operation_error
                .as_ref()
                .unwrap()
                .operation,
            "import CSV"
        );
        let text = rendered_text(&mut app);
        assert!(
            !text.contains("Saved settings could not be verified"),
            "import failure is not a saved-state verification failure"
        );
        assert!(!text.contains("Applying changes is paused"));
        assert!(
            !text.contains("Recovery needs attention"),
            "an invalid import needs no recovery"
        );
        assert!(
            !text.contains("verification will run again"),
            "read-only import does not need a profile"
        );
        assert!(text.contains("Error details"));
        finish(&mut app);
    }
    #[test]
    fn legacy_undo_hides_unused_profiles_without_changing_saved_data() {
        use crate::app::features::system_tweaks::platform::registry::Module;
        let (mut app, setting, _, profile) = ui_fixture();
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(ModulePreference {
                feature: setting.descriptor().id,
                enabled: true,
                mode: ActivationMode::Manual,
                config: setting.config(12),
            }),
            expected: None,
        })
        .unwrap();
        wait_saved(&mut app);
        let saved = app.tuning_snapshot().state.tuning.clone();
        app.ui.tuning.selected_section = TuningPageSection::System;
        let text = rendered_text(&mut app);
        assert!(!text.contains("Gaming"));
        assert!(!text.contains("Test setting"));
        assert!(!text.contains("Saved enabled"));
        assert!(!text.contains("Changes from an earlier version"));
        assert!(!text.contains("No previous settings to restore"));
        assert_eq!(app.tuning_snapshot().state.tuning.raw(), saved.raw());
        assert_eq!(setting.value(), vec![7]);
        finish(&mut app);
    }

    #[test]
    fn legacy_undo_shows_only_applied_settings_and_retains_retry_after_drift() {
        use crate::app::features::system_tweaks::platform::registry::Module;
        let (mut app, setting, _, profile) = ui_fixture();
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(ModulePreference {
                feature: setting.descriptor().id,
                enabled: true,
                mode: ActivationMode::Manual,
                config: setting.config(12),
            }),
            expected: None,
        })
        .unwrap();
        wait_saved(&mut app);
        app.request_tuning(TuningRequest::ActivateManual {
            profile,
            feature: setting.descriptor().id,
            target: setting.targets()[0].clone(),
        })
        .unwrap();
        wait_saved(&mut app);
        assert_eq!(setting.value(), vec![12]);
        app.ui.tuning.selected_section = TuningPageSection::System;
        let context = egui::Context::default();
        section_frame(&context, &mut app, vec![]);
        let output = section_frame(&context, &mut app, vec![]);
        let text = section_text(&output);
        assert!(!text.contains("Gaming"));
        assert!(!text.contains("Saved enabled"));
        assert!(text.contains("Test setting"));
        assert!(text.contains("Undo"));
        let undo_button = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Undo" => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .expect("applied old setting must retain its Undo action")
        };

        feedback_drift(&setting, 20);
        click_section(&context, &mut app, undo_button(&output));
        wait_saved(&mut app);
        assert_eq!(
            setting.value(),
            vec![20],
            "Undo must preserve an external edit"
        );
        let output = section_frame(&context, &mut app, vec![]);
        assert!(section_text(&output).contains("Changed outside this app"));
        assert!(section_text(&output).contains("Test setting"));
        // Returning to the app-written value permits a guarded retry.
        feedback_drift(&setting, 12);
        click_section(&context, &mut app, undo_button(&output));
        wait_saved(&mut app);
        assert_eq!(setting.value(), vec![7]);
        let output = section_frame(&context, &mut app, vec![]);
        assert!(!section_text(&output).contains("Test setting"));
        assert!(!section_text(&output).contains("Changes from an earlier version"));
        assert!(!app.tuning_snapshot().profiles[0].modules[0].enabled);
        finish(&mut app);
    }

    #[test]
    fn compact_tweaks_hides_profile_controls_and_retains_previous_settings() {
        let (mut app, _, _, _) = ui_fixture();
        let text = rendered_text(&mut app);
        assert!(text.contains("System Tweaks"));
        assert!(!text.contains("Restore"));
        assert!(!text.contains("Create profile"));
        assert!(!text.contains("Connect a game"));
        assert!(
            !text.contains("Test setting"),
            "previous details appear only in their own section"
        );
        assert!(
            !text.contains("Preferred GPU"),
            "hardware appears only in its own section"
        );
        finish(&mut app);
    }

    #[test]
    fn modules_observation_details_keep_last_device_and_omission_count_visible() {
        let summary = format!(
            "{}\nLast device: test endpoint\n12 additional devices omitted.",
            "device row\n".repeat(70)
        );
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            draw_observation(ui, &summary)
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
        assert!(
            text.contains("Last device: test endpoint"),
            "the full inventory must be accessible"
        );
        assert!(
            text.contains("12 additional devices omitted."),
            "inventory limits must be explicit"
        );
    }

    #[test]
    fn modules_operation_error_exposes_cause_and_native_code() {
        use crate::app::features::system_tweaks::platform::contract::*;
        let error = ModuleError::new(ErrorKind::Denied, "read saved recovery")
            .with_detail("Recovery file cannot be opened.")
            .unwrap()
            .with_native(NativeError {
                domain: ResourceId::new("win32").unwrap(),
                code: 5,
            });
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            draw_operation_error(ui, &error)
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
        assert!(text.contains("Recovery file cannot be opened."));
        assert!(text.contains("read saved recovery"));
        assert!(text.contains("win32: 5"));
    }

    #[test]
    fn compact_sections_long_error_keeps_following_controls_visible() {
        use crate::app::features::system_tweaks::platform::contract::{ErrorKind, ModuleError};
        let error = ModuleError::new(ErrorKind::Denied, "read saved recovery")
            .with_detail("Long recovery detail.\n".repeat(40))
            .unwrap();
        let mut undo = egui::Rect::NOTHING;
        let mut output = egui::Context::default().run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                draw_operation_error(ui, &error);
                undo = ui.button("Undo my changes").rect;
                ui.label("Selected section content");
            },
        );
        output.textures_delta.clear();
        assert!(
            undo.bottom() < 140.0,
            "expanded error details must leave room for Undo and section content: {undo:?}"
        );
    }

    #[test]
    fn compact_tweaks_page_never_requires_a_profile() {
        let state = Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {})));
        let mut app = empty_modules_app(state);
        let text = rendered_text(&mut app);
        assert!(text.contains("System Tweaks"));
        assert!(!text.contains("Choose a profile"));
        assert!(!text.contains("Create profile"));
        finish(&mut app);
    }

    fn rendered_text(app: &mut AppState) -> String {
        let context = egui::Context::default();
        let mut output =
            context.run_ui(egui::RawInput::default(), |ui| draw_system_tweaks(app, ui));
        // These headless tests inspect layout; there is no GPU renderer to apply textures.
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn finish(app: &mut AppState) {
        app.begin_tuning_shutdown();
        let until = Instant::now() + Duration::from_secs(2);
        while !app.tuning_shutdown_complete() && Instant::now() < until {
            app.poll_tuning();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            app.tuning_shutdown_complete(),
            "retain a still-running context on failure"
        );
    }

    fn empty_modules_app(state: Arc<RwLock<AppStateStorage>>) -> AppState {
        use crate::app::features::system_tweaks::{
            platform::registry::Registry, Tuning, TuningRuntime,
        };
        let registry = Registry::default();
        let runtime = TuningRuntime::start(
            Tuning::from_registry(registry.clone()).headless(
                state.read().unwrap().clone(),
                Box::new(UiSave(Arc::new(std::sync::atomic::AtomicBool::new(false)))),
            ),
            Arc::new(tokio::sync::RwLock::new(Default::default())),
        );
        let mut app = AppState::new_for_test(state, 4);
        app.replace_tuning_for_test(runtime, registry);
        app
    }

    fn section_frame(
        context: &egui::Context,
        app: &mut AppState,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| draw_system_tweaks(app, ui),
        );
        output.textures_delta.clear();
        output
    }

    fn section_text(output: &egui::FullOutput) -> String {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn section_button(output: &egui::FullOutput, title: &str) -> egui::Rect {
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == title => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Visible section title missing: {title}"));
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if (60.0..=140.0).contains(&rect.rect.width())
                        && (18.0..=32.0).contains(&rect.rect.height())
                        && rect.rect.contains(position) =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Section must be part of the compact switch: {title}"))
    }

    fn click_section(context: &egui::Context, app: &mut AppState, rect: egui::Rect) {
        let position = rect.center();
        section_frame(
            context,
            app,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        section_frame(
            context,
            app,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    #[test]
    fn compact_sections_use_one_horizontal_switch_above_scrollable_content() {
        let mut app =
            empty_modules_app(Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))));
        let context = egui::Context::default();
        let output = section_frame(&context, &mut app, vec![]);
        let mut previous_right = 0.0;
        let mut switch_top = None;
        assert!(!section_text(&output).contains("Restore"));
        for title in ["System", "GPU / network", "Check PC"] {
            let rect = section_button(&output, title);
            assert!(
                rect.left() >= previous_right,
                "sections share one horizontal track"
            );
            assert_eq!(*switch_top.get_or_insert(rect.top()), rect.top());
            assert!(
                rect.left() >= 0.0 && rect.right() <= 400.0,
                "section {title} exceeds the compact viewport: {rect:?}"
            );
            assert!(
                rect.bottom() < 90.0,
                "the switch leaves room for content at minimum height"
            );
            previous_right = rect.right();
        }
        assert!(
            section_text(&output).contains("FOR GAMES"),
            "system settings are selected initially"
        );
        finish(&mut app);
    }

    #[test]
    fn memory_integrity_guide_fills_width_and_keeps_action_at_right() {
        let mut app =
            empty_modules_app(Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))));
        let context = egui::Context::default();
        let mut available = egui::Rect::NOTHING;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                available = ui.available_rect_before_wrap();
                draw_memory_integrity_guide(&mut app, ui, false);
            },
        );
        output.textures_delta.clear();
        finish(&mut app);
        let label_rect = |title: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == title => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap()
        };
        let title = label_rect("Memory integrity");
        let action = label_rect("Open settings");
        let card = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.rect.contains(title.center())
                        && rect.rect.contains(action.center()) =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .min_by(|a, b| a.height().total_cmp(&b.height()))
            .unwrap();
        let button = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if (rect.rect.width() - 112.0).abs() < 0.1
                        && rect.rect.contains(action.center()) =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .unwrap();
        assert!((card.left() - available.left()).abs() <= 1.0);
        assert!(
            (card.right() - available.right()).abs() <= 1.0,
            "Card must fill its parent: {card:?}, available: {available:?}"
        );
        assert!(
            (8.0..=12.0).contains(&(card.right() - button.right())),
            "Action must use the right column: {button:?}, card: {card:?}"
        );
        assert!(title.right() < button.left());
    }

    #[test]
    fn system_games_has_manual_memory_integrity_guide_without_running_checks() {
        let mut app =
            empty_modules_app(Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))));
        app.ui.tuning.selected_section = TuningPageSection::System;
        let revision = app.tuning_snapshot().revision;
        let context = egui::Context::default();
        section_frame(&context, &mut app, vec![]);
        let output = section_frame(&context, &mut app, vec![]);
        let text = section_text(&output);
        assert!(text.contains("Memory integrity"));
        assert!(text.contains("Open settings"));
        assert!(text.contains("Turning this off reduces protection"));
        assert!(text.contains("Undo in Windows"));
        let games = text.find("FOR GAMES").unwrap();
        let memory_integrity = text.find("Memory integrity").unwrap();
        let comfort = text.find("FOR COMFORT").unwrap();
        assert!(games < memory_integrity && memory_integrity < comfort);
        assert_eq!(text.matches("Memory integrity").count(), 1);
        app.ui.tuning.selected_section = TuningPageSection::PcChecks;
        section_frame(&context, &mut app, vec![]);
        let output = section_frame(&context, &mut app, vec![]);
        let text = section_text(&output);
        assert!(text.contains("Use Check PC"));
        assert!(!text.contains("Memory integrity"));
        assert!(app.tuning_snapshot().pc_checks.is_empty());
        assert_eq!(app.tuning_snapshot().revision, revision);
        assert_eq!(app.tuning_snapshot().operation, TuningOperationState::Idle);
        finish(&mut app);
    }

    #[test]
    fn compact_sections_navigation_does_not_apply_tweaks_or_run_pc_checks() {
        let mut app =
            empty_modules_app(Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))));
        let target = TargetIdentity::Machine;
        app.ui.tuning.tweak_target = Some(target.clone());
        let revision = app.tuning_snapshot().revision;
        let context = egui::Context::default();
        let output = section_frame(&context, &mut app, vec![]);
        click_section(&context, &mut app, section_button(&output, "Check PC"));
        let output = section_frame(&context, &mut app, vec![]);
        let text = section_text(&output);
        assert!(
            !text.contains("FOR GAMES"),
            "only selected section contents are shown"
        );
        assert!(
            text.contains("Use Check PC"),
            "checking requires an explicit action"
        );
        assert!(app.tuning_snapshot().pc_checks.is_empty());
        assert_eq!(app.tuning_snapshot().revision, revision);
        assert_eq!(app.tuning_snapshot().operation, TuningOperationState::Idle);
        click_section(&context, &mut app, section_button(&output, "System"));
        let output = section_frame(&context, &mut app, vec![]);
        assert!(section_text(&output).contains("FOR GAMES"));
        assert_eq!(app.ui.tuning.tweak_target, Some(target));
        assert_eq!(app.tuning_snapshot().revision, revision);
        finish(&mut app);
    }

    #[test]
    fn p5_empty_shipping_catalog_page_does_not_invent_active_modules() {
        let state = Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {})));
        let mut app = empty_modules_app(state);
        assert!(app.tuning_snapshot().catalog.is_empty());
        let text = rendered_text(&mut app);
        assert!(text.contains("No modules are available in this build."));
        assert!(!text.contains("Verified active"));
        finish(&mut app);
    }

    #[test]
    fn p5_unknown_settings_page_retains_exact_raw_data_and_is_readonly() {
        let raw = r#"{"version":99,"future":{"retained":true}}"#;
        let state = Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
            state.tuning = serde_json::from_str(raw).unwrap();
        })));
        let mut app = empty_modules_app(state.clone());
        let text = rendered_text(&mut app);
        assert!(text.contains("retained and read-only"));
        assert!(!text.contains("Create profile"));
        assert_eq!(state.read().unwrap().tuning.raw(), Some(raw));
        finish(&mut app);
    }
}
