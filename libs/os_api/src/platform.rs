//! Read-only host identification, independent of Windows compatibility manifests.
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsPlatform {
    Windows {
        major: u32,
        minor: u32,
        build: u32,
        workstation: bool,
    },
    Linux,
    Other,
    Unknown,
}

pub fn current_platform() -> OsPlatform {
    static PLATFORM: OnceLock<OsPlatform> = OnceLock::new();
    *PLATFORM.get_or_init(detect_platform)
}

#[cfg(target_os = "windows")]
fn detect_platform() -> OsPlatform {
    use windows::{
        Win32::System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            SystemInformation::OSVERSIONINFOEXW,
        },
        core::{s, w},
    };

    type RtlGetVersion = unsafe extern "system" fn(*mut OSVERSIONINFOEXW) -> i32;
    // ntdll is owned by the process loader; GetModuleHandleW adds no reference to release.
    let Ok(module) = (unsafe { GetModuleHandleW(w!("ntdll.dll")) }) else {
        return OsPlatform::Unknown;
    };
    let Some(address) = (unsafe { GetProcAddress(module, s!("RtlGetVersion")) }) else {
        return OsPlatform::Unknown;
    };
    // RtlGetVersion accepts an OSVERSIONINFOEXW when its exact structure size is supplied.
    let get_version = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, RtlGetVersion>(address)
    };
    let mut version = OSVERSIONINFOEXW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOEXW>() as u32,
        ..Default::default()
    };
    if unsafe { get_version(&mut version) } != 0
        || version.dwPlatformId != 2
        || version.dwMajorVersion == 0
        || version.dwBuildNumber == 0
    {
        return OsPlatform::Unknown;
    }
    OsPlatform::Windows {
        major: version.dwMajorVersion,
        minor: version.dwMinorVersion,
        build: version.dwBuildNumber,
        workstation: version.wProductType == 1,
    }
}

#[cfg(target_os = "linux")]
fn detect_platform() -> OsPlatform {
    OsPlatform::Linux
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn detect_platform() -> OsPlatform {
    OsPlatform::Other
}
