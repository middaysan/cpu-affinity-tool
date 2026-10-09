use crate::app::{
    features::tuning::{
        config::ModulePreference,
        platform::contract::{
            ActivationEligibility, ActivationMode, Capability, ModuleAccess, ModuleDescriptor,
            ModuleStatus, ProfileId, TargetScope,
        },
        presentation::{
            card_controls, edit_configuration, mode_label, preference_with_enabled, scope_label,
            status_label, EditorDraft, ModuleSection,
        },
        SaveError, TuningConfigurationState, TuningOperationState, TuningRequest,
    },
    runtime::AppState,
    shell::presenters::shared_elements::{content_frame, form_text_edit, glass_frame},
};
use eframe::egui;

pub(crate) fn save_error_message(error: SaveError) -> &'static str {
    match error {
        SaveError::Storage => "Could not save. Your previous settings are retained.",
        SaveError::Invalid => {
            "Settings changed or are unavailable. Refresh the editor and try again."
        }
        SaveError::Pending => "Another operation is pending. Wait for it to finish.",
        SaveError::Recovering => "Recovery needs attention before more changes can be made.",
    }
}

pub(crate) fn submit(app: &mut AppState, request: TuningRequest) -> bool {
    app.ui.tuning.error = app.request_tuning(request).err().map(save_error_message);
    app.ui.tuning.error.is_none()
}

fn busy(operation: &TuningOperationState) -> bool {
    matches!(
        operation,
        TuningOperationState::Pending { .. } | TuningOperationState::Recovering
    )
}

fn current_preference(
    app: &AppState,
    profile: ProfileId,
    descriptor: &ModuleDescriptor,
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

pub(crate) fn draw_tuning(app: &mut AppState, root_ui: &mut egui::Ui) {
    if !app.modules_enabled() {
        return;
    }
    let snapshot = app.tuning_snapshot();
    let editable = matches!(
        snapshot.configuration,
        TuningConfigurationState::Off | TuningConfigurationState::Known
    );
    let pending = busy(&snapshot.operation);
    if let Some(before) = app.ui.tuning.creating_revision {
        if snapshot.revision > before && matches!(snapshot.operation, TuningOperationState::Idle) {
            app.ui.tuning.selected_profile = snapshot.profiles.last().map(|p| p.id);
            app.ui.tuning.creating_revision = None;
            app.ui.tuning.profile_name.clear();
            app.ui.tuning.show_new_profile = false;
            app.ui.tuning.editor = None;
        } else if matches!(snapshot.operation, TuningOperationState::Refused { .. }) {
            app.ui.tuning.creating_revision = None;
        }
    }
    if app
        .ui
        .tuning
        .selected_profile
        .is_some_and(|id| !snapshot.profiles.iter().any(|p| p.id == id))
    {
        app.ui.tuning.selected_profile = None;
        app.ui.tuning.editor = None;
    }
    if app.ui.tuning.selected_profile.is_none() {
        app.ui.tuning.selected_profile = snapshot.profiles.first().map(|p| p.id);
    }
    egui::CentralPanel::default().frame(content_frame(root_ui)).show(root_ui, |ui| {
        ui.heading("Modules");
        ui.label("Choose what to change, when to apply it, and which games use it.");
        match snapshot.operation {
            TuningOperationState::Pending { .. } => { ui.horizontal(|ui| { ui.spinner(); ui.label("Saving your changes…"); }); }
            TuningOperationState::Refused { error, .. } => {
                if snapshot.operation_error.is_none() { ui.colored_label(ui.visuals().error_fg_color, save_error_message(error)); }
            }
            TuningOperationState::Recovering => { ui.colored_label(ui.visuals().error_fg_color, "A previous change needs attention. Applying settings is paused."); }
            TuningOperationState::Idle => {}
        }
        if let Some(error) = app.ui.tuning.error { ui.colored_label(ui.visuals().error_fg_color, error); }
        if let Some(error) = &snapshot.operation_error {
            ui.colored_label(ui.visuals().error_fg_color, "Could not complete this action. Open Error details for more information.");
            egui::CollapsingHeader::new("Error details").show(ui, |ui| draw_operation_error(ui, error));
        }
        if !editable {
            ui.label("Saved module settings cannot be understood by this version. They are retained and read-only.");
            return;
        }
        ui.add_space(8.0);
        glass_frame(ui).show(ui, |ui| {
            if snapshot.profiles.is_empty() {
                ui.strong("Start with a profile");
                ui.label("A profile is a saved set of settings. Give it a name such as Gaming, then choose the settings below.");
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Profile");
                    let name = snapshot.profiles.iter().find(|p| Some(p.id) == app.ui.tuning.selected_profile).map_or("Select a profile", |p| p.name.as_str());
                    egui::ComboBox::from_id_salt("tuning-profile").selected_text(name).show_ui(ui, |ui| {
                        for profile in &snapshot.profiles {
                            if ui.selectable_value(&mut app.ui.tuning.selected_profile, Some(profile.id), &profile.name).changed() {
                                app.ui.tuning.editor = None;
                                app.ui.tuning.selected_rule = None;
                                app.ui.tuning.rule_eligibility = None;
                            }
                        }
                    });
                    if ui.add_enabled(!pending, egui::Button::new("New profile")).clicked() { app.ui.tuning.show_new_profile = true; }
                });
            }
            if snapshot.profiles.is_empty() || app.ui.tuning.show_new_profile {
                ui.horizontal_wrapped(|ui| {
                    ui.add(form_text_edit(&mut app.ui.tuning.profile_name).desired_width(240.0).hint_text("Name, e.g. Gaming").char_limit(128));
                    if ui.add_enabled(!pending && !app.ui.tuning.profile_name.trim().is_empty(), egui::Button::new("Create profile")).clicked()
                        && submit(app, TuningRequest::CreateProfile { name: app.ui.tuning.profile_name.trim().to_owned() }) {
                        app.ui.tuning.creating_revision = Some(snapshot.revision);
                    }
                    if !snapshot.profiles.is_empty() && ui.button("Cancel").clicked() { app.ui.tuning.show_new_profile = false; }
                });
            }
            if let Some(profile) = snapshot.profiles.iter().find(|p| Some(p.id) == app.ui.tuning.selected_profile) {
                ui.small("Turn settings on individually. Nothing is applied just by opening this page.");
                egui::CollapsingHeader::new("Profile options").id_salt(profile.id.get()).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!pending && profile.modules.iter().any(|m| m.enabled), egui::Button::new("Turn off all settings")).clicked() {
                            submit(app, TuningRequest::DisableProfile { profile: profile.id });
                        }
                        if ui.add_enabled(!pending, egui::Button::new("Delete profile")).clicked() {
                            submit(app, TuningRequest::RemoveProfile { profile: profile.id });
                        }
                    });
                    ui.small("Turning a setting off restores its original value when this app can safely verify it.");
                });
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for section in ModuleSection::ALL {
                let count = snapshot.catalog.iter().filter(|d| app.tuning_editors().editor(&d.id).is_some_and(|e| e.section() == section)).count();
                if count != 0 { ui.selectable_value(&mut app.ui.tuning.section, section, format!("{} ({count})", section.label())); }
            }
        });
        ui.add_space(4.0);
        ui.label(match app.ui.tuning.section {
            ModuleSection::Performance => "Choose how this app manages your game's processes. Connect a saved game to use automatic settings.",
            ModuleSection::Input => "Choose mouse and keyboard behaviour while playing.",
            ModuleSection::Power => "Choose a power plan or power mode. Original settings are retained for turning it off.",
            ModuleSection::Windows => "These options are changed in Windows. Each card opens the right Settings page.",
            ModuleSection::Diagnostics => "Check devices and performance. These tools provide information and do not change system settings.",
        });
        ui.separator();
        if snapshot.catalog.is_empty() { ui.label("No modules are available in this build."); }
        egui::ScrollArea::vertical().id_salt("modules-content").auto_shrink([false, false]).show(ui, |ui| {
            if !matches!(app.ui.tuning.section, ModuleSection::Windows | ModuleSection::Diagnostics) {
                draw_game_connection(app, ui, pending);
            }
            for descriptor in &snapshot.catalog {
                if app.tuning_editors().editor(&descriptor.id).is_some_and(|e| e.section() == app.ui.tuning.section) {
                    ui.push_id(descriptor.id.as_str(), |ui| draw_module(app, ui, descriptor, pending));
                    ui.add_space(6.0);
                }
            }
            if let Some(profile) = snapshot.profiles.iter().find(|p| Some(p.id) == app.ui.tuning.selected_profile) {
                for card in &profile.modules {
                    if !snapshot.catalog.iter().any(|d| d.id == card.feature) {
                        glass_frame(ui).show(ui, |ui| {
                            ui.strong(&card.title);
                            ui.label("This saved setting is unavailable. Its configuration is retained.");
                            if let Some(status) = card.status { ui.label(status_label(status)); }
                        });
                    }
                }
            }
        });
        if let Some(effect) = snapshot.restoration_effect {
            ui.label(match effect {
                crate::app::features::tuning::platform::contract::Effect::Immediate => "Original settings restored and verified.",
                crate::app::features::tuning::platform::contract::Effect::GameRestartRequired => "Original settings restored. Restart the game to use them.",
                crate::app::features::tuning::platform::contract::Effect::RestartRequired => "Original settings restored. Restart Windows to use them.",
            });
        }
    });
}

fn draw_game_connection(app: &mut AppState, ui: &mut egui::Ui, pending: bool) {
    let snapshot = app.tuning_snapshot();
    let Some(profile) = app.ui.tuning.selected_profile else {
        return;
    };
    let mut games = Vec::new();
    for (gi, group) in snapshot.state.groups.iter().enumerate() {
        let Some(group_id) = snapshot.rules.group_id_for_index(gi) else {
            continue;
        };
        for (ri, game) in group.programs.iter().enumerate() {
            if let Some(rule_id) = snapshot.rules.rule_id_for_index(gi, ri) {
                games.push((
                    (group_id.clone(), rule_id),
                    format!("{} · {}", group.name, game.name),
                ));
            }
        }
    }
    if app
        .ui
        .tuning
        .selected_rule
        .as_ref()
        .is_some_and(|selected| !games.iter().any(|(ids, _)| ids == selected))
    {
        app.ui.tuning.selected_rule = None;
    }
    let connected: Vec<_> = games
        .iter()
        .filter(|(ids, _)| {
            snapshot
                .associations
                .iter()
                .any(|a| a.profile == profile && a.group == ids.0 && a.rule == ids.1)
        })
        .collect();
    glass_frame(ui).show(ui, |ui| {
        ui.strong("Games using this profile");
        if connected.is_empty() { ui.small("No game connected yet. Automatic settings wait until you connect a game."); }
        for (ids, name) in connected {
            ui.horizontal_wrapped(|ui| {
                ui.label(name);
                if ui.add_enabled(!pending, egui::Button::new("Disconnect")).clicked() {
                    submit(app, TuningRequest::UnbindRule { group: ids.0.clone(), rule: ids.1.clone() });
                }
            });
        }
        egui::CollapsingHeader::new("Connect a game").id_salt(profile.get()).show(ui, |ui| {
            if games.is_empty() {
                ui.label("Add a game in Overview first, then return here to connect it.");
                if ui.button("Go to Overview").clicked() { app.ui.set_current_window(crate::app::shell::WindowRoute::Groups(crate::app::shell::GroupRoute::List)); }
                return;
            }
            ui.add_enabled_ui(!pending, |ui| {
                let selected = games.iter().find(|(ids, _)| Some(ids) == app.ui.tuning.selected_rule.as_ref()).map_or("Choose a saved game", |(_, name)| name.as_str());
                egui::ComboBox::from_id_salt("connect-module-game").selected_text(selected).show_ui(ui, |ui| {
                    for (ids, name) in &games { ui.selectable_value(&mut app.ui.tuning.selected_rule, Some(ids.clone()), name); }
                });
                let mut eligibility = app.ui.tuning.rule_eligibility.unwrap_or(ActivationEligibility::Running);
                egui::ComboBox::from_id_salt("connect-module-when").selected_text(match eligibility {
                    ActivationEligibility::Running => "While the game is running", ActivationEligibility::Foreground => "Only while the game is in front",
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut eligibility, ActivationEligibility::Running, "While the game is running");
                    ui.selectable_value(&mut eligibility, ActivationEligibility::Foreground, "Only while the game is in front");
                });
                app.ui.tuning.rule_eligibility = Some(eligibility);
                ui.small("Running allows each setting's When choice. Foreground limits all automatic settings to when this game is in front.");
                if let Some((group, rule)) = app.ui.tuning.selected_rule.clone() {
                    if let Some(existing) = snapshot.associations.iter().find(|a| a.group == group && a.rule == rule && a.profile != profile) {
                        let name = snapshot.profiles.iter().find(|p| p.id == existing.profile).map_or("another profile", |p| p.name.as_str());
                        ui.label(format!("Connecting this game will replace its current profile: {name}."));
                    }
                    if ui.button("Connect to this profile").clicked() { submit(app, TuningRequest::BindRule { group, rule, profile, eligibility }); }
                }
            });
        });
    });
    ui.add_space(6.0);
}

fn draw_module(
    app: &mut AppState,
    ui: &mut egui::Ui,
    descriptor: &ModuleDescriptor,
    pending: bool,
) {
    let snapshot = app.tuning_snapshot();
    let profile = app.ui.tuning.selected_profile;
    let saved = profile.and_then(|id| current_preference(app, id, descriptor));
    let card = if descriptor.access == ModuleAccess::ReadOnly {
        snapshot
            .readonly_cards
            .iter()
            .find(|card| card.feature == descriptor.id)
    } else {
        profile
            .and_then(|id| snapshot.profiles.iter().find(|p| p.id == id))
            .and_then(|p| p.modules.iter().find(|m| m.feature == descriptor.id))
    };
    if let Some(draft) = app
        .ui
        .tuning
        .editor
        .as_ref()
        .filter(|d| Some(d.profile) == profile && d.candidate.feature == descriptor.id)
    {
        if let Some(before) = draft.pending_save_revision {
            if snapshot.revision > before && saved.as_ref() == Some(&draft.candidate) {
                app.ui.tuning.editor = None;
            } else if matches!(snapshot.operation, TuningOperationState::Refused { .. }) {
                app.ui.tuning.editor.as_mut().unwrap().pending_save_revision = None;
            }
        }
    }
    let fallback = Capability::Unsupported("Not observed yet".into());
    let capability = card.map_or(&fallback, |card| &card.capability);
    let status = card
        .and_then(|card| card.status)
        .unwrap_or(ModuleStatus::Unmanaged);
    let mode = selected_mode(descriptor, saved.as_ref());
    let known = app
        .tuning_editors()
        .editor(&descriptor.id)
        .is_some_and(|editor| {
            saved
                .as_ref()
                .is_none_or(|p| editor.accepts_config(&p.config))
        });
    let controls = card_controls(
        descriptor.access,
        capability,
        known,
        saved.as_ref().is_some_and(|p| p.enabled),
        descriptor.activation_modes.contains(&mode),
        pending,
    );
    let editing = app
        .ui
        .tuning
        .editor
        .as_ref()
        .is_some_and(|d| Some(d.profile) == profile && d.candidate.feature == descriptor.id);
    glass_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.strong(descriptor.title);
            if descriptor.access == ModuleAccess::ReadOnly {
                ui.small(if app.tuning_editors().editor(&descriptor.id).is_some_and(|e| e.section() == ModuleSection::Windows) { "Change in Windows" } else { "Information only" });
            } else if let Some(profile) = profile {
                let mut enabled = if editing { app.ui.tuning.editor.as_ref().unwrap().candidate.enabled } else { saved.as_ref().is_some_and(|p| p.enabled) };
                let stale = editing && app.ui.tuning.editor.as_ref().unwrap().expected != saved;
                let allowed = if editing { !pending && !stale && known } else if enabled { controls.disable } else { controls.enable };
                if ui.add_enabled(allowed, egui::Checkbox::new(&mut enabled, "Enabled")).changed() {
                    if editing { app.ui.tuning.editor.as_mut().unwrap().candidate.enabled = enabled; }
                    else if let Some(editor) = app.tuning_editors().editor(&descriptor.id) {
                        let default = ModulePreference { feature: descriptor.id.clone(), enabled: false, mode, config: editor.default_config() };
                        submit(app, TuningRequest::SetModulePreference { profile, preference: Box::new(preference_with_enabled(saved.as_ref(), default, enabled)), expected: saved.clone() });
                    }
                }
                if !editing && ui.add_enabled(controls.configure, egui::Button::new("Settings…")).clicked() {
                    if let Some(editor) = app.tuning_editors().editor(&descriptor.id) {
                        let candidate = saved.clone().unwrap_or_else(|| ModulePreference { feature: descriptor.id.clone(), enabled: false, mode, config: editor.default_config() });
                        app.ui.tuning.editor = Some(EditorDraft { profile, expected: saved.clone(), candidate, pending_save_revision: None });
                    }
                }
            }
        });
        ui.label(descriptor.description);
        if descriptor.access == ModuleAccess::ReadOnly {
            if app.tuning_editors().editor(&descriptor.id).is_some_and(|e| e.section() != ModuleSection::Windows) {
                if let Some(observed) = card.and_then(|card| card.observed.as_ref()) {
                    let line = observed.summary.lines().next().unwrap_or("No information yet");
                    ui.small(line.chars().take(220).collect::<String>());
                }
            }
            ui.small("Information only. This card does not apply or restore system settings.");
            let action = app.tuning_editors().editor(&descriptor.id).and_then(|editor| editor.actions(ui));
            if let Some(action) = action { submit(app, TuningRequest::ModuleAction { feature: descriptor.id.clone(), action }); }
        } else if profile.is_none() {
            ui.small("Create a profile above to choose and enable this setting.");
        } else {
            let enabled = saved.as_ref().is_some_and(|p| p.enabled);
            let missing_process = descriptor.target_scopes.contains(&TargetScope::Process) && card.is_none_or(|c| c.targets.is_empty());
            let linked = snapshot.associations.iter().any(|a| Some(a.profile) == profile);
            let label = if status != ModuleStatus::Unmanaged { status_label(status) }
                else if !enabled { "Off" }
                else if mode == ActivationMode::Manual { "Ready. Use Apply now to activate this setting." }
                else if !linked { "Waiting. Connect a game to this profile above." }
                else if missing_process { "Waiting for the connected game to run." }
                else { "Waiting for the selected game trigger." };
            ui.small(label);
            if let Some(saved) = &saved {
                ui.small(format!("When: {}", mode_label(saved.mode)));
                if let Some(summary) = app.tuning_editors().editor(&descriptor.id).and_then(|editor| editor.summary(&saved.config)) { ui.label(summary); }
            }
            if !known { ui.label("These saved settings need an editor from a compatible app version."); }
            if let Some(mut draft) = app.ui.tuning.editor.take() {
                if Some(draft.profile) == profile && draft.candidate.feature == descriptor.id {
                    let stale = draft.expected != saved;
                    let saving = draft.pending_save_revision.is_some();
                    let mut save = false;
                    let mut cancel = false;
                    ui.separator();
                    ui.strong("Setting options");
                    ui.add_enabled_ui(!pending && !saving && !stale, |ui| {
                        let valid = app.tuning_editors().editor(&descriptor.id).is_some_and(|editor| edit_configuration(editor, ui, &mut draft.candidate.config));
                        ui.horizontal_wrapped(|ui| {
                            ui.label("When");
                            egui::ComboBox::from_id_salt("module-when").selected_text(mode_label(draft.candidate.mode)).show_ui(ui, |ui| {
                                for mode in &descriptor.activation_modes { ui.selectable_value(&mut draft.candidate.mode, *mode, mode_label(*mode)); }
                            });
                        });
                        ui.small("Use Enabled above to include this setting. Changes take effect after saving.");
                        save = ui.add_enabled(valid, egui::Button::new("Save changes").fill(ui.visuals().selection.bg_fill)).clicked();
                    });
                    if stale {
                        ui.label("This setting changed while you were editing. Reload it to continue.");
                        if ui.button("Reload saved settings").clicked() {
                            if let Some(saved) = &saved { draft.expected = Some(saved.clone()); draft.candidate = saved.clone(); }
                        }
                    }
                    cancel |= ui.add_enabled(!saving, egui::Button::new("Cancel")).clicked();
                    if save && submit(app, TuningRequest::SetModulePreference { profile: draft.profile, preference: Box::new(draft.candidate.clone()), expected: draft.expected.clone() }) {
                        draft.pending_save_revision = Some(snapshot.revision);
                    }
                    if !cancel { app.ui.tuning.editor = Some(draft); }
                } else { app.ui.tuning.editor = Some(draft); }
            }
            if let Some(card) = card.filter(|card| card.enabled && card.mode == ActivationMode::Manual) {
                let profile = profile.unwrap();
                ui.small("Stays applied until you turn it off, including after closing this app.");
                let selected = app.ui.tuning.manual_target.as_ref().filter(|(id, feature, target)| *id == profile && *feature == descriptor.id && card.targets.contains(target)).map(|(_, _, target)| target.clone());
                let mut target = selected.or_else(|| (card.targets.len() == 1).then(|| card.targets[0].clone()));
                if card.targets.len() > 1 {
                    egui::ComboBox::from_id_salt("module-target").selected_text("Choose where to apply").show_ui(ui, |ui| {
                        for (index, candidate) in card.targets.iter().enumerate() { ui.selectable_value(&mut target, Some(candidate.clone()), format!("{} {}", scope_label(candidate.scope()), index + 1)); }
                    });
                }
                if let Some(target) = target {
                    app.ui.tuning.manual_target = Some((profile, descriptor.id.clone(), target.clone()));
                    if ui.add_enabled(controls.apply && !editing && !matches!(status, ModuleStatus::VerifiedActive(_)), egui::Button::new("Apply now")).clicked() {
                        submit(app, TuningRequest::ActivateManual { profile, feature: descriptor.id.clone(), target });
                    }
                }
            }
        }
        egui::CollapsingHeader::new("Details").show(ui, |ui| {
            ui.label(format!("Applies to: {}", descriptor.target_scopes.iter().map(|scope| scope_label(*scope)).collect::<Vec<_>>().join(", ")));
            if let Capability::Unsupported(reason) | Capability::Denied(reason) = capability { ui.label(reason); }
            if let Some(observed) = card.and_then(|card| card.observed.as_ref()) { draw_observation(ui, &observed.summary); }
        });
    });
}

fn selected_mode(
    descriptor: &ModuleDescriptor,
    saved: Option<&ModulePreference>,
) -> ActivationMode {
    saved.map_or_else(
        || {
            descriptor
                .activation_modes
                .first()
                .copied()
                .unwrap_or(ActivationMode::Manual)
        },
        |preference| preference.mode,
    )
}

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
    error: &crate::app::features::tuning::platform::contract::ModuleError,
) {
    let mut message = format!("{}: {:?}", error.operation, error.kind);
    if let Some(detail) = error.detail() {
        message.push_str(&format!(" — {detail}"));
    }
    if let Some(native) = &error.native {
        message.push_str(&format!(" ({}: {})", native.domain.as_str(), native.code));
    }
    ui.colored_label(ui.visuals().error_fg_color, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::AppStateStorage;
    use std::{
        sync::{Arc, RwLock},
        time::{Duration, Instant},
    };

    struct UiSave(Arc<std::sync::atomic::AtomicBool>);
    impl crate::app::features::tuning::StatePersistence for UiSave {
        fn save(
            &mut self,
            _: &mut AppStateStorage,
        ) -> Result<crate::app::features::tuning::service::SavedState, SaveError> {
            if self.0.load(std::sync::atomic::Ordering::SeqCst) {
                Err(SaveError::Storage)
            } else {
                Ok(crate::app::features::tuning::service::SavedState::Unavailable)
            }
        }
    }
    fn ui_fixture() -> (
        AppState,
        Arc<crate::app::features::tuning::simple_tests::Setting>,
        Arc<std::sync::atomic::AtomicBool>,
        ProfileId,
    ) {
        use crate::app::features::tuning::{platform::registry::Registry, Tuning, TuningRuntime};
        let state = AppStateStorage::test_fixture(|_| {});
        let setting = crate::app::features::tuning::simple_tests::Setting::new("ui_setting", false);
        let mut registry = Registry::default();
        registry.register(setting.clone()).unwrap();
        let mut production = Registry::default();
        crate::app::features::tuning::modules::diagnostics::register(&mut production).unwrap();
        for id in ["gpu_preference", "measurements"] {
            registry
                .register(
                    production
                        .get(
                            &crate::app::features::tuning::platform::contract::FeatureId::new(id)
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
    #[test]
    fn modules_ui_own_successful_save_closes_editor_without_stale_warning() {
        use crate::app::features::tuning::platform::registry::Module;
        let (mut app, setting, _, profile) = ui_fixture();
        let preference = ModulePreference {
            feature: setting.descriptor().id,
            enabled: true,
            mode: ActivationMode::Session,
            config: setting.config(12),
        };
        app.ui.tuning.editor = Some(EditorDraft {
            profile,
            expected: None,
            candidate: preference.clone(),
            pending_save_revision: Some(app.tuning_snapshot().revision),
        });
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference),
            expected: None,
        })
        .unwrap();
        wait_saved(&mut app);
        let text = rendered_text(&mut app);
        assert!(
            app.ui.tuning.editor.is_none(),
            "a confirmed own save must close the editor"
        );
        assert!(!text.contains("Saved settings changed"));
        finish(&mut app);
    }
    #[test]
    fn modules_ui_saved_edit_closes_even_when_native_restoration_needs_attention() {
        use crate::app::features::tuning::platform::contract::{
            ResourceId, ResourceKey, TargetIdentity,
        };
        use crate::app::features::tuning::platform::registry::Module;
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
        app.ui.tuning.editor = Some(EditorDraft {
            profile,
            expected: Some(original.clone()),
            candidate: candidate.clone(),
            pending_save_revision: Some(app.tuning_snapshot().revision),
        });
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
            app.ui.tuning.editor.is_none(),
            "a saved edit must not become stale when restoration fails"
        );
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
    fn modules_ui_failed_save_retains_editable_draft() {
        use crate::app::features::tuning::platform::registry::Module;
        let (mut app, setting, fail, profile) = ui_fixture();
        let preference = ModulePreference {
            feature: setting.descriptor().id,
            enabled: true,
            mode: ActivationMode::Session,
            config: setting.config(13),
        };
        fail.store(true, std::sync::atomic::Ordering::SeqCst);
        app.ui.tuning.editor = Some(EditorDraft {
            profile,
            expected: None,
            candidate: preference.clone(),
            pending_save_revision: Some(app.tuning_snapshot().revision),
        });
        app.request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference.clone()),
            expected: None,
        })
        .unwrap();
        wait_saved(&mut app);
        rendered_text(&mut app);
        assert_eq!(app.ui.tuning.editor.as_ref().unwrap().candidate, preference);
        assert!(
            app.ui
                .tuning
                .editor
                .as_ref()
                .unwrap()
                .pending_save_revision
                .is_none(),
            "failed save must allow retry"
        );
        finish(&mut app);
    }
    #[test]
    fn modules_ui_import_error_does_not_claim_saved_settings_or_activation_broke() {
        use crate::app::features::tuning::{
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
    fn modules_ui_groups_actions_and_keeps_technical_details_closed() {
        use crate::app::features::tuning::presentation::ModuleSection;
        let (mut app, _, _, _) = ui_fixture();
        let text = rendered_text(&mut app);
        assert!(text.contains("Test setting"));
        assert!(
            !text.contains("Preferred GPU"),
            "Windows actions do not belong in game settings"
        );
        app.ui.tuning.section = ModuleSection::Windows;
        let text = rendered_text(&mut app);
        assert!(text.contains("Open Windows Settings"));
        assert!(!text.contains("Test setting"));
        assert!(
            !text.contains("Current switch/preference"),
            "raw observations start collapsed"
        );
        finish(&mut app);
    }

    fn simple_descriptor(access: ModuleAccess) -> ModuleDescriptor {
        use crate::app::features::tuning::platform::contract::*;
        ModuleDescriptor {
            id: FeatureId::new("simple_card_test").unwrap(),
            config_version: SchemaVersion::new(1).unwrap(),
            title: "Test module",
            description: "Test card",
            access,
            target_scopes: vec![TargetScope::User],
            activation_modes: vec![ActivationMode::ForegroundSession],
            effect: Effect::Immediate,
        }
    }

    #[test]
    fn modules_observation_details_keep_last_device_and_omission_count_visible() {
        let summary = format!(
            "{}\nLast device: test endpoint\n12 additional devices omitted.",
            "device row\n".repeat(70)
        );
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            draw_observation(ui, &summary)
        });
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
        use crate::app::features::tuning::platform::contract::*;
        let error = ModuleError::new(ErrorKind::Denied, "read saved recovery")
            .with_detail("Recovery file cannot be opened.")
            .unwrap()
            .with_native(NativeError {
                domain: ResourceId::new("win32").unwrap(),
                code: 5,
            });
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            draw_operation_error(ui, &error)
        });
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
    fn simple_ui_unsaved_foreground_card_uses_supported_trigger() {
        let descriptor = simple_descriptor(ModuleAccess::Managed);
        let mode = selected_mode(&descriptor, None);
        let controls = card_controls(
            descriptor.access,
            &Capability::Available,
            true,
            false,
            descriptor.activation_modes.contains(&mode),
            false,
        );
        assert!(controls.enable, "first toggle must use a supported trigger");
        assert_eq!(mode, ActivationMode::ForegroundSession);
    }

    #[test]
    fn simple_ui_readonly_card_without_profile_does_not_request_profile() {
        let state = Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {})));
        let mut app = AppState::new_for_test(state, 4);
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            draw_module(
                &mut app,
                ui,
                &simple_descriptor(ModuleAccess::ReadOnly),
                false,
            );
        });
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("Choose a profile"), "{text}");
        assert!(text.contains("Information only"));
        finish(&mut app);
    }

    fn rendered_text(app: &mut AppState) -> String {
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |ui| draw_tuning(app, ui))
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
        use crate::app::features::tuning::{platform::registry::Registry, Tuning, TuningRuntime};
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
