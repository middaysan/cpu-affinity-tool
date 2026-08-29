use std::sync::mpsc::Receiver;

/// Simple commands from the tray to the application
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCmd {
    Show,
    Quit,
}

fn menu_command(id: &str) -> Option<TrayCmd> {
    match id {
        "1" => Some(TrayCmd::Show),
        "3" => Some(TrayCmd::Quit),
        _ => None,
    }
}

fn tray_command_for_double_click(left_button: bool) -> Option<TrayCmd> {
    left_button.then_some(TrayCmd::Show)
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
    use super::{menu_command, tray_command_for_double_click, TrayCmd};

    #[test]
    fn menu_events_map_to_typed_commands() {
        assert_eq!(menu_command("1"), Some(TrayCmd::Show));
        assert_eq!(menu_command("3"), Some(TrayCmd::Quit));
        assert_eq!(menu_command("unknown"), None);
    }

    #[test]
    fn only_left_double_click_restores_the_window() {
        assert_eq!(tray_command_for_double_click(true), Some(TrayCmd::Show));
        assert_eq!(tray_command_for_double_click(false), None);
    }
}

#[cfg(target_os = "windows")]
mod sys {
    use super::{TrayCmd, TrayRuntime};
    use std::sync::mpsc;
    use tray_icon::{
        menu::{Menu, MenuEvent, MenuId, MenuItem},
        Icon, MouseButton, TrayIconBuilder, TrayIconEvent,
    };

    /// Initializes the tray. The application shell owns all window operations.
    pub fn init_tray(ctx: eframe::egui::Context) -> Result<TrayRuntime, String> {
        // Command channel
        let (tx, rx) = mpsc::channel::<TrayCmd>();

        // Build menu
        let menu = Menu::new();
        let show = MenuItem::with_id(MenuId::new("1"), "Restore", true, None);
        let quit = MenuItem::with_id(MenuId::new("3"), "Quit", true, None);

        menu.append(&show).map_err(|e| e.to_string())?;
        menu.append(&quit).map_err(|e| e.to_string())?;

        // Icon: load PNG 32x32 RGBA from assets/icon.ico
        let icon_rgba = include_bytes!("../assets/icon.ico");
        let (rgba, width, height) =
            decode_png_rgba(icon_rgba).map_err(|e| format!("Failed to decode tray icon: {e}"))?;
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
                    let _ = tx.send(command);
                    ctx.request_repaint();
                }
            }));
        }

        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::DoubleClick { button, .. } = event {
                    let left_button = matches!(button, MouseButton::Left);
                    if let Some(command) = super::tray_command_for_double_click(left_button) {
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

    fn decode_png_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
        let img = image::load_from_memory(bytes)
            .map_err(|e| format!("image load_from_memory failed: {e}"))?
            .to_rgba8();
        let (w, h) = (img.width(), img.height());
        Ok((img.to_vec(), w, h))
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
