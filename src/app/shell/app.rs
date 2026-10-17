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
    central, footer, group_editor, header, installed_app_picker, logs, run_settings, system_tweaks,
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
    #[cfg(all(target_os = "windows", feature = "windows"))]
    crash_report_viewport_focused: Option<bool>,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    windows_event_log_first_frame_rendered: bool,
    closing_requested: bool,
    is_hidden: bool,
    start_hidden_pending: bool,
}

const MONITOR_EVENT_GUI_DRAIN_LIMIT: usize = 64;

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
) -> bool {
    platform_supports_hide_to_tray && has_tray_runtime && minimized == Some(true)
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

        cc.egui_ctx
            .set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());

        let mut state = AppState::new();
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
                #[cfg(all(target_os = "windows", feature = "windows"))]
                crash_report_viewport_focused: None,
                #[cfg(all(target_os = "windows", feature = "windows"))]
                windows_event_log_first_frame_rendered: false,
                closing_requested: false,
                is_hidden: false,
                start_hidden_pending: false,
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
                    #[cfg(all(target_os = "windows", feature = "windows"))]
                    crash_report_viewport_focused: None,
                    #[cfg(all(target_os = "windows", feature = "windows"))]
                    windows_event_log_first_frame_rendered: false,
                    closing_requested: false,
                    is_hidden: false,
                    start_hidden_pending: false,
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
            self.state.log_manager.add_important_sticky_once(
                "ERROR: Shortcut launch was blocked because shortcut forwarding did not start"
                    .to_string(),
            );
            return;
        }

        self.start_hidden_pending = should_start_minimized(
            self.state.start_minimized(),
            crate::app::adapters::os::supports_hide_to_tray(),
            self.tray_runtime.is_some(),
        );
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
            #[cfg(all(target_os = "windows", feature = "windows"))]
            crash_report_viewport_focused: None,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            windows_event_log_first_frame_rendered: false,
            closing_requested: false,
            is_hidden: false,
            start_hidden_pending: false,
        }
    }
}

impl eframe::App for App {
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        if let Some(bridge) = &mut self.debug_mcp {
            bridge.raw_input(&mut self.state, ctx, input);
        }
        super::render_preparation::pace_scroll(ctx, input, self.state.cpu_load.graph.is_some());
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
        let draw_active = self.state.cpu_load.graph.is_none();
        #[cfg(not(all(target_os = "windows", feature = "windows")))]
        let draw_active = true;
        self.render_main_ui(ui, draw_active);
        #[cfg(all(target_os = "windows", feature = "windows"))]
        crate::app::features::cpu_load::view::draw_modal(&mut self.state, ui.ctx());
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
}

#[cfg(test)]
mod tests {
    use super::{
        reduce_tray_commands, theme_preference_for_index, viewport_gained_focus, App,
        TrayCommandAction, MONITOR_EVENT_GUI_DRAIN_LIMIT,
    };
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
        assert!(super::should_hide_to_tray(true, true, Some(true)));
        assert!(!super::should_hide_to_tray(true, false, Some(true)));
        assert!(!super::should_hide_to_tray(false, true, Some(true)));
        assert!(!super::should_hide_to_tray(true, true, Some(false)));
        assert!(!super::should_hide_to_tray(true, true, None));
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
        });
        output.textures_delta.clear();
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
            self.state.begin_tuning_shutdown();
        }
        ctx.request_repaint();
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
                    if sticky {
                        self.state.log_manager.add_sticky_once(message.to_string());
                    } else {
                        self.state.log_manager.add_entry(message.to_string());
                    }
                }
                if let Some(message) = event.tuning_log_message() {
                    self.state.log_manager.add_important_entry(message);
                }

                repaint_requested |= event.needs_repaint();
            }

            drain_status = rx.finish_drain();
        }

        if drain_status.dropped_monitor_messages > 0 || drain_status.dropped_warnings > 0 {
            self.state.log_manager.add_important_entry(format!(
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
        if self.start_hidden_pending {
            self.start_hidden_pending = false;
            self.hide_to_tray(ctx);
            return false;
        }
        if self.is_hidden {
            ctx.request_repaint_after(Duration::from_millis(250));
            return false;
        }

        if should_hide_to_tray(
            crate::app::adapters::os::supports_hide_to_tray(),
            self.tray_runtime.is_some(),
            ctx.input(|i| i.viewport().minimized),
        ) {
            self.hide_to_tray(ctx);
            return false;
        }

        true
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.is_hidden = true;
        #[cfg(all(target_os = "windows", feature = "windows"))]
        self.state.poll_cpu_load(false);

        #[cfg(target_os = "windows")]
        if let Err(error) = crate::app::adapters::os::normalize_winit_event_target_window() {
            self.state.log_manager.add_sticky_once(error);
        }

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
        self.start_hidden_pending = false;
        self.is_hidden = false;
        #[cfg(all(target_os = "windows", feature = "windows"))]
        self.state.request_crash_report_refresh();

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
        }
    }

    fn render_main_ui(&mut self, ui: &mut egui::Ui, draw_active: bool) {
        {
            crate::render_profile_scope!(Header);
            header::draw_top_panel(&mut self.state, ui);
        }
        {
            crate::render_profile_scope!(Footer);
            footer::draw_bottom_panel(&mut self.state, ui);
        }
        {
            crate::render_profile_scope!(Central);
            if draw_active {
                Self::draw_active_view(&mut self.state, ui);
            } else {
                // The opaque CPU modal owns the content; keep only the shared chrome.
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
