use std::sync::mpsc::Receiver;

/// Simple commands from the tray to the application
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCmd {
    Show(RestoreRequest),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub struct RestoreRequest {
    pub requested_at: std::time::Instant,
}

impl RestoreRequest {
    #[cfg(any(test, target_os = "windows"))]
    pub fn now() -> Self {
        Self {
            requested_at: std::time::Instant::now(),
        }
    }
}

#[cfg(any(test, target_os = "windows"))]
fn menu_command(id: &str) -> Option<TrayCmd> {
    match id {
        "1" => Some(TrayCmd::Show(RestoreRequest::now())),
        "3" => Some(TrayCmd::Quit),
        _ => None,
    }
}

#[cfg(any(test, target_os = "windows"))]
#[derive(Clone, Copy)]
enum TrayPointerEvent {
    LeftPress,
    LeftDoublePress,
    LeftRelease,
    OtherButton,
}

#[cfg(any(test, target_os = "windows"))]
#[derive(Default)]
struct TrayDoubleClick(std::sync::atomic::AtomicBool);

#[cfg(any(test, target_os = "windows"))]
impl TrayDoubleClick {
    fn command(&self, event: TrayPointerEvent) -> Option<TrayCmd> {
        use std::sync::atomic::Ordering::Relaxed;
        match event {
            TrayPointerEvent::LeftDoublePress => self.0.store(true, Relaxed),
            TrayPointerEvent::LeftRelease => {
                if self.0.swap(false, Relaxed) {
                    return Some(TrayCmd::Show(RestoreRequest::now()));
                }
            }
            TrayPointerEvent::LeftPress | TrayPointerEvent::OtherButton => {
                self.0.store(false, Relaxed)
            }
        }
        None
    }
}

pub struct TrayRuntime {
    rx: Receiver<TrayCmd>,
    #[cfg(target_os = "windows")]
    _tray_icon: tray_icon::TrayIcon,
}

impl TrayRuntime {
    pub fn drain_commands(&self) -> Vec<TrayCmd> {
        std::iter::from_fn(|| self.rx.try_recv().ok()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{menu_command, TrayCmd, TrayDoubleClick, TrayPointerEvent};

    #[test]
    fn menu_events_map_to_typed_commands() {
        assert!(matches!(menu_command("1"), Some(TrayCmd::Show(_))));
        assert_eq!(menu_command("3"), Some(TrayCmd::Quit));
        assert_eq!(menu_command("unknown"), None);
    }

    #[test]
    fn left_double_press_must_not_restore_before_button_release() {
        assert!(
            TrayDoubleClick::default()
                .command(TrayPointerEvent::LeftDoublePress)
                .is_none(),
            "WM_LBUTTONDBLCLK is the second DOWN, not the completed gesture"
        );
    }

    #[test]
    fn only_left_double_click_restores_the_window() {
        let gesture = TrayDoubleClick::default();
        for event in [
            TrayPointerEvent::LeftPress,
            TrayPointerEvent::LeftRelease,
            TrayPointerEvent::LeftDoublePress,
        ] {
            assert!(gesture.command(event).is_none());
        }
        let released_at = std::time::Instant::now();
        assert!(matches!(
            gesture.command(TrayPointerEvent::LeftRelease),
            Some(TrayCmd::Show(request)) if request.requested_at >= released_at
        ));
        assert!(gesture.command(TrayPointerEvent::LeftRelease).is_none());
    }

    #[test]
    fn another_press_cancels_an_unfinished_double_click() {
        for cancel in [TrayPointerEvent::LeftPress, TrayPointerEvent::OtherButton] {
            let gesture = TrayDoubleClick::default();
            assert!(gesture.command(TrayPointerEvent::LeftDoublePress).is_none());
            assert!(gesture.command(cancel).is_none());
            assert!(
                gesture.command(TrayPointerEvent::LeftRelease).is_none(),
                "a missing release must not turn a later single click into Restore"
            );
        }
    }
}

#[cfg(target_os = "windows")]
mod sys {
    use super::{TrayCmd, TrayRuntime};
    use std::sync::mpsc;
    use tray_icon::{
        menu::{Menu, MenuEvent, MenuId, MenuItem},
        Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
    };

    /// Initializes the tray. The application shell owns all window operations.
    pub fn init_tray(ctx: eframe::egui::Context) -> Result<TrayRuntime, String> {
        #[cfg(all(debug_assertions, feature = "diagnostics-test-controls"))]
        if std::env::var_os("CPU_AFFINITY_TOOL_TEST_TRAY_FAILURE").is_some() {
            return Err("Synthetic tray initialization failure".into());
        }
        // Command channel
        let (tx, rx) = mpsc::channel::<TrayCmd>();

        // Build menu
        let menu = Menu::new();
        let show = MenuItem::with_id(MenuId::new("1"), "Restore", true, None);
        let quit = MenuItem::with_id(MenuId::new("3"), "Quit", true, None);

        menu.append(&show).map_err(|e| e.to_string())?;
        menu.append(&quit).map_err(|e| e.to_string())?;

        // Decode the bundled ICO (including BMP/DIB payload support).
        let icon_rgba = include_bytes!("../assets/icon.ico");
        let (rgba, width, height) =
            decode_ico_rgba(icon_rgba).map_err(|e| format!("Failed to decode tray icon: {e}"))?;
        let icon = Icon::from_rgba(rgba, width, height)
            .map_err(|e| format!("Failed to create tray icon: {e}"))?;

        // Create tray
        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("CPU Affinity Tool")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| format!("Failed to build tray icon: {e}"))?;

        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(command) = super::menu_command(event.id.0.as_str()) {
                    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
                    eprintln!("DEBUG: [Tray gesture] menu {command:?}");
                    let _ = tx.send(command);
                    ctx.request_repaint();
                }
            }));
        }

        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            let gesture = super::TrayDoubleClick::default();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                use super::TrayPointerEvent as Pointer;
                let pointer = match event {
                    TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } => Some(Pointer::LeftDoublePress),
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Down,
                        ..
                    } => Some(Pointer::LeftPress),
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } => Some(Pointer::LeftRelease),
                    TrayIconEvent::DoubleClick { .. } | TrayIconEvent::Click { .. } => {
                        Some(Pointer::OtherButton)
                    }
                    _ => None,
                };
                if let Some(pointer) = pointer {
                    if let Some(command) = gesture.command(pointer) {
                        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
                        eprintln!("DEBUG: [Tray gesture] left double-click released {command:?}");
                        let _ = tx.send(command);
                        ctx.request_repaint();
                    }
                }
            }));
        }

        Ok(TrayRuntime {
            rx,
            _tray_icon: tray_icon,
        })
    }

    #[cfg(test)]
    mod icon_tests {
        #[test]
        fn bundled_ico_decodes_with_visible_alpha() {
            let (rgba, width, height) =
                super::decode_ico_rgba(include_bytes!("../assets/icon.ico")).unwrap();
            assert_eq!((width, height), (64, 64));
            assert_eq!(rgba.len(), 64 * 64 * 4);
            assert!(rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
        }
    }

    fn decode_ico_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Ico)
            .map_err(|e| format!("ICO decoding failed: {e}"))?
            .to_rgba8();
        let (w, h) = (img.width(), img.height());
        Ok((img.into_raw(), w, h))
    }
}

#[cfg(not(target_os = "windows"))]
mod sys {
    use super::{TrayCmd, TrayRuntime};

    pub fn init_tray(_ctx: eframe::egui::Context) -> Result<TrayRuntime, String> {
        let (_tx, rx) = std::sync::mpsc::channel::<TrayCmd>();
        Ok(TrayRuntime { rx })
    }
}

pub use sys::init_tray;
