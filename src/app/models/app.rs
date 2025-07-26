use crate::app::views::{central, group_editor, header, logs, run_settings};

use crate::app::controllers;
use crate::app::models::AppState;

use eframe::egui;
use std::path::PathBuf;

/// The main application structure that implements the eframe::App trait.
/// This is the core of the application that connects the state with controllers and views.
pub struct App {
    /// The application state that holds all data and configuration
    pub state: AppState,
    /// The main controller that handles the application's control flow
    pub main_controller: controllers::MainController,
}

impl App {
    /// Creates a new instance of the App with initialized state and controller.
    ///
    /// Initializes the application state with the provided context, creates a new
    /// main controller, and starts any applications marked for autorun.
    ///
    /// # Parameters
    ///
    /// * `cc` - The creation context provided by the eframe framework
    ///
    /// # Returns
    ///
    /// A new `App` instance with initialized state and controller
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut state = AppState::new(&cc.egui_ctx);
        let main_controller = controllers::MainController::new();
        state.start_app_with_autorun();

        Self {
            state,
            main_controller,
        }
    }
}

impl eframe::App for App {
    /// The main update method called by the eframe framework on each frame.
    ///
    /// This method is responsible for:
    /// 1. Requesting periodic repaints
    /// 2. Setting the UI theme based on theme index
    /// 3. Processing file drop events
    /// 4. Rendering the UI based on the current window controller
    /// 5. Handling controller changes
    ///
    /// # Parameters
    ///
    /// * `ctx` - The egui context for this frame
    /// * `_frame` - The eframe frame (unused in this implementation)
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request a repaint after 1 second to ensure the UI stays responsive
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        // Set the UI theme based on the theme index in the persistent state
        let visuals = match self.state.persistent_state.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);

        // Handle file drop events; check OS events and update dropped_files if any.
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            let files: Vec<PathBuf> = ctx.input(|i| {
                i.raw
                    .dropped_files
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
            // Branch into different views based on the current window controller.
            match &controller.window_controller {
                controllers::WindowController::Groups(group_view) => match group_view {
                    controllers::Group::ListGroups => {
                        central::draw_central_panel(app_state, ui_ctx);
                    }
                    controllers::Group::Create => {
                        group_editor::create_group_window(app_state, ui_ctx);
                    }
                    controllers::Group::Edit => {
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
            self.main_controller
                .set_window(app_state.current_window.clone());
        }
    }
}
