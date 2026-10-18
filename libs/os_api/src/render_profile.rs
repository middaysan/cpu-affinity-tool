//! Read-only clocks for the explicitly enabled nonshipping render profiler.
pub fn qpc_clock() -> Option<(i64, i64)> {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let (mut counter, mut frequency) = (0, 0);
    unsafe { QueryPerformanceFrequency(&mut frequency) }.ok()?;
    unsafe { QueryPerformanceCounter(&mut counter) }.ok()?;
    Some((counter, frequency))
}
#[derive(Clone, Copy)]
pub struct ThreadClock {
    pub thread: u32,
    pub cycles: u64,
}
pub fn thread_clock() -> Option<ThreadClock> {
    use windows::Win32::System::Threading::{GetCurrentThread, GetCurrentThreadId};
    use windows::Win32::System::WindowsProgramming::QueryThreadCycleTime;
    let mut cycles = 0;
    unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) }.ok()?;
    Some(ThreadClock {
        thread: unsafe { GetCurrentThreadId() },
        cycles,
    })
}
