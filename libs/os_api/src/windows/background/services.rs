//! Bounded SCM membership and independent live service/configuration recheck.
use super::{directory, native, trust::TrustCache, unsupported};
use crate::{
    background::*,
    gaming::{GamingError, ProcessIdentity},
    windows::OS,
};
use std::{mem::size_of, ptr};
use windows::{
    Win32::System::Services::*,
    core::{PCWSTR, PWSTR},
};

struct ServiceHandle(SC_HANDLE);
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn manager() -> Result<ServiceHandle, GamingError> {
    unsafe {
        OpenSCManagerW(
            PCWSTR::null(),
            PCWSTR::null(),
            SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE,
        )
    }
    .map(ServiceHandle)
    .map_err(|e| native("OpenSCManager background", e))
}

fn bounded_string(value: PWSTR, buffer: &[u8]) -> Result<String, GamingError> {
    let start = buffer.as_ptr() as usize;
    let address = value.0 as usize;
    let end = start
        .checked_add(buffer.len())
        .ok_or(unsupported("Invalid SCM buffer"))?;
    if address < start || address >= end || !address.is_multiple_of(2) {
        return Err(unsupported("Invalid SCM string address"));
    }
    let length = ((end - address) / 2).min(32768);
    let chars = unsafe { std::slice::from_raw_parts(value.0, length) };
    let stop = chars
        .iter()
        .position(|c| *c == 0)
        .ok_or(unsupported("Unterminated SCM string"))?;
    String::from_utf16(&chars[..stop]).map_err(|_| unsupported("Invalid SCM string encoding"))
}

pub(super) fn snapshot() -> Result<Vec<ServiceMember>, GamingError> {
    let manager = manager()?;
    // SCM's documented maximum page; an incomplete page is never treated as complete membership.
    let mut aligned = vec![0usize; (256 * 1024) / size_of::<usize>()];
    let buffer =
        unsafe { std::slice::from_raw_parts_mut(aligned.as_mut_ptr().cast::<u8>(), 256 * 1024) };
    let mut needed = 0;
    let mut count = 0;
    let mut resume = 0;
    unsafe {
        EnumServicesStatusExW(
            manager.0,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_ACTIVE,
            Some(buffer),
            &mut needed,
            &mut count,
            Some(&mut resume),
            PCWSTR::null(),
        )
    }
    .map_err(|e| native("EnumServicesStatusExW background", e))?;
    if resume != 0 || count as usize > buffer.len() / size_of::<ENUM_SERVICE_STATUS_PROCESSW>() {
        return Err(unsupported("Incomplete SCM membership inventory"));
    }
    let mut members = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let entry = unsafe {
            ptr::read_unaligned(
                buffer
                    .as_ptr()
                    .add(index * size_of::<ENUM_SERVICE_STATUS_PROCESSW>())
                    .cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
            )
        };
        members.push(ServiceMember {
            name: bounded_string(entry.lpServiceName, buffer)?,
            pid: entry.ServiceStatusProcess.dwProcessId,
            running: entry.ServiceStatusProcess.dwCurrentState == SERVICE_RUNNING,
        });
    }
    Ok(members)
}

pub(super) fn verify(
    role: Role,
    process: &ProcessIdentity,
    trust: &mut TrustCache,
) -> Result<RoleEvidence, GamingError> {
    let name = role.service().ok_or(unsupported("Service role absent"))?;
    let manager = manager()?;
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let service = ServiceHandle(
        unsafe {
            OpenServiceW(
                manager.0,
                PCWSTR(wide.as_ptr()),
                SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG,
            )
        }
        .map_err(|e| native("OpenService background", e))?,
    );
    let mut status = SERVICE_STATUS_PROCESS::default();
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
            size_of::<SERVICE_STATUS_PROCESS>(),
        )
    };
    let mut needed = 0;
    unsafe { QueryServiceStatusEx(service.0, SC_STATUS_PROCESS_INFO, Some(bytes), &mut needed) }
        .map_err(|e| native("QueryServiceStatusEx background", e))?;
    if status.dwCurrentState != SERVICE_RUNNING || status.dwProcessId != process.pid {
        return Err(GamingError::Changed);
    }
    let mut aligned = vec![0usize; 8192 / size_of::<usize>()];
    let buffer = unsafe { std::slice::from_raw_parts_mut(aligned.as_mut_ptr().cast::<u8>(), 8192) };
    unsafe {
        QueryServiceConfigW(
            service.0,
            Some(buffer.as_mut_ptr().cast()),
            buffer.len() as u32,
            &mut needed,
        )
    }
    .map_err(|e| native("QueryServiceConfigW background", e))?;
    let config = unsafe { ptr::read_unaligned(buffer.as_ptr().cast::<QUERY_SERVICE_CONFIGW>()) };
    let account = bounded_string(config.lpServiceStartName, buffer)?;
    let binary = bounded_string(config.lpBinaryPathName, buffer)?;
    let system = directory(false)?;
    let expected = system.join(if role == Role::DiagnosticsTelemetry {
        "svchost.exe"
    } else {
        "SearchIndexer.exe"
    });
    let expected_text = expected.to_string_lossy();
    let (binary_path, arguments) = if let Some(rest) = binary.strip_prefix('"') {
        rest.split_once('"')
    } else {
        binary
            .split_once(char::is_whitespace)
            .or(Some((binary.as_str(), "")))
    }
    .ok_or(unsupported("Invalid service binary configuration"))?;
    let arguments = arguments
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    let known_arguments = match role {
        Role::DiagnosticsTelemetry => matches!(
            arguments.as_str(),
            "-k utcsvc -p" | "-k utcsvc -p -s diagtrack"
        ),
        Role::SearchIndexer => matches!(arguments.as_str(), "" | "/embedding"),
        _ => false,
    };
    if !binary_path.eq_ignore_ascii_case(&expected_text)
        || !account.eq_ignore_ascii_case("LocalSystem")
    {
        return Err(unsupported(
            "Unexpected service account or binary configuration",
        ));
    }
    if !known_arguments || !matches!(config.dwServiceType.0, 0x10 | 0x20) {
        return Err(unsupported("Unqualified service host arguments or type"));
    }
    let facts = OS::tuning_process_identity(process.pid)
        .map_err(|_| unsupported("Cannot verify service token"))?;
    if facts.creation_time() != process.creation_token
        || facts.image_path() != process.image_path
        || facts.token().sid() != "S-1-5-18"
        || facts.token().session_id() != 0
    {
        return Err(unsupported(
            "Service is not the exact LocalSystem Session0 instance",
        ));
    }
    let (dll_path, dll_identity) = if role == Role::DiagnosticsTelemetry {
        use winreg::{
            RegKey,
            enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY},
        };
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(
                r"SYSTEM\CurrentControlSet\Services\DiagTrack\Parameters",
                KEY_READ | KEY_WOW64_64KEY,
            )
            .map_err(|_| unsupported("Cannot verify DiagTrack DLL configuration"))?;
        let dll: String = key
            .get_value("ServiceDll")
            .map_err(|_| unsupported("Cannot read DiagTrack DLL"))?;
        let expected = system.join("diagtrack.dll");
        let expanded = if dll.to_ascii_lowercase().starts_with(r"%systemroot%\") {
            directory(true)?.join(&dll[13..])
        } else {
            std::path::PathBuf::from(&dll)
        };
        if !expanded
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
        {
            return Err(unsupported("Unexpected DiagTrack service DLL"));
        }
        let identity = trust.verify(&expected)?;
        (
            Some(expected.to_string_lossy().into_owned()),
            Some(identity),
        )
    } else {
        (None, None)
    };
    // Enumerated membership is re-read independently before a mutation by its caller.
    Ok(RoleEvidence::Service {
        name: name.into(),
        service_type: config.dwServiceType.0,
        arguments,
        dll_path,
        dll_identity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scm_strings_must_terminate_inside_the_returned_native_buffer() {
        let mut buffer = [65u16, 66, 0, 67];
        let native = PWSTR(buffer.as_mut_ptr());
        let bytes =
            unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 2) };
        assert_eq!(bounded_string(native, bytes).unwrap(), "AB");
        assert!(bounded_string(PWSTR::null(), bytes).is_err());
        assert!(bounded_string(PWSTR((buffer.as_ptr() as usize + 1) as _), bytes).is_err());
        assert!(bounded_string(PWSTR(unsafe { buffer.as_ptr().add(3) } as _), bytes).is_err());
        assert!(bounded_string(native, &bytes[..4]).is_err());
    }

    #[test]
    #[ignore = "explicit read-only Windows SCM inventory acceptance"]
    fn native_inventory_is_bounded_and_service_membership_is_complete() {
        let members = snapshot().unwrap();
        assert!(!members.is_empty());
        assert!(members.len() < 8192);
        assert!(
            members
                .iter()
                .all(|member| !member.name.is_empty() && member.name.len() < 256)
        );
        let rows = super::super::Resolver::default().inspect().unwrap();
        assert!(rows.len() <= MAX_TARGETS);
        for row in rows {
            eprintln!(
                "{:?} pid={} verified={} reason={}",
                row.role, row.pid, row.verified, row.reason
            );
        }
    }
}
