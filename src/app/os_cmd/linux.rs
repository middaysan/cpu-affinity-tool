// os_cmd_unix.rs
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::os::unix::process::CommandExt;
use libc::{cpu_set_t, CPU_SET, CPU_ZERO, sched_setaffinity, pid_t};
use std::mem::MaybeUninit;
use std::io::Error;

pub struct OsCmd;

impl super::OsCmdTrait for OsCmd {
    fn parse_dropped_file(file_path: PathBuf) -> Option<(PathBuf, Vec<String>)> {
        Some((file_path, Vec::new()))
    }

    fn run(file_path: PathBuf, args: Vec<String>, cores: &[usize], _priority: super::PriorityClass) -> Result<(), String> {
        let mut cmd = Command::new(&file_path);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        if !args.is_empty() {
            cmd.args(&args);
        }

        let child = cmd.spawn()
            .map_err(|e| format!("Failed to spawn process {:?}: {}", file_path, e))?;

        let pid = child.id() as pid_t;
        unsafe {
            let mut cpuset: cpu_set_t = MaybeUninit::zeroed().assume_init();
            CPU_ZERO(&mut cpuset);
            for &core in cores {
                CPU_SET(core, &mut cpuset);
            }

            let res = sched_setaffinity(pid, std::mem::size_of::<cpu_set_t>(), &cpuset);
            if res != 0 {
                return Err(format!("Failed to set affinity: {}", Error::last_os_error()));
            }
        }

        Ok(())
    }
}
