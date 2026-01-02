use std::sync::mpsc::Receiver;

/// Простые команды от трея к приложению
#[derive(Debug, Clone)]
pub enum TrayCmd {
    Show,
}

#[cfg(target_os = "windows")]
mod sys {
    use super::{Receiver, TrayCmd};
    use tray_icon::{
        menu::{Menu, MenuEvent, MenuId, MenuItem},
        Icon, TrayIcon, TrayIconBuilder, TrayIconEvent, ClickType,
    };
    use std::sync::mpsc;

    // ID пунктов меню - больше не используются как константы, но оставим для справки или удалим
    // const ID_SHOW: &str = "1";
    // const ID_HIDE: &str = "2";
    // const ID_QUIT: &str = "3";

    pub struct TrayHandle {
        pub tray_icon: TrayIcon,
        pub rx: Receiver<TrayCmd>,
    }

    #[derive(Clone, Copy)]
    struct SendHwnd(isize);
    unsafe impl Send for SendHwnd {}
    unsafe impl Sync for SendHwnd {}

    /// Инициализирует трей. Не требует WindowHandle — создаёт собственное скрытое окно для сообщений.
    pub fn init_tray(ctx: eframe::egui::Context, hwnd: windows::Win32::Foundation::HWND) -> Result<TrayHandle, String> {
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
        let (rgba, width, height) = decode_png_rgba(icon_rgba)
            .map_err(|e| format!("Failed to decode tray icon: {e}"))?;
        let icon = Icon::from_rgba(rgba, width, height)
            .map_err(|e| format!("Failed to create tray icon: {e}"))?;

        // Создаём трей
        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("CPU Affinity Tool")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()
            .map_err(|e| format!("Failed to build tray icon: {e}"))?;

        let hwnd = SendHwnd(hwnd.0 as isize);

        // Подписываемся на клики по меню — шлём в наш tx
        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let hwnd = windows::Win32::Foundation::HWND(hwnd.0 as *mut core::ffi::c_void);
                let id = event.id.0.as_str();
                #[cfg(debug_assertions)]
                println!("DEBUG: [Tray Thread] MenuEvent received: id={}", id);
                
                match id {
                    "1" => { 
                        #[cfg(debug_assertions)]
                        println!("DEBUG: [Tray Thread] Calling OS::restore_and_focus");
                        os_api::OS::restore_and_focus(hwnd);
                        let _ = tx.send(TrayCmd::Show); 
                    }
                    "3" => { 
                        #[cfg(debug_assertions)]
                        println!("DEBUG: [Tray Thread] Executing immediate exit (Quit)");
                        std::process::exit(0);
                    }
                    _ => {
                        #[cfg(debug_assertions)]
                        println!("DEBUG: [Tray Thread] Unknown MenuId: {}", id);
                    }
                }

                ctx.request_repaint();
            }));
        }

        // Обработка кликов по иконке
        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                let hwnd = windows::Win32::Foundation::HWND(hwnd.0 as *mut core::ffi::c_void);
                
                #[cfg(debug_assertions)]
                println!("DEBUG: TrayIconEvent received: {:?}", event);
                
                // Реагируем только на двойной клик (обычно это ЛКМ на Windows)
                if event.click_type == ClickType::Double {
                    #[cfg(debug_assertions)]
                    println!("DEBUG: [Tray Thread] Double click detected. Calling OS::restore_and_focus");
                    
                    os_api::OS::restore_and_focus(hwnd);
                    let _ = tx.send(TrayCmd::Show);
                    ctx.request_repaint();
                }
            }));
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
