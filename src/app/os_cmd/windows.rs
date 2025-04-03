use std::path::PathBuf;
use std::process::{Command, Child, Stdio};
use std::os::windows::io::AsRawHandle;
use windows::Win32::System::Threading::SetProcessAffinityMask;
use windows::Win32::Foundation::HANDLE;
use parselnk::Lnk;
use shlex;

impl super::PlatformSystemCMD {
    pub fn run(file_path: PathBuf, cores: &[usize]) -> Result<(), String> {
        let affinity_mask = Self::build_affinity_mask(cores);
        let (target, args) = Self::resolve_target(file_path)?;
        let child = Self::spawn_process(&target, &args)?;
        Self::apply_affinity(&child, affinity_mask)?;

        Ok(())
    }

    fn build_affinity_mask(cores: &[usize]) -> usize {
        cores.iter().map(|&i| 1 << i).sum()
    }

    fn resolve_target(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String> {
        if file_path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("lnk"))
            .unwrap_or(false)
        {
            Self::resolve_lnk_target_with_args(&file_path)
                .ok_or_else(|| format!("Не удалось разрешить .lnk файл: {:?}", file_path))
        } else {
            Ok((file_path, Vec::new()))
        }
    }

    fn spawn_process(target: &PathBuf, args: &[String]) -> Result<Child, String> {
        let mut cmd = Command::new(target);
        if !args.is_empty() {
            cmd.args(args);
        }

        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        cmd.spawn().map_err(|e| format!("Не удалось запустить процесс {:?}: {:?}", target, e))
    }

    fn apply_affinity(child: &Child, affinity_mask: usize) -> Result<(), String> {
        unsafe {
            let handle = HANDLE(child.as_raw_handle() as *mut std::ffi::c_void);
            SetProcessAffinityMask(handle, affinity_mask)
                .map_err(|e| format!("Не удалось установить affinity mask: {:?}", e))
        }
    }

    fn resolve_lnk_target_with_args(lnk_path: &PathBuf) -> Option<(PathBuf, Vec<String>)> {
        let link = Lnk::try_from(lnk_path.as_path()).ok()?;
        let target = link.link_info.local_base_path.clone().map(PathBuf::from)?;
        let args = link.string_data.command_line_arguments.unwrap_or_default();
        let split_args = shlex::split(&args).unwrap_or_else(|| vec![args]);
        Some((target, split_args))
    }
}
