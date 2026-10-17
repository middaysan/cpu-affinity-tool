//! Read-only clocks for the explicitly enabled nonshipping render profiler.
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
