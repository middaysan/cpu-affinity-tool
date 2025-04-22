// linux_process_ops.rs

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use libc::{
    pid_t, sched_param, sched_setscheduler, setpriority, PRIO_PROCESS, SCHED_FIFO, SCHED_RR,
};
use nix::sched::{sched_setaffinity, CpuSet};
use shlex;

use crate::PriorityClass;

pub struct OS;

impl OS {
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
        let mut cpu_set = CpuSet::new();
        for i in 0..usize::BITS {
            if (mask & (1 << i)) != 0 {
                cpu_set.set(i as usize).map_err(|e| e.to_string())?;
            }
        }
        sched_setaffinity(pid, &cpu_set).map_err(|e| e.to_string())
    }

    fn set_priority(child: &Child, p: PriorityClass) -> Result<(), String> {
        let pid = child.id() as pid_t;
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
    ) -> Result<(), String> {
        let mask = cores.iter().fold(0usize, |acc, &i| acc | (1 << i));
        let child = Self::spawn(&file_path, &args)?;
        Self::set_affinity(&child, mask)?;
        Self::set_priority(&child, priority)?;
        Ok(())
    }
}
