use crate::app::features::diagnostics;
use crate::app::features::execution;
use crate::app::features::execution::InstalledPackageTrackingState;
#[cfg(test)]
use crate::app::instance_forwarding::ForwardedIpcCommand;
#[cfg(any(test, all(target_os = "windows", feature = "windows")))]
use crate::app::instance_forwarding::{
    parse_ipc_command_frame, run_rule_outcome_to_response, serialize_ipc_response_frame,
    IpcCommand, IpcResponse, IpcResponseCode,
};
use crate::app::models::RunningApps;
use crate::app::runtime::{AppState, RunRuleOutcome};
use crate::app::shell::events::ShellEvent;
use crate::app::shell::presenters::{
    central, footer, group_editor, header, installed_app_picker, logs, run_settings,
};
#[cfg(all(target_os = "windows", feature = "windows"))]
use crate::app::shell::sessions::ShortcutCreationRole;
use crate::app::shell::{GroupRoute, WindowRoute};
use crate::app::startup::StartupIntent;
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
    #[cfg(test)]
    forwarded_command_rx: Option<Receiver<ForwardedIpcCommand>>,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    forwarding_runtime: Option<AppForwardingRuntime>,
    #[cfg(target_os = "windows")]
    _tray_icon_guard: Option<tray_icon::TrayIcon>,
    #[cfg(target_os = "windows")]
    hwnd: Option<windows::Win32::Foundation::HWND>,
    is_hidden: bool,
}

#[cfg(all(target_os = "windows", feature = "windows"))]
pub struct AppForwardingRuntime {
    _guard: os_api::LocalIpcGuard,
    endpoint: os_api::LocalIpcEndpoint,
    server: Option<os_api::LocalIpcServer>,
}

#[cfg(all(target_os = "windows", feature = "windows"))]
impl AppForwardingRuntime {
    pub fn pending(guard: os_api::LocalIpcGuard, endpoint: os_api::LocalIpcEndpoint) -> Self {
        Self {
            _guard: guard,
            endpoint,
            server: None,
        }
    }

    fn start_server(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.server.is_some() {
            return Ok(());
        }

        let repaint_ctx = ctx.clone();
        let wake: os_api::LocalIpcWake = Arc::new(move || {
            repaint_ctx.request_repaint();
        });
        let server = os_api::OS::start_local_ipc_server_with_wake(&self.endpoint, Some(wake))?;
        self.server = Some(server);
        Ok(())
    }

    #[cfg(test)]
    fn server_started(&self) -> bool {
        self.server.is_some()
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

        let mut state = AppState::new();
        Self::bootstrap_runtime_without_startup(&mut state, execution::spawn_monitors);

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
                    #[cfg(test)]
                    forwarded_command_rx: None,
                    #[cfg(all(target_os = "windows", feature = "windows"))]
                    forwarding_runtime: None,
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
                    #[cfg(test)]
                    forwarded_command_rx: None,
                    #[cfg(all(target_os = "windows", feature = "windows"))]
                    forwarding_runtime: None,
                    #[cfg(target_os = "windows")]
                    _tray_icon_guard: None,
                    #[cfg(target_os = "windows")]
                    hwnd,
                    is_hidden: false,
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
            Arc<RwLock<InstalledPackageTrackingState>>,
            Arc<RwLock<crate::app::models::AppStateStorage>>,
        ) -> Receiver<ShellEvent>,
    {
        Self::bootstrap_runtime_without_startup(state, spawn_monitors);
        Self::handle_startup_intent(state, startup_intent);
    }

    fn bootstrap_runtime_without_startup<F>(state: &mut AppState, spawn_monitors: F)
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
            tray_rx: None,
            #[cfg(test)]
            forwarded_command_rx: None,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            forwarding_runtime: None,
            #[cfg(target_os = "windows")]
            _tray_icon_guard: None,
            #[cfg(target_os = "windows")]
            hwnd: None,
            is_hidden: false,
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_tray_events(ctx);
        self.handle_monitor_events(ctx);
        #[cfg(all(target_os = "windows", feature = "windows"))]
        self.handle_local_ipc_requests(ctx);
        #[cfg(test)]
        self.handle_forwarded_commands(ctx);
        self.state.poll_installed_app_picker_refresh();

        if !self.should_render(ctx) {
            return;
        }

        self.apply_theme(ctx);
        self.handle_file_drops(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.is_hidden {
            return;
        }

        self.render_main_ui(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::App;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use super::AppForwardingRuntime;
    use crate::app::instance_forwarding::{
        parse_ipc_response_frame, serialize_ipc_command_frame, ForwardedIpcCommand, IpcCommand,
        IpcResponseCode,
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema};
    use crate::app::runtime::AppState;
    use crate::app::shell::events::ShellEvent;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use crate::app::shell::sessions::ShortcutCreationRole;
    use crate::app::startup::StartupIntent;
    use eframe::egui;
    use os_api::PriorityClass;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use os_api::{LocalIpcClientError, LocalIpcEndpoint, OS};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::{Arc, RwLock};
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use std::time::SystemTime;

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
            Arc::new(RwLock::new(AppStateStorage {
                version: 5,
                groups: vec![CoreGroup {
                    name: "Games".to_string(),
                    cores: vec![0, 1],
                    programs,
                    is_hidden: false,
                    run_all_button: true,
                }],
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

        App::bootstrap_runtime(
            &mut state,
            &ctx,
            StartupIntent::NormalGui,
            move |_, _, _| rx,
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

    #[test]
    fn test_bootstrap_runtime_normal_gui_runs_autorun() {
        let ctx = egui::Context::default();
        let (_tx, rx) = mpsc::channel();
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
            move |_, _, _| rx,
        );

        let messages = state
            .log_manager
            .entries
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages
            .iter()
            .any(|message| message.contains("AutorunApp") && message.contains("already running")));
        assert!(!messages
            .iter()
            .any(|message| message.contains("ManualApp") || message.contains("ManualApp.exe")));
    }

    #[test]
    fn test_bootstrap_runtime_run_rule_skips_autorun_and_runs_requested_rule() {
        let ctx = egui::Context::default();
        let (_tx, rx) = mpsc::channel();
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
            move |_, _, _| rx,
        );

        let messages = state
            .log_manager
            .entries
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages
            .iter()
            .any(|message| message.contains("ShortcutApp") && message.contains("already running")));
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
        assert_eq!(response.code, IpcResponseCode::Accepted);
        let messages = app
            .state
            .log_manager
            .entries
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages
            .iter()
            .any(|message| message.contains("ShortcutApp") && message.contains("already running")));
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
        assert_eq!(response.code, IpcResponseCode::Accepted);
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
            if let Some(server) = &runtime.server {
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
        match command {
            IpcCommand::RunRule { group_id, rule_id } => {
                run_rule_outcome_to_response(self.state.run_group_program(group_id, rule_id))
            }
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
        let mut visuals = match theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        crate::app::shell::presenters::shared_elements::apply_widget_visuals(&mut visuals);
        ctx.set_visuals(visuals);
        ctx.style_mut_of(
            ctx.theme(),
            crate::app::shell::presenters::shared_elements::apply_widget_style,
        );
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

    fn render_main_ui(&mut self, ui: &mut egui::Ui) {
        header::draw_top_panel(&mut self.state, ui);
        footer::draw_bottom_panel(&mut self.state, ui);
        Self::draw_active_view(&mut self.state, ui);
    }

    fn draw_active_view(app_state: &mut AppState, ui: &mut egui::Ui) {
        match app_state.ui.current_window.clone() {
            WindowRoute::Groups(group_route) => match group_route {
                GroupRoute::List => central::draw_central_panel(app_state, ui),
                GroupRoute::Create => group_editor::create_group_window(app_state, ui),
                GroupRoute::Edit => group_editor::edit_group_window(app_state, ui),
            },
            WindowRoute::Logs => logs::draw_logs_window(app_state, ui),
            WindowRoute::AppRunSettings => run_settings::draw_app_run_settings(app_state, ui),
            WindowRoute::InstalledAppPicker => {
                installed_app_picker::draw_installed_app_picker(app_state, ui)
            }
        }
    }
}
