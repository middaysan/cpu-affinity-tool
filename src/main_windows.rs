#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod tray;

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
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
    // Creating tokio runtime manually
    let rt = Runtime::new().expect("failed to create tokio runtime");
    let _guard = rt.enter();

    #[cfg(debug_assertions)]
    println!("DEBUG: Tokio runtime created and entered.");

    // Set self-priority to Below Normal to avoid interfering with high-load apps
    #[warn(unused_variables)]
    if let Err(_e) =
        app::adapters::os::set_current_process_priority(os_api::PriorityClass::BelowNormal)
    {
        #[cfg(debug_assertions)]
        eprintln!("DEBUG: Failed to set self priority: {}", _e);
    }

    let options = NativeOptions {
        run_and_return: true,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_min_inner_size([470.0, 600.0])
            .with_max_inner_size([470.0, 1000.0])
            .with_maximize_button(false), // Disable maximize button
        ..Default::default()
    };

    #[cfg(debug_assertions)]
    {
        println!("DEBUG: NativeOptions initialized:");
        println!("  - Renderer: {:?}", options.renderer);
        println!("  - V-Sync: {}", options.glow_options.vsync);
        println!("  - Run and return: {}", options.run_and_return);
        println!("========================================================");
    }

    // Running eframe on the main thread
    let res = run_native(
        "CPU Affinity Tool",
        options,
        Box::new(move |cc| {
            let startup_requires_forwarding =
                matches!(&startup_intent, StartupIntent::RunRule { .. })
                    && forwarding_runtime.is_some();
            let mut app = App::new_without_startup_intent(cc);
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
    );

    if let Err(e) = res {
        eprintln!("Application error: {}", e);
        std::process::exit(EXIT_GUI_STARTUP_ERROR);
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
