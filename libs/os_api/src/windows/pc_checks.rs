//! Fixed read-only queries. No WMI, child processes, sensors or traffic capture.

use crate::pc_checks::{
    MAX_FIRMWARE_BYTES, MemorySnapshot, PcCheckError, PcSnapshot, RamConfiguration,
    commit_from_pages, parse_ram_firmware,
};
use windows::Win32::System::{
    ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION},
    SystemInformation::{GetSystemFirmwareTable, GlobalMemoryStatusEx, MEMORYSTATUSEX, RSMB},
};

fn native_error(operation: &'static str, error: windows::core::Error) -> PcCheckError {
    PcCheckError::Native {
        operation,
        code: error.code().0,
    }
}

fn memory_snapshot() -> Result<MemorySnapshot, PcCheckError> {
    let mut memory = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut memory) }
        .map_err(|error| native_error("GlobalMemoryStatusEx", error))?;
    if memory.ullTotalPhys == 0 || memory.ullAvailPhys > memory.ullTotalPhys {
        return Err(PcCheckError::InvalidData {
            source: "Physical memory counters",
        });
    }
    // GetPerformanceInfo reports system commit, rather than interpreting
    // GlobalMemoryStatusEx's per-process page-file availability as system use.
    let mut performance = PERFORMANCE_INFORMATION {
        cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    let commit = unsafe {
        GetPerformanceInfo(
            &mut performance,
            size_of::<PERFORMANCE_INFORMATION>() as u32,
        )
    }
    .map_err(|error| native_error("GetPerformanceInfo", error))
    .and_then(|()| {
        commit_from_pages(
            performance.CommitTotal as u64,
            performance.CommitLimit as u64,
            performance.CommitPeak as u64,
            performance.PageSize as u64,
        )
    });
    Ok(MemorySnapshot {
        physical_total_bytes: memory.ullTotalPhys,
        physical_available_bytes: memory.ullAvailPhys,
        commit,
    })
}

fn ram_configuration() -> Result<RamConfiguration, PcCheckError> {
    let required = unsafe { GetSystemFirmwareTable(RSMB, 0, None) } as usize;
    if required == 0 {
        return Err(native_error(
            "GetSystemFirmwareTable",
            windows::core::Error::from_thread(),
        ));
    }
    if required > MAX_FIRMWARE_BYTES {
        return Err(PcCheckError::Limit {
            source: "SMBIOS RAM configuration",
        });
    }
    let mut raw = vec![0_u8; required];
    let completed = unsafe { GetSystemFirmwareTable(RSMB, 0, Some(&mut raw)) } as usize;
    if completed == 0 {
        return Err(native_error(
            "GetSystemFirmwareTable",
            windows::core::Error::from_thread(),
        ));
    }
    // Do not repeatedly allocate if firmware size changes during the query.
    if completed != required {
        return Err(PcCheckError::InvalidData {
            source: "SMBIOS RAM configuration",
        });
    }
    parse_ram_firmware(&raw)
}

pub(crate) fn collect_snapshot() -> PcSnapshot {
    let cpu = super::OS::get_cpu_model();
    let cpu_model =
        (!cpu.is_empty() && cpu != "Unknown CPU").then(|| cpu.chars().take(160).collect());
    let graphics_adapters = crate::gaming_diagnostics::display_inventory()
        .map(|displays| {
            let mut names: Vec<String> = Vec::new();
            for display in displays {
                let name: String = display.adapter_name.trim().chars().take(160).collect();
                if !name.is_empty()
                    && !names
                        .iter()
                        .any(|existing| existing.eq_ignore_ascii_case(&name))
                {
                    names.push(name);
                }
            }
            names
        })
        .map_err(|error| match error {
            crate::gaming_diagnostics::DiagnosticError::Unsupported => PcCheckError::Unsupported,
            crate::gaming_diagnostics::DiagnosticError::Native { operation, code } => {
                PcCheckError::Native {
                    operation,
                    code: code as i32,
                }
            }
            crate::gaming_diagnostics::DiagnosticError::Limit { operation } => {
                PcCheckError::Limit { source: operation }
            }
        });
    PcSnapshot {
        memory: memory_snapshot(),
        ram: ram_configuration(),
        cpu_model,
        graphics_adapters,
    }
}
