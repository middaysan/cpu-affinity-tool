use crate::app::views::{central, footer, group_editor, header, logs, run_settings};

use crate::app::controllers;
use crate::app::models::AppState;

use crate::tray::{init_tray, TrayCmd};
use eframe::egui;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

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
        #[cfg(debug_assertions)]
        {
            println!("========================================================");
            println!("DEBUG: [Main Thread] App::new started");
            println!(
                "DEBUG: [Eframe] Backend: {}",
                if cc.gl.is_some() {
                    "Glow (OpenGL)"
                } else {
                    "WGPU"
                }
            );
            println!(
                "DEBUG: [Eframe] Integration Info: {:?}",
                cc.integration_info
            );
            cc.egui_ctx
                .options(|o| println!("DEBUG: [Egui] Context Options: {:?}", o));
            println!("========================================================");
        }

        let mut state = AppState::new(&cc.egui_ctx);
        let main_controller = controllers::MainController::new();

        // Get HWND on Windows
        #[cfg(target_os = "windows")]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = cc.window_handle() {
                let raw = handle.as_raw();
                match raw {
                    RawWindowHandle::Win32(h) => {
                        state.hwnd = Some(windows::Win32::Foundation::HWND(
                            h.hwnd.get() as *mut core::ffi::c_void
                        ));
                    }
                    _ => {
                        #[cfg(debug_assertions)]
                        println!("DEBUG: Not a Win32 window handle");
                    }
                }
            }
        }

        // Now that start_app_with_autorun is synchronous, we can call it directly
        state.start_app_with_autorun();

        // Initialize the system tray (Windows). On other OSes, init_tray() will return a stub.
        #[cfg(target_os = "windows")]
        let tray_res = if let Some(hwnd) = state.hwnd {
            init_tray(cc.egui_ctx.clone(), hwnd)
        } else {
            Err("HWND not found".to_string())
        };

        #[cfg(not(target_os = "windows"))]
        let tray_res = init_tray(cc.egui_ctx.clone());

        match tray_res {
            Ok(handle) => {
                // Save the receiver channel to the state
                state.tray_rx = Some(handle.rx);

                // On Windows, the TrayIcon must be kept alive
                #[cfg(target_os = "windows")]
                {
                    state.tray_icon_guard = Some(handle.tray_icon);
                }
            }
            Err(e) => {
                // Log the error but don't crash — the application will continue to work without the tray.
                state
                    .log_manager
                    .add_entry(format!("Tray init failed: {e}"));
            }
        }

        Self {
            state,
            main_controller,
        }
    }
}

impl eframe::App for App {
    /// The main update method called by the eframe framework on each frame.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Handle commands from the system tray
        self.handle_tray_events(ctx);

        // 2. Visibility check and minimization handling
        if !self.should_render(ctx) {
            return;
        }

        // 3. Apply UI theme
        self.apply_theme(ctx);

        // 4. Handle file drops
        self.handle_file_drops(ctx);

        // 5. Render main UI
        self.render_main_ui(ctx);

        // 6. Synchronize controller state
        self.sync_controller_state();
    }
}

impl App {
    /// Handles commands coming from the system tray.
    fn handle_tray_events(&mut self, ctx: &egui::Context) {
        let mut show_requested = false;

        if let Some(rx) = &self.state.tray_rx {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    TrayCmd::Show => show_requested = true,
                }
            }
        }

        if show_requested {
            self.show_from_tray(ctx);
        }
    }

    /// Checks if the UI should be rendered at the moment.
    /// Also handles the logic for hiding the application when minimized.
    fn should_render(&mut self, ctx: &egui::Context) -> bool {
        // If the application is hidden — limit the update frequency to save CPU
        if self.state.is_hidden {
            thread::sleep(Duration::from_millis(100));
            ctx.request_repaint();
            return false;
        }

        // If the user minimized the window — hide it to the tray
        if ctx.input(|i| i.viewport().minimized == Some(true)) {
            self.hide_to_tray(ctx);
            return false;
        }

        true
    }

    /// Hides the application window and switches it to tray mode.
    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.state.is_hidden = true;

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.state.hwnd {
            os_api::OS::set_taskbar_visible(hwnd, false);
            // Restore the window before moving, as minimized windows cannot be moved programmatically in Windows
            os_api::OS::restore_and_focus(hwnd);
        }

        // Move the window far off-screen instead of Visible(false) to avoid issues with restoration
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            -10000.0, -10000.0,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }

    /// Restores the application window from the tray.
    fn show_from_tray(&mut self, ctx: &egui::Context) {
        self.state.is_hidden = false;

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.state.hwnd {
            os_api::OS::set_taskbar_visible(hwnd, true);
            os_api::OS::restore_and_focus(hwnd);
        }

        // Return the window to the visible area
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            100.0, 100.0,
        )));
        ctx.request_repaint();
    }

    /// Applies the selected UI theme (light/dark).
    fn apply_theme(&self, ctx: &egui::Context) {
        let theme_index = self.state.get_theme_index();
        let visuals = match theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);
    }

    /// Handles file drops into the application window.
    fn handle_file_drops(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.dropped_files.is_empty()) {
            return;
        }

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

    /// Renders the main application interface.
    fn render_main_ui(&mut self, ctx: &egui::Context) {
        let app_state = &mut self.state;
        self.main_controller.render_with(ctx, |controller, ui_ctx| {
            header::draw_top_panel(app_state, ui_ctx);

            // Render content depending on the active controller
            Self::draw_active_view(app_state, ui_ctx, controller);

            footer::draw_bottom_panel(app_state, ui_ctx);
        });
    }

    /// Selects and renders the appropriate view depending on the controller state.
    fn draw_active_view(
        app_state: &mut AppState,
        ctx: &egui::Context,
        controller: &controllers::MainController,
    ) {
        match &controller.window_controller {
            controllers::WindowController::Groups(group_view) => match group_view {
                controllers::Group::ListGroups => central::draw_central_panel(app_state, ctx),
                controllers::Group::Create => group_editor::create_group_window(app_state, ctx),
                controllers::Group::Edit => group_editor::edit_group_window(app_state, ctx),
            },
            controllers::WindowController::Logs => logs::draw_logs_window(app_state, ctx),
            controllers::WindowController::AppRunSettings => {
                run_settings::draw_app_run_settings(app_state, ctx)
            }
        }
    }

    /// Synchronizes the controller state if it has been changed.
    fn sync_controller_state(&mut self) {
        if self.state.controller_changed {
            self.state.controller_changed = false;
            let current_window = self.state.current_window.clone();
            self.main_controller.set_window(current_window);
        }
    }
}
