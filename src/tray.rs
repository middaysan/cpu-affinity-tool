use std::sync::mpsc::Receiver;

/// Простые команды от трея к приложению
#[derive(Debug, Clone)]
pub enum TrayCmd {
    Show,
}

#[cfg(target_os = "windows")]
mod sys {
    use super::{Receiver, TrayCmd};
    use std::sync::mpsc;
    use tray_icon::{
        menu::{Menu, MenuEvent, MenuId, MenuItem},
        Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    pub struct TrayHandle {
        pub tray_icon: TrayIcon,
        pub rx: Receiver<TrayCmd>,
    }

    #[derive(Clone, Copy)]
    struct SendHwnd(isize);
    unsafe impl Send for SendHwnd {}
    unsafe impl Sync for SendHwnd {}

    /// Инициализирует трей. Не требует WindowHandle — создаёт собственное скрытое окно для сообщений.
    pub fn init_tray(
        ctx: eframe::egui::Context,
        hwnd: windows::Win32::Foundation::HWND,
    ) -> Result<TrayHandle, String> {
        // Канал команд
        let (tx, rx) = mpsc::channel::<TrayCmd>();

        // Построим меню
        let menu = Menu::new();
        let show = MenuItem::with_id(MenuId::new("1"), "Restore", true, None);
        let quit = MenuItem::with_id(MenuId::new("3"), "Quit", true, None);

        menu.append(&show).map_err(|e| e.to_string())?;
        menu.append(&quit).map_err(|e| e.to_string())?;

        // Иконка: грузим PNG 32x32 RGBA из assets/icon.ico
        let icon_rgba = include_bytes!("../assets/icon.ico");
        let (rgba, width, height) =
            decode_png_rgba(icon_rgba).map_err(|e| format!("Failed to decode tray icon: {e}"))?;
        let icon = Icon::from_rgba(rgba, width, height)
            .map_err(|e| format!("Failed to create tray icon: {e}"))?;

        // Создаём трей
        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("CPU Affinity Tool")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| format!("Failed to build tray icon: {e}"))?;

        let hwnd_val = SendHwnd(hwnd.0 as isize);

        // Обработка событий через Tokio
        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            tokio::spawn(async move {
                let menu_channel = MenuEvent::receiver();
                let tray_channel = TrayIconEvent::receiver();

                loop {
                    // Опрос событий меню
                    while let Ok(event) = menu_channel.try_recv() {
                        let id = event.id.0.as_str();
                        match id {
                            "1" => {
                                let hwnd = windows::Win32::Foundation::HWND(
                                    hwnd_val.0 as *mut core::ffi::c_void,
                                );
                                os_api::OS::restore_and_focus(hwnd);
                                let _ = tx.send(TrayCmd::Show);
                                ctx.request_repaint();
                            }
                            "3" => {
                                std::process::exit(0);
                            }
                            _ => {}
                        }
                    }

                    // Опрос событий иконки
                    while let Ok(event) = tray_channel.try_recv() {
                        if let TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        } = event
                        {
                            let hwnd = windows::Win32::Foundation::HWND(
                                hwnd_val.0 as *mut core::ffi::c_void,
                            );
                            os_api::OS::restore_and_focus(hwnd);
                            let _ = tx.send(TrayCmd::Show);
                            ctx.request_repaint();
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(16)).await;
                }
            });
        }

        Ok(TrayHandle { tray_icon, rx })
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
    use super::{Receiver, TrayCmd};

    pub struct TrayHandle {
        pub rx: Receiver<TrayCmd>,
    }

    pub fn init_tray(_ctx: eframe::egui::Context) -> Result<TrayHandle, String> {
        let (_tx, rx) = std::sync::mpsc::channel::<TrayCmd>();
        Ok(TrayHandle { rx })
    }
}

pub use sys::init_tray;
