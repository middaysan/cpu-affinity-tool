//! Read-only CPU telemetry. Native collectors are Windows-only and must be owned
//! and polled by a background worker, including detailed trace destruction.

#[cfg(target_os = "windows")]
pub use crate::windows::cpu_load::{
    CpuContribution, CpuHistoryContribution, CpuHistoryFrame, CpuHistoryGap, CpuHistoryTarget,
    CpuHistoryTick, CpuHistoryValue, CpuProcess, CpuSampler, DetailedTrace, ProcessCpuSample,
    ProcessSampler, ResourceSample, history_buffer_bytes, history_clock, logical_cpu_count,
    parking_snapshot, resource_sample,
};

/// Format a sample's wall time in the Windows user's local time zone.
/// This converts a timestamp only; it does not query any CPU collector.
#[cfg(target_os = "windows")]
pub fn sample_clock_label(timestamp: std::time::SystemTime) -> String {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

    // FILETIME counts 100 ns intervals since 1601, SystemTime since 1970.
    let ticks = timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_nanos() / 100).ok())
        .and_then(|ticks| ticks.checked_add(116_444_736_000_000_000));
    let Some(ticks) = ticks else {
        return "—".into();
    };
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // Both output buffers are owned here; None requests the current Windows zone.
    if unsafe { FileTimeToSystemTime(&file_time, &mut utc) }.is_err() {
        return "—".into();
    }
    if unsafe { SystemTimeToTzSpecificLocalTime(None, &utc, &mut local) }.is_err() {
        return format!("{:02}:{:02}:{:02} UTC", utc.wHour, utc.wMinute, utc.wSecond);
    }
    format!(
        "{:02}:{:02}:{:02}",
        local.wHour, local.wMinute, local.wSecond
    )
}

#[cfg(all(test, target_os = "windows"))]
mod time_tests {
    #[test]
    fn sample_time_is_a_local_clock_and_rejects_out_of_range_values() {
        let label = super::sample_clock_label(std::time::SystemTime::now());
        assert_eq!(label.len(), 8);
        assert_eq!(&label[2..3], ":");
        assert_eq!(&label[5..6], ":");
        assert_eq!(
            super::sample_clock_label(std::time::UNIX_EPOCH - std::time::Duration::from_secs(1)),
            "—"
        );
    }
}
