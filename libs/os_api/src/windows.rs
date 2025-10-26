use std::fs;
use std::mem::{size_of, zeroed};
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::ptr::null_mut;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
    ShowWindow,
};

use winreg::RegKey;
use winreg::enums::*;

use ntapi::ntpsapi::{
    NtQueryInformationProcess, PROCESS_BASIC_INFORMATION, ProcessBasicInformation,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, STILL_ACTIVE};
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::System::ProcessStatus::K32EnumProcesses;
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetExitCodeProcess, GetPriorityClass,
    GetProcessAffinityMask, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    OpenProcess, PROCESS_CREATION_FLAGS, PROCESS_QUERY_INFORMATION, PROCESS_SET_INFORMATION,
    REALTIME_PRIORITY_CLASS, SetPriorityClass, SetProcessAffinityMask,
};
use windows::Win32::UI::Shell::{
    CommandLineToArgvW, IShellLinkW, SLGP_UNCPRIORITY, SLR_NO_UI, ShellLink,
};
use windows::core::{HSTRING, Interface, PCWSTR};

use crate::PriorityClass;

pub struct OS;

impl OS {
    // helper: map our PriorityClass to WinAPI constant
    fn transform_to_win_priority(p: PriorityClass) -> PROCESS_CREATION_FLAGS {
        match p {
            PriorityClass::Idle => IDLE_PRIORITY_CLASS,
            PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
            PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
            PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
            PriorityClass::High => HIGH_PRIORITY_CLASS,
            PriorityClass::Realtime => REALTIME_PRIORITY_CLASS,
        }
    }

    // helper: map WinAPI priority constant to our PriorityClass
    fn from_win_priority(p: u32) -> PriorityClass {
        match p {
            x if x == IDLE_PRIORITY_CLASS.0 => PriorityClass::Idle,
            x if x == BELOW_NORMAL_PRIORITY_CLASS.0 => PriorityClass::BelowNormal,
            x if x == NORMAL_PRIORITY_CLASS.0 => PriorityClass::Normal,
            x if x == ABOVE_NORMAL_PRIORITY_CLASS.0 => PriorityClass::AboveNormal,
            x if x == HIGH_PRIORITY_CLASS.0 => PriorityClass::High,
            x if x == REALTIME_PRIORITY_CLASS.0 => PriorityClass::Realtime,
            _ => PriorityClass::Normal, // Default to Normal if unknown
        }
    }

    /// Gets the current CPU affinity mask for a process.
    ///
    /// # Parameters
    ///
    /// * `pid` - The process ID
    ///
    /// # Returns
    ///
    /// A Result containing either the CPU affinity mask as a usize or an error message
    pub fn get_process_affinity(pid: u32) -> Result<usize, String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid)
                .map_err(|e| format!("Failed to open process {}: {}", pid, e))?;

            let mut process_mask: usize = 0;
            let mut system_mask: usize = 0;

            let result = GetProcessAffinityMask(
                handle,
                &mut process_mask as *mut usize,
                &mut system_mask as *mut usize,
            );

            let _ = CloseHandle(handle);

            result
                .map(|_| process_mask)
                .map_err(|e| format!("Failed to get affinity mask for process {}: {}", pid, e))
        }
    }

    /// Gets the current priority class for a process.
    ///
    /// # Parameters
    ///
    /// * `pid` - The process ID
    ///
    /// # Returns
    ///
    /// A Result containing either the priority class as a PriorityClass enum or an error message
    pub fn get_process_priority(pid: u32) -> Result<PriorityClass, String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid)
                .map_err(|e| format!("Failed to open process {}: {}", pid, e))?;

            let priority = GetPriorityClass(handle);

            let _ = CloseHandle(handle);

            if priority == 0 {
                Err(format!("Failed to get priority for process {}", pid))
            } else {
                Ok(Self::from_win_priority(priority))
            }
        }
    }

    /// Sets the CPU affinity mask for a process by PID.
    ///
    /// # Parameters
    ///
    /// * `pid` - The process ID
    /// * `mask` - The CPU affinity mask to set
    ///
    /// # Returns
    ///
    /// A Result containing either () on success or an error message
    pub fn set_process_affinity_by_pid(pid: u32, mask: usize) -> Result<(), String> {
        unsafe {
            let handle = OpenProcess(PROCESS_SET_INFORMATION, false, pid)
                .map_err(|e| format!("Failed to open process {}: {}", pid, e))?;

            let result = SetProcessAffinityMask(handle, mask);

            let _ = CloseHandle(handle);

            result.map_err(|e| format!("Failed to set affinity mask for process {}: {}", pid, e))
        }
    }

    /// Sets the priority class for a process by PID.
    ///
    /// # Parameters
    ///
    /// * `pid` - The process ID
    /// * `priority` - The priority class to set
    ///
    /// # Returns
    ///
    /// A Result containing either () on success or an error message
    pub fn set_process_priority_by_pid(pid: u32, priority: PriorityClass) -> Result<(), String> {
        unsafe {
            let handle = OpenProcess(PROCESS_SET_INFORMATION, false, pid)
                .map_err(|e| format!("Failed to open process {}: {}", pid, e))?;

            let result = SetPriorityClass(handle, Self::transform_to_win_priority(priority));

            let _ = CloseHandle(handle);

            result.map_err(|e| format!("Failed to set priority for process {}: {}", pid, e))
        }
    }

    fn parse_url_file(path: &PathBuf) -> Result<String, String> {
        let content =
            fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;

        for line in content.lines() {
            if let Some(url) = line.strip_prefix("URL=") {
                return Ok(url.trim().to_string());
            }
        }

        Err("URL= not found".into())
    }

    fn resolve_url(path: &PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        let url_path = Self::parse_url_file(path)?;
        let parced_path = OS::get_program_path_for_uri(&url_path.split(':').nth(0).unwrap_or(""));
        if parced_path.is_ok() {
            let parced_path = parced_path.unwrap();
            return Ok((parced_path, vec![url_path]));
        } else {
            return Err(format!("Failed to parse URL: {}", parced_path.unwrap_err()));
        }
    }

    fn split_windows_args(args: &str) -> Vec<String> {
        if args.is_empty() {
            return Vec::new();
        }
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = std::ffi::OsStr::new(args)
            .encode_wide()
            .chain([0])
            .collect();
        let mut argc: i32 = 0;
        unsafe {
            let ptrs = CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut argc);
            if ptrs.is_null() || argc <= 0 {
                return vec![args.to_string()];
            }
            let mut out = Vec::with_capacity(argc as usize);
            for i in 0..argc {
                let p = (*ptrs.add(i as usize)).0;
                let len = (0..).take_while(|&k| *p.add(k) != 0).count();
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
                out.push(s);
            }
            // CommandLineToArgvW allocates memory that should be freed, but in windows crate
            // the returned type may handle this automatically or we use LocalFree if available
            out
        }
    }

    fn resolve_lnk(path: &PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        unsafe {
            // COM init (STA достаточно для IShellLink)
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| format!("CoInitializeEx failed: {e}"))?;

            // Создаём ShellLink и грузим .lnk
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("CoCreateInstance(ShellLink) failed: {e}"))?;
            let persist: IPersistFile = link
                .cast()
                .map_err(|e| format!("IPersistFile cast failed: {e}"))?;
            let h = HSTRING::from(path.as_os_str());
            persist
                .Load(PCWSTR(h.as_ptr()), STGM_READ)
                .map_err(|e| format!("Load {:?} failed: {e}", path))?;

            // Разрешаем ярлык без UI, по умолчанию таймаут 3 сек
            link.Resolve(HWND(std::ptr::null_mut()), SLR_NO_UI.0 as u32)
                .map_err(|e| format!("Resolve failed: {e}"))?;

            // Получаем путь к цели
            let mut wbuf = [0u16; 32768];
            let mut find = WIN32_FIND_DATAW::default();
            link.GetPath(&mut wbuf, &mut find as *mut _, SLGP_UNCPRIORITY.0 as u32)
                .map_err(|e| format!("GetPath failed: {e}"))?;
            let n = wbuf.iter().position(|&c| c == 0).unwrap_or(wbuf.len());
            let target = PathBuf::from(String::from_utf16_lossy(&wbuf[..n]));

            // Получаем строку аргументов и парсим по правилам Windows
            let mut abuf = [0u16; 32768];
            link.GetArguments(&mut abuf)
                .map_err(|e| format!("GetArguments failed: {e}"))?;
            let an = abuf.iter().position(|&c| c == 0).unwrap_or(abuf.len());
            let args_str = String::from_utf16_lossy(&abuf[..an]);
            let args_vec = Self::split_windows_args(&args_str);

            CoUninitialize();
            Ok((target, args_vec))
        }
    }

    fn spawn(target: &PathBuf, args: &[String]) -> Result<Child, String> {
        let mut cmd = Command::new(target);
        if !args.is_empty() {
            cmd.args(args);
        }
        cmd.stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("spawn {:?} failed: {}", target, e))
    }

    fn set_affinity(child: &Child, mask: usize) -> Result<(), String> {
        let h = HANDLE(child.as_raw_handle());
        unsafe { SetProcessAffinityMask(h, mask) }
            .map_err(|e| format!("SetProcessAffinityMask failed: {}", e))
    }

    fn set_priority(child: &Child, p: PriorityClass) -> Result<(), String> {
        let h = HANDLE(child.as_raw_handle());
        unsafe { SetPriorityClass(h, Self::transform_to_win_priority(p)) }
            .map_err(|e| format!("SetPriorityClass failed: {}", e))
    }

    #[allow(dead_code)]
    fn get_parent_pid(pid: u32) -> Option<u32> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid).ok()?;
            let mut info: PROCESS_BASIC_INFORMATION = zeroed();
            let status = NtQueryInformationProcess(
                h.0 as _,
                ProcessBasicInformation,
                &mut info as *mut _ as *mut _,
                size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                null_mut(),
            );
            let _ = CloseHandle(h);
            if status < 0 {
                None
            } else {
                Some(info.InheritedFromUniqueProcessId as u32)
            }
        }
    }

    #[allow(dead_code)]
    fn get_all_pids() -> Vec<u32> {
        let mut buf = vec![0u32; 1024];
        let mut ret = 0;
        unsafe {
            if K32EnumProcesses(buf.as_mut_ptr(), (buf.len() * 4) as u32, &mut ret).as_bool()
                == false
            {
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

    /// Finds all descendant processes of a given parent process.
    ///
    /// This method uses an iterative breadth-first search approach to find all
    /// child processes and their descendants. It avoids recursion to prevent
    /// potential stack overflow with deep process trees.
    ///
    /// # Parameters
    ///
    /// * `parent_pid` - The parent process ID to find descendants for
    /// * `descendants` - A mutable vector to store the found descendant PIDs
    ///
    /// # Implementation Details
    ///
    /// - Uses a queue to track processes to check
    /// - Uses a HashSet to efficiently track which PIDs have been processed
    /// - Performs a breadth-first traversal of the process tree
    pub fn find_all_descendants(parent_pid: u32, descendants: &mut Vec<u32>) {
        use std::collections::{HashSet, VecDeque};

        // Use a queue for breadth-first traversal
        let mut queue = VecDeque::new();
        queue.push_back(parent_pid);

        // Use a HashSet to efficiently track processed PIDs
        let mut processed = HashSet::new();
        processed.insert(parent_pid);

        while let Some(current_pid) = queue.pop_front() {
            // Find immediate children of the current process
            let children = Self::find_child_pids(current_pid);

            for child in children {
                // Skip if we've already processed this PID
                if processed.insert(child) {
                    // Add to descendants list
                    if !descendants.contains(&child) {
                        descendants.push(child);
                    }

                    // Add to queue to process its children
                    queue.push_back(child);
                }
            }
        }
    }

    pub fn parse_dropped_file(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        let file_ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| format!("Failed to get file extension for {:?}", file_path))?;

        if file_ext == "url" {
            return Self::resolve_url(&file_path);
        } else if file_ext == "lnk" {
            return Self::resolve_lnk(&file_path);
        }

        // If the file is not a URL or LNK, return the file path as is
        Ok((file_path, Vec::new()))
    }

    pub fn is_pid_live(pid: u32) -> bool {
        unsafe {
            let handle = match OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) {
                Ok(h) => h,
                Err(_) => return false,
            };

            let mut exit_code: u32 = 0;
            let result = GetExitCodeProcess(handle, &mut exit_code);
            let _ = CloseHandle(handle);

            result.is_ok() && exit_code == STILL_ACTIVE.0 as u32
        }
    }

    pub fn focus_window_by_pid(pid: u32) -> bool {
        use std::sync::atomic::{AtomicU32, Ordering};
        use windows::Win32::Foundation::LPARAM;
        use windows::core::BOOL;

        static mut FOUND_HWND: HWND = HWND(null_mut());
        static TARGET_PID: AtomicU32 = AtomicU32::new(0);

        unsafe extern "system" fn enum_windows_proc(hwnd: HWND, _: LPARAM) -> BOOL {
            let mut window_pid = 0u32;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
            }

            if window_pid == TARGET_PID.load(Ordering::Relaxed) {
                if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                    unsafe {
                        FOUND_HWND = hwnd;
                    }
                    return BOOL(0);
                }
            }
            BOOL(1)
        }

        unsafe {
            TARGET_PID.store(pid, Ordering::Relaxed);
            FOUND_HWND = HWND(null_mut());

            let _ = EnumWindows(Some(enum_windows_proc), LPARAM(0));

            if FOUND_HWND.0 != null_mut() {
                let _ = ShowWindow(FOUND_HWND, SW_RESTORE);
                let _ = SetForegroundWindow(FOUND_HWND);
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

    pub fn get_program_path_for_uri(uri_scheme: &str) -> Result<PathBuf, String> {
        let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);

        let scheme_key = hkcr
            .open_subkey(uri_scheme)
            .map_err(|e| format!("Scheme {} not found: {}", uri_scheme, e))?;

        let _: String = scheme_key
            .get_value("URL Protocol")
            .map_err(|_| "Not a valid URI protocol".to_string())?;

        let command_key_path = format!(r"{}\shell\open\command", uri_scheme);
        let command_key = hkcr
            .open_subkey(command_key_path)
            .map_err(|e| format!("Command key not found: {}", e))?;

        let command: String = command_key
            .get_value("")
            .map_err(|e| format!("Failed to get command string: {}", e))?;

        let exe_path = if command.starts_with('"') {
            // Extract from quotes
            command
                .split('"')
                .nth(1)
                .ok_or("Failed to parse command path")?
        } else {
            command
                .split_whitespace()
                .next()
                .ok_or("Failed to parse command path")?
        };

        Ok(PathBuf::from(exe_path))
    }
}
