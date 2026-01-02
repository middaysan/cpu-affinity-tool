use crate::app::views::{central, footer, group_editor, header, logs, run_settings};

use crate::app::controllers;
use crate::app::models::AppState;

use eframe::egui;
use std::path::PathBuf;
use crate::tray::{init_tray, TrayCmd};

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

        // Get HWND on Windows
        #[cfg(target_os = "windows")]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = cc.window_handle() {
                let raw = handle.as_raw();
                match raw {
                    RawWindowHandle::Win32(h) => {
                        state.hwnd = Some(windows::Win32::Foundation::HWND(h.hwnd.get() as *mut core::ffi::c_void));
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

        // Инициализируем системный трей (Windows). На других ОС init_tray() вернёт заглушку.
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
                // Канал получателя сохраняем в состояние
                state.tray_rx = Some(handle.rx);

                // На Windows надо держать TrayIcon живым
                #[cfg(target_os = "windows")]
                {
                    state.tray_icon_guard = Some(handle.tray_icon);
                }
            }
            Err(e) => {
                // Логируем ошибку, но не падаем — приложение продолжит работать и без трея.
                state.log_manager.add_entry(format!("Tray init failed: {e}"));
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
        // Лог раз в ~5 секунд для проверки живучести
        #[cfg(debug_assertions)]
        {
            let current_time = ctx.input(|i| i.time);
            if current_time % 5.0 < 0.02 { // Грубая проверка
                println!("DEBUG: [Main Thread] update() is running. Time: {:.1}s", current_time);
            }
        }

        // Обработаем команды из трея перед отрисовкой
        if let Some(rx) = &self.state.tray_rx {
            while let Ok(cmd) = rx.try_recv() {
                #[cfg(debug_assertions)]
                println!("DEBUG: [Main Thread] Received command from tray: {:?}", cmd);
                match cmd {
                    TrayCmd::Show => {
                        #[cfg(debug_assertions)]
                        println!("DEBUG: [Main Thread] Executing Show (focus only)");
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
            }
        }

        // Перехватываем системную кнопку сворачивания для скрытия в трей
        if ctx.input(|i| i.viewport().minimized == Some(true)) {
            #[cfg(target_os = "windows")]
            if let Some(hwnd) = self.state.hwnd {
                #[cfg(debug_assertions)]
                println!("DEBUG: [Main Thread] Window minimized by system, hiding to tray");
                os_api::OS::hide_window(hwnd);
            }
        }

        // Set the UI theme based on the theme index in the persistent state
        let theme_index = self.state.get_theme_index();
        let visuals = match theme_index {
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

            // Draw the bottom panel (common for all views)
            footer::draw_bottom_panel(app_state, ui_ctx);
        });

        // If the window controller has been updated, notify the main panel.
        if app_state.controller_changed {
            app_state.controller_changed = false;
            self.main_controller
                .set_window(app_state.current_window.clone());
        }
    }
}
