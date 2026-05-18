use crate::app::features::diagnostics;
use crate::app::features::execution;
use crate::app::features::execution::InstalledPackageTrackingState;
use crate::app::models::RunningApps;
use crate::app::runtime::AppState;
use crate::app::shell::events::ShellEvent;
use crate::app::shell::presenters::{
    central, footer, group_editor, header, installed_app_picker, logs, run_settings,
};
use crate::app::shell::{GroupRoute, WindowRoute};
use crate::tray::{init_tray, TrayCmd};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::RwLock as TokioRwLock;

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
        Self::bootstrap_runtime(&mut state, &cc.egui_ctx, execution::spawn_monitors);

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

    fn bootstrap_runtime<F>(state: &mut AppState, _egui_ctx: &egui::Context, spawn_monitors: F)
    where
        F: FnOnce(
            Arc<TokioRwLock<RunningApps>>,
            Arc<RwLock<InstalledPackageTrackingState>>,
            Arc<RwLock<crate::app::models::AppStateStorage>>,
        ) -> Receiver<ShellEvent>,
    {
        diagnostics::log_startup(&mut state.log_manager, &state.persistent_state);
        state.runtime.monitor_rx = Some(spawn_monitors(
            state.runtime.running_apps_handle(),
            state.runtime.installed_package_tracking_handle(),
            state.persistent_state.clone(),
        ));
        state.start_app_with_autorun();
    }

    #[cfg(test)]
    fn new_for_test(state: AppState) -> Self {
        Self {
            state,
            tray_rx: None,
            #[cfg(target_os = "windows")]
            _tray_icon_guard: None,
            #[cfg(target_os = "windows")]
            hwnd: None,
            is_hidden: false,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_tray_events(ctx);
        self.handle_monitor_events(ctx);
        self.state.poll_installed_app_picker_refresh();

        if !self.should_render(ctx) {
            return;
        }

        self.apply_theme(ctx);
        self.handle_file_drops(ctx);
        self.render_main_ui(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::App;
    use crate::app::models::{AppStateStorage, CpuSchema};
    use crate::app::runtime::AppState;
    use crate::app::shell::events::ShellEvent;
    use eframe::egui;
    use std::sync::mpsc;
    use std::sync::{Arc, RwLock};

    fn sample_state() -> AppState {
        AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage {
                version: 5,
                groups: vec![],
                cpu_schema: CpuSchema {
                    model: "Test CPU".to_string(),
                    clusters: Vec::new(),
                },
                theme_index: 0,
                process_monitoring_enabled: false,
                rule_identities: None,
                loaded_version: 5,
                pending_pre_v6_backup: false,
            })),
            4,
        )
    }

    #[test]
    fn test_bootstrap_runtime_logs_startup_and_drains_monitor_notifications() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::channel();
        let mut state = sample_state();

        App::bootstrap_runtime(&mut state, &ctx, move |_, _, _| rx);

        assert!(state.runtime.monitor_rx.is_some());
        assert!(state
            .log_manager
            .entries
            .iter()
            .any(|entry| entry.message == "Application started"));
        assert!(state
            .log_manager
            .entries
            .iter()
            .any(|entry| entry.message.starts_with("Detected CPU:")));

        let mut app = App::new_for_test(state);
        tx.send(ShellEvent::Warning("WARNING: monitor warning".to_string()))
            .unwrap();
        tx.send(ShellEvent::Monitor("MONITOR: corrected".to_string()))
            .unwrap();

        app.handle_monitor_events(&ctx);

        assert!(app
            .state
            .log_manager
            .entries
            .iter()
            .any(|entry| entry.message == "WARNING: monitor warning"));
        assert!(app
            .state
            .log_manager
            .entries
            .iter()
            .any(|entry| entry.message == "MONITOR: corrected"));
        assert!(app
            .state
            .runtime
            .monitor_rx
            .as_ref()
            .is_some_and(|rx| rx.try_recv().is_err()));
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

    fn handle_monitor_events(&mut self, ctx: &egui::Context) {
        let mut repaint_requested = false;

        if let Some(rx) = &self.state.runtime.monitor_rx {
            while let Ok(event) = rx.try_recv() {
                if let Some((message, sticky)) = event.legacy_log_message() {
                    if sticky {
                        self.state.log_manager.add_sticky_once(message.to_string());
                    } else {
                        self.state.log_manager.add_entry(message.to_string());
                    }
                }

                repaint_requested |= event.needs_repaint();
            }
        }

        if repaint_requested {
            ctx.request_repaint();
        }
    }

    fn should_render(&mut self, ctx: &egui::Context) -> bool {
        if self.is_hidden {
            ctx.request_repaint_after(Duration::from_millis(250));
            return false;
        }

        if crate::app::adapters::os::supports_hide_to_tray()
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
            crate::app::adapters::os::set_taskbar_visible(hwnd, false);
            crate::app::adapters::os::restore_and_focus_window(hwnd);
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
            crate::app::adapters::os::set_taskbar_visible(hwnd, true);
            crate::app::adapters::os::restore_and_focus_window(hwnd);
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            100.0, 100.0,
        )));
        ctx.request_repaint();
    }

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
