use std::path::PathBuf;
use std::process::{Command, Child, Stdio};
use std::os::windows::io::AsRawHandle;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, ShowWindow, SetForegroundWindow,
    IsWindowVisible,
    SW_RESTORE,
};

use ntapi::ntpsapi::{NtQueryInformationProcess, ProcessBasicInformation, PROCESS_BASIC_INFORMATION};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION,
    SetProcessAffinityMask, SetPriorityClass,
    IDLE_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
    NORMAL_PRIORITY_CLASS, ABOVE_NORMAL_PRIORITY_CLASS,
    HIGH_PRIORITY_CLASS, REALTIME_PRIORITY_CLASS,
};
use windows_sys::Win32::System::ProcessStatus::K32EnumProcesses;
use windows_sys::Win32::Foundation::HANDLE;

use parselnk::Lnk;
use shlex;

use crate::PriorityClass;

pub struct OS;

impl OS {
    // helper: map our PriorityClass to WinAPI constant
    fn to_win_priority(p: PriorityClass) -> u32 {
        match p {
            PriorityClass::Idle       => IDLE_PRIORITY_CLASS,
            PriorityClass::BelowNormal=> BELOW_NORMAL_PRIORITY_CLASS,
            PriorityClass::Normal     => NORMAL_PRIORITY_CLASS,
            PriorityClass::AboveNormal=> ABOVE_NORMAL_PRIORITY_CLASS,
            PriorityClass::High       => HIGH_PRIORITY_CLASS,
            PriorityClass::Realtime   => REALTIME_PRIORITY_CLASS,
        }
    }

    fn resolve_lnk(path: &PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        let link = Lnk::try_from(path.as_path())
            .map_err(|e| format!("parse LNK failed {:?}: {}", path, e))?;
        let target = link
            .link_info
            .local_base_path
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| {
                link.string_data.relative_path.as_ref().map(|rel| {
                    let rp = PathBuf::from(rel);
                    if rp.is_absolute() {
                        rp
                    } else {
                        link.string_data
                            .working_dir
                            .as_ref()
                            .map(|wd| PathBuf::from(wd).join(rp.clone()))
                            .unwrap_or(rp)
                    }
                })
            })
            .ok_or_else(|| format!("no target in LNK {:?}", path))?;
        let args = link.string_data.command_line_arguments.unwrap_or_default();
        let vec = shlex::split(&args).unwrap_or_else(|| vec![args]);
        Ok((target, vec))
    }

    fn spawn(target: &PathBuf, args: &[String]) -> Result<Child, String> {
        let mut cmd = Command::new(target);
        if !args.is_empty() { cmd.args(args); }
        cmd.stdout(Stdio::inherit())
           .stderr(Stdio::inherit())
           .spawn()
           .map_err(|e| format!("spawn {:?} failed: {}", target, e))
    }

    fn set_affinity(child: &Child, mask: usize) -> Result<(), String> {
        let h = child.as_raw_handle() as HANDLE;
        let ok = unsafe { SetProcessAffinityMask(h, mask) };
        if ok == 0 { Err("SetProcessAffinityMask failed".into()) } else { Ok(()) }
    }

    fn set_priority(child: &Child, p: PriorityClass) -> Result<(), String> {
        let h = child.as_raw_handle() as HANDLE;
        let ok = unsafe { SetPriorityClass(h, Self::to_win_priority(p)) };
        if ok == 0 { Err("SetPriorityClass failed".into()) } else { Ok(()) }
    }

    #[allow(dead_code)]
    fn get_parent_pid(pid: u32) -> Option<u32> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
            if h.is_null() { return None; }
            let mut info: PROCESS_BASIC_INFORMATION = zeroed();
            let status = NtQueryInformationProcess(
                h as _,
                ProcessBasicInformation,
                &mut info as *mut _ as *mut _,
                size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                null_mut(),
            );
            if status < 0 { None } else { Some(info.InheritedFromUniqueProcessId as u32) }
        }
    }

    #[allow(dead_code)]
    fn get_all_pids() -> Vec<u32> {
        let mut buf = vec![0u32; 1024];
        let mut ret = 0;
        unsafe {
            if K32EnumProcesses(buf.as_mut_ptr(), (buf.len() * 4) as u32, &mut ret) == 0 {
                panic!("K32EnumProcesses failed");
            }
        }
        let cnt = ret as usize / 4;
        buf.truncate(cnt);
        buf
    }

    #[allow(dead_code)]
    fn find_child_pids(parent: u32) -> Vec<u32> {
        Self::get_all_pids()
            .into_iter()
            .filter(|&pid| Self::get_parent_pid(pid) == Some(parent))
            .collect()
    }

    #[allow(dead_code)]
    pub fn find_all_descendants(parent_pid: u32, descendants: &mut Vec<u32>) {
        for &child in &Self::find_child_pids(parent_pid) {
            if !descendants.contains(&child) {
                descendants.push(child);
                Self::find_all_descendants(child, descendants);
            }
        }
    }

    pub fn parse_dropped_file(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        if file_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("lnk"))
            .unwrap_or(false)
        {
            Self::resolve_lnk(&file_path)
        } else {
            Ok((file_path, Vec::new()))
        }
    }

    pub fn is_pid_live(pid: u32) -> bool {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
    
            use windows_sys::Win32::System::Threading::GetExitCodeProcess;
            use windows_sys::Win32::Foundation::STILL_ACTIVE;
            let mut exit_code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut exit_code as *mut u32);
            windows_sys::Win32::Foundation::CloseHandle(handle);
    
            if ok == 0 {
                false
            } else {
                exit_code == STILL_ACTIVE as u32
            }
        }
    }

    pub fn focus_window_by_pid(pid: u32) -> bool {
        use std::sync::atomic::{AtomicU32, Ordering};
    
        static mut FOUND_HWND: windows_sys::Win32::Foundation::HWND = 0 as windows_sys::Win32::Foundation::HWND;
        static TARGET_PID: AtomicU32 = AtomicU32::new(0);
    
        unsafe extern "system" fn enum_windows_proc(hwnd: windows_sys::Win32::Foundation::HWND, _: isize) -> i32 {
            let mut window_pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, &mut window_pid); }
    
            if window_pid == TARGET_PID.load(Ordering::Relaxed) {
                if unsafe { IsWindowVisible(hwnd) } != 0 {
                    unsafe { FOUND_HWND = hwnd; }
                    return 0;
                }
            }
            1
        }
    
        unsafe {
            TARGET_PID.store(pid, Ordering::Relaxed);
            FOUND_HWND = 0 as windows_sys::Win32::Foundation::HWND;
    
            EnumWindows(Some(enum_windows_proc), 0);
    
            if FOUND_HWND != 0 as windows_sys::Win32::Foundation::HWND {
                ShowWindow(FOUND_HWND, SW_RESTORE);
                SetForegroundWindow(FOUND_HWND);
                true
            } else {
                false
            }
        }
    }

    pub fn run(
        file_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
    ) -> Result<u32, String> {
        let mask = cores.iter().fold(0usize, |acc, &i| acc | (1 << i));
        let child = Self::spawn(&file_path, &args)?;
        Self::set_affinity(&child, mask)?;
        Self::set_priority(&child, priority)?;
        Ok(child.id())
    }
}