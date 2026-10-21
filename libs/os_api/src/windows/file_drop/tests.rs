use super::*;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SendMessageW, WINDOW_EX_STYLE, WINDOW_STYLE, WS_EX_ACCEPTFILES,
};

#[test]
fn shell_transfer_allows_the_complete_window_scoped_message_set() {
    assert!(FILE_MESSAGES.contains(&windows::Win32::UI::WindowsAndMessaging::WM_COPYDATA));
}

fn payload(paths: &[&str], position: [i32; 2]) -> Vec<u8> {
    let mut bytes = vec![0u8; 20];
    bytes[..4].copy_from_slice(&20u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&position[0].to_le_bytes());
    bytes[8..12].copy_from_slice(&position[1].to_le_bytes());
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    for path in paths {
        for unit in path.encode_utf16().chain([0]) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}
fn handle(bytes: &[u8]) -> HDROP {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len()).unwrap();
        let pointer = GlobalLock(memory).cast::<u8>();
        assert!(!pointer.is_null());
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
        let _ = GlobalUnlock(memory);
        HDROP(memory.0)
    }
}
struct TestWindow(HWND);
impl TestWindow {
    fn new() -> Self {
        Self(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("STATIC"),
                windows::core::w!("Shell drop regression"),
                WINDOW_STYLE::default(),
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        })
    }
}
impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

#[test]
fn shell_payload_keeps_unicode_paths_and_the_original_release_position() {
    let paths = [
        r"C:\Приложения\игра.exe",
        r"C:\Links\shortcut.lnk",
        r"C:\Links\site.url",
    ];
    let result = decode_drop(&payload(&paths, [117, 253])).unwrap();
    assert_eq!(result.paths, paths.map(PathBuf::from));
    assert_eq!(result.position, [117, 253]);
}

#[test]
fn malformed_or_unbounded_shell_payload_never_becomes_a_partial_import() {
    let good = payload(&[r"C:\one.exe"], [5, 7]);
    assert!(decode_drop(&good[..19]).is_err());
    assert!(decode_drop(&good[..good.len() - 2]).is_err());
    assert!(decode_drop(&vec![0; MAX_DROP_BYTES + 1]).is_err());
    let mut offset = good.clone();
    offset[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_drop(&offset).is_err());
    assert!(decode_drop(&payload(&[r"C:\one.exe", "relative.exe"], [0, 0])).is_err());
    assert!(decode_drop(&payload(&vec![r"C:\one.exe"; MAX_DROP_FILES + 1], [0, 0])).is_err());
    assert!(
        decode_drop(&payload(
            &[&format!("C:\\{}", "x".repeat(MAX_PATH_UNITS))],
            [0, 0]
        ))
        .is_err()
    );
}

#[test]
fn shell_ansi_file_lists_are_supported_without_a_lossy_utf8_conversion() {
    let mut bytes = payload(&[], [1, 2]);
    bytes.truncate(20);
    bytes[16] = 0;
    bytes.extend_from_slice(b"C:\\one.exe\0C:\\two.lnk\0\0");
    assert_eq!(
        decode_drop(&bytes).unwrap().paths,
        [PathBuf::from(r"C:\one.exe"), PathBuf::from(r"C:\two.lnk")]
    );
}

#[test]
fn native_shell_drop_is_delivered_once_and_registration_is_window_scoped() {
    let window = TestWindow::new();
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = wakes.clone();
    let guard = WindowFileDropGuard::install(
        window.0.0 as usize,
        Arc::new(move || {
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }),
    )
    .unwrap();
    assert_ne!(
        unsafe { GetWindowLongW(window.0, GWL_EXSTYLE) } as u32 & WS_EX_ACCEPTFILES.0,
        0
    );
    assert!(WindowFileDropGuard::install(window.0.0 as usize, Arc::new(|| {})).is_err());
    let bytes = payload(&[r"C:\one.exe", r"C:\two.lnk"], [31, 47]);
    let dropped = handle(&bytes);
    unsafe {
        SendMessageW(
            window.0,
            WM_DROPFILES,
            Some(WPARAM(dropped.0 as usize)),
            Some(LPARAM(0)),
        );
    }
    assert_eq!(guard.next_drop().unwrap(), decode_drop(&bytes).unwrap());
    assert!(guard.next_drop().is_none());
    assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
    drop(guard);
    assert_eq!(
        unsafe { GetWindowLongW(window.0, GWL_EXSTYLE) } as u32 & WS_EX_ACCEPTFILES.0,
        0
    );
    assert!(WindowFileDropGuard::install(window.0.0 as usize, Arc::new(|| {})).is_ok());
}

#[test]
fn native_shell_drop_point_is_not_shifted_again_when_the_area_flag_is_set() {
    let window = TestWindow::new();
    let guard = WindowFileDropGuard::install(window.0.0 as usize, Arc::new(|| {})).unwrap();
    let mut bytes = payload(&[r"C:\one.exe"], [31, 47]);
    bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
    unsafe {
        SendMessageW(
            window.0,
            WM_DROPFILES,
            Some(WPARAM(handle(&bytes).0 as usize)),
            Some(LPARAM(0)),
        );
    }
    assert_eq!(guard.next_drop().unwrap().position, [31, 47]);
    assert!(guard.next_drop().is_none());
}

#[test]
fn native_drops_outside_the_actual_client_area_are_rejected_in_both_coordinate_modes() {
    let window = TestWindow::new();
    let guard = WindowFileDropGuard::install(window.0.0 as usize, Arc::new(|| {})).unwrap();
    for screen in [false, true] {
        let mut bytes = payload(&[r"C:\one.exe"], [10_000, 10_000]);
        bytes[12..16].copy_from_slice(&u32::from(screen).to_le_bytes());
        unsafe {
            SendMessageW(
                window.0,
                WM_DROPFILES,
                Some(WPARAM(handle(&bytes).0 as usize)),
                Some(LPARAM(0)),
            );
        }
        assert!(guard.next_drop().is_none());
    }
}

#[test]
fn destroying_a_registered_window_before_its_guard_does_not_retain_a_callback() {
    let window = TestWindow::new();
    let guard = WindowFileDropGuard::install(window.0.0 as usize, Arc::new(|| {})).unwrap();
    let sink = Rc::downgrade(&guard.sink);
    drop(window);
    assert_eq!(sink.strong_count(), 1);
    drop(guard);
    assert!(sink.upgrade().is_none());
    assert_eq!(
        FILE_MESSAGES,
        [
            WM_DROPFILES,
            windows::Win32::UI::WindowsAndMessaging::WM_COPYDATA,
            0x49
        ]
    );
}

#[test]
fn pending_shell_drop_is_bounded_and_foreign_gui_threads_cannot_register() {
    let window = TestWindow::new();
    let hwnd = window.0.0 as usize;
    assert!(
        std::thread::spawn(move || WindowFileDropGuard::install(hwnd, Arc::new(|| {})).is_err())
            .join()
            .unwrap()
    );
    let guard = WindowFileDropGuard::install(hwnd, Arc::new(|| {})).unwrap();
    for name in [r"C:\first.exe", r"C:\second.exe"] {
        let data = handle(&payload(&[name], [3, 5]));
        unsafe {
            SendMessageW(
                window.0,
                WM_DROPFILES,
                Some(WPARAM(data.0 as usize)),
                Some(LPARAM(0)),
            );
        }
    }
    assert_eq!(
        guard.next_drop().unwrap().paths,
        [PathBuf::from(r"C:\first.exe")]
    );
    assert!(guard.next_drop().is_none());
    let data = handle(&payload(&["relative.exe"], [3, 5]));
    unsafe {
        SendMessageW(
            window.0,
            WM_DROPFILES,
            Some(WPARAM(data.0 as usize)),
            Some(LPARAM(0)),
        );
    }
    assert!(guard.next_drop().is_none());
}
