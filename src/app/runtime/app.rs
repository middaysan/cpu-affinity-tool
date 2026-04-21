use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::{monitors, startup, AppState};
use crate::app::views::{
    central, footer, group_editor, header, installed_app_picker, logs, run_settings,
};
use crate::tray::{init_tray, TrayCmd};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

pub struct App {
    pub state: AppState,
    tray_rx: Option<Receiver<TrayCmd>>,
    #[cfg(target_os = "windows")]
    _tray_icon_guard: Option<tray_icon::TrayIcon>,
    #[cfg(target_os = "windows")]
    hwnd: Option<windows::Win32::Foundation::HWND>,
    is_hidden: bool,
}

impl App {
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

        let mut state = AppState::new();
        startup::log_startup(&mut state);
        state.runtime.monitor_rx = Some(monitors::spawn_monitors(
            state.runtime.running_apps.clone(),
            state.runtime.installed_package_tracking.clone(),
            state.persistent_state.clone(),
            cc.egui_ctx.clone(),
        ));

        #[cfg(target_os = "windows")]
        let mut hwnd = None;

        #[cfg(target_os = "windows")]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = cc.window_handle() {
                let raw = handle.as_raw();
                match raw {
                    RawWindowHandle::Win32(h) => {
                        hwnd = Some(windows::Win32::Foundation::HWND(
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

        state.start_app_with_autorun();

        #[cfg(target_os = "windows")]
        let tray_res = if let Some(hwnd_value) = hwnd {
            init_tray(cc.egui_ctx.clone(), hwnd_value)
        } else {
            Err("HWND not found".to_string())
        };

        #[cfg(not(target_os = "windows"))]
        let tray_res = init_tray(cc.egui_ctx.clone());

        match tray_res {
            Ok(handle) => {
                let tray_rx = Some(handle.rx);

                #[cfg(target_os = "windows")]
                let tray_icon_guard = Some(handle.tray_icon);

                Self {
                    state,
                    tray_rx,
                    #[cfg(target_os = "windows")]
                    _tray_icon_guard: tray_icon_guard,
                    #[cfg(target_os = "windows")]
                    hwnd,
                    is_hidden: false,
                }
            }
            Err(e) => {
                state
                    .log_manager
                    .add_sticky_once(format!("Tray init failed: {e}"));
                Self {
                    state,
                    tray_rx: None,
                    #[cfg(target_os = "windows")]
                    _tray_icon_guard: None,
                    #[cfg(target_os = "windows")]
                    hwnd,
                    is_hidden: false,
                }
            }
        }
    }
}

impl eframe::App for App {
    /// The main update method called by the eframe framework on each frame.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Handle commands from the system tray
        self.handle_tray_events(ctx);

        // 2. Handle notifications from background monitors
        self.handle_monitor_events();

        // 3. Poll background refresh tasks owned by AppState
        self.state.poll_installed_app_picker_refresh();

        // 4. Visibility check and minimization handling
        if !self.should_render(ctx) {
            return;
        }

        // 5. Apply UI theme
        self.apply_theme(ctx);

        // 6. Handle file drops
        self.handle_file_drops(ctx);

        // 7. Render main UI
        self.render_main_ui(ctx);
    }
}

impl App {
    fn handle_tray_events(&mut self, ctx: &egui::Context) {
        let mut show_requested = false;

        if let Some(rx) = &self.tray_rx {
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

    fn handle_monitor_events(&mut self) {
        if let Some(rx) = &self.state.runtime.monitor_rx {
            while let Ok(msg) = rx.try_recv() {
                if msg.starts_with("WARNING:") {
                    self.state.log_manager.add_sticky_once(msg);
                } else {
                    self.state.log_manager.add_entry(msg);
                }
            }
        }
    }

    fn should_render(&mut self, ctx: &egui::Context) -> bool {
        if self.is_hidden {
            ctx.request_repaint_after(Duration::from_millis(250));
            return false;
        }

        if os_api::OS::supports_hide_to_tray()
            && ctx.input(|i| i.viewport().minimized == Some(true))
        {
            self.hide_to_tray(ctx);
            return false;
        }

        true
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.is_hidden = true;

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.hwnd {
            os_api::OS::set_taskbar_visible(hwnd, false);
            os_api::OS::restore_and_focus(hwnd);
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            -10000.0, -10000.0,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }

    fn show_from_tray(&mut self, ctx: &egui::Context) {
        self.is_hidden = false;

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.hwnd {
            os_api::OS::set_taskbar_visible(hwnd, true);
            os_api::OS::restore_and_focus(hwnd);
        }

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
            self.state.ui.dropped_files = Some(files);
        }
    }

    fn render_main_ui(&mut self, ctx: &egui::Context) {
        header::draw_top_panel(&mut self.state, ctx);
        Self::draw_active_view(&mut self.state, ctx);
        footer::draw_bottom_panel(&mut self.state, ctx);
    }

    fn draw_active_view(app_state: &mut AppState, ctx: &egui::Context) {
        match app_state.ui.current_window.clone() {
            WindowRoute::Groups(group_route) => match group_route {
                GroupRoute::List => central::draw_central_panel(app_state, ctx),
                GroupRoute::Create => group_editor::create_group_window(app_state, ctx),
                GroupRoute::Edit => group_editor::edit_group_window(app_state, ctx),
            },
            WindowRoute::Logs => logs::draw_logs_window(app_state, ctx),
            WindowRoute::AppRunSettings => run_settings::draw_app_run_settings(app_state, ctx),
            WindowRoute::InstalledAppPicker => {
                installed_app_picker::draw_installed_app_picker(app_state, ctx)
            }
        }
    }
}
