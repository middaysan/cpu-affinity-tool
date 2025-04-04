// os_cmd_windows.rs
use std::path::PathBuf;
use std::process::{Command, Child, Stdio};
use std::os::windows::io::AsRawHandle;
use windows::Win32::System::Threading::{
    SetProcessAffinityMask, SetPriorityClass, IDLE_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
    NORMAL_PRIORITY_CLASS, ABOVE_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS, REALTIME_PRIORITY_CLASS,
};
use windows::Win32::Foundation::HANDLE;
use parselnk::Lnk;
use shlex;


pub struct OsCmd;

impl super::OsCmdTrait for OsCmd {
    fn parse_dropped_file(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        if file_path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("lnk"))
            .unwrap_or(false)
        {
            resolve_target_with_args(&file_path)
                .map_err(|e| format!("Failed to parse LNK file: {:?}: {}", file_path, e))
        } else {
            Ok((file_path, Vec::new()))
        }
    }

    fn run(file_path: PathBuf, args: Vec<String>, cores: &[usize], priority: super::PriorityClass) -> Result<(), String> {
        let affinity_mask = cores.iter().map(|&i| 1 << i).sum();
        let child = spawn_process(&file_path, &args)?;
        apply_affinity(&child, affinity_mask)?;
        set_process_priority(&child, priority)?;
        Ok(())
    }
}

fn resolve_target_with_args(lnk_path: &PathBuf) -> Result<(PathBuf, Vec<String>), String> {
    // Пытаемся открыть ярлык
    let link = Lnk::try_from(lnk_path.as_path()).map_err(|e| format!("Failed to open LNK file {:?}: {}", lnk_path, e)).unwrap();
    
    // 1. Пытаемся взять путь из link_info.local_base_path
    let target = if let Some(ref path) = link.link_info.local_base_path {
        PathBuf::from(path)
    } else if let Some(ref rel_path) = link.string_data.relative_path {
        if rel_path.is_absolute() {
            PathBuf::from(rel_path)
        } else if let Some(ref work_dir) = link.string_data.working_dir {
            PathBuf::from(work_dir).join(rel_path)
        } else {
            PathBuf::from(rel_path)
        }
    } else {
        return Err(format!(
            "The LNK file {:?} does not specify an extractable target path (LinkTargetIdList parsing not implemented)",
            lnk_path
        ));
    };

    // Обработка аргументов
    let args = link.string_data.command_line_arguments.unwrap_or_default();
    let split_args = shlex::split(&args).unwrap_or_else(|| vec![args]);

    Ok((target, split_args))
}

fn spawn_process(target: &PathBuf, args: &[String]) -> Result<Child, String> {
    let mut cmd = Command::new(target);
    if !args.is_empty() {
        cmd.args(args);
    }
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.spawn().map_err(|e| format!("Failed to start process {:?}: {:?}", target, e))
}

fn apply_affinity(child: &Child, affinity_mask: usize) -> Result<(), String> {
    unsafe {
        let handle = HANDLE(child.as_raw_handle() as *mut std::ffi::c_void);
        SetProcessAffinityMask(handle, affinity_mask)
            .map_err(|e| format!("Failed to set affinity mask: {:?}", e))
    }
}

fn set_process_priority(child: &Child, priority: super::PriorityClass) -> Result<(), String> {
    let priority_value = match priority {
        super::PriorityClass::Idle => IDLE_PRIORITY_CLASS,
        super::PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
        super::PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
        super::PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        super::PriorityClass::High => HIGH_PRIORITY_CLASS,
        super::PriorityClass::Realtime => REALTIME_PRIORITY_CLASS,
    };
    unsafe {
        let handle = HANDLE(child.as_raw_handle() as *mut std::ffi::c_void);
        let result = SetPriorityClass(handle, priority_value);
        if result.is_ok() {
            Ok(())
        } else {
            Err("Failed to set process priority".into())
        }
    }
}
