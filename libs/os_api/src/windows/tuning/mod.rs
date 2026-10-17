mod identity;
pub use identity::TuningProcessLifetimeGuard;

impl crate::windows::OS {
    /// Existing recovery data is never discarded by the simplified controller.
    pub fn tuning_legacy_evidence_exists() -> Result<bool, crate::tuning::TuningNativeError> {
        use windows::Win32::{
            System::Com::CoTaskMemFree,
            UI::Shell::{FOLDERID_ProgramData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
        };
        let native = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) }
            .map_err(|error| native_error("ProgramData", error))?;
        let text = unsafe { native.to_string() };
        unsafe {
            CoTaskMemFree(Some(native.0.cast()));
        }
        let root =
            std::path::PathBuf::from(text.map_err(|_| {
                crate::tuning::TuningNativeError::Unsupported("ProgramData encoding")
            })?)
            .join("CpuAffinityTool")
            .join("tuning");
        for leaf in [
            "journal-v1.json",
            "journal-v1.staging",
            "journal-v1.previous",
        ] {
            match std::fs::symlink_metadata(root.join(leaf)) {
                Ok(_) => return Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(code_error(
                        "legacy recovery check",
                        error.raw_os_error().unwrap_or(1) as u32,
                    ));
                }
            }
        }
        Ok(false)
    }
}

use crate::tuning::TuningNativeError;
pub(super) fn code_error(operation: &'static str, code: u32) -> TuningNativeError {
    match code {
        5 => TuningNativeError::Denied { operation, code },
        32 | 33 => TuningNativeError::Busy,
        _ => TuningNativeError::Io { operation, code },
    }
}
pub(super) fn native_error(
    operation: &'static str,
    error: windows::core::Error,
) -> TuningNativeError {
    let raw = error.code().0 as u32;
    code_error(
        operation,
        if raw & 0xffff0000 == 0x80070000 {
            raw & 0xffff
        } else {
            raw
        },
    )
}
