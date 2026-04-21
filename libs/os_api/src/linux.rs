use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use libc::{
    PRIO_PROCESS, SCHED_FIFO, SCHED_RR, getpriority, pid_t, sched_getscheduler, sched_param,
    sched_setscheduler, setpriority,
};
use nix::sched::{CpuSet, sched_getaffinity, sched_setaffinity};
use nix::unistd::Pid;

use crate::PriorityClass;
use crate::{InstalledAppCatalogEntry, InstalledAppCatalogTarget, InstalledPackageRuntimeInfo};

pub struct OS;

pub struct ProcessTree {
    pub parent_of: HashMap<u32, u32>,
    pub children_of: HashMap<u32, Vec<u32>>,
    pub names: HashMap<u32, String>,
}

#[derive(Debug, Default)]
struct DesktopEntry {
    name: String,
    exec: String,
    hidden: bool,
    no_display: bool,
    entry_type: Option<String>,
}

impl OS {
    pub const fn supports_hide_to_tray() -> bool {
        false
    }

    pub const fn supports_installed_app_picker() -> bool {
        true
    }

    fn compose_mask_from_cores(cores: &[usize]) -> Result<usize, String> {
        let mut mask = 0usize;

        for &core in cores {
            let bit = 1usize
                .checked_shl(core as u32)
                .ok_or_else(|| format!("core index {core} out of range for affinity mask"))?;
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
        for bit in 0..usize::BITS as usize {
            if (mask & (1usize << bit)) != 0 {
                cpu_set.set(bit).map_err(|e| e.to_string())?;
            }
        }

        Ok(cpu_set)
    }

    fn mask_from_cpuset(set: &CpuSet) -> usize {
        let mut mask = 0usize;

        for bit in 0..usize::BITS as usize {
            if set.is_set(bit).unwrap_or(false) {
                mask |= 1usize << bit;
            }
        }

        mask
    }

    fn pid(pid: u32) -> Pid {
        Pid::from_raw(pid as i32)
    }

    fn set_priority_for_pid(pid: pid_t, priority: PriorityClass) -> Result<(), String> {
        match priority {
            PriorityClass::Realtime => {
                let params = sched_param { sched_priority: 50 };
                let ret = unsafe { sched_setscheduler(pid, SCHED_FIFO, &params) };
                if ret == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error().to_string())
                }
            }
            _ => {
                let nice = Self::to_nice(priority);
                let ret = unsafe { setpriority(PRIO_PROCESS, pid as u32, nice) };
                if ret == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error().to_string())
                }
            }
        }
    }

    fn to_nice(priority: PriorityClass) -> i32 {
        match priority {
            PriorityClass::Idle => 19,
            PriorityClass::BelowNormal => 10,
            PriorityClass::Normal => 0,
            PriorityClass::AboveNormal => -5,
            PriorityClass::High => -10,
            PriorityClass::Realtime => -20,
        }
    }

    fn spawn(target: &Path, args: &[String]) -> Result<Child, String> {
        let mut cmd = Command::new(target);
        if !args.is_empty() {
            cmd.args(args);
        }

        cmd.stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("spawn {:?} failed: {e}", target))
    }

    fn set_affinity(child: &Child, mask: usize) -> Result<(), String> {
        let cpu_set = Self::cpuset_from_mask(mask)?;
        sched_setaffinity(Self::pid(child.id()), &cpu_set).map_err(|e| e.to_string())
    }

    fn set_priority(child: &Child, priority: PriorityClass) -> Result<(), String> {
        Self::set_priority_for_pid(child.id() as pid_t, priority)
    }

    fn resolve_non_absolute_command(command: &str) -> Option<PathBuf> {
        if command.contains('/') {
            let path = PathBuf::from(command);
            return path.exists().then_some(path);
        }

        env::var_os("PATH").and_then(|path_var| {
            env::split_paths(&path_var)
                .map(|dir| dir.join(command))
                .find(|candidate| candidate.exists())
        })
    }

    fn resolve_command(command: &str) -> Result<PathBuf, String> {
        let path = PathBuf::from(command);
        if path.is_absolute() {
            if path.exists() {
                return Ok(path);
            }

            return Err(format!("executable '{}' does not exist", path.display()));
        }

        Self::resolve_non_absolute_command(command)
            .ok_or_else(|| format!("failed to resolve executable '{command}'"))
    }

    fn strip_exec_field_codes(token: &str) -> Option<String> {
        let mut chars = token.chars().peekable();
        let mut out = String::new();

        while let Some(ch) = chars.next() {
            if ch != '%' {
                out.push(ch);
                continue;
            }

            match chars.next() {
                Some('%') => out.push('%'),
                Some(code)
                    if matches!(
                        code,
                        'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm'
                    ) => {}
                Some(code) => {
                    out.push('%');
                    out.push(code);
                }
                None => out.push('%'),
            }
        }

        let cleaned = out.trim();
        (!cleaned.is_empty()).then(|| cleaned.to_string())
    }

    fn desktop_exec_tokens(exec: &str) -> Result<Vec<String>, String> {
        let raw = shlex::split(exec).unwrap_or_else(|| vec![exec.to_string()]);
        let tokens: Vec<String> = raw
            .into_iter()
            .filter_map(|token| Self::strip_exec_field_codes(&token))
            .collect();

        if tokens.is_empty() {
            return Err("Exec is empty".into());
        }

        Ok(tokens)
    }

    fn desktop_exec_to_target(exec: &str) -> Result<(PathBuf, Vec<String>), String> {
        let tokens = Self::desktop_exec_tokens(exec)?;

        let (command, args) = if tokens[0] == "env" {
            let mut index = 1usize;
            while index < tokens.len()
                && tokens[index].contains('=')
                && !tokens[index].starts_with('-')
            {
                index += 1;
            }

            if index >= tokens.len() {
                return Err("desktop Exec= only contained env assignments".into());
            }

            (tokens[index].clone(), tokens[index + 1..].to_vec())
        } else {
            (tokens[0].clone(), tokens[1..].to_vec())
        };

        let target = Self::resolve_command(&command)?;
        Ok((target, args))
    }

    fn parse_desktop_file(path: &Path) -> Result<(PathBuf, Vec<String>), String> {
        let content =
            fs::read_to_string(path).map_err(|e| format!("failed to read .desktop: {e}"))?;
        let entry = Self::parse_desktop_entry(&content)?;
        Self::desktop_exec_to_target(&entry.exec)
    }

    fn parse_desktop_entry(content: &str) -> Result<DesktopEntry, String> {
        let mut in_desktop_entry = false;
        let mut entry = DesktopEntry::default();

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                in_desktop_entry = line.eq_ignore_ascii_case("[Desktop Entry]");
                continue;
            }

            if !in_desktop_entry {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();

            match key.trim() {
                "Name" if entry.name.is_empty() => entry.name = value.to_string(),
                "Exec" if entry.exec.is_empty() => entry.exec = value.to_string(),
                "Type" => entry.entry_type = Some(value.to_string()),
                "Hidden" => entry.hidden = value.eq_ignore_ascii_case("true"),
                "NoDisplay" => entry.no_display = value.eq_ignore_ascii_case("true"),
                _ => {}
            }
        }

        if entry.exec.is_empty() {
            return Err("Exec= not found in .desktop".into());
        }

        if entry.name.is_empty() {
            entry.name = "Unknown".into();
        }

        Ok(entry)
    }

    fn proc_path(pid: u32, entry: &str) -> PathBuf {
        PathBuf::from("/proc").join(pid.to_string()).join(entry)
    }

    fn read_proc_stat(pid: u32) -> Result<(u32, String), String> {
        let stat = fs::read_to_string(Self::proc_path(pid, "stat"))
            .map_err(|e| format!("failed to read /proc/{pid}/stat: {e}"))?;
        let open = stat
            .find('(')
            .ok_or_else(|| format!("failed to parse /proc/{pid}/stat"))?;
        let close = stat
            .rfind(')')
            .ok_or_else(|| format!("failed to parse /proc/{pid}/stat"))?;

        let comm = stat[open + 1..close].to_string();
        let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        if rest.len() < 3 {
            return Err(format!("failed to parse parent pid for /proc/{pid}/stat"));
        }

        let parent_pid = rest[1]
            .parse::<u32>()
            .map_err(|e| format!("failed to parse parent pid for /proc/{pid}: {e}"))?;

        Ok((parent_pid, comm))
    }

    fn process_name_from_pid(pid: u32) -> String {
        Self::get_process_image_path(pid)
            .ok()
            .and_then(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_string())
            })
            .or_else(|| {
                fs::read_to_string(Self::proc_path(pid, "comm"))
                    .ok()
                    .map(|name| name.trim().to_string())
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| pid.to_string())
    }

    fn desktop_file_search_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();

        let mut push_dir = |path: PathBuf| {
            if seen.insert(path.clone()) {
                dirs.push(path);
            }
        };

        if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
            push_dir(PathBuf::from(data_home).join("applications"));
        } else if let Some(home) = env::var_os("HOME") {
            push_dir(PathBuf::from(home).join(".local/share/applications"));
        }

        if let Some(home) = env::var_os("HOME") {
            push_dir(PathBuf::from(home).join(".local/share/flatpak/exports/share/applications"));
        }

        let xdg_dirs =
            env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        for dir in env::split_paths(&xdg_dirs) {
            push_dir(dir.join("applications"));
        }

        push_dir(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

        dirs
    }

    fn collect_desktop_files(dir: &Path, output: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                Self::collect_desktop_files(&path, output);
                continue;
            }

            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("desktop"))
            {
                output.push(path);
            }
        }
    }

    fn find_desktop_file_by_name(file_name: &str) -> Option<PathBuf> {
        Self::desktop_file_search_dirs()
            .into_iter()
            .map(|dir| dir.join(file_name))
            .find(|path| path.exists())
    }

    pub fn get_process_affinity(pid: u32) -> Result<usize, String> {
        let set = sched_getaffinity(Self::pid(pid)).map_err(|e| e.to_string())?;
        Ok(Self::mask_from_cpuset(&set))
    }

    pub fn get_process_priority(pid: u32) -> Result<PriorityClass, String> {
        let pid = pid as pid_t;
        let policy = unsafe { sched_getscheduler(pid) };
        if policy == SCHED_FIFO || policy == SCHED_RR {
            return Ok(PriorityClass::Realtime);
        }

        errno::set_errno(errno::Errno(0));
        let nice = unsafe { getpriority(PRIO_PROCESS, pid as u32) };
        let err = errno::errno().0;
        if nice == -1 && err != 0 {
            return Err(io::Error::last_os_error().to_string());
        }

        Ok(match nice {
            n if n >= 15 => PriorityClass::Idle,
            n if n >= 5 => PriorityClass::BelowNormal,
            n if n >= -4 => PriorityClass::Normal,
            n if n >= -9 => PriorityClass::AboveNormal,
            _ => PriorityClass::High,
        })
    }

    pub fn set_process_affinity_by_pid(pid: u32, mask: usize) -> Result<(), String> {
        let cpu_set = Self::cpuset_from_mask(mask)?;
        sched_setaffinity(Self::pid(pid), &cpu_set).map_err(|e| e.to_string())
    }

    pub fn set_process_priority_by_pid(pid: u32, priority: PriorityClass) -> Result<(), String> {
        Self::set_priority_for_pid(pid as pid_t, priority)
    }

    pub fn set_current_process_priority(priority: PriorityClass) -> Result<(), String> {
        Self::set_priority_for_pid(0, priority)
    }

    pub fn parse_dropped_file(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        let path = fs::read_link(&file_path).unwrap_or(file_path.clone());

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("desktop"))
        {
            return Self::parse_desktop_file(&path);
        }

        Ok((path, Vec::new()))
    }

    pub fn run(
        file_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
    ) -> Result<u32, String> {
        let mask = Self::compose_mask_from_cores(cores)?;
        let child = Self::spawn(&file_path, &args)?;
        let pid = child.id();

        Self::set_affinity(&child, mask)?;
        Self::set_priority(&child, priority)?;

        Ok(pid)
    }

    pub fn snapshot_process_tree() -> Result<ProcessTree, String> {
        let mut parent_of = HashMap::new();
        let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut names = HashMap::new();

        for pid in Self::get_all_pids() {
            let Ok((parent_pid, _comm)) = Self::read_proc_stat(pid) else {
                continue;
            };

            parent_of.insert(pid, parent_pid);
            children_of.entry(parent_pid).or_default().push(pid);
            names.insert(pid, Self::process_name_from_pid(pid));
        }

        Ok(ProcessTree {
            parent_of,
            children_of,
            names,
        })
    }

    pub fn get_parent_pid(pid: u32) -> Option<u32> {
        Self::read_proc_stat(pid)
            .ok()
            .map(|(parent_pid, _)| parent_pid)
    }

    pub fn get_all_pids() -> Vec<u32> {
        let mut pids = Vec::new();

        let Ok(entries) = fs::read_dir("/proc") else {
            return pids;
        };

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if let Ok(pid) = file_name.parse::<u32>() {
                pids.push(pid);
            }
        }

        pids
    }

    pub fn find_pids_by_name(target_name: &str) -> Vec<u32> {
        if target_name.trim().is_empty() {
            return Vec::new();
        }

        let target_name = target_name
            .split('.')
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();

        Self::snapshot_process_tree()
            .map(|tree| {
                tree.names
                    .into_iter()
                    .filter_map(|(pid, name)| {
                        let process_name = name.split('.').next().unwrap_or("").to_lowercase();
                        (process_name == target_name).then_some(pid)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_all_process_names() -> Vec<(u32, String)> {
        Self::snapshot_process_tree()
            .map(|tree| tree.names.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn find_child_pids(parent: u32) -> Vec<u32> {
        Self::snapshot_process_tree()
            .ok()
            .and_then(|tree| tree.children_of.get(&parent).cloned())
            .unwrap_or_default()
    }

    pub fn find_all_descendants(parent_pid: u32, descendants: &mut Vec<u32>) {
        if let Ok(tree) = Self::snapshot_process_tree() {
            let mut visited: HashSet<u32> = descendants.iter().copied().collect();
            let mut stack = vec![parent_pid];

            while let Some(parent) = stack.pop() {
                if let Some(children) = tree.children_of.get(&parent) {
                    for &child in children {
                        if visited.insert(child) {
                            descendants.push(child);
                            stack.push(child);
                        }
                    }
                }
            }
        }
    }

    pub fn is_pid_live(pid: u32) -> bool {
        Self::proc_path(pid, "").is_dir()
    }

    pub fn get_process_image_path(pid: u32) -> Result<PathBuf, String> {
        fs::read_link(Self::proc_path(pid, "exe"))
            .map_err(|e| format!("failed to read /proc/{pid}/exe: {e}"))
    }

    pub fn focus_window_by_pid(_pid: u32) -> bool {
        false
    }

    pub fn get_program_path_for_uri(uri_scheme: &str) -> Result<PathBuf, String> {
        let output = Command::new("xdg-mime")
            .args([
                "query",
                "default",
                &format!("x-scheme-handler/{uri_scheme}"),
            ])
            .output()
            .map_err(|e| format!("failed to execute xdg-mime: {e}"))?;

        if !output.status.success() {
            return Err(format!("xdg-mime failed with status {}", output.status));
        }

        let desktop_file = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if desktop_file.is_empty() {
            return Err(format!(
                "no default application found for URI scheme '{uri_scheme}'"
            ));
        }

        let path = Self::find_desktop_file_by_name(&desktop_file)
            .ok_or_else(|| format!("desktop file '{desktop_file}' not found"))?;
        Self::parse_desktop_file(&path).map(|(path, _)| path)
    }

    pub fn get_cpu_model() -> String {
        fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .find(|line| line.starts_with("model name"))
                    .and_then(|line| line.split(':').nth(1))
                    .map(|value| value.trim().to_string())
            })
            .unwrap_or_else(|| "Unknown CPU".to_string())
    }

    pub fn list_supported_start_apps() -> Result<Vec<InstalledAppCatalogEntry>, String> {
        let mut desktop_files = Vec::new();
        for dir in Self::desktop_file_search_dirs() {
            Self::collect_desktop_files(&dir, &mut desktop_files);
        }

        let mut entries = Vec::new();
        let mut seen = HashSet::new();

        for desktop_file in desktop_files {
            let Ok(content) = fs::read_to_string(&desktop_file) else {
                continue;
            };
            let Ok(entry) = Self::parse_desktop_entry(&content) else {
                continue;
            };

            if entry.hidden || entry.no_display {
                continue;
            }

            if entry
                .entry_type
                .as_deref()
                .is_some_and(|value| !value.eq_ignore_ascii_case("Application"))
            {
                continue;
            }

            if Self::desktop_exec_to_target(&entry.exec).is_err() {
                continue;
            }

            let identity = format!(
                "{}|{}",
                entry.name.to_lowercase(),
                desktop_file.to_string_lossy().to_lowercase()
            );
            if !seen.insert(identity) {
                continue;
            }

            entries.push(InstalledAppCatalogEntry {
                name: entry.name,
                target: InstalledAppCatalogTarget::Path(desktop_file),
            });
        }

        entries.sort_by_cached_key(|entry| entry.name.to_lowercase());
        Ok(entries)
    }

    pub fn activate_application(_aumid: &str) -> Result<u32, String> {
        Err("Installed app activation is not supported on Linux".into())
    }

    pub fn get_process_app_user_model_id(_pid: u32) -> Result<Option<String>, String> {
        Ok(None)
    }

    pub fn resolve_installed_package_runtime_info(
        _aumid: &str,
    ) -> Result<InstalledPackageRuntimeInfo, String> {
        Err("Installed package metadata is not supported on Linux".into())
    }

    pub fn open_directory(path: &Path) -> Result<(), String> {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open directory '{}': {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::OS;

    #[test]
    fn test_strip_exec_field_codes_handles_desktop_placeholders() {
        assert_eq!(OS::strip_exec_field_codes("%u"), None);
        assert_eq!(
            OS::strip_exec_field_codes("--profile=%k"),
            Some("--profile=".to_string())
        );
        assert_eq!(
            OS::strip_exec_field_codes("100%%"),
            Some("100%".to_string())
        );
    }

    #[test]
    fn test_desktop_exec_tokens_drop_field_codes() {
        let tokens =
            OS::desktop_exec_tokens(r#"flatpak run app.id --arg %u --title="Hello %c""#).unwrap();
        assert_eq!(
            tokens,
            vec![
                "flatpak".to_string(),
                "run".to_string(),
                "app.id".to_string(),
                "--arg".to_string(),
                "--title=Hello".to_string(),
            ]
        );
    }
}
