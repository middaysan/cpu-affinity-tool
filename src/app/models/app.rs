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
        // 1. Обработка команд из системного трея
        self.handle_tray_events(ctx);

        // 2. Проверка видимости и обработка сворачивания
        if !self.should_render(ctx) {
            return;
        }

        // 3. Применение темы оформления
        self.apply_theme(ctx);

        // 4. Обработка перетаскивания файлов
        self.handle_file_drops(ctx);

        // 5. Отрисовка основного интерфейса
        self.render_main_ui(ctx);

        // 6. Синхронизация состояния контроллеров
        self.sync_controller_state();
    }
}

impl App {
    /// Обрабатывает команды, поступающие из системного трея.
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

    /// Проверяет, нужно ли отрисовывать UI в данный момент.
    /// Также обрабатывает логику скрытия приложения при сворачивании.
    fn should_render(&mut self, ctx: &egui::Context) -> bool {
        // Если приложение скрыто — ограничиваем частоту обновлений для экономии CPU
        if self.state.is_hidden {
            thread::sleep(Duration::from_millis(100));
            ctx.request_repaint();
            return false;
        }

        // Если пользователь свернул окно — скрываем его в трей
        if ctx.input(|i| i.viewport().minimized == Some(true)) {
            self.hide_to_tray(ctx);
            return false;
        }

        true
    }

    /// Скрывает окно приложения и переводит его в режим работы из трея.
    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.state.is_hidden = true;

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.state.hwnd {
            os_api::OS::set_taskbar_visible(hwnd, false);
            // Восстанавливаем окно перед перемещением, так как минимизированные окна нельзя программно двигать в Windows
            os_api::OS::restore_and_focus(hwnd);
        }

        // Убираем окно далеко за пределы экрана вместо Visible(false), чтобы избежать проблем с восстановлением
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            -10000.0, -10000.0,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }

    /// Восстанавливает окно приложения из трея.
    fn show_from_tray(&mut self, ctx: &egui::Context) {
        self.state.is_hidden = false;

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.state.hwnd {
            os_api::OS::set_taskbar_visible(hwnd, true);
            os_api::OS::restore_and_focus(hwnd);
        }

        // Возвращаем окно в видимую область
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            100.0, 100.0,
        )));
        ctx.request_repaint();
    }

    /// Применяет выбранную тему оформления (светлая/темная).
    fn apply_theme(&self, ctx: &egui::Context) {
        let theme_index = self.state.get_theme_index();
        let visuals = match theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);
    }

    /// Обрабатывает событие сброса файлов в окно приложения.
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

    /// Отрисовывает основной интерфейс приложения.
    fn render_main_ui(&mut self, ctx: &egui::Context) {
        let app_state = &mut self.state;
        self.main_controller.render_with(ctx, |controller, ui_ctx| {
            header::draw_top_panel(app_state, ui_ctx);

            // Отрисовываем содержимое в зависимости от активного контроллера
            Self::draw_active_view(app_state, ui_ctx, controller);

            footer::draw_bottom_panel(app_state, ui_ctx);
        });
    }

    /// Выбирает и отрисовывает нужный вид (view) в зависимости от состояния контроллера.
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

    /// Синхронизирует состояние контроллеров, если оно было изменено.
    fn sync_controller_state(&mut self) {
        if self.state.controller_changed {
            self.state.controller_changed = false;
            let current_window = self.state.current_window.clone();
            self.main_controller.set_window(current_window);
        }
    }
}
