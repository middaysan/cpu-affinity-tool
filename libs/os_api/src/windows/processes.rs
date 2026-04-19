use std::collections::{HashMap, HashSet, VecDeque};
use std::mem::size_of;
use std::path::PathBuf;

use windows::Win32::Foundation::STILL_ACTIVE;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{K32EnumProcesses, K32GetModuleFileNameExW};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::OS;
use super::common::{HandleGuard, OsError, open_process};

/// One snapshot used for all process-tree operations.
pub struct ProcessTree {
    pub parent_of: HashMap<u32, u32>,
    pub children_of: HashMap<u32, Vec<u32>>,
    pub names: HashMap<u32, String>,
}

fn snapshot_process_tree_internal() -> Result<ProcessTree, OsError> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
        let _hg = HandleGuard(snap);

        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut pe).is_err() {
            return Err(OsError::Msg("Process32FirstW failed".into()));
        }

        let mut parent_of = HashMap::new();
        let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut names = HashMap::new();

        loop {
            let pid = pe.th32ProcessID;
            let ppid = pe.th32ParentProcessID;

            let len = pe
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(pe.szExeFile.len());
            let exe_file = String::from_utf16_lossy(&pe.szExeFile[..len]);

            if pid != 0 {
                parent_of.insert(pid, ppid);
                children_of.entry(ppid).or_default().push(pid);
                names.insert(pid, exe_file);
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
            names,
        })
    }
}

#[allow(dead_code)]
fn get_parent_pid(pid: u32) -> Option<u32> {
    let tree = snapshot_process_tree_internal().ok()?;
    tree.parent_of.get(&pid).copied()
}

#[allow(dead_code)]
fn get_all_pids() -> Vec<u32> {
    let mut cap = 1024usize;

    loop {
        let mut buf = vec![0u32; cap];
        let mut needed: u32 = 0;

        let ok = unsafe { K32EnumProcesses(buf.as_mut_ptr(), (buf.len() * 4) as u32, &mut needed) }
            .as_bool();

        if !ok {
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

#[allow(dead_code)]
fn find_child_pids(parent: u32) -> Vec<u32> {
    match snapshot_process_tree_internal() {
        Ok(tree) => tree.children_of.get(&parent).cloned().unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

impl OS {
    pub fn snapshot_process_tree() -> Result<ProcessTree, String> {
        snapshot_process_tree_internal().map_err(|e| e.to_string())
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

    /// Finds all descendant processes of a given parent process.
    ///
    /// Preserves original behavior: doesn't add duplicates if `descendants` already contains some PIDs.
    pub fn find_all_descendants(parent_pid: u32, descendants: &mut Vec<u32>) {
        if let Ok(tree) = snapshot_process_tree_internal() {
            Self::find_all_descendants_with_tree(parent_pid, descendants, &tree);
        }
    }

    /// Finds all descendant processes using a pre-captured process tree snapshot.
    pub fn find_all_descendants_with_tree(
        parent_pid: u32,
        descendants: &mut Vec<u32>,
        tree: &ProcessTree,
    ) {
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

    pub fn is_pid_live(pid: u32) -> bool {
        unsafe {
            let handle = match open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)
                .or_else(|_| open_process(pid, PROCESS_QUERY_INFORMATION))
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

    pub fn get_process_image_path(pid: u32) -> Result<PathBuf, String> {
        (|| unsafe {
            let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)
                .or_else(|_| open_process(pid, PROCESS_QUERY_INFORMATION))?;
            let _hg = HandleGuard(handle);

            let mut buffer = [0u16; 2048];
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

#[cfg(test)]
mod tests {
    use super::{OS, ProcessTree};
    use std::collections::HashMap;

    #[test]
    fn test_find_all_descendants_with_tree_preserves_no_duplicate_semantics() {
        let tree = ProcessTree {
            parent_of: HashMap::from([(2, 1), (3, 1), (4, 2), (5, 4)]),
            children_of: HashMap::from([(1, vec![2, 3]), (2, vec![4]), (4, vec![5])]),
            names: HashMap::new(),
        };

        let mut descendants = vec![4];
        OS::find_all_descendants_with_tree(1, &mut descendants, &tree);

        assert_eq!(descendants, vec![4, 2, 3, 5]);
    }
}
