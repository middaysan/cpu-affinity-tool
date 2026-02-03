use std::fs;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::ptr::null_mut;

use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HLOCAL, HWND, LPARAM, LocalFree, STILL_ACTIVE, WPARAM,
};
use windows::Win32::Globalization::{MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::core::{BOOL, Interface, PCWSTR, PWSTR};
// LocalFree is in Foundation for this windows crate version
use windows::Win32::System::ProcessStatus::{K32EnumProcesses, K32GetModuleFileNameExW};
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, CREATE_SUSPENDED, CreateProcessW,
    GetExitCodeProcess, GetPriorityClass, GetProcessAffinityMask, HIGH_PRIORITY_CLASS,
    IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, OpenProcess, PROCESS_ACCESS_RIGHTS,
    PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, REALTIME_PRIORITY_CLASS,
    ResumeThread, STARTUPINFOW, SetPriorityClass, SetProcessAffinityMask,
};
use windows::Win32::UI::Shell::{
    CommandLineToArgvW, IShellLinkW, SLGP_UNCPRIORITY, SLR_NO_UI, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, EnumWindows, GWL_EXSTYLE, GetForegroundWindow,
    GetWindowLongW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_HIDE, SW_RESTORE,
    SW_SHOW, SetForegroundWindow, SetWindowLongW, ShowWindow, ShowWindowAsync, WM_NULL,
    WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use winreg::RegKey;
use winreg::enums::*;

use crate::PriorityClass;

// ---- internal error type (public API still returns String) ----
#[derive(Debug)]
enum OsError {
    Win(windows::core::Error),
    Msg(String),
}

impl std::fmt::Display for OsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OsError::Win(e) => write!(f, "{e}"),
            OsError::Msg(s) => write!(f, "{s}"),
        }
    }
}

impl From<windows::core::Error> for OsError {
    fn from(e: windows::core::Error) -> Self {
        Self::Win(e)
    }
}

// ---- tiny RAII helpers ----
struct HandleGuard(HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_invalid() {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

pub struct OS;

// One snapshot used for all process-tree operations.
struct ProcessTree {
    parent_of: std::collections::HashMap<u32, u32>,
    children_of: std::collections::HashMap<u32, Vec<u32>>,
}

impl OS {
    // ---- helpers ----

    fn open_process(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Result<HANDLE, OsError> {
        unsafe { Ok(OpenProcess(access, false, pid)?) }
    }

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
            _ => PriorityClass::Normal,
        }
    }

    fn to_wide_z(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain([0]).collect()
    }

    fn to_wide_z_str(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain([0]).collect()
    }

    // Windows CreateProcess quoting rules: minimal implementation good enough for args/paths.
    fn quote_arg_windows(arg: &str) -> String {
        if arg.is_empty() {
            return "\"\"".to_string();
        }
        let needs_quotes = arg
            .bytes()
            .any(|b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r');
        if !needs_quotes && !arg.contains('"') {
            return arg.to_string();
        }

        let mut out = String::new();
        out.push('"');

        let mut backslashes = 0usize;
        for ch in arg.chars() {
            match ch {
                '\\' => backslashes += 1,
                '"' => {
                    // escape all backslashes + the quote
                    out.push_str(&"\\".repeat(backslashes * 2 + 1));
                    out.push('"');
                    backslashes = 0;
                }
                _ => {
                    // if it's not a quote, backslashes are NOT escaped (unless they're at the end)
                    out.push_str(&"\\".repeat(backslashes));
                    out.push(ch);
                    backslashes = 0;
                }
            }
        }
        // escape trailing backslashes before closing quote
        out.push_str(&"\\".repeat(backslashes * 2));
        out.push('"');
        out
    }

    fn build_command_line(exe: &PathBuf, args: &[String]) -> String {
        let exe_s = exe.to_string_lossy();
        let mut parts = Vec::with_capacity(1 + args.len());
        parts.push(Self::quote_arg_windows(&exe_s));
        for a in args {
            parts.push(Self::quote_arg_windows(a));
        }
        parts.join(" ")
    }

    fn expand_env(s: &str) -> String {
        // Convert to wide with trailing NUL
        let wide = Self::to_wide_z_str(s);
        // First call with zero buffer to get required size (includes terminating NUL)
        unsafe {
            let needed = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), None);
            if needed == 0 {
                return s.to_string();
            }
            let mut buf = vec![0u16; needed as usize];
            let written = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buf));
            if written == 0 {
                return s.to_string();
            }
            // Remove trailing NUL if present
            if let Some(pos) = buf.iter().position(|&c| c == 0) {
                buf.truncate(pos);
            }
            String::from_utf16_lossy(&buf)
        }
    }

    fn decode_ansi(bytes: &[u8]) -> Option<String> {
        // Fallback: decode using system ANSI code page (CP_ACP = 0)
        unsafe {
            let needed = MultiByteToWideChar(
                0,                                // CP_ACP
                MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), // flags
                bytes,
                None,
            );
            if needed <= 0 {
                return None;
            }
            let mut buf = vec![0u16; needed as usize];
            let written =
                MultiByteToWideChar(0, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut buf));
            if written <= 0 {
                return None;
            }
            Some(String::from_utf16_lossy(&buf[..written as usize]))
        }
    }

    fn snapshot_process_tree() -> Result<ProcessTree, OsError> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
            let _hg = HandleGuard(snap);

            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe).is_err() {
                return Err(OsError::Msg("Process32FirstW failed".into()));
            }

            let mut parent_of = std::collections::HashMap::new();
            let mut children_of: std::collections::HashMap<u32, Vec<u32>> =
                std::collections::HashMap::new();

            loop {
                let pid = pe.th32ProcessID;
                let ppid = pe.th32ParentProcessID;

                if pid != 0 {
                    parent_of.insert(pid, ppid);
                    children_of.entry(ppid).or_default().push(pid);
                }

                let mut next: PROCESSENTRY32W = std::mem::zeroed();
                next.dwSize = size_of::<PROCESSENTRY32W>() as u32;
                pe = next;

                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }

            Ok(ProcessTree {
                parent_of,
                children_of,
            })
        }
    }

    // ---- public API (unchanged signatures) ----

    /// Gets the current CPU affinity mask for a process.
    ///
    /// **Note:** On systems with more than 64 logical CPUs (Processor Groups),
    /// this function only returns the affinity mask for the current processor group.
    pub fn get_process_affinity(pid: u32) -> Result<usize, String> {
        (|| unsafe {
            let handle = Self::open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)
                .or_else(|_| Self::open_process(pid, PROCESS_QUERY_INFORMATION))?;
            let _hg = HandleGuard(handle);

            let mut process_mask: usize = 0;
            let mut system_mask: usize = 0;

            GetProcessAffinityMask(
                handle,
                &mut process_mask as *mut _,
                &mut system_mask as *mut _,
            )?;
            // Do NOT change behavior by erroring on multi-group systems. Keep returning the mask.
            Ok(process_mask)
        })()
        .map_err(|e: OsError| format!("Failed to get affinity mask for process {}: {}", pid, e))
    }

    /// Gets the current priority class for a process.
    pub fn get_process_priority(pid: u32) -> Result<PriorityClass, String> {
        (|| unsafe {
            let handle = Self::open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)
                .or_else(|_| Self::open_process(pid, PROCESS_QUERY_INFORMATION))?;
            let _hg = HandleGuard(handle);

            let priority = GetPriorityClass(handle);
            if priority == 0 {
                return Err(OsError::Msg("GetPriorityClass returned 0".into()));
            }
            Ok(Self::from_win_priority(priority))
        })()
        .map_err(|e: OsError| format!("Failed to get priority for process {}: {}", pid, e))
    }

    /// Sets the CPU affinity mask for a process by PID.
    ///
    /// **Note:** On systems with more than 64 logical CPUs (Processor Groups),
    /// this function only sets the affinity for the current processor group.
    pub fn set_process_affinity_by_pid(pid: u32, mask: usize) -> Result<(), String> {
        (|| unsafe {
            let handle = Self::open_process(pid, PROCESS_SET_INFORMATION)?;
            let _hg = HandleGuard(handle);

            SetProcessAffinityMask(handle, mask)?;
            Ok(())
        })()
        .map_err(|e: OsError| format!("Failed to set affinity mask for process {}: {}", pid, e))
    }

    /// Sets the priority class for a process by PID.
    pub fn set_process_priority_by_pid(pid: u32, priority: PriorityClass) -> Result<(), String> {
        (|| unsafe {
            let handle = Self::open_process(pid, PROCESS_SET_INFORMATION)?;
            let _hg = HandleGuard(handle);

            SetPriorityClass(handle, Self::transform_to_win_priority(priority))?;
            Ok(())
        })()
        .map_err(|e: OsError| format!("Failed to set priority for process {}: {}", pid, e))
    }

    /// Sets the priority class for the current process.
    pub fn set_current_process_priority(priority: PriorityClass) -> Result<(), String> {
        unsafe {
            let handle = windows::Win32::System::Threading::GetCurrentProcess();
            SetPriorityClass(handle, Self::transform_to_win_priority(priority))
                .map_err(|e| format!("Failed to set current process priority: {}", e))
        }
    }

    fn parse_url_file(path: &PathBuf) -> Result<String, String> {
        // Try to read .url with BOM handling (UTF-16LE/BE) then UTF-8 fallback
        let bytes = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
        let content = if bytes.starts_with(&[0xFF, 0xFE]) && bytes.len() >= 2 {
            // UTF-16 LE with BOM
            let u16s: Vec<u16> = bytes[2..]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&u16s)
        } else if bytes.starts_with(&[0xFE, 0xFF]) && bytes.len() >= 2 {
            // UTF-16 BE with BOM
            let u16s: Vec<u16> = bytes[2..]
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&u16s)
        } else {
            match String::from_utf8(bytes.clone()) {
                Ok(s) => s,
                Err(_) => Self::decode_ansi(&bytes)
                    .ok_or("Failed to decode .url file using UTF-8 or ANSI")?,
            }
        };

        for line in content.lines() {
            if let Some(url) = line.strip_prefix("URL=") {
                return Ok(url.trim().to_string());
            }
        }

        Err("URL= not found".into())
    }

    fn resolve_url(path: &PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        let url = Self::parse_url_file(path)?;
        let scheme = url
            .split(':')
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("Invalid URL (no scheme): {}", url))?;

        let exe = Self::get_program_path_for_uri(scheme)
            .map_err(|e| format!("Failed to parse URL: {}", e))?;

        Ok((exe, vec![url]))
    }

    fn split_windows_args(args: &str) -> Vec<String> {
        if args.is_empty() {
            return Vec::new();
        }

        let wide: Vec<u16> = std::ffi::OsStr::new(args)
            .encode_wide()
            .chain([0])
            .collect();
        let mut argc: i32 = 0;

        unsafe {
            let argv = CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut argc);
            if argv.is_null() || argc <= 0 {
                return vec![args.to_string()];
            }

            let mut out = Vec::with_capacity(argc as usize);

            for i in 0..argc {
                let p = (*argv.add(i as usize)).0;
                if p.is_null() {
                    out.push(String::new());
                    continue;
                }
                let mut len = 0usize;
                while *p.add(len) != 0 {
                    len += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
                out.push(s);
            }

            let _ = LocalFree(Some(HLOCAL(argv as *mut core::ffi::c_void))); // avoid leak
            out
        }
    }

    fn resolve_lnk(path: &PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        (|| unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| OsError::Win(e))?;
            let _com = ComGuard;

            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let persist: IPersistFile = link.cast()?;

            // IPersistFile::Load needs NUL-terminated PCWSTR.
            let wide = Self::to_wide_z(path.as_os_str());
            persist.Load(PCWSTR(wide.as_ptr()), STGM_READ)?;

            link.Resolve(HWND(null_mut()), SLR_NO_UI.0 as u32)?;

            let mut wbuf = [0u16; 32768];
            let mut find = WIN32_FIND_DATAW::default();
            link.GetPath(&mut wbuf, &mut find as *mut _, SLGP_UNCPRIORITY.0 as u32)?;
            let n = wbuf.iter().position(|&c| c == 0).unwrap_or(wbuf.len());
            let target_raw = String::from_utf16_lossy(&wbuf[..n]);
            let target = PathBuf::from(Self::expand_env(&target_raw));

            let mut abuf = [0u16; 32768];
            link.GetArguments(&mut abuf)?;
            let an = abuf.iter().position(|&c| c == 0).unwrap_or(abuf.len());
            let args_str = String::from_utf16_lossy(&abuf[..an]);
            let args_expanded = Self::expand_env(&args_str);
            let args_vec = Self::split_windows_args(&args_expanded);

            Ok((target, args_vec))
        })()
        .map_err(|e: OsError| format!("resolve_lnk {:?} failed: {}", path, e))
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    fn set_affinity(child: &Child, mask: usize) -> Result<(), String> {
        let h = HANDLE(child.as_raw_handle());
        unsafe { SetProcessAffinityMask(h, mask) }
            .map_err(|e| format!("SetProcessAffinityMask failed: {}", e))
    }

    #[allow(dead_code)]
    fn set_priority(child: &Child, p: PriorityClass) -> Result<(), String> {
        let h = HANDLE(child.as_raw_handle());
        unsafe { SetPriorityClass(h, Self::transform_to_win_priority(p)) }
            .map_err(|e| format!("SetPriorityClass failed: {}", e))
    }

    #[allow(dead_code)]
    fn get_parent_pid(pid: u32) -> Option<u32> {
        let tree = Self::snapshot_process_tree().ok()?;
        tree.parent_of.get(&pid).copied()
    }

    #[allow(dead_code)]
    fn get_all_pids() -> Vec<u32> {
        // K32EnumProcesses needs retry with growing buffer.
        let mut cap = 1024usize;
        loop {
            let mut buf = vec![0u32; cap];
            let mut needed: u32 = 0;

            let ok =
                unsafe { K32EnumProcesses(buf.as_mut_ptr(), (buf.len() * 4) as u32, &mut needed) }
                    .as_bool();

            if !ok {
                // Avoid panicking from a library method; return empty result on error.
                return Vec::new();
            }

            let count = needed as usize / 4;
            if count < buf.len() {
                buf.truncate(count);
                return buf;
            }

            cap *= 2;
            if cap > 1_048_576 {
                buf.truncate(count);
                return buf;
            }
        }
    }

    /// Finds all process IDs that match the target name (case-insensitive, up to the first dot).
    pub fn find_pids_by_name(target_name: &str) -> Vec<u32> {
        let mut pids = Vec::new();
        if target_name.is_empty() {
            return pids;
        }
        unsafe {
            let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(s) => s,
                Err(_) => return pids,
            };
            let _hg = HandleGuard(snap);

            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe).is_err() {
                return pids;
            }

            loop {
                let len = pe
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(pe.szExeFile.len());
                let exe_file = String::from_utf16_lossy(&pe.szExeFile[..len]);

                // Extract part before the first dot
                let process_name = exe_file.split('.').next().unwrap_or("");

                if process_name.eq_ignore_ascii_case(target_name) {
                    pids.push(pe.th32ProcessID);
                }

                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        pids
    }

    /// Returns all running process IDs and their executable names.
    pub fn get_all_process_names() -> Vec<(u32, String)> {
        let mut results = Vec::new();
        unsafe {
            let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(s) => s,
                Err(_) => return results,
            };
            let _hg = HandleGuard(snap);

            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe).is_err() {
                return results;
            }

            loop {
                let len = pe
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(pe.szExeFile.len());
                let exe_file = String::from_utf16_lossy(&pe.szExeFile[..len]);

                results.push((pe.th32ProcessID, exe_file));

                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        results
    }

    #[allow(dead_code)]
    fn find_child_pids(parent: u32) -> Vec<u32> {
        match Self::snapshot_process_tree() {
            Ok(tree) => tree.children_of.get(&parent).cloned().unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// Finds all descendant processes of a given parent process.
    ///
    /// Preserves original behavior: doesn't add duplicates if `descendants` already contains some PIDs.
    pub fn find_all_descendants(parent_pid: u32, descendants: &mut Vec<u32>) {
        let tree = match Self::snapshot_process_tree() {
            Ok(t) => t,
            Err(_) => return,
        };

        use std::collections::{HashSet, VecDeque};

        let mut existing: HashSet<u32> = descendants.iter().copied().collect();
        let mut processed: HashSet<u32> = HashSet::new();

        let mut queue = VecDeque::new();
        queue.push_back(parent_pid);
        processed.insert(parent_pid);

        while let Some(current) = queue.pop_front() {
            if let Some(children) = tree.children_of.get(&current) {
                for &child in children {
                    if processed.insert(child) {
                        if existing.insert(child) {
                            descendants.push(child);
                        }
                        queue.push_back(child);
                    }
                }
            }
        }
    }

    pub fn parse_dropped_file(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        let file_ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| format!("Failed to get file extension for {:?}", file_path))?;

        if file_ext.eq_ignore_ascii_case("url") {
            return Self::resolve_url(&file_path);
        } else if file_ext.eq_ignore_ascii_case("lnk") {
            return Self::resolve_lnk(&file_path);
        }

        Ok((file_path, Vec::new()))
    }

    pub fn is_pid_live(pid: u32) -> bool {
        unsafe {
            let handle = match Self::open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)
                .or_else(|_| Self::open_process(pid, PROCESS_QUERY_INFORMATION))
            {
                Ok(h) => h,
                Err(_) => return false,
            };
            let _hg = HandleGuard(handle);

            let mut exit_code: u32 = 0;
            let result = GetExitCodeProcess(handle, &mut exit_code);

            result.is_ok() && exit_code == STILL_ACTIVE.0 as u32
        }
    }

    pub fn focus_window_by_pid(pid: u32) -> bool {
        #[repr(C)]
        struct Ctx {
            target_pid: u32,
            found: HWND,
        }

        unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // Explicitly wrap raw pointer deref in an unsafe block (Rust 2024 requires this)
            let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };

            let mut window_pid = 0u32;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
            }

            if window_pid == ctx.target_pid && unsafe { IsWindowVisible(hwnd).as_bool() } {
                ctx.found = hwnd;
                return BOOL(0); // stop enumeration
            }

            BOOL(1)
        }

        unsafe {
            let mut ctx = Box::new(Ctx {
                target_pid: pid,
                found: HWND(null_mut()),
            });

            let ctx_ptr = ctx.as_mut() as *mut Ctx;
            let _ = EnumWindows(Some(enum_windows_proc), LPARAM(ctx_ptr as isize));

            if ctx.found.0 == null_mut() {
                return false;
            }

            let _ = AllowSetForegroundWindow(ASFW_ANY);
            let _ = ShowWindowAsync(ctx.found, SW_RESTORE);
            let _ = SetForegroundWindow(ctx.found);

            // Return actual status, not “we called the function”.
            let fg = GetForegroundWindow();
            fg == ctx.found
        }
    }

    pub fn run(
        file_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
    ) -> Result<u32, String> {
        // validate/compose mask safely
        let mut mask = 0usize;
        for &i in cores {
            let bit = 1usize
                .checked_shl(i as u32)
                .ok_or_else(|| format!("core index {} out of range for affinity mask", i))?;
            mask |= bit;
        }

        if mask == 0 {
            return Err("affinity mask is empty".into());
        }

        // Create process suspended, set affinity/priority, then resume.
        (|| unsafe {
            let cmdline = Self::build_command_line(&file_path, &args);
            let mut cmd_w = Self::to_wide_z_str(&cmdline);

            let mut si: STARTUPINFOW = std::mem::zeroed();
            si.cb = size_of::<STARTUPINFOW>() as u32;

            let mut pi: PROCESS_INFORMATION = std::mem::zeroed();

            // NOTE: not inheriting handles explicitly here to avoid new feature deps.
            // In most cases child will still share the same console/default std handles.
            CreateProcessW(
                PCWSTR(null_mut()),
                Option::from(PWSTR(cmd_w.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_SUSPENDED,
                None,
                None,
                &si,
                &mut pi,
            )?;

            let process = pi.hProcess;
            let thread = pi.hThread;

            // Ensure handles are closed even if setting affinity/priority fails
            let _pg = HandleGuard(process);
            let _tg = HandleGuard(thread);

            SetProcessAffinityMask(process, mask)?;
            SetPriorityClass(process, Self::transform_to_win_priority(priority))?;

            let _ = ResumeThread(thread);

            Ok(pi.dwProcessId)
        })()
        .map_err(|e: OsError| format!("run {:?} failed: {}", file_path, e))
    }

    pub fn get_program_path_for_uri(uri_scheme: &str) -> Result<PathBuf, String> {
        // Use registry-based resolution for stability across windows crate versions.
        Self::get_program_path_for_uri_registry(uri_scheme)
    }

    fn get_program_path_for_uri_registry(uri_scheme: &str) -> Result<PathBuf, String> {
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
        // Expand environment variables and parse command line to extract executable path
        let expanded = Self::expand_env(&command);
        let parts = Self::split_windows_args(&expanded);
        let first = parts.first().ok_or("Command string is empty")?;
        Ok(PathBuf::from(first))
    }

    /// Toggles the window's visibility in the taskbar.
    /// show = true: Standard application window (AppWindow).
    /// show = false: Tool window (hidden from taskbar/Alt-Tab).
    pub fn set_taskbar_visible(hwnd: HWND, show: bool) {
        unsafe {
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            let mut new_style = style;

            if show {
                new_style &= !(WS_EX_TOOLWINDOW.0 as i32);
                new_style |= WS_EX_APPWINDOW.0 as i32;
            } else {
                new_style |= WS_EX_TOOLWINDOW.0 as i32;
                new_style &= !(WS_EX_APPWINDOW.0 as i32);
            }

            if new_style != style {
                SetWindowLongW(hwnd, GWL_EXSTYLE, new_style);
            }
        }
    }

    pub fn hide_window(hwnd: HWND) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    pub fn show_window(hwnd: HWND) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
    }

    pub fn restore_and_focus(hwnd: HWND) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    pub fn poke_window(hwnd: HWND) {
        unsafe {
            // Post to the window
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            // Also post to the thread message queue to ensure GetMessage/PeekMessage wakes up
            let _ = PostMessageW(None, WM_NULL, WPARAM(0), LPARAM(0));
        }
    }

    pub fn get_cpu_model() -> String {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        hklm.open_subkey(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
            .and_then(|key| {
                let s: String = key.get_value("ProcessorNameString")?;
                Ok(s.trim_matches(|c: char| c.is_whitespace() || c == '\0')
                    .to_string())
            })
            .unwrap_or_else(|_| "Unknown CPU".to_string())
    }

    pub fn get_process_image_path(pid: u32) -> Result<PathBuf, String> {
        (|| unsafe {
            let handle = Self::open_process(
                pid,
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_QUERY_INFORMATION,
            )?;
            let _hg = HandleGuard(handle);

            let mut buffer = [0u16; 1024];
            let len = K32GetModuleFileNameExW(Some(handle), None, &mut buffer);
            if len == 0 {
                return Err(OsError::Win(windows::core::Error::from_thread()));
            }

            let path_str = String::from_utf16_lossy(&buffer[..len as usize]);
            Ok(PathBuf::from(path_str))
        })()
        .map_err(|e: OsError| format!("Failed to get image path for process {}: {}", pid, e))
    }
}
