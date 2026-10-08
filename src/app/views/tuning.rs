use crate::app::{
    features::tuning::{
        config::ModulePreference,
        platform::contract::{
            ActivationMode, Capability, ModuleAccess, ModuleDescriptor, ModuleStatus, ProfileId,
        },
        presentation::{
            card_controls, edit_configuration, mode_label, preference_with_enabled, scope_label,
            status_label, EditorDraft,
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

pub(crate) fn submit(app: &mut AppState, request: TuningRequest) {
    app.ui.tuning.error = app.request_tuning(request).err().map(save_error_message);
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
    let snapshot = app.tuning_snapshot();
    let editable = matches!(
        snapshot.configuration,
        TuningConfigurationState::Off | TuningConfigurationState::Known
    );
    let pending = busy(&snapshot.operation);
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
        ui.label("Save settings in a profile and choose when to apply them.");
        match snapshot.operation {
            TuningOperationState::Pending { .. } => { ui.spinner(); ui.label("Saving or applying settings…"); }
            TuningOperationState::Refused { error, .. } => { ui.colored_label(ui.visuals().error_fg_color, save_error_message(error)); }
            TuningOperationState::Recovering => { ui.colored_label(ui.visuals().error_fg_color, "Recovery required. Changes are paused."); }
            TuningOperationState::Idle => {}
        }
        if let Some(error) = app.ui.tuning.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(error) = &snapshot.operation_error {
            draw_operation_error(ui, error);
        }
        if !editable {
            ui.label("Saved module settings cannot be understood by this version. They are retained and read-only.");
            return;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(form_text_edit(&mut app.ui.tuning.profile_name).hint_text("Profile name").char_limit(128));
            if ui.add_enabled(!pending && !app.ui.tuning.profile_name.trim().is_empty(), egui::Button::new("Create profile")).clicked() {
                let name = app.ui.tuning.profile_name.trim().to_owned();
                submit(app, TuningRequest::CreateProfile { name });
            }
        });
        if !snapshot.profiles.is_empty() {
            let selected_name = snapshot.profiles.iter().find(|p| Some(p.id) == app.ui.tuning.selected_profile)
                .map_or("Select profile", |p| p.name.as_str());
            egui::ComboBox::from_id_salt("tuning-profile").selected_text(selected_name).show_ui(ui, |ui| {
                for profile in &snapshot.profiles {
                    if ui.selectable_value(&mut app.ui.tuning.selected_profile, Some(profile.id), &profile.name).changed() {
                        app.ui.tuning.editor = None;
                    }
                }
            });
            if let Some(profile) = snapshot.profiles.iter().find(|p| Some(p.id) == app.ui.tuning.selected_profile) {
                ui.horizontal(|ui| {
                    if ui.add_enabled(!pending && profile.modules.iter().any(|m| m.enabled), egui::Button::new("Disable profile")).clicked() {
                        submit(app, TuningRequest::DisableProfile { profile: profile.id });
                    }
                    if ui.add_enabled(!pending, egui::Button::new("Remove profile")).clicked() {
                        submit(app, TuningRequest::RemoveProfile { profile: profile.id });
                    }
                });
            }
        }
        ui.separator();
        if snapshot.catalog.is_empty() {
            ui.label("No modules are available in this build.");
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for descriptor in &snapshot.catalog {
                draw_module(app, ui, descriptor, pending);
            }
            if let Some(profile) = snapshot.profiles.iter().find(|p| Some(p.id) == app.ui.tuning.selected_profile) {
                for card in &profile.modules {
                    if !snapshot.catalog.iter().any(|d| d.id == card.feature) {
                        glass_frame(ui).show(ui, |ui| {
                            ui.strong(&card.title);
                            ui.label("This saved module is unavailable. Its configuration is retained and read-only.");
                            ui.label(if card.enabled { "Saved preference: enabled" } else { "Saved preference: disabled" });
                            if let Some(status) = card.status { ui.label(status_label(status)); }
                        });
                    }
                }
            }
        });
        if let Some(effect) = snapshot.restoration_effect {
            let text = match effect {
                crate::app::features::tuning::platform::contract::Effect::Immediate => "Restoration verified.",
                crate::app::features::tuning::platform::contract::Effect::GameRestartRequired => "Original settings restored; restart the game to take effect.",
                crate::app::features::tuning::platform::contract::Effect::RestartRequired => "Original settings restored; restart Windows to take effect.",
            };
            ui.label(text);
        }
    });
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
    let fallback = Capability::Unsupported(if profile.is_some() {
        "Save this module's preferences to observe its target.".into()
    } else {
        "Choose a profile to configure this module.".into()
    });
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
    glass_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.strong(descriptor.title);
        ui.label(descriptor.description);
        ui.label(format!("Scope: {}", descriptor.target_scopes.iter().map(|scope| scope_label(*scope)).collect::<Vec<_>>().join(", ")));
        ui.label(status_label(status));
        if let Capability::Unsupported(reason) | Capability::Denied(reason) = capability {
            if descriptor.access == ModuleAccess::Managed || card.is_some() {
            ui.label(reason.chars().take(512).collect::<String>());
            }
        }
        if let Some(saved) = &saved {
            ui.label(format!("Saved: {} · {}", if saved.enabled { "enabled" } else { "disabled" }, mode_label(saved.mode)));
            if let Some(summary) = app.tuning_editors().editor(&descriptor.id).and_then(|editor| editor.summary(&saved.config)) {
                ui.label(format!("Desired: {}", summary.chars().take(512).collect::<String>()));
            }
        }
        if let Some(observed) = card.and_then(|card| card.observed.as_ref()) {
            draw_observation(ui, &observed.summary);
        } else {
            ui.label("Not observed yet.");
        }
        if descriptor.access == ModuleAccess::ReadOnly {
            ui.label("Read only · no system settings are changed.");
            let action = app.tuning_editors().editor(&descriptor.id).and_then(|editor| editor.actions(ui));
            if let Some(action) = action {
                submit(app, TuningRequest::ModuleAction { feature: descriptor.id.clone(), action });
            }
            return;
        }
        if descriptor.target_scopes.contains(&crate::app::features::tuning::platform::contract::TargetScope::Process)
            && card.is_none_or(|card| card.targets.is_empty()) {
            ui.label("Configure the trigger now; application waits for a verified process from a saved rule.");
        }
        if !known { ui.label("Configuration is retained and read-only because its editor is unavailable."); }
        if let Some(profile) = profile {
            let mut enabled = saved.as_ref().is_some_and(|preference| preference.enabled);
            let can_toggle = if enabled { controls.disable } else { controls.enable };
            if ui.add_enabled(can_toggle, egui::Checkbox::new(&mut enabled, "Manage this module")).changed() {
                if let Some(editor) = app.tuning_editors().editor(&descriptor.id) {
                    let default = ModulePreference {
                        feature: descriptor.id.clone(), enabled: false,
                        mode: descriptor.activation_modes.first().copied().unwrap_or(ActivationMode::Manual),
                        config: editor.default_config(),
                    };
                    let preference = preference_with_enabled(saved.as_ref(), default, enabled);
                    submit(app, TuningRequest::SetModulePreference { profile, preference: Box::new(preference), expected: saved.clone() });
                }
            }
            if ui.add_enabled(controls.configure, egui::Button::new("Configure")).clicked() {
                if let Some(editor) = app.tuning_editors().editor(&descriptor.id) {
                    let candidate = saved.clone().unwrap_or_else(|| ModulePreference {
                        feature: descriptor.id.clone(), enabled: false,
                        mode: descriptor.activation_modes.first().copied().unwrap_or(ActivationMode::Manual),
                        config: editor.default_config(),
                    });
                    app.ui.tuning.editor = Some(EditorDraft { profile, expected: saved.clone(), candidate });
                }
            }
            if let Some(mut draft) = app.ui.tuning.editor.take() {
                if draft.profile == profile && draft.candidate.feature == descriptor.id {
                    let stale = draft.expected != saved;
                    let mut save = false;
                    let mut cancel = false;
                    ui.add_enabled_ui(!pending && !stale, |ui| {
                        let mut valid = false;
                        if let Some(editor) = app.tuning_editors().editor(&descriptor.id) {
                            valid = edit_configuration(editor, ui, &mut draft.candidate.config);
                            if !valid {
                                ui.label("This configuration cannot be edited by this version.");
                            }
                        }
                        egui::ComboBox::from_id_salt(("tuning-mode", profile.get(), descriptor.id.as_str())).selected_text(mode_label(draft.candidate.mode)).show_ui(ui, |ui| {
                            for mode in &descriptor.activation_modes { ui.selectable_value(&mut draft.candidate.mode, *mode, mode_label(*mode)); }
                        });
                        ui.add_enabled_ui(valid, |ui| { ui.checkbox(&mut draft.candidate.enabled, "Enabled in this profile"); });
                        save = ui.add_enabled(valid, egui::Button::new("Save module settings")).clicked();
                    });
                    if stale { ui.label("Saved settings changed. Reopen the editor before saving."); }
                    cancel |= ui.button("Cancel editor").clicked();
                    if save {
                        submit(app, TuningRequest::SetModulePreference { profile, preference: Box::new(draft.candidate.clone()), expected: draft.expected.clone() });
                    }
                    if !cancel { app.ui.tuning.editor = Some(draft); }
                } else {
                    app.ui.tuning.editor = Some(draft);
                }
            }
            if controls.disable && ui.button("Disable module").clicked() {
                if let Some(mut preference) = saved.clone() {
                    preference.enabled = false;
                    submit(app, TuningRequest::SetModulePreference { profile, preference: Box::new(preference), expected: saved.clone() });
                }
            }
            if let Some(card) = card.filter(|card| card.enabled && card.mode == ActivationMode::Manual) {
                ui.label("After manual application, settings stay active after closing the app. Disable the module to restore them.");
                let selected = app.ui.tuning.manual_target.as_ref().filter(|(id, feature, target)| {
                    *id == profile && *feature == descriptor.id && card.targets.contains(target)
                }).map(|(_, _, target)| target.clone());
                let mut target = selected.or_else(|| (card.targets.len() == 1).then(|| card.targets[0].clone()));
                if card.targets.len() > 1 {
                    egui::ComboBox::from_id_salt(("tuning-target", profile.get(), descriptor.id.as_str()))
                        .selected_text(target.as_ref().map_or("Select target", |target| scope_label(target.scope())))
                        .show_ui(ui, |ui| {
                            for (index, candidate) in card.targets.iter().enumerate() {
                                ui.selectable_value(&mut target, Some(candidate.clone()), format!("{} {}", scope_label(candidate.scope()), index + 1));
                            }
                        });
                }
                if let Some(target) = target {
                    app.ui.tuning.manual_target = Some((profile, descriptor.id.clone(), target.clone()));
                    if ui.add_enabled(controls.apply && !matches!(status, ModuleStatus::VerifiedActive(_)), egui::Button::new("Apply manually")).clicked() {
                        submit(app, TuningRequest::ActivateManual { profile, feature: descriptor.id.clone(), target });
                    }
                }
            }
        }
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
        assert!(text.contains("Read only"));
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

    #[test]
    fn p5_empty_shipping_catalog_page_does_not_invent_active_modules() {
        let state = Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {})));
        let mut app = AppState::new_for_test(state, 4);
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
        let mut app = AppState::new_for_test(state.clone(), 4);
        let text = rendered_text(&mut app);
        assert!(text.contains("retained and read-only"));
        assert!(!text.contains("Create profile"));
        assert_eq!(state.read().unwrap().tuning.raw(), Some(raw));
        finish(&mut app);
    }
}
