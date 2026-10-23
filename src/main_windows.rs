#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(all(debug_assertions, feature = "render-profiling"))]
mod native_frame_probe;
mod tray;

use app::features::diagnostics::crash_reports::{
    handle_native_loop_outcome, install_panic_hook, CrashReportContext, StartupPhase,
    REPORT_DIRECTORY_NAME,
};
use app::instance_forwarding::{
    prepare_startup_forwarding_with, EntryAction, ForwardingClientError, ForwardingRetryPolicy,
    PreparedStartupForwarding, StartupForwardingClock, StartupForwardingPlatform,
    EXIT_CLI_PARSE_ERROR, EXIT_GUI_STARTUP_ERROR,
};
use app::shell::{App, AppForwardingRuntime};
use app::startup::parse_startup_args;
use app::startup::StartupIntent;
use eframe::{run_native, NativeOptions};
use os_api::{LocalIpcClientError, LocalIpcEndpoint, LocalIpcGuard, OS};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

#[cfg(test)]
mod startup_visibility_probe;

fn main() {
    let startup_args = std::env::args().skip(1).collect::<Vec<_>>();
    let startup_intent = match parse_startup_args(&startup_args) {
        Ok(intent) => intent,
        Err(err) => {
            eprintln!("Invalid startup arguments: {err:?}");
            std::process::exit(EXIT_CLI_PARSE_ERROR);
        }
    };
    let PreparedStartupForwarding {
        action,
        mut forwarding_runtime,
        forwarding_warning,
    } = prepare_startup_forwarding(startup_intent);
    let startup_intent = match action {
        EntryAction::RunGui(intent) => intent,
        EntryAction::Exit(code) => std::process::exit(code),
    };
    let crash_report_context = Arc::new(CrashReportContext::new(crash_report_directory()));
    if let Err(error) = install_panic_hook(crash_report_context.clone()) {
        eprintln!("Crash report hook was not installed: {error}");
    }
    #[cfg(all(feature = "diagnostics-test-controls", debug_assertions))]
    let diagnostics_test_fault = requested_diagnostics_test_fault();
    #[cfg(all(feature = "diagnostics-test-controls", debug_assertions))]
    if matches!(
        diagnostics_test_fault,
        Some(DiagnosticsTestFault::PreRuntimePanic)
    ) {
        panic!("synthetic pre-runtime panic");
    }

    #[cfg(debug_assertions)]
    {
        println!("========================================================");
        println!("DEBUG: Application starting...");
        println!(
            "DEBUG: OS: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        println!("DEBUG: Reactive mode: YES (Wait-based event loop)");
        println!("========================================================");
    }
    // Restrict our process before creating Tokio/GUI/background workers.
    let mut scheduling_messages = app::startup::prepare_background_scheduling_with(
        app::adapters::os::set_current_process_priority,
        app::adapters::os::pin_current_process_to_last_cpu,
    )
    .to_vec();
    #[cfg(feature = "render-profiling")]
    let _render_profile_session = match app::features::diagnostics::render_profile::start_from_env()
    {
        Ok(session) => session,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(EXIT_GUI_STARTUP_ERROR);
        }
    };
    #[cfg(all(debug_assertions, feature = "render-profiling"))]
    if std::env::var_os("CPU_AFFINITY_NATIVE_PROBE").is_some() {
        native_frame_probe::install();
    }
    // Creating tokio runtime manually
    let rt = Runtime::new().expect("failed to create tokio runtime");
    let _guard = rt.enter();
    if let Err(error) = OS::prefer_current_gui_thread() {
        scheduling_messages.push(format!("Own GUI scheduling: {error}"));
    }

    #[cfg(debug_assertions)]
    println!("DEBUG: Tokio runtime created and entered.");

    let storage = app::adapters::storage::StorageAdapter::load();
    let start_hidden = storage
        .shared()
        .read()
        .map(|state| state.start_minimized)
        .unwrap_or(false)
        && app::adapters::os::supports_hide_to_tray();
    let options = windows_native_options(start_hidden);

    #[cfg(debug_assertions)]
    {
        println!("DEBUG: NativeOptions initialized:");
        println!("  - Renderer: {:?}", options.renderer);
        println!("  - WGPU: DX12 / MemoryUsage / debug graphics layers disabled");
        println!("  - Run and return: {}", options.run_and_return);
        println!("========================================================");
    }

    // Running eframe on the main thread
    crash_report_context.set_phase(StartupPhase::RunningUi);
    let run_application = move || {
        run_native(
            "CPU Affinity Tool",
            options,
            Box::new(move |cc| {
                if let Some(window) = cc.winit_window() {
                    // egui-winit 0.36.2 reapplies initial limits in physical pixels.
                    // Keep them logical so Winit's WM_GETMINMAXINFO follows DPI
                    // changes rather than clamping to the first monitor's pixels.
                    let [minimum, maximum] = windows_window_limits(
                        &windows_viewport(start_hidden),
                        cc.egui_ctx.zoom_factor(),
                    );
                    window.set_min_inner_size(minimum);
                    window.set_max_inner_size(maximum);
                }
                #[cfg(feature = "render-profiling")]
                if let Some(state) = &cc.wgpu_render_state {
                    let info = state.adapter.get_info();
                    app::features::diagnostics::render_profile::renderer(format!(
                        "WGPU {:?} / {} / {} / MemoryUsage",
                        info.backend, info.name, info.driver
                    ));
                }
                let startup_requires_forwarding =
                    matches!(&startup_intent, StartupIntent::RunRule { .. })
                        && forwarding_runtime.is_some();
                let mut app = App::new_with_storage(cc, storage);
                for message in scheduling_messages {
                    app.state.log_manager.add_sticky_once(message);
                }
                let forwarding_ready = app.install_forwarding_runtime(
                    forwarding_runtime.take(),
                    forwarding_warning,
                    &cc.egui_ctx,
                );
                app.handle_startup_intent_after_forwarding(
                    startup_intent,
                    startup_requires_forwarding && !forwarding_ready,
                );
                Ok(Box::new(app))
            }),
        )
    };
    #[cfg(all(feature = "diagnostics-test-controls", debug_assertions))]
    let res = if matches!(
        diagnostics_test_fault,
        Some(DiagnosticsTestFault::NativeLoopError)
    ) {
        Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(
            "synthetic native loop error",
        ))))
    } else {
        run_application()
    };
    #[cfg(not(all(feature = "diagnostics-test-controls", debug_assertions)))]
    let res = run_application();
    let res = handle_native_loop_outcome(&crash_report_context, res);
    crash_report_context.set_phase(StartupPhase::Closing);

    if let Err(e) = res {
        eprintln!("Application error: {}", e);
        std::process::exit(EXIT_GUI_STARTUP_ERROR);
    }
    drop(_guard);
    stop_async_runtime(rt);
}

fn stop_async_runtime(runtime: Runtime) {
    // Native OS calls cannot always be cancelled. Shell shutdown has already
    // revoked admission and completed the owned configuration/recovery path.
    runtime.shutdown_timeout(Duration::from_millis(250));
}

fn windows_viewport(start_hidden: bool) -> eframe::egui::ViewportBuilder {
    eframe::egui::ViewportBuilder::default()
        .with_drag_and_drop(false)
        .with_visible(!start_hidden)
        .with_active(!start_hidden)
        .with_taskbar(!start_hidden)
        .with_inner_size([400.0, 700.0])
        .with_min_inner_size([400.0, 600.0])
        .with_max_inner_size([400.0, 1000.0])
        .with_maximize_button(false)
}

fn windows_window_limits(
    viewport: &eframe::egui::ViewportBuilder,
    zoom_factor: f32,
) -> [Option<winit::dpi::Size>; 2] {
    [viewport.min_inner_size, viewport.max_inner_size].map(|size| {
        size.map(|size| {
            winit::dpi::LogicalSize::new(size.x * zoom_factor, size.y * zoom_factor).into()
        })
    })
}

#[cfg(test)]
mod viewport_tests {
    #[test]
    fn shell_file_drops_replace_the_elevated_ole_drop_target() {
        for hidden in [false, true] {
            assert_eq!(super::windows_viewport(hidden).drag_and_drop, Some(false));
        }
    }

    #[test]
    fn startup_self_scheduling_keeps_topology_and_tokio_workers_correct() {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "viewport_tests::native_startup_scheduling_probe",
                "--nocapture",
            ])
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
    }

    #[test]
    #[ignore = "isolated own-process startup/Tokio probe invoked by its parent test"]
    fn native_startup_scheduling_probe() {
        let threads = crate::app::features::topology::detect_total_threads(0);
        let messages = crate::app::startup::prepare_background_scheduling_with(
            crate::app::adapters::os::set_current_process_priority,
            crate::app::adapters::os::pin_current_process_to_last_cpu,
        );
        assert!(messages[0].contains("BelowNormal"));
        assert_eq!(
            crate::app::features::topology::detect_total_threads(0),
            threads,
            "self affinity must not hide the machine's CPU topology"
        );
        if messages[1].contains("requires one Windows processor group") {
            return;
        }
        assert!(
            messages[1].starts_with("Own process: CPU"),
            "{:?}",
            messages
        );
        let mask = os_api::OS::get_process_affinity(std::process::id()).unwrap();
        assert_eq!(mask.count_ones(), 1);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            tokio::spawn(async move {
                assert_eq!(
                    os_api::OS::get_process_affinity(std::process::id()).unwrap(),
                    mask
                );
                assert_eq!(
                    os_api::OS::get_process_priority(std::process::id()).unwrap(),
                    os_api::PriorityClass::BelowNormal
                );
            })
            .await
            .unwrap();
        });
        println!("STARTUP_SELF_SCHEDULING threads={threads} mask=0x{mask:X} tokio_worker=true logs={messages:?}");
    }

    #[test]
    fn closing_does_not_wait_forever_for_a_blocked_native_api_call() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let (release, finish) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        runtime.spawn_blocking(move || {
            started.send(()).unwrap();
            let _ = finish.recv();
            let _ = finished.send(());
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let began = std::time::Instant::now();
        super::stop_async_runtime(runtime);
        assert!(began.elapsed() < std::time::Duration::from_secs(2));
        release.send(()).unwrap();
        done.recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
    }

    #[test]
    fn compact_layout_window_keeps_fixed_width_and_resizable_height() {
        let viewport = super::windows_viewport(false);
        let minimum = viewport.min_inner_size.unwrap();
        let maximum = viewport.max_inner_size.unwrap();
        assert_eq!(viewport.inner_size.unwrap().x, 400.0);
        assert_eq!(minimum.x, 400.0);
        assert_eq!(maximum.x, 400.0);
        assert!(maximum.y > minimum.y);
    }

    #[test]
    fn window_limits_follow_dpi_without_changing_logical_width_or_height() {
        use winit::dpi::{LogicalSize, Size};

        for hidden in [false, true] {
            for zoom in [1.0, 1.25] {
                let viewport = super::windows_viewport(hidden);
                let [minimum, maximum] = super::windows_window_limits(&viewport, zoom);
                let (minimum, maximum) = (minimum.unwrap(), maximum.unwrap());
                for height in [600.0, 700.0, 1000.0] {
                    let desired = LogicalSize::new(400.0 * zoom as f64, height * zoom as f64);
                    let mut scale = 1.0;
                    let mut physical = desired.to_physical::<u32>(scale);
                    // Model Winit's preserved logical client size, followed by
                    // Windows checking the current min/max bounds at each DPI.
                    for next_scale in [1.75, 1.0, 1.25, 2.0, 1.5, 1.0] {
                        let resized = physical
                            .to_logical::<f64>(scale)
                            .to_physical::<u32>(next_scale);
                        physical =
                            Size::clamp(Size::Physical(resized), minimum, maximum, next_scale)
                                .to_physical::<u32>(next_scale);
                        let expected = desired.to_physical::<u32>(next_scale);
                        assert_eq!(physical.width, expected.width);
                        assert!(physical.height.abs_diff(expected.height)<=1,"DPI round trips may round one physical pixel, but must not multiply logical size: {physical:?} vs {expected:?}");
                        scale = next_scale;
                    }
                }
            }
        }
    }

    #[test]
    fn startup_preference_controls_initial_visibility_focus_and_taskbar() {
        for hidden in [false, true] {
            let viewport = super::windows_viewport(hidden);
            assert_eq!(viewport.visible, Some(!hidden));
            assert_eq!(viewport.active, Some(!hidden));
            assert_eq!(viewport.taskbar, Some(!hidden));
            assert!(viewport.position.is_none());
        }
    }
}

fn crash_report_directory() -> std::path::PathBuf {
    #[cfg(all(feature = "diagnostics-test-controls", debug_assertions))]
    if let Some(path) = std::env::var_os("CPU_AFFINITY_TOOL_TEST_CRASH_REPORT_DIR") {
        return std::path::PathBuf::from(path);
    }

    app::adapters::storage::StorageAdapter::active_data_dir().join(REPORT_DIRECTORY_NAME)
}

#[cfg(all(feature = "diagnostics-test-controls", debug_assertions))]
#[derive(Clone, Copy)]
enum DiagnosticsTestFault {
    PreRuntimePanic,
    NativeLoopError,
}

#[cfg(all(feature = "diagnostics-test-controls", debug_assertions))]
fn requested_diagnostics_test_fault() -> Option<DiagnosticsTestFault> {
    match std::env::var("CPU_AFFINITY_TOOL_TEST_CRASH_FAULT").as_deref() {
        Ok("pre-runtime-panic") => Some(DiagnosticsTestFault::PreRuntimePanic),
        Ok("native-loop-error") => Some(DiagnosticsTestFault::NativeLoopError),
        _ => None,
    }
}

fn prepare_startup_forwarding(
    startup_intent: StartupIntent,
) -> PreparedStartupForwarding<AppForwardingRuntime> {
    let mut platform = WindowsStartupForwardingPlatform::default();
    let mut clock = RealStartupForwardingClock::new();
    prepare_startup_forwarding_with(
        startup_intent,
        &mut platform,
        &mut clock,
        windows_forwarding_retry_policy(),
    )
}

fn windows_forwarding_retry_policy() -> ForwardingRetryPolicy {
    ForwardingRetryPolicy {
        total_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(2),
        retry_sleep: Duration::from_millis(25),
    }
}

struct RealStartupForwardingClock {
    started: Instant,
}

impl RealStartupForwardingClock {
    fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl StartupForwardingClock for RealStartupForwardingClock {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Default)]
struct WindowsStartupForwardingPlatform {
    endpoint: Option<LocalIpcEndpoint>,
}

impl WindowsStartupForwardingPlatform {
    fn endpoint(&self) -> Result<&LocalIpcEndpoint, String> {
        self.endpoint
            .as_ref()
            .ok_or_else(|| "local shortcut endpoint was not initialized".to_string())
    }
}

impl StartupForwardingPlatform for WindowsStartupForwardingPlatform {
    type Guard = LocalIpcGuard;
    type Runtime = AppForwardingRuntime;

    fn resolve_endpoint(&mut self) -> Result<(), String> {
        self.endpoint = Some(OS::local_ipc_endpoint()?);
        Ok(())
    }

    fn try_claim_primary_guard(&mut self) -> Result<Option<Self::Guard>, String> {
        let result = OS::try_claim_local_ipc_primary_guard(self.endpoint()?);
        if let Err(err) = &result {
            eprintln!("Shortcut forwarding guard check failed: {err}");
        }
        result
    }

    fn start_forwarding_runtime(&mut self, guard: Self::Guard) -> Result<Self::Runtime, String> {
        Ok(AppForwardingRuntime::pending(
            guard,
            self.endpoint()?.clone(),
        ))
    }

    fn send_request(
        &mut self,
        request: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, ForwardingClientError> {
        let endpoint = self.endpoint().map_err(ForwardingClientError::Io)?;
        OS::send_local_ipc_request(endpoint, request, timeout).map_err(|err| match err {
            LocalIpcClientError::NoServer => ForwardingClientError::NoServer,
            LocalIpcClientError::ServerNotReady => ForwardingClientError::ServerNotReady,
            LocalIpcClientError::Timeout => ForwardingClientError::Timeout,
            LocalIpcClientError::SecurityRejected(err) => {
                eprintln!("Shortcut forwarding security rejected: {err}");
                ForwardingClientError::SecurityRejected(err)
            }
            LocalIpcClientError::Io(err) => {
                eprintln!("Shortcut forwarding failed: {err}");
                ForwardingClientError::Io(err)
            }
        })
    }
}

// Keep renderer configuration and initial root visibility in one startup contract.
fn windows_native_options(start_hidden: bool) -> NativeOptions {
    let mut options = NativeOptions {
        run_and_return: true,
        renderer: eframe::Renderer::Wgpu,
        viewport: windows_viewport(start_hidden),
        ..Default::default()
    };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
        setup.instance_descriptor.flags = wgpu::InstanceFlags::empty();
        let original = Arc::clone(&setup.device_descriptor);
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = original(adapter);
            descriptor.memory_hints = wgpu::MemoryHints::MemoryUsage;
            descriptor
        });
    }
    options
}

#[cfg(test)]
mod windows_renderer_tests {
    #[test]
    fn full_app_uses_dx12_and_preserves_startup_visibility() {
        for hidden in [false, true] {
            let options = super::windows_native_options(hidden);
            assert_eq!(options.renderer, eframe::Renderer::Wgpu);
            assert!(options.run_and_return);
            assert!(
                options.viewport.title.is_none(),
                "native title comes from run_native"
            );
            assert_eq!(options.viewport.active, Some(!hidden));
            assert_eq!(options.viewport.taskbar, Some(!hidden));
            assert_eq!(options.viewport.visible, Some(!hidden));
            let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = options.wgpu_options.wgpu_setup
            else {
                panic!("expected owned WGPU device")
            };
            assert_eq!(setup.instance_descriptor.backends, wgpu::Backends::DX12);
            assert!(setup.instance_descriptor.flags.is_empty());
        }
    }
}
