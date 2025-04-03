use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::os::unix::process::CommandExt;
use libc::{cpu_set_t, CPU_SET, CPU_ZERO, sched_setaffinity, pid_t};
use std::mem::MaybeUninit;
use std::io::Error;

impl PlatformSystemCMD {
    pub fn run(file_path: PathBuf, cores: &[usize]) -> Result<(), String> {
        let mut cmd = Command::new(&file_path);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

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

        println!("Affinity set. Process started: {:?}", file_path);
        Ok(())
    }
}
