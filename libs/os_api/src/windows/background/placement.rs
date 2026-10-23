//! Whole-core topology and conditional default CPU Sets. No hard affinity writes.
use super::super::common::HandleGuard;
use super::{native, unsupported};
use crate::background::{PlacementWitness, TargetProof};
use crate::{
    background::{PlacementCpu, topology::*},
    gaming::{GamingError, ProcessIdentity},
};
use windows::Win32::{
    Foundation::ERROR_NO_MORE_FILES,
    System::{Diagnostics::ToolHelp::*, Threading::*},
};
use windows::Win32::{
    Foundation::{ERROR_INSUFFICIENT_BUFFER, HANDLE},
    System::SystemInformation::{
        GetLogicalProcessorInformationEx, GetSystemCpuSetInformation, RelationProcessorCore,
    },
};

fn buffer(
    mut query: impl FnMut(Option<*mut u8>, &mut u32) -> windows::core::Result<()>,
) -> Result<Vec<u8>, GamingError> {
    for _ in 0..3 {
        let mut length = 0;
        let result = query(None, &mut length);
        if result
            .as_ref()
            .is_err_and(|error| error.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult())
        {
            return Err(native("CPU placement topology size", result.unwrap_err()));
        }
        if length == 0 || length > 1024 * 1024 {
            return Err(unsupported("CPU placement topology bound"));
        }
        let capacity = length;
        let mut aligned = vec![0u64; (length as usize).div_ceil(8)];
        let pointer = aligned.as_mut_ptr().cast::<u8>();
        if let Err(error) = query(Some(pointer), &mut length) {
            if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() {
                continue;
            }
            return Err(native("CPU placement topology", error));
        }
        if length == 0 || length > capacity {
            return Err(GamingError::Changed);
        }
        return Ok(unsafe { std::slice::from_raw_parts(pointer, length as usize) }.to_vec());
    }
    Err(GamingError::Changed)
}
fn core_bytes() -> Result<Vec<u8>, GamingError> {
    buffer(|pointer, length| unsafe {
        GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            pointer.map(|pointer| pointer.cast()),
            length,
        )
    })
}
pub(super) fn topology_on_handle(handle: Option<HANDLE>) -> Result<Vec<PlacementCpu>, GamingError> {
    if !cfg!(target_arch = "x86_64") {
        return Err(unsupported("CPU placement requires qualified x64 topology"));
    }
    let before = core_bytes()?;
    let sets = buffer(|pointer, length| unsafe {
        GetSystemCpuSetInformation(
            pointer.map(|pointer| pointer.cast()),
            *length,
            length,
            handle,
            Some(0),
        )
        .ok()
    })?;
    let after = core_bytes()?;
    if before != after {
        return Err(GamingError::Changed);
    }
    let cores =
        decode_cores(&before).map_err(|_| unsupported("Unverified physical-core topology"))?;
    let sets = decode_sets(&sets).map_err(|_| unsupported("Unverified CPU Set topology"))?;
    join(&sets, &cores).map_err(|_| unsupported("Physical cores and CPU Sets disagree"))
}
pub(crate) fn placement_topology(
    process: Option<&ProcessIdentity>,
) -> Result<Vec<PlacementCpu>, GamingError> {
    match process {
        Some(process) => super::super::gaming::with_process(process, false, |handle| {
            topology_on_handle(Some(handle))
        }),
        None => topology_on_handle(None),
    }
}
pub(super) fn sets(handle: HANDLE) -> Result<Vec<u32>, GamingError> {
    for _ in 0..3 {
        let mut count = 0;
        unsafe { GetProcessDefaultCpuSets(handle, None, &mut count).ok() }
            .or_else(|error| {
                if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|error| native("GetProcessDefaultCpuSets size", error))?;
        if count > 16384 {
            return Err(unsupported("Default CPU Set bound"));
        }
        if count == 0 {
            return Ok(vec![]);
        }
        let mut ids = vec![0u32; count as usize];
        if let Err(error) =
            unsafe { GetProcessDefaultCpuSets(handle, Some(&mut ids), &mut count).ok() }
        {
            if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() {
                continue;
            }
            return Err(native("GetProcessDefaultCpuSets", error));
        }
        if count as usize > ids.len() {
            return Err(GamingError::Changed);
        }
        ids.truncate(count as usize);
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(GamingError::Changed);
        }
        return Ok(ids);
    }
    Err(GamingError::Changed)
}
fn restrictions(handle: HANDLE, pid: u32, witness: &PlacementWitness) -> Result<(), GamingError> {
    let mut groups = [0u16; 256];
    let mut count = groups.len() as u16;
    unsafe { GetProcessGroupAffinity(handle, &mut count, groups.as_mut_ptr()).ok() }
        .map_err(|error| native("GetProcessGroupAffinity background", error))?;
    if count != 1 {
        return Err(unsupported(
            "Multi-group per-thread affinity is not qualified for background placement",
        ));
    }
    let mut mask = 0usize;
    let mut system = 0usize;
    unsafe { GetProcessAffinityMask(handle, &mut mask, &mut system) }
        .map_err(|error| native("GetProcessAffinityMask background", error))?;
    if mask == 0
        || witness.desired.iter().any(|id| {
            !witness.topology.iter().any(|cpu| {
                cpu.id == *id && cpu.group == groups[0] && mask & (1usize << cpu.logical) != 0
            })
        })
    {
        return Err(unsupported(
            "Background pool conflicts with existing hard affinity",
        ));
    }
    let snapshot = HandleGuard(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
            .map_err(|error| native("Background thread snapshot", error))?,
    );
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    unsafe { Thread32First(snapshot.0, &mut entry) }
        .map_err(|error| native("Thread32First background", error))?;
    let mut scanned = 0;
    let mut members = 0;
    loop {
        scanned += 1;
        if scanned > 65536 {
            return Err(unsupported("Background thread inventory bound"));
        }
        if entry.th32OwnerProcessID == pid {
            members += 1;
            if members > 4096 {
                return Err(unsupported("Background target thread bound"));
            }
            let thread = HandleGuard(
                unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, entry.th32ThreadID) }
                    .map_err(|error| native("OpenThread background query", error))?,
            );
            if unsafe { GetProcessIdOfThread(thread.0) } != pid {
                return Err(GamingError::Changed);
            }
            let mut selected = 0;
            unsafe { GetThreadSelectedCpuSets(thread.0, None, &mut selected).ok() }
                .or_else(|error| {
                    if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })
                .map_err(|error| native("GetThreadSelectedCpuSets background", error))?;
            if selected != 0 {
                return Err(unsupported(
                    "Thread-selected CPU Sets take precedence; background placement skipped",
                ));
            }
            let mut affinity = windows::Win32::System::SystemInformation::GROUP_AFFINITY::default();
            unsafe { GetThreadGroupAffinity(thread.0, &mut affinity).ok() }
                .map_err(|error| native("GetThreadGroupAffinity background", error))?;
            if affinity.Group != groups[0] || affinity.Mask != mask {
                return Err(unsupported(
                    "Per-thread hard affinity differs; background placement skipped",
                ));
            }
        }
        match unsafe { Thread32Next(snapshot.0, &mut entry) } {
            Ok(()) => {}
            Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
            Err(error) => return Err(native("Thread32Next background", error)),
        }
    }
    if members == 0 {
        return Err(GamingError::Changed);
    }
    Ok(())
}
pub(crate) fn validate_placement(
    process: &ProcessIdentity,
    witness: &PlacementWitness,
) -> Result<(), GamingError> {
    super::super::gaming::with_process(process, false, |handle| {
        if !witness.valid() {
            return Err(GamingError::InvalidValue);
        }
        if topology_on_handle(Some(handle))? != witness.topology
            || sets(handle)? != witness.baseline
        {
            return Err(GamingError::Changed);
        }
        restrictions(handle, process.pid, witness)
    })
}
// Production reaches this helper only through the independently qualified role
// gate. Native tests call it with their own retained child handle.
pub(super) fn write_sets_on_handle(
    handle: HANDLE,
    pid: u32,
    expected: &[u32],
    desired: &[u32],
    witness: &PlacementWitness,
) -> Result<(), GamingError> {
    if !witness.permits(expected, desired) {
        return Err(GamingError::InvalidValue);
    }
    if topology_on_handle(Some(handle))? != witness.topology || sets(handle)? != expected {
        return Err(GamingError::Changed);
    }
    let applying = desired == witness.desired;
    if applying {
        restrictions(handle, pid, witness)?;
    }
    unsafe {
        SetProcessDefaultCpuSets(
            handle,
            if desired.is_empty() {
                None
            } else {
                Some(desired)
            },
        )
        .ok()
    }
    .map_err(|error| native("SetProcessDefaultCpuSets background", error))?;
    if sets(handle)? != desired || topology_on_handle(Some(handle))? != witness.topology {
        return Err(GamingError::Changed);
    }
    if applying {
        restrictions(handle, pid, witness)?;
    }
    Ok(())
}
impl super::Resolver {
    pub(crate) fn write_cpu_sets_bound(
        &mut self,
        process: &ProcessIdentity,
        expected: &[u32],
        desired: &[u32],
        proof: &TargetProof,
        placement: &PlacementWitness,
    ) -> Result<(), GamingError> {
        self.with_admitted(process, Some(proof), |handle, _| {
            write_sets_on_handle(handle, process.pid, expected, desired, placement)
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "explicit read-only joined GLPI/CPU-Set topology probe"]
    fn native_joined_background_topology() {
        let topology = super::placement_topology(None).unwrap();
        assert!(!topology.is_empty());
        assert_eq!(
            topology.len(),
            crate::gaming::cpu_topology(None).unwrap().len()
        );
        eprintln!(
            "Joined topology: {} logical CPUs, {} physical cores, {} groups",
            topology.len(),
            topology
                .iter()
                .map(|cpu| (cpu.group, cpu.physical_core))
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            topology
                .iter()
                .map(|cpu| cpu.group)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );
        if topology
            .iter()
            .map(|cpu| cpu.efficiency_class)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1
        {
            assert!(
                !crate::background::efficient_pool(&topology)
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
