#[cfg(target_os = "windows")]
pub mod windows_tray {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::*,
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Shell::*,
                WindowsAndMessaging::*,
            },
        },
    };
    use once_cell::sync::Lazy;

    type Callback = Box<dyn Fn() + Send + Sync>;

    static CALLBACKS: Lazy<Arc<Mutex<HashMap<usize, Callback>>>> =
        Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

    static NEXT_ID: Lazy<Mutex<usize>> = Lazy::new(|| Mutex::new(1000));

    #[derive(Clone)]
    pub struct WindowsTray {
        icon_path: Option<String>,
        tip: String,
    }

    impl WindowsTray {
        pub fn new(icon_path: Option<&str>, tip: &str) -> Self {
            Self {
                icon_path: icon_path.map(String::from),
                tip: tip.to_string(),
            }
        }

        pub fn add_menu_item<F>(&mut self, label: &str, callback: F)
        where
            F: Fn() + Send + Sync + 'static,
        {
            let mut id_lock = NEXT_ID.lock().unwrap();
            let id = *id_lock;
            *id_lock += 1;

            CALLBACKS.lock().unwrap().insert(id, Box::new(callback));
            MENU_ITEMS.lock().unwrap().push((id, label.to_string()));
        }

        pub fn run(&self) -> windows::core::Result<()> {
            unsafe {
                let hinstance = GetModuleHandleW(None)?;

                let class_name_vec = to_wide("tray_window_class");
                let class_name = class_name_vec.as_ptr();
                let wnd_class = WNDCLASSW {
                    hInstance: HINSTANCE(hinstance.0),
                    lpszClassName: PCWSTR(class_name),
                    lpfnWndProc: Some(wnd_proc),
                    ..Default::default()
                };

                RegisterClassW(&wnd_class);

                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR(class_name),
                    PCWSTR::null(),
                    WS_OVERLAPPEDWINDOW,
                    0, 0, 0, 0,
                    None,
                    None,
                    Some(HINSTANCE(hinstance.0)),
                    None,
                )?;

                let hicon = if let Some(path) = &self.icon_path {
                    let wide_path = to_wide(path);
                    match LoadImageW(
                        None,
                        PCWSTR(wide_path.as_ptr()),
                        IMAGE_ICON,
                        0,
                        0,
                        LR_LOADFROMFILE,
                    ) {
                        Ok(handle) => HICON(handle.0),
                        Err(_) => LoadIconW(None, IDI_APPLICATION)?
                    }
                } else {
                    LoadIconW(None, IDI_APPLICATION)?
                };

                let mut nid = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: 1,
                    uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                    uCallbackMessage: WM_USER + 1,
                    hIcon: HICON(hicon.0),
                    szTip: [0; 128],
                    ..Default::default()
                };

                let tip_wide = to_wide(&self.tip);
                nid.szTip[..tip_wide.len()].copy_from_slice(&tip_wide);

                if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                    eprintln!("Failed to add the notification icon.");
                }

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).into() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                if !Shell_NotifyIconW(NIM_DELETE, &nid).as_bool() {
                    eprintln!("Failed to delete the notification icon.");
                }

                Ok(())
            }
        }
    }

    static MENU_ITEMS: Lazy<Arc<Mutex<Vec<(usize, String)>>>> = Lazy::new(|| Arc::new(Mutex::new(vec![])));

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            x if x == WM_USER + 1 => {
                if lparam.0 as u32 == WM_RBUTTONUP {
                    let hmenu = CreatePopupMenu().unwrap();
                    let menu_items = MENU_ITEMS.lock().unwrap();
                    for &(id, ref label) in menu_items.iter() {
                        let wide_label = to_wide(label);
                        AppendMenuW(hmenu, MF_STRING, id, PCWSTR(wide_label.as_ptr())).unwrap();
                        // `wide_label` должен жить до конца вызова AppendMenuW
                    }

                    let mut p = POINT::default();
                    GetCursorPos(&mut p).expect("Failed to get cursor position");
                    let _ = SetForegroundWindow(hwnd);
                    let _ = TrackPopupMenu(hmenu, TPM_BOTTOMALIGN, p.x, p.y, None, hwnd, None);
                    DestroyMenu(hmenu).expect("Failed to destroy menu");
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wparam.0 as u32 & 0xFFFF) as usize;
                if let Some(cb) = CALLBACKS.lock().unwrap().get(&id) {
                    cb();
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
}