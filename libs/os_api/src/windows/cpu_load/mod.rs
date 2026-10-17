mod accounting;
mod contributors;
mod timeline;
mod trace;

use std::collections::HashMap;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::Instant;

use windows::Win32::Foundation::{FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
    PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
    PdhGetFormattedCounterValue, PdhOpenQueryW, QueryPerformanceCounter, QueryPerformanceFrequency,
};
use windows::Win32::System::Threading::{
    GetActiveProcessorCount, GetActiveProcessorGroupCount, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::PCWSTR;

use accounting::{process_percent, processor_coordinates};
pub use contributors::{CpuContribution, CpuProcess};
pub use timeline::{
    CpuHistoryContribution, CpuHistoryFrame, CpuHistoryGap, CpuHistoryTarget, CpuHistoryValue,
    history_buffer_bytes,
};
pub use trace::{CpuHistoryTick, DetailedTrace, ProcessCpuSample};

pub fn logical_cpu_count() -> usize {
    unsafe { GetActiveProcessorCount(u16::MAX) as usize }
}

pub fn history_clock() -> Result<(f64, std::time::SystemTime), String> {
    let mut frequency = 0;
    let mut at = 0;
    unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(|e| e.to_string())?;
    unsafe { QueryPerformanceCounter(&mut at) }.map_err(|e| e.to_string())?;
    if frequency <= 0 {
        return Err("CPU event clock is unavailable".into());
    }
    Ok((at as f64 / frequency as f64, std::time::SystemTime::now()))
}

/// Read-only self-measurement for explicit telemetry overhead probes.
pub struct ResourceSample {
    pub cpu_ticks: u64,
    pub cycles: u64,
    pub private_bytes: usize,
    pub working_set: usize,
}
pub fn resource_sample() -> Result<ResourceSample, String> {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    let handle = unsafe { GetCurrentProcess() };
    let mut created = FILETIME::default();
    let mut exited = created;
    let mut kernel = created;
    let mut user = created;
    unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|e| e.to_string())?;
    let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
    memory.cb = std::mem::size_of_val(&memory) as u32;
    let mut cycles = 0;
    unsafe {
        windows::Win32::System::WindowsProgramming::QueryProcessCycleTime(handle, &mut cycles)
    }
    .map_err(|e| e.to_string())?;
    unsafe {
        GetProcessMemoryInfo(
            handle,
            (&raw mut memory).cast::<PROCESS_MEMORY_COUNTERS>(),
            std::mem::size_of_val(&memory) as u32,
        )
    }
    .map_err(|e| e.to_string())?;
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Ok(ResourceSample {
        cpu_ticks: ticks(kernel) + ticks(user),
        cycles,
        private_bytes: memory.PrivateUsage,
        working_set: memory.WorkingSetSize,
    })
}

fn coordinates() -> Result<Vec<(u16, u32)>, String> {
    let groups = unsafe { GetActiveProcessorGroupCount() };
    let counts: Vec<_> = (0..groups)
        .map(|group| unsafe { GetActiveProcessorCount(group) })
        .collect();
    processor_coordinates(&counts)
}

fn status(operation: &str, result: u32) -> Result<(), String> {
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "{operation} failed (Windows status 0x{result:08X})"
        ))
    }
}

/// Read the Windows Parked flag; zero utilization is not parking evidence.
pub fn parking_snapshot() -> Result<Vec<Option<bool>>, String> {
    let sets = crate::gaming::cpu_topology(None).map_err(|e| format!("Read CPU parking: {e:?}"))?;
    Ok(parking_by_cpu(&coordinates()?, &sets))
}

fn parking_by_cpu(coordinates: &[(u16, u32)], sets: &[crate::gaming::CpuSet]) -> Vec<Option<bool>> {
    let mut flags = HashMap::new();
    for cpu in sets {
        flags
            .entry((cpu.group, u32::from(cpu.logical_processor)))
            .and_modify(|state| {
                if *state != Some(cpu.parked) {
                    *state = None;
                }
            })
            .or_insert(Some(cpu.parked));
    }
    coordinates
        .iter()
        .map(|key| flags.get(key).copied().flatten())
        .collect()
}

fn busy_from_idle(result: u32, counter_status: u32, idle: f64) -> Option<f32> {
    if result != 0
        || !matches!(
            counter_status,
            PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
        )
        || !idle.is_finite()
        || !(0.0..=100.0).contains(&idle)
    {
        return None;
    }
    Some((100.0 - idle) as f32)
}

/// One language-neutral PDH query for all logical CPUs. Poll once per second,
/// and reuse the result for every affinity group. Values measure total busy
/// time, not frequency-scaled processor utility. The first sample is missing.
pub struct CpuSampler {
    query: PDH_HQUERY,
    counters: Vec<PDH_HCOUNTER>,
    coordinates: Vec<(u16, u32)>,
    primed: bool,
}

impl CpuSampler {
    pub fn new() -> Result<Self, String> {
        let coordinates = coordinates()?;
        let mut query = PDH_HQUERY::default();
        status("PdhOpenQueryW", unsafe {
            PdhOpenQueryW(PCWSTR::null(), 0, &mut query)
        })?;
        let mut sampler = Self {
            query,
            counters: Vec::with_capacity(coordinates.len()),
            coordinates,
            primed: false,
        };
        for &(group, cpu) in &sampler.coordinates {
            // Read idle time and its documented complement. The inverse busy
            // counter can reject near-idle samples with PDH arithmetic errors.
            // PDH's default 100% cap bounds idle timer rounding at full idle.
            let path: Vec<u16> = format!(r"\Processor Information({group},{cpu})\% Idle Time")
                .encode_utf16()
                .chain([0])
                .collect();
            let mut counter = PDH_HCOUNTER::default();
            status("PdhAddEnglishCounterW", unsafe {
                PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter)
            })?;
            sampler.counters.push(counter);
        }
        Ok(sampler)
    }

    pub fn sample(&mut self) -> Result<Vec<Option<f32>>, String> {
        if coordinates()? != self.coordinates {
            return Err("Windows CPU topology changed; restart CPU monitoring".into());
        }
        status("PdhCollectQueryData", unsafe {
            PdhCollectQueryData(self.query)
        })?;
        if !std::mem::replace(&mut self.primed, true) {
            return Ok(vec![None; self.counters.len()]);
        }
        Ok(self
            .counters
            .iter()
            .map(|&counter| {
                let mut value = PDH_FMT_COUNTERVALUE::default();
                let result = unsafe {
                    PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut value)
                };
                // PDH_FMT_DOUBLE selects this documented union member.
                busy_from_idle(result, value.CStatus, unsafe {
                    value.Anonymous.doubleValue
                })
            })
            .collect())
    }
}

impl Drop for CpuSampler {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

struct PinnedProcess {
    pid: u32,
    handle: OwnedHandle,
    verified_at: i64,
}

impl PinnedProcess {
    fn open(pid: u32, creation_token: u64) -> Result<Self, String> {
        if pid == 0 || creation_token == 0 {
            return Err("CPU monitoring requires an exact process instance".into());
        }
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        }
        .map_err(|error| format!("OpenProcess for CPU monitoring PID {pid}: {error}"))?;
        // OpenProcess returned a newly owned handle; OwnedHandle closes it.
        let mut process = Self {
            pid,
            handle: unsafe { OwnedHandle::from_raw_handle(handle.0) },
            verified_at: 0,
        };
        let (created, _) = process.times()?;
        if created != creation_token {
            return Err(format!(
                "PID {pid} no longer identifies the selected process"
            ));
        }
        // Capture the ETW clock after token verification and before the alive
        // check, so this proof cannot predate a prior incarnation's exit.
        unsafe { QueryPerformanceCounter(&mut process.verified_at) }.map_err(|e| e.to_string())?;
        if !process.alive()? {
            return Err(format!("Selected process PID {pid} has exited"));
        }
        Ok(process)
    }

    fn native(&self) -> HANDLE {
        HANDLE(self.handle.as_raw_handle())
    }

    fn alive(&self) -> Result<bool, String> {
        match unsafe { WaitForSingleObject(self.native(), 0) } {
            WAIT_TIMEOUT => Ok(true),
            WAIT_OBJECT_0 => Ok(false),
            _ => Err(format!(
                "Process lifetime check failed for PID {}: {}",
                self.pid,
                windows::core::Error::from_thread()
            )),
        }
    }

    fn times(&self) -> Result<(u64, u64), String> {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                self.native(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(|error| format!("GetProcessTimes for PID {}: {error}", self.pid))?;
        let ticks =
            |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
        let total = ticks(kernel)
            .checked_add(ticks(user))
            .ok_or("Process CPU time overflow")?;
        Ok((ticks(created), total))
    }
}

/// Aggregate CPU time for one retained, verified PID/creation-token instance.
/// The percentage is normalized over all active logical CPUs (whole machine),
/// so one saturated thread on a 20-thread host contributes about five percent.
pub struct ProcessSampler {
    process: PinnedProcess,
    cpus: usize,
    previous: Option<(Instant, u64)>,
}

impl ProcessSampler {
    pub fn open(pid: u32, creation_token: u64) -> Result<Self, String> {
        Ok(Self {
            process: PinnedProcess::open(pid, creation_token)?,
            cpus: coordinates()?.len(),
            previous: None,
        })
    }

    pub fn sample(&mut self) -> Result<Option<f32>, String> {
        if !self.process.alive()? {
            return Err(format!(
                "Selected process PID {} has exited",
                self.process.pid
            ));
        }
        if logical_cpu_count() != self.cpus {
            return Err("Windows CPU topology changed; restart CPU monitoring".into());
        }
        let (_, ticks) = self.process.times()?;
        let now = Instant::now();
        let previous = self.previous.replace((now, ticks));
        Ok(previous.and_then(|(previous_at, previous_ticks)| {
            process_percent(
                previous_ticks,
                ticks,
                now.duration_since(previous_at).as_nanos(),
                self.cpus,
            )
        }))
    }
}

#[cfg(test)]
mod idle_counter_tests {
    use super::*;
    use windows::Win32::System::Performance::{
        PDH_CALC_NEGATIVE_DENOMINATOR, PDH_CSTATUS_INVALID_DATA,
    };

    #[test]
    fn idle_measurement_produces_busy_time_and_keeps_real_failures_unknown() {
        assert_eq!(busy_from_idle(0, PDH_CSTATUS_VALID_DATA, 100.0), Some(0.0));
        assert_eq!(busy_from_idle(0, PDH_CSTATUS_NEW_DATA, 75.0), Some(25.0));
        assert_eq!(busy_from_idle(0, PDH_CSTATUS_VALID_DATA, 0.0), Some(100.0));
        assert_eq!(
            busy_from_idle(
                PDH_CALC_NEGATIVE_DENOMINATOR,
                PDH_CALC_NEGATIVE_DENOMINATOR,
                0.0
            ),
            None
        );
        assert_eq!(busy_from_idle(0, PDH_CSTATUS_INVALID_DATA, 0.0), None);
        assert_eq!(busy_from_idle(0, PDH_CSTATUS_VALID_DATA, f64::NAN), None);
        assert_eq!(busy_from_idle(0, PDH_CSTATUS_VALID_DATA, -1.0), None);
    }

    #[test]
    fn parking_maps_real_flags_by_processor_group_and_never_infers_from_idle_load() {
        use crate::gaming::CpuSet;
        let cpu = |group, index, parked| CpuSet {
            id: u32::from(index),
            group,
            logical_processor: index,
            core: index,
            efficiency_class: 0,
            parked,
            allocated: false,
            allocated_to_target: false,
        };
        let coordinates = vec![(0, 0), (0, 1), (1, 0), (1, 1)];
        let records = vec![cpu(1, 0, true), cpu(0, 0, false), cpu(0, 1, true)];
        assert_eq!(
            parking_by_cpu(&coordinates, &records),
            vec![Some(false), Some(true), Some(true), None]
        );
        let conflicting = vec![cpu(0, 0, true), cpu(0, 0, false), cpu(0, 0, true)];
        assert_eq!(
            parking_by_cpu(&coordinates, &conflicting),
            vec![None, None, None, None]
        );
    }

    #[test]
    #[ignore = "read-only real Windows PDH sampling; takes six seconds, no elevation"]
    fn native_idle_counter_retains_complete_idle_history() {
        let mut sampler = CpuSampler::new().unwrap();
        assert!(sampler.sample().unwrap().iter().all(Option::is_none));
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let values = sampler.sample().unwrap();
            let parking = parking_snapshot().unwrap();
            assert_eq!(values.len(), parking.len());
            assert!(
                parking.iter().all(Option::is_some),
                "missing CPU parking flags: {parking:?}"
            );
            println!(
                "CPU sample: valid={}/{}, parked={}",
                values.iter().flatten().count(),
                values.len(),
                parking.iter().filter(|v| **v == Some(true)).count()
            );
            assert!(!values.is_empty());
            assert!(
                values
                    .iter()
                    .all(|v| v.is_some_and(|v| (0.0..=100.0).contains(&v))),
                "missing post-warmup CPU values: {values:?}"
            );
        }
    }
}
