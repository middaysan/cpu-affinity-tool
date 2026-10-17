//! Opt-in native first-show regression. Runs only in an isolated test process.
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindowVisible, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND,
};

static PHASE: AtomicUsize = AtomicUsize::new(0);
static EVENTS: Mutex<Vec<(usize, u32, usize)>> = Mutex::new(Vec::new());

unsafe extern "system" fn event(
    _: HWINEVENTHOOK,
    kind: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if object == 0 && child == 0 {
        if let Ok(mut rows) = EVENTS.lock() {
            if rows.len() < 1024 {
                rows.push((hwnd.0 as usize, kind, PHASE.load(Ordering::Relaxed)));
            }
        }
    }
}

struct Hooks([HWINEVENTHOOK; 2]);
impl Hooks {
    fn new() -> Self {
        let pid = std::process::id();
        let hooks = unsafe {
            [
                SetWinEventHook(
                    EVENT_OBJECT_SHOW,
                    EVENT_OBJECT_HIDE,
                    None,
                    Some(event),
                    pid,
                    0,
                    0,
                ),
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(event),
                    pid,
                    0,
                    0,
                ),
            ]
        };
        assert!(
            hooks.iter().all(|h| !h.0.is_null()),
            "native event hooks unavailable"
        );
        Self(hooks)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        for hook in self.0 {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
    }
}

struct Probe {
    hwnd: HWND,
    start: Instant,
    phase: usize,
    visible: [usize; 4],
    ui_calls: usize,
    result: std::sync::Arc<Mutex<Option<([usize; 4], usize)>>>,
}
impl eframe::App for Probe {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.visible[self.phase] += usize::from(unsafe { IsWindowVisible(self.hwnd) }.as_bool());
        let next = match self.start.elapsed().as_secs_f64() {
            t if t >= 2.6 => 4,
            t if t >= 1.9 => 3,
            t if t >= 1.3 => 2,
            t if t >= 0.7 => 1,
            _ => 0,
        };
        if next != self.phase {
            if next == 4 {
                *self.result.lock().unwrap() = Some((self.visible, self.ui_calls));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.phase = next;
                PHASE.store(next, Ordering::Relaxed);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(next != 2));
            }
        }
        ctx.request_repaint_after(Duration::from_millis(40));
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.ui_calls += 1;
        ui.label("Isolated startup visibility regression");
    }
}

#[test]
#[ignore = "isolated native window visibility probe; no UAC or system settings"]
fn hidden_root_never_shows_before_explicit_restore() {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    PHASE.store(0, Ordering::Relaxed);
    EVENTS.lock().unwrap().clear();
    let _hooks = Hooks::new();
    let result = std::sync::Arc::new(Mutex::new(None));
    let saved = result.clone();
    let root = std::sync::Arc::new(AtomicUsize::new(0));
    let created = root.clone();
    eframe::run_native(
        "Startup visibility probe",
        eframe::NativeOptions {
            run_and_return: true,
            viewport: egui::ViewportBuilder::default()
                .with_visible(false)
                .with_active(false)
                .with_taskbar(false)
                .with_inner_size([280.0, 140.0]),
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_any_thread(true);
            })),
            ..Default::default()
        },
        Box::new(move |cc| {
            let RawWindowHandle::Win32(h) = cc.window_handle().unwrap().as_raw() else {
                panic!("Win32 required")
            };
            let hwnd = HWND(h.hwnd.get() as *mut _);
            created.store(hwnd.0 as usize, Ordering::Relaxed);
            Ok(Box::new(Probe {
                hwnd,
                start: Instant::now(),
                phase: 0,
                visible: [0; 4],
                ui_calls: 0,
                result: saved,
            }))
        }),
    )
    .unwrap();
    let root = root.load(Ordering::Relaxed);
    let rows = EVENTS
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.0 == root)
        .copied()
        .collect::<Vec<_>>();
    let (visible, ui_calls) = result.lock().unwrap().unwrap();
    eprintln!("STARTUP_VISIBILITY visible={visible:?} ui_calls={ui_calls} events={rows:?}");
    assert_eq!(visible[0], 0, "root became visible before Restore");
    assert!(
        !rows
            .iter()
            .any(|r| r.2 == 0 && (r.1 == EVENT_OBJECT_SHOW || r.1 == EVENT_SYSTEM_FOREGROUND)),
        "startup emitted SHOW/focus"
    );
    assert!(visible[1] > 0 && visible[3] > 0, "explicit restore failed");
    assert_eq!(visible[2], 0, "native hide did not remain hidden");
    assert!(ui_calls >= 3);
}
