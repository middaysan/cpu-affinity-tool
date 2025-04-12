use crate::app::views::{run_settings, central, group_editor, header, logs};

use crate::app::controllers;
use crate::app::models::AffinityAppState;

use std::path::PathBuf;
use eframe::egui;

pub struct AffinityApp {
    pub state: AffinityAppState,
    pub main_controller: controllers::MainController,
}

impl AffinityApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let state = AffinityAppState::new(&cc.egui_ctx);
        let main_controller = controllers::MainController::new();
 
        Self { state, main_controller: main_controller }
    }
}

impl eframe::App for AffinityApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Set theme based on the persistent state.

        let visuals = match self.state.persistent_state.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);


        // Handle file drop events; check OS events and update dropped_files if any.
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            let files: Vec<PathBuf> = ctx.input(|i| {
                i.raw.dropped_files
                    .iter()
                    .filter_map(|f| f.path.clone())
                    .collect()
            });
            if !files.is_empty() {
                self.state.dropped_files = Some(files);
            }
        }

        // Render UI based on the current window controller.
        let app_state = &mut self.state;
        self.main_controller.render_with(ctx, |controller, ui_ctx| {
            // Draw the top panel (common for all views)
            header::draw_top_panel(app_state, ui_ctx);
            // Branch into different views based on current window controller.
            match &controller.window_controller {
                controllers::WindowController::Groups(group_view) => match group_view {
                    controllers::Group::ListGroups => {
                        central::draw_central_panel(app_state, ui_ctx);
                    }
                    controllers::Group::CreateGroup => {
                        group_editor::create_group_window(app_state, ui_ctx);
                    }
                    controllers::Group::EditGroup => {
                        group_editor::edit_group_window(app_state, ui_ctx);
                    }
                },
                controllers::WindowController::Logs => {
                    logs::draw_logs_window(app_state, ui_ctx);
                }
                controllers::WindowController::AppRunSettings => {
                    run_settings::draw_app_run_settings(app_state, ui_ctx);
                }
            }
        });

        // If the window controller has been updated, notify the main panel.
        if app_state.controller_changed {
            app_state.controller_changed = false;
            self.main_controller.set_window(app_state.current_window.clone());
        }
    }
}
