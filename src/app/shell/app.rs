use crate::app::features::diagnostics;
use crate::app::features::execution;
use crate::app::features::execution::{InstalledPackageTrackingState, MonitorEventReceiver};
#[cfg(test)]
use crate::app::instance_forwarding::ForwardedIpcCommand;
#[cfg(any(test, all(target_os = "windows", feature = "windows")))]
use crate::app::instance_forwarding::{
    parse_ipc_command_frame, run_rule_outcome_to_response, serialize_ipc_response_frame,
    IpcCommand, IpcResponse, IpcResponseCode,
};
use crate::app::models::RunningApps;
use crate::app::runtime::{AppState, RunRuleOutcome};
#[cfg(all(target_os = "windows", feature = "windows"))]
use crate::app::shell::presenters::crash_reports;
use crate::app::shell::presenters::{
    central, group_editor, header, installed_app_picker, logs, run_settings, service_controls,
    system_tweaks,
};
#[cfg(all(target_os = "windows", feature = "windows"))]
use crate::app::shell::sessions::ShortcutCreationRole;
use crate::app::shell::{GroupRoute, WindowRoute};
use crate::app::startup::StartupIntent;
use crate::tray::{init_tray, TrayCmd, TrayRuntime};
use eframe::egui;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::RwLock as TokioRwLock;

pub struct App {
    pub state: AppState,
    #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
    debug_mcp: Option<diagnostics::debug_mcp::Bridge>,
    tray_runtime: Option<TrayRuntime>,
    #[cfg(test)]
    forwarded_command_rx: Option<Receiver<ForwardedIpcCommand>>,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    forwarding_runtime: Option<AppForwardingRuntime>,
    #[cfg(target_os = "windows")]
    hwnd: Option<windows::Win32::Foundation::HWND>,
    #[cfg(target_os = "windows")]
    winit_window: Option<Arc<winit::window::Window>>,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    file_drop_guard: Option<os_api::WindowFileDropGuard>,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    crash_report_viewport_focused: Option<bool>,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    windows_event_log_first_frame_rendered: bool,
    closing_requested: bool,
    is_hidden: bool,
    startup_visibility_pending: bool,
    restore_pending: bool,
}

const MONITOR_EVENT_GUI_DRAIN_LIMIT: usize = 64;

fn file_import_allowed(route: &WindowRoute, graph_open: bool) -> bool {
    matches!(route, WindowRoute::Groups(GroupRoute::List)) && !graph_open
}

#[test]
fn file_import_requires_visible_group_cards_without_a_graph() {
    for route in [
        WindowRoute::Logs,
        WindowRoute::Tuning,
        WindowRoute::AppRunSettings,
        WindowRoute::InstalledAppPicker,
        WindowRoute::Groups(GroupRoute::Edit),
    ] {
        assert!(!file_import_allowed(&route, false));
    }
    assert!(file_import_allowed(
        &WindowRoute::Groups(GroupRoute::List),
        false
    ));
    assert!(!file_import_allowed(
        &WindowRoute::Groups(GroupRoute::List),
        true
    ));
}

#[cfg(all(target_os = "windows", feature = "windows"))]
fn append_shell_drop(
    input: &mut egui::RawInput,
    drop: os_api::WindowFileDrop,
    pixels_per_point: f32,
) {
    #[derive(Debug)]
    struct ShellFile(PathBuf);
    impl egui::DroppedFile for ShellFile {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("File import uses paths only".into())
        }
    }
    let position = egui::pos2(drop.position[0] as f32, drop.position[1] as f32) / pixels_per_point;
    input.events.push(egui::Event::PointerMoved(position));
    input.dropped_files.extend(
        drop.paths
            .into_iter()
            .map(|path| Arc::new(ShellFile(path)) as egui::DroppedFileHandle),
    );
}

#[cfg(any(test, target_os = "windows"))]
fn set_taskbar_visibility_with(
    visible: bool,
    set_skip_taskbar: impl FnOnce(bool),
    set_native_style: impl FnOnce(bool),
) {
    set_skip_taskbar(!visible);
    set_native_style(visible);
}

fn should_start_minimized(enabled: bool, supported: bool, tray_available: bool) -> bool {
    enabled && supported && tray_available
}

#[test]
fn start_minimized_requires_a_reachable_tray() {
    assert!(should_start_minimized(true, true, true));
    assert!(!should_start_minimized(false, true, true));
    assert!(!should_start_minimized(true, true, false));
    assert!(!should_start_minimized(true, false, true));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayCommandAction {
    None,
    Show,
    Quit,
}

fn reduce_tray_commands(
    closing_requested: bool,
    commands: impl IntoIterator<Item = TrayCmd>,
) -> TrayCommandAction {
    if closing_requested {
        return TrayCommandAction::Quit;
    }

    let mut action = TrayCommandAction::None;
    for command in commands {
        match command {
            TrayCmd::Show => {
                if action != TrayCommandAction::Quit {
                    action = TrayCommandAction::Show;
                }
            }
            TrayCmd::Quit => action = TrayCommandAction::Quit,
        }
    }
    action
}

fn should_hide_to_tray(
    platform_supports_hide_to_tray: bool,
    has_tray_runtime: bool,
    minimized: Option<bool>,
    restore_pending: bool,
) -> bool {
    platform_supports_hide_to_tray
        && has_tray_runtime
        && !restore_pending
        && minimized == Some(true)
}

#[cfg(any(test, all(target_os = "windows", feature = "windows")))]
fn viewport_gained_focus(previous: Option<bool>, current: Option<bool>) -> bool {
    matches!((previous, current), (Some(false), Some(true)))
}

fn theme_preference_for_index(theme_index: usize) -> egui::ThemePreference {
    match theme_index {
        0 => egui::ThemePreference::System,
        1 => egui::ThemePreference::Light,
        _ => egui::ThemePreference::Dark,
    }
}

#[cfg(all(target_os = "windows", feature = "windows"))]
struct ForwardingServerLifetime<Guard, Server> {
    guard: Option<Guard>,
    server: Option<Server>,
}

#[cfg(all(target_os = "windows", feature = "windows"))]
impl<Guard, Server> Drop for ForwardingServerLifetime<Guard, Server> {
    fn drop(&mut self) {
        // A pending runtime still owns its guard. Once started, LocalIpcServer transfers that
        // guard to its worker so an unfinished cancellation cannot admit a replacement owner.
        drop(self.server.take());
        drop(self.guard.take());
    }
}

#[cfg(all(target_os = "windows", feature = "windows"))]
pub struct AppForwardingRuntime {
    endpoint: os_api::LocalIpcEndpoint,
    lifetime: ForwardingServerLifetime<os_api::LocalIpcGuard, os_api::LocalIpcServer>,
}

#[cfg(all(target_os = "windows", feature = "windows"))]
impl AppForwardingRuntime {
    pub fn pending(guard: os_api::LocalIpcGuard, endpoint: os_api::LocalIpcEndpoint) -> Self {
        Self {
            endpoint,
            lifetime: ForwardingServerLifetime {
                guard: Some(guard),
                server: None,
            },
        }
    }

    fn start_server(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.lifetime.server.is_some() {
            return Ok(());
        }

        let repaint_ctx = ctx.clone();
        let wake: os_api::LocalIpcWake = Arc::new(move || {
            repaint_ctx.request_repaint();
        });
        let guard = self
            .lifetime
            .guard
            .take()
            .expect("pending forwarding runtime must retain its primary guard");
        let server = os_api::OS::start_local_ipc_server_with_wake_and_primary_guard(
            &self.endpoint,
            guard,
            Some(wake),
        )?;
        self.lifetime.server = Some(server);
        Ok(())
    }

    fn server(&self) -> Option<&os_api::LocalIpcServer> {
        self.lifetime.server.as_ref()
    }

    #[cfg(test)]
    fn server_started(&self) -> bool {
        self.server().is_some()
    }
}

impl App {
    #[cfg_attr(all(feature = "windows", not(feature = "linux")), allow(dead_code))]
    pub fn new_with_startup_intent(
        cc: &eframe::CreationContext<'_>,
        startup_intent: StartupIntent,
    ) -> Self {
        let mut app = Self::new_without_startup_intent(cc);
        app.handle_startup_intent_after_forwarding(startup_intent, false);
        app
    }

    pub fn new_without_startup_intent(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_state(cc, AppState::new())
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn new_with_storage(
        cc: &eframe::CreationContext<'_>,
        storage: crate::app::adapters::storage::StorageAdapter,
    ) -> Self {
        Self::new_with_state(cc, AppState::from_storage(storage))
    }

    fn new_with_state(cc: &eframe::CreationContext<'_>, mut state: AppState) -> Self {
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

        crate::app::shell::presenters::shared_elements::install_ui_fonts(&cc.egui_ctx);

        let monitor_ctx = cc.egui_ctx.clone();
        let wake: execution::MonitorWake = Arc::new(move || monitor_ctx.request_repaint());
        state.runtime.set_wake(wake.clone());
        Self::bootstrap_runtime_without_startup(
            &mut state,
            move |running_apps, running_app_statuses, package_tracking, persistent_state| {
                execution::spawn_monitors_with_wake(
                    running_apps,
                    running_app_statuses,
                    package_tracking,
                    persistent_state,
                    Some(wake),
                )
            },
        );

        #[cfg(target_os = "windows")]
        let mut hwnd = None;

        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
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
        let tray_res = init_tray(cc.egui_ctx.clone());

        #[cfg(all(target_os = "windows", feature = "windows"))]
        let file_drop_guard = hwnd.and_then(|hwnd| {
            let repaint = cc.egui_ctx.clone();
            match os_api::WindowFileDropGuard::install(
                hwnd.0 as usize,
                Arc::new(move || repaint.request_repaint()),
            ) {
                Ok(guard) => Some(guard),
                Err(error) => {
                    state.log_manager.add_important_sticky_once(format!(
                        "Shell file drops unavailable: {error}"
                    ));
                    None
                }
            }
        });

        #[cfg(not(target_os = "windows"))]
        let tray_res = init_tray(cc.egui_ctx.clone());

        #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
        let debug_mcp =
            diagnostics::debug_mcp::Bridge::start(&cc.egui_ctx).unwrap_or_else(|error| {
                state
                    .log_manager
                    .add_sticky_once(format!("Debug MCP: {error}"));
                None
            });
        let start_hidden = should_start_minimized(
            state.start_minimized(),
            crate::app::adapters::os::supports_hide_to_tray(),
            tray_res.is_ok(),
        );
        match tray_res {
            Ok(tray_runtime) => Self {
                state,
                #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
                debug_mcp,
                tray_runtime: Some(tray_runtime),
                #[cfg(test)]
                forwarded_command_rx: None,
                #[cfg(all(target_os = "windows", feature = "windows"))]
                forwarding_runtime: None,
                #[cfg(target_os = "windows")]
                hwnd,
                #[cfg(target_os = "windows")]
                winit_window: cc.winit_window().cloned(),
                #[cfg(all(target_os = "windows", feature = "windows"))]
                file_drop_guard,
                #[cfg(all(target_os = "windows", feature = "windows"))]
                crash_report_viewport_focused: None,
                #[cfg(all(target_os = "windows", feature = "windows"))]
                windows_event_log_first_frame_rendered: false,
                closing_requested: false,
                is_hidden: start_hidden,
                startup_visibility_pending: true,
                restore_pending: false,
            },
            Err(e) => {
                state
                    .log_manager
                    .add_sticky_once(format!("Tray init failed: {e}"));
                Self {
                    state,
                    #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
                    debug_mcp,
                    tray_runtime: None,
                    #[cfg(test)]
                    forwarded_command_rx: None,
                    #[cfg(all(target_os = "windows", feature = "windows"))]
                    forwarding_runtime: None,
                    #[cfg(target_os = "windows")]
                    hwnd,
                    #[cfg(target_os = "windows")]
                    winit_window: cc.winit_window().cloned(),
                    #[cfg(all(target_os = "windows", feature = "windows"))]
                    file_drop_guard,
                    #[cfg(all(target_os = "windows", feature = "windows"))]
                    crash_report_viewport_focused: None,
                    #[cfg(all(target_os = "windows", feature = "windows"))]
                    windows_event_log_first_frame_rendered: false,
                    closing_requested: false,
                    is_hidden: false,
                    startup_visibility_pending: true,
                    restore_pending: false,
                }
            }
        }
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn install_forwarding_runtime(
        &mut self,
        mut forwarding_runtime: Option<AppForwardingRuntime>,
        warning: Option<String>,
        ctx: &egui::Context,
    ) -> bool {
        let mut server_start_warning = None;
        let shortcut_role = if let Some(runtime) = forwarding_runtime.as_mut() {
            match runtime.start_server(ctx) {
                Ok(()) => ShortcutCreationRole::Primary,
                Err(err) => {
                    server_start_warning = Some(err);
                    forwarding_runtime = None;
                    ShortcutCreationRole::Unsupported
                }
            }
        } else if warning.as_deref() == Some("another instance owns the local shortcut endpoint") {
            ShortcutCreationRole::NonPrimary
        } else {
            ShortcutCreationRole::Unsupported
        };
        self.state.set_shortcut_creation_role(shortcut_role);

        if let Some(warning) = warning {
            self.state
                .log_manager
                .add_sticky_once(format!("Shortcut forwarding disabled: {warning}"));
        }
        if let Some(warning) = server_start_warning {
            self.state.log_manager.add_sticky_once(format!(
                "Shortcut forwarding server failed to start: {warning}"
            ));
        }
        let server_ready = forwarding_runtime.is_some();
        self.forwarding_runtime = forwarding_runtime;
        server_ready
    }

    #[cfg(test)]
    fn bootstrap_runtime<F>(
        state: &mut AppState,
        _egui_ctx: &egui::Context,
        startup_intent: StartupIntent,
        spawn_monitors: F,
    ) where
        F: FnOnce(
            Arc<TokioRwLock<RunningApps>>,
            execution::RunningAppStatusCache,
            Arc<RwLock<InstalledPackageTrackingState>>,
            Arc<RwLock<crate::app::models::AppStateStorage>>,
        ) -> MonitorEventReceiver,
    {
        Self::bootstrap_runtime_without_startup(state, spawn_monitors);
        Self::handle_startup_intent(state, startup_intent);
    }

    fn bootstrap_runtime_without_startup<F>(state: &mut AppState, spawn_monitors: F)
    where
        F: FnOnce(
            Arc<TokioRwLock<RunningApps>>,
            execution::RunningAppStatusCache,
            Arc<RwLock<InstalledPackageTrackingState>>,
            Arc<RwLock<crate::app::models::AppStateStorage>>,
        ) -> MonitorEventReceiver,
    {
        diagnostics::log_startup(&mut state.log_manager, &state.persistent_state);
        state.runtime.monitor_rx = Some(spawn_monitors(
            state.runtime.running_apps_handle(),
            state.runtime.running_app_statuses_handle(),
            state.runtime.installed_package_tracking_handle(),
            state.persistent_state.clone(),
        ));
    }

    pub fn handle_startup_intent_after_forwarding(
        &mut self,
        startup_intent: StartupIntent,
        forwarding_failed: bool,
    ) {
        if forwarding_failed && matches!(startup_intent, StartupIntent::RunRule { .. }) {
            self.is_hidden = false;
            self.state.log_manager.add_important_sticky_once(
                "ERROR: Shortcut launch was blocked because shortcut forwarding did not start"
                    .to_string(),
            );
            return;
        }

        Self::handle_startup_intent(&mut self.state, startup_intent);
    }

    fn handle_startup_intent(state: &mut AppState, startup_intent: StartupIntent) {
        match startup_intent {
            StartupIntent::NormalGui => state.start_app_with_autorun(),
            StartupIntent::RunRule { group_id, rule_id } => {
                match state.run_group_program(group_id, rule_id) {
                    RunRuleOutcome::Accepted | RunRuleOutcome::LaunchRejected(_) => {}
                    RunRuleOutcome::MissingGroup => state.log_manager.add_important_sticky_once(
                        "ERROR: Shortcut launch group was not found".to_string(),
                    ),
                    RunRuleOutcome::MissingRule => state.log_manager.add_important_sticky_once(
                        "ERROR: Shortcut launch rule was not found".to_string(),
                    ),
                }
            }
        }
    }

    #[cfg(test)]
    fn new_for_test(state: AppState) -> Self {
        Self {
            state,
            #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
            debug_mcp: None,
            tray_runtime: None,
            #[cfg(test)]
            forwarded_command_rx: None,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            forwarding_runtime: None,
            #[cfg(target_os = "windows")]
            hwnd: None,
            #[cfg(target_os = "windows")]
            winit_window: None,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            file_drop_guard: None,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            crash_report_viewport_focused: None,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            windows_event_log_first_frame_rendered: false,
            closing_requested: false,
            is_hidden: false,
            startup_visibility_pending: false,
            restore_pending: false,
        }
    }
}

impl eframe::App for App {
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn repaint_delay(&mut self, ctx: &egui::Context, input: &egui::RawInput) -> Duration {
        if self.closing_requested || self.is_hidden || self.restore_pending {
            return Duration::ZERO;
        }
        super::render_preparation::scroll_repaint_delay(
            ctx,
            input,
            self.state
                .ui
                .monitoring_modal_open(self.state.cpu_load.graph.is_some())
                || self.state.mcp.modal_open(),
        )
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.state.cpu_load.export_input(ctx, input);
        if !self.closing_requested && self.state.ui.dropped_files.is_none() {
            if let Some(drop) = self
                .file_drop_guard
                .as_ref()
                .and_then(|guard| guard.next_drop())
            {
                append_shell_drop(input, drop, ctx.pixels_per_point());
            }
        }
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        if let Some(bridge) = &mut self.debug_mcp {
            bridge.raw_input(&mut self.state, ctx, input);
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(all(target_os = "windows", feature = "render-profiling"))]
        diagnostics::render_profile::begin_logic(ctx.cumulative_frame_nr());
        #[cfg(all(target_os = "windows", feature = "render-profiling"))]
        if ctx.input(|i| i.key_pressed(egui::Key::F8)) {
            diagnostics::render_profile::note("user_stutter_marker");
        }
        crate::render_profile_scope!(GuiLogic);
        if ctx.input(|input| input.viewport().close_requested()) {
            self.begin_close(ctx);
            if self.state.tuning_shutdown_outcome().is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        self.handle_tray_events(ctx);
        #[cfg(all(target_os = "windows", feature = "windows"))]
        self.state.poll_mcp(ctx);
        if self.closing_requested {
            #[cfg(all(target_os = "windows", feature = "windows"))]
            self.handle_local_ipc_requests(ctx);
            #[cfg(test)]
            self.handle_forwarded_commands(ctx);
            self.state.poll_tuning();
            if self.state.tuning_shutdown_outcome().is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            return;
        }
        let tuning_visible =
            !self.is_hidden && matches!(self.state.ui.current_window, WindowRoute::Tuning);
        #[cfg(all(target_os = "windows", feature = "windows"))]
        let tuning_visible = tuning_visible && self.state.cpu_load.graph.is_none();
        self.state.set_tuning_cards_visible(tuning_visible);
        if self.state.poll_tuning() {
            ctx.request_repaint();
        }
        if let Some(interval) = self.state.tuning_poll_interval() {
            ctx.request_repaint_after(interval);
        }
        #[cfg(all(target_os = "windows", feature = "windows"))]
        {
            self.state.poll_cpu_load(!self.is_hidden);
            if self.state.cpu_load_enabled() && !self.is_hidden {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        self.handle_monitor_events(ctx);
        #[cfg(all(target_os = "windows", feature = "windows"))]
        self.handle_local_ipc_requests(ctx);
        #[cfg(test)]
        self.handle_forwarded_commands(ctx);
        self.state.poll_installed_app_picker_refresh();
        #[cfg(all(target_os = "windows", feature = "windows"))]
        {
            let viewport_focused = ctx.input(|input| input.viewport().focused);
            if viewport_gained_focus(self.crash_report_viewport_focused, viewport_focused) {
                self.state.request_crash_report_refresh();
            }
            if viewport_focused.is_some() {
                self.crash_report_viewport_focused = viewport_focused;
            }
            if self.state.poll_crash_report_refresh() {
                ctx.request_repaint();
            }
            if let Some(interval) = self.state.crash_report_worker_poll_interval() {
                ctx.request_repaint_after(interval);
            }
            if self.windows_event_log_first_frame_rendered
                && self.state.start_windows_event_log_scan_after_shell_gate()
            {
                ctx.request_repaint();
            }
            if self.state.poll_windows_event_log() {
                ctx.request_repaint();
            }
            if let Some(interval) = self.state.windows_event_log_worker_poll_interval() {
                ctx.request_repaint_after(interval);
            }
        }

        if !self.should_render(ctx) {
            return;
        }

        self.apply_theme(ctx);
        self.handle_file_drops(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(all(target_os = "windows", feature = "render-profiling"))]
        diagnostics::render_profile::begin_logic(ui.ctx().cumulative_frame_nr());
        #[cfg(all(target_os = "windows", feature = "render-profiling"))]
        let _profile_frame = diagnostics::render_profile::frame_end(
            diagnostics::render_profile::View {
                mode: if self.state.cpu_load.graph.is_some() {
                    if self.state.cpu_load.show_processes {
                        "process_table"
                    } else {
                        "cpu_graph"
                    }
                } else {
                    match &self.state.ui.current_window {
                        WindowRoute::Groups(_) => "groups",
                        WindowRoute::Logs => "logs",
                        WindowRoute::Tuning => "tweaks",
                        _ => "other",
                    }
                },
                selected_cpu: self.state.cpu_load.selected_cpu,
                all_cpus: self.state.cpu_load.show_all_cpus,
                input_events: ui.ctx().input(|i| i.events.len()),
                pass: ui.ctx().current_pass_index(),
            },
            _frame.info().cpu_usage,
        );
        crate::render_profile_scope!(GuiUi);
        if self.closing_requested || self.is_hidden {
            return;
        }

        #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
        if let Some(bridge) = &self.debug_mcp {
            bridge.begin_ui(ui.ctx());
        }
        #[cfg(all(target_os = "windows", feature = "windows"))]
        let draw_active = !self
            .state
            .ui
            .monitoring_modal_open(self.state.cpu_load.graph.is_some())
            && !self.state.mcp.modal_open();
        #[cfg(not(all(target_os = "windows", feature = "windows")))]
        let draw_active = !self.state.ui.monitoring_modal_open(false);
        self.render_main_ui(ui, draw_active);
        #[cfg(all(target_os = "windows", feature = "windows"))]
        crate::app::features::cpu_load::view::draw_modal(&mut self.state, ui.ctx());
        #[cfg(all(target_os = "windows", feature = "windows"))]
        {
            self.state.mcp.draw(ui.ctx());
        }
        if self.state.runtime.take_status_read_pending() {
            // A cached status read must not consume the only runtime notification.
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        #[cfg(all(target_os = "windows", feature = "windows"))]
        {
            self.windows_event_log_first_frame_rendered = true;
        }
        #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
        if let Some(bridge) = &mut self.debug_mcp {
            bridge.end_ui(&mut self.state, ui.ctx());
        }
        #[cfg(all(target_os = "windows", feature = "windows"))]
        super::render_preparation::warm_fonts(ui.ctx());
    }

    #[cfg(target_os = "windows")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // The backend can wait in driver cleanup after this callback returns.
        self.hide_native_window();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        reduce_tray_commands, theme_preference_for_index, viewport_gained_focus, App,
        TrayCommandAction, MONITOR_EVENT_GUI_DRAIN_LIMIT,
    };

    #[cfg(all(target_os = "windows", feature = "windows", feature = "debug-mcp"))]
    #[test]
    fn native_frame_deferral_bypasses_shutdown_hidden_and_restore() {
        use eframe::App as _;
        let fixture = AppStateStorage::test_fixture(|state| {
            state.cpu_load_monitoring_enabled = true;
            state.groups.push(CoreGroup {
                name: "test".into(),
                cores: vec![0],
                programs: vec![],
                is_hidden: false,
                run_all_button: false,
            });
        });
        let mut shell =
            App::new_for_test(AppState::new_for_test(Arc::new(RwLock::new(fixture)), 4));
        let group_id = shell.state.rules.group_id_for_index(0).unwrap().0;
        crate::app::features::cpu_load::debug_api::apply(
            &mut shell.state,
            &crate::app::features::diagnostics::debug_mcp::Command::OpenGraph {
                group_id,
                rule_id: None,
            },
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 30.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::new("cpu-scroll-frame-budget"),
                std::time::Instant::now() + std::time::Duration::from_secs(60),
            )
        });
        for mode in 0..3 {
            shell.closing_requested = mode == 0;
            shell.is_hidden = mode == 1;
            shell.restore_pending = mode == 2;
            assert_eq!(shell.repaint_delay(&ctx, &input), std::time::Duration::ZERO);
        }
        shell.closing_requested = false;
        shell.is_hidden = false;
        shell.restore_pending = false;
        assert!(shell.repaint_delay(&ctx, &input) > std::time::Duration::ZERO);
    }
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use super::{AppForwardingRuntime, ForwardingServerLifetime};
    use crate::app::features::execution;
    use crate::app::instance_forwarding::{
        parse_ipc_response_frame, serialize_ipc_command_frame, ForwardedIpcCommand, IpcCommand,
        IpcResponseCode,
    };

    #[test]
    fn crash_report_refresh_only_triggers_on_a_real_focus_gain() {
        assert!(viewport_gained_focus(Some(false), Some(true)));
        assert!(!viewport_gained_focus(None, Some(true)));
        assert!(!viewport_gained_focus(Some(true), Some(true)));
        assert!(!viewport_gained_focus(Some(false), None));
    }
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema};
    use crate::app::runtime::AppState;
    use crate::app::shell::events::ShellEvent;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use crate::app::shell::sessions::ShortcutCreationRole;
    use crate::app::startup::StartupIntent;
    use crate::tray::TrayCmd;
    use eframe::egui;
    use os_api::PriorityClass;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use os_api::{LocalIpcClientError, LocalIpcEndpoint, OS};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::{Arc, RwLock};
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use std::time::SystemTime;

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_forwarding_server_lifetime_drops_server_before_guard() {
        use std::sync::Mutex;

        struct DropProbe {
            name: &'static str,
            events: Arc<Mutex<Vec<&'static str>>>,
        }

        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.events.lock().unwrap().push(self.name);
            }
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        let lifetime = ForwardingServerLifetime {
            guard: Some(DropProbe {
                name: "guard",
                events: events.clone(),
            }),
            server: Some(DropProbe {
                name: "server",
                events: events.clone(),
            }),
        };

        drop(lifetime);

        assert_eq!(*events.lock().unwrap(), vec!["server", "guard"]);
    }

    #[test]
    fn test_theme_index_maps_to_native_egui_preference() {
        assert_eq!(theme_preference_for_index(0), egui::ThemePreference::System);
        assert_eq!(theme_preference_for_index(1), egui::ThemePreference::Light);
        assert_eq!(theme_preference_for_index(2), egui::ThemePreference::Dark);
        assert_eq!(theme_preference_for_index(99), egui::ThemePreference::Dark);
    }

    #[test]
    fn tray_command_reducer_coalesces_show_requests() {
        assert_eq!(
            reduce_tray_commands(false, [TrayCmd::Show, TrayCmd::Show]),
            TrayCommandAction::Show
        );
    }

    #[test]
    fn tray_command_reducer_makes_quit_terminal_regardless_of_order() {
        assert_eq!(
            reduce_tray_commands(false, [TrayCmd::Show, TrayCmd::Quit, TrayCmd::Show]),
            TrayCommandAction::Quit
        );
        assert_eq!(
            reduce_tray_commands(false, [TrayCmd::Quit, TrayCmd::Show]),
            TrayCommandAction::Quit
        );
    }

    #[test]
    fn tray_command_reducer_ignores_later_show_after_quit_is_latched() {
        assert_eq!(
            reduce_tray_commands(true, [TrayCmd::Show]),
            TrayCommandAction::Quit
        );
        assert_eq!(reduce_tray_commands(false, []), TrayCommandAction::None);
    }

    #[test]
    fn hide_to_tray_requires_a_live_tray_runtime() {
        assert!(super::should_hide_to_tray(true, true, Some(true), false));
        assert!(!super::should_hide_to_tray(true, false, Some(true), false));
        assert!(!super::should_hide_to_tray(false, true, Some(true), false));
        assert!(!super::should_hide_to_tray(true, true, Some(false), false));
        assert!(!super::should_hide_to_tray(true, true, None, false));
    }

    #[test]
    fn tray_restore_does_not_rehide_on_stale_minimized_input() {
        assert!(!super::should_hide_to_tray(true, true, Some(true), true));
        assert!(!super::should_hide_to_tray(true, true, None, true));
        let mut app = App::new_for_test(sample_state());
        let ctx = egui::Context::default();
        app.show_from_tray(&ctx);
        assert!(app.restore_pending);
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .minimized = Some(true);
        let mut output = ctx.run_ui(input.clone(), |ui| assert!(app.should_render(ui.ctx())));
        output.textures_delta.clear();
        assert!(
            app.restore_pending,
            "old minimized input cannot acknowledge Restore"
        );
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .minimized = Some(false);
        let mut output = ctx.run_ui(input, |ui| assert!(app.should_render(ui.ctx())));
        output.textures_delta.clear();
        assert!(
            !app.restore_pending,
            "native restoration must re-arm minimize-to-tray"
        );
    }

    #[test]
    fn taskbar_visibility_updates_winit_policy_and_native_style_together() {
        use std::cell::RefCell;
        let calls = RefCell::new(Vec::new());
        // Initial hide, tray Restore, visible fallback and another hide/restore.
        for visible in [false, true, true, false, true] {
            calls.borrow_mut().clear();
            super::set_taskbar_visibility_with(
                visible,
                |skip| calls.borrow_mut().push(("winit_skip", skip)),
                |show| calls.borrow_mut().push(("native_show", show)),
            );
            assert_eq!(
                *calls.borrow(),
                [("winit_skip", !visible), ("native_show", visible)],
                "native style alone leaves Winit's Explorer-restart policy stale"
            );
        }
    }

    #[test]
    fn tray_hide_uses_visibility_without_moving_or_focusing_the_window() {
        let mut app = App::new_for_test(sample_state());
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| app.hide_to_tray(ui.ctx()));
        output.textures_delta.clear();
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.contains(&egui::ViewportCommand::Visible(false)));
        assert!(!commands.iter().any(|c| matches!(
            c,
            egui::ViewportCommand::OuterPosition(_)
                | egui::ViewportCommand::Focus
                | egui::ViewportCommand::Visible(true)
        )));
        assert!(app.is_hidden);
    }

    #[test]
    fn tray_restore_shows_and_unminimizes_without_repositioning() {
        let mut app = App::new_for_test(sample_state());
        app.is_hidden = true;
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            app.handle_tray_commands(ui.ctx(), [TrayCmd::Show]);
        });
        output.textures_delta.clear();
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.contains(&egui::ViewportCommand::Visible(true)));
        assert!(commands.contains(&egui::ViewportCommand::Minimized(false)));
        assert!(!commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::OuterPosition(_))));
        assert!(!app.is_hidden);
    }

    #[test]
    fn startup_visibility_is_reachable_without_a_tray_and_has_no_position_commands() {
        for tray_available in [false, true] {
            for enabled in [false, true] {
                let mut app = App::new_for_test(sample_state());
                app.is_hidden = super::should_start_minimized(enabled, true, tray_available);
                app.startup_visibility_pending = true;
                let ctx = egui::Context::default();
                let mut output = ctx.run_ui(Default::default(), |ui| {
                    assert_eq!(app.should_render(ui.ctx()), !(enabled && tray_available));
                });
                output.textures_delta.clear();
                let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
                assert!(commands.contains(&egui::ViewportCommand::Visible(
                    !(enabled && tray_available)
                )));
                assert!(!commands.iter().any(|c| matches!(
                    c,
                    egui::ViewportCommand::OuterPosition(_) | egui::ViewportCommand::Focus
                )));
                assert!(!app.startup_visibility_pending);
            }
        }
    }

    #[test]
    fn failed_cold_shortcut_forwarding_keeps_startup_error_reachable() {
        let mut app = App::new_for_test(sample_state());
        app.is_hidden = true;
        app.startup_visibility_pending = true;
        app.handle_startup_intent_after_forwarding(
            StartupIntent::RunRule {
                group_id: crate::app::shared::ids::GroupId("missing".into()),
                rule_id: crate::app::shared::ids::RuleId("missing".into()),
            },
            true,
        );
        assert!(!app.is_hidden);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            Default::default(),
            |ui| assert!(app.should_render(ui.ctx())),
        );
        output.textures_delta.clear();
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Visible(true)));
    }

    #[test]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn shell_drop_uses_release_coordinates_at_current_dpi_without_reading_files() {
        let paths = vec![
            PathBuf::from(r"C:\Missing\Пример.exe"),
            PathBuf::from(r"C:\Missing\two.lnk"),
        ];
        let mut input = egui::RawInput::default();
        input
            .events
            .push(egui::Event::PointerMoved(egui::pos2(300.0, 400.0)));
        super::append_shell_drop(
            &mut input,
            os_api::WindowFileDrop {
                paths: paths.clone(),
                position: [150, 225],
            },
            1.5,
        );
        let ctx = egui::Context::default();
        let mut app = App::new_for_test(sample_state());
        ctx.run_ui(input, |ui| app.handle_file_drops(ui.ctx()))
            .textures_delta
            .clear();
        assert_eq!(app.state.ui.dropped_files, Some(paths));
        assert_eq!(
            app.state.ui.file_drop_position,
            Some(egui::pos2(100.0, 150.0))
        );
        let later = egui::RawInput {
            events: vec![egui::Event::PointerMoved(egui::pos2(10.0, 20.0))],
            ..Default::default()
        };
        ctx.run_ui(later, |ui| app.handle_file_drops(ui.ctx()))
            .textures_delta
            .clear();
        assert_eq!(
            app.state.ui.file_drop_position,
            Some(egui::pos2(100.0, 150.0))
        );
    }

    #[test]
    fn file_drops_keep_all_paths_without_reading_files_or_clearing_pending_input() {
        #[derive(Debug)]
        struct PathOnlyDrop(PathBuf);

        impl egui::DroppedFile for PathOnlyDrop {
            fn path(&self) -> &std::path::Path {
                &self.0
            }

            fn bytes(&self) -> Result<Vec<u8>, String> {
                panic!("Import must use the path, not read the dropped file contents");
            }
        }

        let paths = vec![
            PathBuf::from(r"C:\Games\Game one.exe"),
            PathBuf::from(r"C:\Games\Game two.exe"),
        ];
        let input = egui::RawInput {
            dropped_files: paths
                .iter()
                .map(|path| Arc::new(PathOnlyDrop(path.clone())) as egui::DroppedFileHandle)
                .collect(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut app = App::new_for_test(sample_state());
        ctx.run_ui(input, |ui| app.handle_file_drops(ui.ctx()))
            .textures_delta
            .clear();
        assert_eq!(app.state.ui.dropped_files.as_ref(), Some(&paths));

        ctx.run_ui(egui::RawInput::default(), |ui| {
            app.handle_file_drops(ui.ctx())
        })
        .textures_delta
        .clear();
        assert_eq!(app.state.ui.dropped_files.as_ref(), Some(&paths));
    }

    #[test]
    fn tray_quit_latch_ignores_show_in_a_later_logic_tick() {
        let mut app = App::new_for_test(sample_state());
        let ctx = egui::Context::default();
        app.is_hidden = true;

        assert!(app.handle_tray_commands(&ctx, [TrayCmd::Quit]));
        assert!(app.closing_requested);

        assert!(app.handle_tray_commands(&ctx, [TrayCmd::Show]));
        assert!(app.is_hidden);
    }

    #[test]
    fn p5_tray_quit_enters_draining_before_viewport_close() {
        let mut app = App::new_for_test(sample_state());
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(app.handle_tray_commands(ui.ctx(), [TrayCmd::Quit]));
            assert!(app.closing_requested);
            #[cfg(target_os = "windows")]
            assert!(
                app.is_hidden,
                "accepted shutdown must stop showing the root"
            );
        });
        output.textures_delta.clear();
        #[cfg(target_os = "windows")]
        assert!(
            output.viewport_output.values().any(|viewport| {
                viewport
                    .commands
                    .contains(&egui::ViewportCommand::Visible(false))
            }),
            "the root must hide while background work drains"
        );
        assert!(
            output.viewport_output.values().all(|viewport| {
                !viewport
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::ViewportCommand::Close))
            }),
            "Quit must first close admission and begin worker draining"
        );
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    #[ignore = "isolated native minimize/restore/teardown regression; no UAC or user settings"]
    fn restored_root_is_hidden_before_graphics_teardown() {
        use std::sync::Mutex;
        use std::time::{Duration, Instant};
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        use winit::platform::windows::EventLoopBuilderExtWindows;

        struct Probe {
            shell: App,
            start: Instant,
            phase: u8,
            result: Arc<Mutex<Option<(u8, bool)>>>,
        }
        impl eframe::App for Probe {
            fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                let elapsed = self.start.elapsed();
                match self.phase {
                    0 if elapsed >= Duration::from_millis(200) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        self.phase = 1;
                    }
                    1 if ctx.input(|i| i.viewport().minimized) == Some(true) => {
                        self.shell.hide_to_tray(ctx);
                        self.phase = 2;
                    }
                    2 if elapsed >= Duration::from_millis(500) => {
                        self.shell.show_from_tray(ctx);
                        self.phase = 3;
                    }
                    3 if elapsed >= Duration::from_millis(800)
                        && ctx.input(|i| i.viewport().minimized) == Some(false) =>
                    {
                        self.phase = 4;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    _ => {}
                }
                if elapsed >= Duration::from_secs(5) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ctx.request_repaint_after(Duration::from_millis(20));
            }

            fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
                ui.label("Isolated minimize/restore/close regression");
            }

            fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
                self.shell.on_exit(gl);
                let visible = unsafe { IsWindowVisible(self.shell.hwnd.unwrap()) }.as_bool();
                *self.result.lock().unwrap() = Some((self.phase, visible));
            }
        }

        let result = Arc::new(Mutex::new(None));
        let saved = result.clone();
        eframe::run_native(
            "Close lifecycle regression",
            eframe::NativeOptions {
                run_and_return: true,
                viewport: egui::ViewportBuilder::default().with_inner_size([280.0, 140.0]),
                event_loop_builder: Some(Box::new(|builder| {
                    builder.with_any_thread(true);
                })),
                ..Default::default()
            },
            Box::new(move |cc| {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let RawWindowHandle::Win32(raw) = cc.window_handle().unwrap().as_raw() else {
                    panic!("Win32 required");
                };
                let mut shell = App::new_for_test(sample_state());
                shell.hwnd = Some(windows::Win32::Foundation::HWND(raw.hwnd.get() as *mut _));
                shell.winit_window = cc.winit_window().cloned();
                Ok(Box::new(Probe {
                    shell,
                    start: Instant::now(),
                    phase: 0,
                    result: saved,
                }))
            }),
        )
        .unwrap();
        let (phase, visible) = result.lock().unwrap().unwrap();
        assert_eq!(phase, 4, "native minimize/restore did not complete");
        assert!(
            !visible,
            "the root must be hidden before backend teardown can block"
        );
    }

    #[test]
    fn p5_forwarded_rule_is_rejected_after_quit_latches() {
        let mut app = App::new_for_test(sample_state());
        app.closing_requested = true;
        let response = app.handle_ipc_command(IpcCommand::RunRule {
            group_id: crate::app::shared::ids::GroupId("missing-group".into()),
            rule_id: crate::app::shared::ids::RuleId("missing-rule".into()),
        });
        assert_eq!(response.code, IpcResponseCode::LaunchRejected);
    }

    fn sample_state() -> AppState {
        AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
                state.version = 5;
                state.groups = vec![];
                state.cpu_schema = CpuSchema {
                    model: "Test CPU".to_string(),
                    clusters: Vec::new(),
                };
                state.theme_index = 0;
                state.process_monitoring_enabled = false;
                state.windows_event_log_diagnostics_enabled = true;
                state.start_minimized = false;
                state.rule_identities = None;
                state.loaded_version = 5;
                state.pending_pre_v6_backup = false;
                state.tuning = Default::default();
            }))),
            4,
        )
    }

    #[test]
    fn cpu19_modal_skips_background_content_and_restores_it_on_close() {
        fn contains(shape: &egui::Shape, text: &str) -> bool {
            match shape {
                egui::Shape::Text(t) => t.galley.text().contains(text),
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| contains(s, text)),
                _ => false,
            }
        }
        let mut app = App::new_for_test(sample_state());
        app.state.ui.current_window =
            crate::app::shell::WindowRoute::Groups(crate::app::shell::GroupRoute::Create);
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        for visible in [true, false, true] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.render_main_ui(ui, visible)
            });
            let rendered = output
                .shapes
                .iter()
                .any(|s| contains(&s.shape, "CPU topology"));
            output.textures_delta.clear();
            assert_eq!(rendered, visible);
        }
    }

    fn app_to_run(name: &str, autorun: bool) -> AppToRun {
        let mut app = AppToRun::new_path(
            PathBuf::from(format!(r"C:\{name}.lnk")),
            Vec::new(),
            PathBuf::from(format!(r"C:\{name}.exe")),
            PriorityClass::Normal,
            autorun,
        );
        app.name = name.to_string();
        app
    }

    fn app_with_forwarded_commands(state: AppState) -> (App, mpsc::Sender<ForwardedIpcCommand>) {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new_for_test(state);
        app.forwarded_command_rx = Some(rx);
        (app, tx)
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn unique_ipc_endpoint(label: &str) -> LocalIpcEndpoint {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!(
            "cpu-affinity-tool-app-test-{label}-{}-{nanos}",
            std::process::id()
        );
        LocalIpcEndpoint {
            pipe_name: format!(r"\\.\pipe\{name}"),
            mutex_name: format!(r"Local\{name}-primary"),
        }
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_install_forwarding_runtime_marks_secondary_shortcut_role() {
        let mut app = App::new_for_test(sample_state());

        app.install_forwarding_runtime(
            None,
            Some("another instance owns the local shortcut endpoint".to_string()),
            &egui::Context::default(),
        );

        assert_eq!(
            app.state.shortcut_creation_role(),
            ShortcutCreationRole::NonPrimary
        );
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_pending_forwarding_runtime_starts_server_only_when_installed() {
        let endpoint = unique_ipc_endpoint("pending");
        let guard = OS::try_claim_local_ipc_primary_guard(&endpoint)
            .expect("guard claim should not fail")
            .expect("guard should be available");
        let runtime = AppForwardingRuntime::pending(guard, endpoint.clone());
        assert!(!runtime.server_started());

        let before_install =
            OS::send_local_ipc_request(&endpoint, b"ping", std::time::Duration::from_millis(50));
        assert!(matches!(before_install, Err(LocalIpcClientError::NoServer)));

        let mut app = App::new_for_test(sample_state());
        app.install_forwarding_runtime(Some(runtime), None, &egui::Context::default());

        assert_eq!(
            app.state.shortcut_creation_role(),
            ShortcutCreationRole::Primary
        );
        assert!(app
            .forwarding_runtime
            .as_ref()
            .is_some_and(AppForwardingRuntime::server_started));
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_run_rule_startup_is_blocked_when_forwarding_server_install_fails() {
        let endpoint = unique_ipc_endpoint("install-fails");
        let guard = OS::try_claim_local_ipc_primary_guard(&endpoint)
            .expect("guard claim should not fail")
            .expect("guard should be available");
        let _pipe_squatter =
            OS::start_local_ipc_server(&endpoint).expect("test squatter pipe should start");
        let runtime = AppForwardingRuntime::pending(guard, endpoint);
        let state = sample_state_with_programs(vec![app_to_run("ShortcutApp", false)]);
        let group_id = state.rules.group_id_for_index(0).unwrap();
        let rule_id = state.rules.rule_id_for_index(0, 0).unwrap();
        let key = state.persistent_state.read().unwrap().groups[0].programs[0].get_key();
        assert!(state
            .runtime
            .add_running_app(&key, 12345, group_id.clone(), rule_id.clone()));
        let mut app = App::new_for_test(state);

        let forwarding_ready =
            app.install_forwarding_runtime(Some(runtime), None, &egui::Context::default());
        app.handle_startup_intent_after_forwarding(
            StartupIntent::RunRule { group_id, rule_id },
            !forwarding_ready,
        );

        assert!(!forwarding_ready);
        assert_eq!(
            app.state.shortcut_creation_role(),
            ShortcutCreationRole::Unsupported
        );
        let messages = app
            .state
            .log_manager
            .entries
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages.iter().any(|message| {
            *message == "ERROR: Shortcut launch was blocked because shortcut forwarding did not start"
        }));
        assert!(!messages
            .iter()
            .any(|message| message.contains("ShortcutApp") && message.contains("already running")));
    }

    fn sample_state_with_programs(programs: Vec<AppToRun>) -> AppState {
        AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
                state.version = 5;
                state.groups = vec![CoreGroup {
                    name: "Games".to_string(),
                    cores: vec![0, 1],
                    programs,
                    is_hidden: false,
                    run_all_button: true,
                }];
                state.cpu_schema = CpuSchema {
                    model: "Test CPU".to_string(),
                    clusters: Vec::new(),
                };
                state.theme_index = 0;
                state.process_monitoring_enabled = false;
                state.windows_event_log_diagnostics_enabled = true;
                state.start_minimized = false;
                state.rule_identities = None;
                state.loaded_version = 5;
                state.pending_pre_v6_backup = false;
                state.tuning = Default::default();
            }))),
            4,
        )
    }

    #[test]
    fn test_bootstrap_runtime_logs_startup_and_drains_monitor_notifications() {
        let ctx = egui::Context::default();
        let (tx, rx) = execution::monitor_event_channel();
        let mut state = sample_state();

        App::bootstrap_runtime(
            &mut state,
            &ctx,
            StartupIntent::NormalGui,
            move |_, _, _, _| rx,
        );

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
        tx.try_send(ShellEvent::Warning("WARNING: monitor warning".to_string()));
        tx.try_send(ShellEvent::Monitor("MONITOR: corrected".to_string()));

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

    #[test]
    fn monitor_event_handling_drains_a_bounded_batch_then_continues() {
        let ctx = egui::Context::default();
        let (tx, rx) = execution::monitor_event_channel();
        let mut app = App::new_for_test(sample_state());
        app.state.runtime.monitor_rx = Some(rx);

        for index in 0..(MONITOR_EVENT_GUI_DRAIN_LIMIT + 1) {
            tx.try_send(ShellEvent::Monitor(format!("MONITOR: event-{index}")));
        }

        app.handle_monitor_events(&ctx);
        assert_eq!(
            app.state.log_manager.entries.len(),
            MONITOR_EVENT_GUI_DRAIN_LIMIT
        );

        app.handle_monitor_events(&ctx);
        assert_eq!(
            app.state.log_manager.entries.len(),
            MONITOR_EVENT_GUI_DRAIN_LIMIT + 1
        );
    }

    #[test]
    fn test_bootstrap_runtime_normal_gui_rejects_unverified_existing_autorun() {
        let ctx = egui::Context::default();
        let (_tx, rx) = execution::monitor_event_channel();
        let mut state = sample_state_with_programs(vec![
            app_to_run("AutorunApp", true),
            app_to_run("ManualApp", false),
        ]);
        let group_id = state.rules.group_id_for_index(0).unwrap();
        let autorun_rule_id = state.rules.rule_id_for_index(0, 0).unwrap();
        let autorun_key = state.persistent_state.read().unwrap().groups[0].programs[0].get_key();
        assert!(state
            .runtime
            .add_running_app(&autorun_key, 12345, group_id, autorun_rule_id));

        App::bootstrap_runtime(
            &mut state,
            &ctx,
            StartupIntent::NormalGui,
            move |_, _, _, _| rx,
        );

        let messages = state
            .log_manager
            .entries
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages.iter().any(|message| message.contains("AutorunApp")
            && message.contains("tracked process identity is not available")));
        assert!(!messages
            .iter()
            .any(|message| message.contains("ManualApp") || message.contains("ManualApp.exe")));
    }

    #[test]
    fn test_bootstrap_runtime_run_rule_rejects_unverified_existing_target() {
        let ctx = egui::Context::default();
        let (_tx, rx) = execution::monitor_event_channel();
        let mut state = sample_state_with_programs(vec![
            app_to_run("AutorunApp", true),
            app_to_run("ShortcutApp", false),
        ]);
        let group_id = state.rules.group_id_for_index(0).unwrap();
        let requested_rule_id = state.rules.rule_id_for_index(0, 1).unwrap();
        let requested_key = state.persistent_state.read().unwrap().groups[0].programs[1].get_key();
        assert!(state.runtime.add_running_app(
            &requested_key,
            12345,
            group_id.clone(),
            requested_rule_id.clone()
        ));

        App::bootstrap_runtime(
            &mut state,
            &ctx,
            StartupIntent::RunRule {
                group_id,
                rule_id: requested_rule_id,
            },
            move |_, _, _, _| rx,
        );

        let messages = state
            .log_manager
            .entries
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages
            .iter()
            .any(|message| message.contains("ShortcutApp")
                && message.contains("tracked process identity is not available")));
        assert!(!messages
            .iter()
            .any(|message| message.contains("AutorunApp") || message.contains("AutorunApp.exe")));
    }

    #[test]
    fn test_forwarded_run_rule_command_dispatches_and_replies() {
        let ctx = egui::Context::default();
        let state = sample_state_with_programs(vec![
            app_to_run("AutorunApp", true),
            app_to_run("ShortcutApp", false),
        ]);
        let group_id = state.rules.group_id_for_index(0).unwrap();
        let requested_rule_id = state.rules.rule_id_for_index(0, 1).unwrap();
        let requested_key = state.persistent_state.read().unwrap().groups[0].programs[1].get_key();
        assert!(state.runtime.add_running_app(
            &requested_key,
            12345,
            group_id.clone(),
            requested_rule_id.clone()
        ));
        let (mut app, command_tx) = app_with_forwarded_commands(state);
        let (response_tx, response_rx) = mpsc::channel();

        command_tx
            .send(ForwardedIpcCommand {
                command: IpcCommand::RunRule {
                    group_id,
                    rule_id: requested_rule_id,
                },
                response_tx,
            })
            .unwrap();

        app.handle_forwarded_commands(&ctx);

        let response = response_rx.try_recv().unwrap();
        assert_eq!(response.code, IpcResponseCode::LaunchRejected);
        let messages = app
            .state
            .log_manager
            .entries
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages
            .iter()
            .any(|message| message.contains("ShortcutApp")
                && message.contains("tracked process identity is not available")));
        assert!(!messages
            .iter()
            .any(|message| message.contains("AutorunApp") || message.contains("AutorunApp.exe")));
    }

    #[test]
    fn test_forwarded_command_drains_while_window_is_hidden() {
        let ctx = egui::Context::default();
        let state = sample_state_with_programs(vec![app_to_run("ManualApp", false)]);
        let (mut app, command_tx) = app_with_forwarded_commands(state);
        let (response_tx, response_rx) = mpsc::channel();
        app.is_hidden = true;

        command_tx
            .send(ForwardedIpcCommand {
                command: IpcCommand::RunRule {
                    group_id: crate::app::shared::ids::GroupId("missing-group".to_string()),
                    rule_id: crate::app::shared::ids::RuleId("missing-rule".to_string()),
                },
                response_tx,
            })
            .unwrap();

        app.handle_forwarded_commands(&ctx);

        let response = response_rx.try_recv().unwrap();
        assert_eq!(response.code, IpcResponseCode::MissingGroup);
    }

    #[test]
    fn test_local_ipc_frame_dispatches_and_serializes_response() {
        let state = sample_state_with_programs(vec![app_to_run("ManualApp", false)]);
        let group_id = state.rules.group_id_for_index(0).unwrap();
        let rule_id = state.rules.rule_id_for_index(0, 0).unwrap();
        let key = state.persistent_state.read().unwrap().groups[0].programs[0].get_key();
        assert!(state
            .runtime
            .add_running_app(&key, 12345, group_id.clone(), rule_id.clone()));
        let mut app = App::new_for_test(state);
        let request =
            serialize_ipc_command_frame(&IpcCommand::RunRule { group_id, rule_id }).unwrap();

        let response_frame = app.handle_local_ipc_request_frame(&request);

        let response = parse_ipc_response_frame(&response_frame).unwrap();
        assert_eq!(response.code, IpcResponseCode::LaunchRejected);
    }

    #[test]
    fn test_local_ipc_frame_returns_protocol_error_for_invalid_frame() {
        let mut app = App::new_for_test(sample_state());

        let response_frame = app.handle_local_ipc_request_frame(b"not-json");

        let response = parse_ipc_response_frame(&response_frame).unwrap();
        assert_eq!(response.code, IpcResponseCode::ProtocolError);
    }
}

impl App {
    fn begin_close(&mut self, ctx: &egui::Context) {
        if !self.closing_requested {
            self.closing_requested = true;
            #[cfg(target_os = "windows")]
            {
                self.is_hidden = true;
                self.hide_native_window();
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            self.state.begin_tuning_shutdown();
        }
        ctx.request_repaint();
    }

    #[cfg(target_os = "windows")]
    fn hide_native_window(&self) {
        if let Some(window) = &self.winit_window {
            // Apply on the owning GUI thread before teardown stops pumping messages;
            // a queued viewport command alone is too late in App::on_exit.
            window.set_visible(false);
        }
    }

    fn handle_tray_events(&mut self, ctx: &egui::Context) -> bool {
        let commands = self
            .tray_runtime
            .as_ref()
            .map(TrayRuntime::drain_commands)
            .unwrap_or_default();

        self.handle_tray_commands(ctx, commands)
    }

    fn handle_tray_commands(
        &mut self,
        ctx: &egui::Context,
        commands: impl IntoIterator<Item = TrayCmd>,
    ) -> bool {
        match reduce_tray_commands(self.closing_requested, commands) {
            TrayCommandAction::None => false,
            TrayCommandAction::Show => {
                self.show_from_tray(ctx);
                false
            }
            TrayCommandAction::Quit => {
                self.begin_close(ctx);
                true
            }
        }
    }

    fn handle_monitor_events(&mut self, ctx: &egui::Context) {
        let mut repaint_requested = false;
        let mut drained = 0usize;
        let mut drain_status = execution::MonitorDrainStatus::default();

        if let Some(rx) = &self.state.runtime.monitor_rx {
            while drained < MONITOR_EVENT_GUI_DRAIN_LIMIT {
                let Ok(event) = rx.try_recv() else {
                    break;
                };
                drained += 1;
                if let Some((message, sticky)) = event.legacy_log_message() {
                    self.state
                        .log_manager
                        .add_monitor_entry(message.to_string(), sticky);
                    self.state.ui.tracked_processes.last_activity = Some(std::time::Instant::now());
                }
                if let Some(message) = event.tuning_log_message() {
                    self.state.log_manager.add_important_entry(message);
                }

                repaint_requested |= event.needs_repaint();
            }

            drain_status = rx.finish_drain();
        }

        if drain_status.dropped_monitor_messages > 0 || drain_status.dropped_warnings > 0 {
            self.state.log_manager.add_monitor_important_entry(format!(
                "WARNING: Monitoring notification queue was saturated; skipped {} routine message(s) and {} warning(s). The current runtime state was refreshed.",
                drain_status.dropped_monitor_messages, drain_status.dropped_warnings
            ));
        }

        if repaint_requested || drain_status.needs_repaint {
            ctx.request_repaint();
        }
        if drain_status.has_more_work || drained == MONITOR_EVENT_GUI_DRAIN_LIMIT {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    #[cfg(test)]
    fn handle_forwarded_commands(&mut self, ctx: &egui::Context) {
        let mut commands = Vec::new();
        if let Some(rx) = &self.forwarded_command_rx {
            while let Ok(command) = rx.try_recv() {
                commands.push(command);
            }
        }

        if commands.is_empty() {
            return;
        }

        for command in commands {
            let response = self.handle_ipc_command(command.command);
            let _ = command.response_tx.send(response);
        }

        ctx.request_repaint();
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn handle_local_ipc_requests(&mut self, ctx: &egui::Context) {
        let mut requests = Vec::new();
        if let Some(runtime) = &self.forwarding_runtime {
            if let Some(server) = runtime.server() {
                while let Ok(request) = server.try_recv() {
                    requests.push(request);
                }
            }
        }

        if requests.is_empty() {
            return;
        }

        for request in requests {
            let response = self.handle_local_ipc_request_frame(&request.request);
            let _ = request.response_tx.send(response);
        }

        ctx.request_repaint();
    }

    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    fn handle_local_ipc_request_frame(&mut self, request: &[u8]) -> Vec<u8> {
        let response = match parse_ipc_command_frame(request) {
            Ok(command) => self.handle_ipc_command(command),
            Err(_) => IpcResponse {
                code: IpcResponseCode::ProtocolError,
                detail: None,
            },
        };

        serialize_ipc_response_frame(&response)
            .unwrap_or_else(|_| br#"{"version":1,"code":"protocol_error"}"#.to_vec())
    }

    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    fn handle_ipc_command(&mut self, command: IpcCommand) -> IpcResponse {
        if self.closing_requested {
            return IpcResponse {
                code: IpcResponseCode::LaunchRejected,
                detail: None,
            };
        }
        match command {
            IpcCommand::RunRule { group_id, rule_id } => {
                run_rule_outcome_to_response(self.state.run_group_program(group_id, rule_id))
            }
        }
    }

    fn should_render(&mut self, ctx: &egui::Context) -> bool {
        if self.startup_visibility_pending {
            self.startup_visibility_pending = false;
            if self.is_hidden {
                self.hide_to_tray(ctx);
            } else {
                #[cfg(target_os = "windows")]
                self.set_taskbar_visible(true);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
        }
        if self.is_hidden {
            ctx.request_repaint_after(Duration::from_millis(250));
            return false;
        }

        let minimized = ctx.input(|i| i.viewport().minimized);
        if minimized == Some(false) {
            self.restore_pending = false;
        }
        if should_hide_to_tray(
            crate::app::adapters::os::supports_hide_to_tray(),
            self.tray_runtime.is_some(),
            minimized,
            self.restore_pending,
        ) {
            self.hide_to_tray(ctx);
            return false;
        }

        true
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.is_hidden = true;
        self.restore_pending = false;
        #[cfg(all(target_os = "windows", feature = "windows"))]
        self.state.poll_cpu_load(false);

        #[cfg(target_os = "windows")]
        if let Err(error) = crate::app::adapters::os::normalize_winit_event_target_window() {
            self.state.log_manager.add_sticky_once(error);
        }

        #[cfg(target_os = "windows")]
        self.set_taskbar_visible(false);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn show_from_tray(&mut self, ctx: &egui::Context) {
        self.startup_visibility_pending = false;
        self.is_hidden = false;
        // Winit can deliver the previous minimized=true once more before applying
        // Minimized(false). Do not interpret that stale observation as a new minimize.
        self.restore_pending = true;
        #[cfg(all(target_os = "windows", feature = "windows"))]
        self.state.request_crash_report_refresh();

        #[cfg(target_os = "windows")]
        self.set_taskbar_visible(true);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    #[cfg(target_os = "windows")]
    fn set_taskbar_visible(&self, visible: bool) {
        use winit::platform::windows::WindowExtWindows;
        set_taskbar_visibility_with(
            visible,
            |skip| {
                if let Some(window) = &self.winit_window {
                    // Winit reapplies this retained policy when Explorer recreates its taskbar.
                    window.set_skip_taskbar(skip);
                }
            },
            |show| {
                if let Some(hwnd) = self.hwnd {
                    crate::app::adapters::os::set_taskbar_visible(hwnd, show);
                }
            },
        );
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(theme_preference_for_index(self.state.get_theme_index()));
        ctx.all_styles_mut(|style| {
            crate::app::shell::presenters::shared_elements::apply_widget_style(style);
            let mut visuals = if style.visuals.dark_mode {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            crate::app::shell::presenters::shared_elements::apply_widget_visuals(&mut visuals);
            style.visuals = visuals;
        });
    }

    fn handle_file_drops(&mut self, ctx: &egui::Context) {
        #[cfg(all(target_os = "windows", feature = "windows"))]
        let graph_open = self
            .state
            .ui
            .monitoring_modal_open(self.state.cpu_load.graph.is_some())
            || self.state.mcp.modal_open();
        #[cfg(not(all(target_os = "windows", feature = "windows")))]
        let graph_open = self.state.ui.monitoring_modal_open(false);
        if !file_import_allowed(&self.state.ui.current_window, graph_open) {
            return;
        }
        if ctx.input(|i| i.raw.dropped_files.is_empty()) {
            return;
        }

        let files: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });

        if !files.is_empty() {
            self.state.ui.dropped_files = Some(files);
            self.state.ui.file_drop_position =
                ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos()));
        }
    }

    fn render_main_ui(&mut self, ui: &mut egui::Ui, draw_active: bool) {
        {
            crate::render_profile_scope!(Header);
            header::draw_top_panel(&mut self.state, ui);
        }
        {
            // Keep the diagnostic bucket stable; service dialogs reserve no panel.
            crate::render_profile_scope!(Footer);
            service_controls::draw_modals(&mut self.state, ui.ctx());
        }
        {
            crate::render_profile_scope!(Central);
            if draw_active {
                Self::draw_active_view(&mut self.state, ui);
            } else {
                // The opaque monitoring modal owns the content; keep only the shared chrome.
                egui::CentralPanel::default().show(ui, |_| {});
            }
        }
    }

    fn draw_active_view(app_state: &mut AppState, ui: &mut egui::Ui) {
        match app_state.ui.current_window.clone() {
            WindowRoute::Groups(group_route) => match group_route {
                GroupRoute::List => central::draw_central_panel(app_state, ui),
                GroupRoute::Create => group_editor::create_group_window(app_state, ui),
                GroupRoute::Edit => group_editor::edit_group_window(app_state, ui),
            },
            WindowRoute::Logs => logs::draw_logs_window(app_state, ui),
            WindowRoute::Tuning => system_tweaks::draw_system_tweaks(app_state, ui),
            #[cfg(all(target_os = "windows", feature = "windows"))]
            WindowRoute::CrashReports => crash_reports::draw_crash_reports_window(app_state, ui),
            WindowRoute::AppRunSettings => run_settings::draw_app_run_settings(app_state, ui),
            WindowRoute::InstalledAppPicker => {
                installed_app_picker::draw_installed_app_picker(app_state, ui)
            }
        }
    }
}
