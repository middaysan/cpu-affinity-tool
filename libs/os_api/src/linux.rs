// linux_process_ops.rs

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;

use libc::{
    PRIO_PROCESS, SCHED_FIFO, SCHED_RR, getpriority, pid_t, sched_getscheduler, sched_param,
    sched_setscheduler, setpriority,
};
use nix::sched::{CpuSet, sched_getaffinity, sched_setaffinity};
use shlex;

use crate::PriorityClass;

pub struct OS;

impl OS {
    // ---- helpers to reduce duplication ----
    fn compose_mask_from_cores(cores: &[usize]) -> Result<usize, String> {
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
        Ok(mask)
    }

    fn cpuset_from_mask(mask: usize) -> Result<CpuSet, String> {
        if mask == 0 {
            return Err("affinity mask is empty".into());
        }
        let mut cpu_set = CpuSet::new();
        for i in 0..usize::BITS {
            if (mask & (1usize << i)) != 0 {
                cpu_set.set(i as usize).map_err(|e| e.to_string())?;
            }
        }
        Ok(cpu_set)
    }

    fn mask_from_cpuset(set: &CpuSet) -> usize {
        let mut mask: usize = 0;
        for i in 0..usize::BITS as usize {
            if set.is_set(i).unwrap_or(false) {
                mask |= 1usize << i;
            }
        }
        mask
    }

    fn set_priority_for_pid(pid: pid_t, p: PriorityClass) -> Result<(), String> {
        match p {
            PriorityClass::Realtime => {
                let param = sched_param { sched_priority: 50 };
                let ret = unsafe { sched_setscheduler(pid, SCHED_FIFO, &param) };
                if ret == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error().to_string())
                }
            }
            _ => {
                let nice = Self::to_nice(p);
                let ret = unsafe { setpriority(PRIO_PROCESS, pid, nice) };
                if ret == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error().to_string())
                }
            }
        }
    }
    fn to_nice(p: PriorityClass) -> i32 {
        match p {
            PriorityClass::Idle => 19,
            PriorityClass::BelowNormal => 10,
            PriorityClass::Normal => 0,
            PriorityClass::AboveNormal => -5,
            PriorityClass::High => -10,
            PriorityClass::Realtime => -20,
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
        let pid = child.id() as pid_t;
        let cpu_set = Self::cpuset_from_mask(mask)?;
        sched_setaffinity(pid, &cpu_set).map_err(|e| e.to_string())
    }

    fn set_priority(child: &Child, p: PriorityClass) -> Result<(), String> {
        let pid = child.id() as pid_t;
        Self::set_priority_for_pid(pid, p)
    }

    /// Gets the current CPU affinity mask for a process.
    /// Note: Only the lowest `usize::BITS` CPUs are represented in the returned mask.
    pub fn get_process_affinity(pid: u32) -> Result<usize, String> {
        let pid = pid as pid_t;
        let mut set = CpuSet::new();
        sched_getaffinity(pid, &mut set).map_err(|e| e.to_string())?;
        Ok(Self::mask_from_cpuset(&set))
    }

    /// Gets the current priority class for a process.
    pub fn get_process_priority(pid: u32) -> Result<PriorityClass, String> {
        let pid_t = pid as pid_t;
        // Realtime if scheduled with FIFO or RR
        let policy = unsafe { sched_getscheduler(pid_t) };
        if policy == SCHED_FIFO || policy == SCHED_RR {
            return Ok(PriorityClass::Realtime);
        }

        // Otherwise map nice value to our classes
        // getpriority returns value in range -20..19; on error it returns -1 but errno must be checked.
        errno::set_errno(errno::Errno(0));
        let prio = unsafe { getpriority(PRIO_PROCESS, pid_t) };
        let err = errno::errno().0;
        if prio == -1 && err != 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let p = match prio {
            n if n >= 15 => PriorityClass::Idle,
            n if n >= 5 => PriorityClass::BelowNormal,
            n if n >= -4 => PriorityClass::Normal,
            n if n >= -9 => PriorityClass::AboveNormal,
            _ /* <= -10 */ => PriorityClass::High,
        };
        Ok(p)
    }

    /// Sets the CPU affinity mask for a process by PID.
    pub fn set_process_affinity_by_pid(pid: u32, mask: usize) -> Result<(), String> {
        let pid = pid as pid_t;
        let cpu_set = Self::cpuset_from_mask(mask)?;
        sched_setaffinity(pid, &cpu_set).map_err(|e| e.to_string())
    }

    /// Sets the priority class for a process by PID.
    pub fn set_process_priority_by_pid(pid: u32, priority: PriorityClass) -> Result<(), String> {
        let pid = pid as pid_t;
        Self::set_priority_for_pid(pid, priority)
    }

    pub fn parse_dropped_file(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        let path = fs::read_link(&file_path).unwrap_or(file_path.clone());

        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("desktop"))
            .unwrap_or(false)
        {
            return Self::parse_desktop_file(&path);
        }

        Ok((path, Vec::new()))
    }

    fn parse_desktop_file(path: &Path) -> Result<(PathBuf, Vec<String>), String> {
        let content =
            fs::read_to_string(path).map_err(|e| format!("failed to read .desktop: {}", e))?;

        for line in content.lines() {
            if line.starts_with("Exec=") {
                let cmdline = line["Exec=".len()..].trim();
                let vec = shlex::split(cmdline).unwrap_or_else(|| vec![cmdline.to_string()]);
                if vec.is_empty() {
                    return Err("Exec is empty".to_string());
                }
                let target = PathBuf::from(&vec[0]);
                return Ok((target, vec[1..].to_vec()));
            }
        }

        Err("Exec= not found in .desktop".into())
    }

    pub fn run(
        file_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
    ) -> Result<u32, String> {
        // Compose mask with validation similar to Windows implementation
        let mask = Self::compose_mask_from_cores(cores)?;
        let child = Self::spawn(&file_path, &args)?;
        let pid = child.id();
        Self::set_affinity(&child, mask)?;
        Self::set_priority(&child, priority)?;
        Ok(pid)
    }

    /// Gets the parent process ID of a given process.
    ///
    /// This function reads the `/proc/{pid}/stat` file to get the parent process ID.
    ///
    /// # Parameters
    ///
    /// * `pid` - The process ID to get the parent of
    ///
    /// # Returns
    ///
    /// The parent process ID, or None if the process doesn't exist or the parent couldn't be determined
    pub fn get_parent_pid(pid: u32) -> Option<u32> {
        // Read the /proc/{pid}/stat file
        let stat_path = format!("/proc/{}/stat", pid);
        let stat_content = match fs::read_to_string(&stat_path) {
            Ok(content) => content,
            Err(_) => return None,
        };

        // Parse the stat file to get the parent PID (4th field)
        // Format: pid (comm) state ppid ...
        let parts: Vec<&str> = stat_content.split_whitespace().collect();

        // The 4th field (index 3) is the parent PID
        // But we need to handle the case where the command name (comm) contains spaces
        // So we find the closing parenthesis and count from there
        if let Some(paren_pos) = stat_content.rfind(')') {
            let after_paren = &stat_content[paren_pos + 1..];
            let after_parts: Vec<&str> = after_paren.split_whitespace().collect();

            // The parent PID is the 3rd field after the closing parenthesis
            if after_parts.len() >= 3 {
                return u32::from_str(after_parts[2]).ok();
            }
        }

        None
    }

    /// Gets all process IDs in the system.
    ///
    /// This function reads the `/proc` directory to get all process IDs.
    ///
    /// # Returns
    ///
    /// A vector of all process IDs
    pub fn get_all_pids() -> Vec<u32> {
        let mut pids = Vec::new();

        // Read the /proc directory
        let proc_dir = match fs::read_dir("/proc") {
            Ok(dir) => dir,
            Err(_) => return pids,
        };

        // Iterate over all entries in the /proc directory
        for entry in proc_dir {
            if let Ok(entry) = entry {
                let file_name = entry.file_name();
                let file_name_str = file_name.to_string_lossy();

                // Check if the entry is a directory and its name is a number (PID)
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && file_name_str.chars().all(|c| c.is_digit(10))
                {
                    // Parse the directory name as a PID
                    if let Ok(pid) = u32::from_str(&file_name_str) {
                        pids.push(pid);
                    }
                }
            }
        }

        pids
    }

    /// Finds all process IDs that match the target name (case-insensitive, up to the first dot).
    pub fn find_pids_by_name(target_name: &str) -> Vec<u32> {
        let mut pids = Vec::new();
        if target_name.is_empty() {
            return pids;
        }

        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() {
                    // Try to read /proc/PID/comm which contains the process name
                    let comm_path = entry.path().join("comm");
                    if let Ok(comm) = fs::read_to_string(comm_path) {
                        let comm = comm.trim();
                        // Extract part before the first dot
                        let process_name = comm.split('.').next().unwrap_or("");
                        if process_name.eq_ignore_ascii_case(target_name) {
                            pids.push(pid);
                        }
                    }
                }
            }
        }
        pids
    }

    /// Returns all running process IDs and their executable names.
    pub fn get_all_process_names() -> Vec<(u32, String)> {
        let mut results = Vec::new();
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() {
                    // Try to read /proc/PID/comm which contains the process name
                    let comm_path = entry.path().join("comm");
                    if let Ok(comm) = fs::read_to_string(comm_path) {
                        results.push((pid, comm.trim().to_string()));
                    }
                }
            }
        }
        results
    }

    /// Finds all child process IDs of a given parent process.
    ///
    /// This function uses the `get_all_pids` and `get_parent_pid` functions to find all child processes.
    ///
    /// # Parameters
    ///
    /// * `parent` - The parent process ID
    ///
    /// # Returns
    ///
    /// A vector of child process IDs
    pub fn find_child_pids(parent: u32) -> Vec<u32> {
        let mut children = Vec::new();

        // Get all PIDs in the system
        let all_pids = Self::get_all_pids();

        // Check each PID to see if its parent is the specified parent
        for pid in all_pids {
            if let Some(ppid) = Self::get_parent_pid(pid) {
                if ppid == parent {
                    children.push(pid);
                }
            }
        }

        children
    }

    /// Recursively finds all descendant processes of a given parent process.
    ///
    /// This function uses the `find_child_pids` function to find all child processes
    /// and then recursively finds all descendants of those child processes.
    ///
    /// # Parameters
    ///
    /// * `parent_pid` - The parent process ID
    /// * `descendants` - A mutable vector to store the descendant process IDs
    pub fn find_all_descendants(parent_pid: u32, descendants: &mut Vec<u32>) {
        // Find all direct children of the parent process
        let children = Self::find_child_pids(parent_pid);

        // For each child, add it to the descendants list and recursively find its descendants
        for child in children {
            // Avoid infinite recursion if the child is already in the descendants list
            if !descendants.contains(&child) {
                descendants.push(child);
                Self::find_all_descendants(child, descendants);
            }
        }
    }

    /// Checks if a process with a given PID is still running.
    ///
    /// This function checks if the `/proc/{pid}` directory exists.
    ///
    /// # Parameters
    ///
    /// * `pid` - The process ID to check
    ///
    /// # Returns
    ///
    /// `true` if the process is running, `false` otherwise
    pub fn is_pid_live(pid: u32) -> bool {
        let proc_path = format!("/proc/{}", pid);

        // Check if the /proc/{pid} directory exists
        if let Ok(metadata) = fs::metadata(&proc_path) {
            return metadata.is_dir();
        }

        false
    }

    /// Attempts to focus a window belonging to a process with a given PID.
    ///
    /// This is a simplified implementation that always returns false.
    /// A proper implementation would require X11 or Wayland APIs to focus windows.
    ///
    /// # Parameters
    ///
    /// * `pid` - The process ID of the window to focus
    ///
    /// # Returns
    ///
    /// `true` if the window was successfully focused, `false` otherwise
    pub fn focus_window_by_pid(pid: u32) -> bool {
        // TODO: Implement window focusing using X11 or Wayland APIs
        // This would require additional dependencies like x11rb or wayland-client

        // For now, just return false to indicate that focusing failed
        false
    }

    /// Gets the program path for a given URI scheme.
    ///
    /// This function checks the XDG MIME database to find the default application
    /// for the given URI scheme.
    ///
    /// # Parameters
    ///
    /// * `uri_scheme` - The URI scheme to get the program path for (e.g., "http", "mailto")
    ///
    /// # Returns
    ///
    /// The program path, or an error if the program couldn't be found
    pub fn get_program_path_for_uri(uri_scheme: &str) -> Result<PathBuf, String> {
        // Try to get the default application for the URI scheme using xdg-mime
        let output = Command::new("xdg-mime")
            .args(&[
                "query",
                "default",
                &format!("x-scheme-handler/{}", uri_scheme),
            ])
            .output()
            .map_err(|e| format!("Failed to execute xdg-mime: {}", e))?;

        if !output.status.success() {
            return Err(format!("xdg-mime failed with status: {}", output.status));
        }

        // Parse the output to get the desktop file name
        let desktop_file = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if desktop_file.is_empty() {
            return Err(format!(
                "No default application found for URI scheme: {}",
                uri_scheme
            ));
        }

        // Look for the desktop file in the standard locations
        let desktop_dirs = [
            "/usr/share/applications",
            "/usr/local/share/applications",
            "~/.local/share/applications",
        ];

        for dir in &desktop_dirs {
            let dir_path = if dir.starts_with("~/") {
                let home = std::env::var("HOME").unwrap_or_else(|_| "".to_string());
                PathBuf::from(home).join(&dir[2..])
            } else {
                PathBuf::from(dir)
            };

            let desktop_path = dir_path.join(&desktop_file);
            if desktop_path.exists() {
                // Parse the desktop file to get the Exec line
                return Self::parse_desktop_file(&desktop_path).map(|(path, _)| path);
            }
        }

        Err(format!("Desktop file not found: {}", desktop_file))
    }

    pub fn get_cpu_model() -> String {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .find(|line| line.starts_with("model name"))
                    .and_then(|line| line.split(':').nth(1))
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_else(|| "Unknown CPU".to_string())
    }
}
