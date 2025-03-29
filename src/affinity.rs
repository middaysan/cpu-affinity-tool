use std::path::PathBuf;
use std::process::Command;
use std::os::windows::io::AsRawHandle;
use windows::Win32::System::Threading::SetProcessAffinityMask;
use windows::Win32::Foundation::HANDLE;
use parselnk::Lnk;
use shlex;

pub fn run_with_affinity(file_path: PathBuf, cores: &[usize]) {
    let affinity_mask: usize = cores.iter().map(|&i| 1 << i).sum();

    let (resolved, args) = if file_path.extension().and_then(|e| e.to_str()) == Some("lnk") {
        resolve_lnk_target_with_args(&file_path).unwrap_or((file_path.clone(), vec![]))
    } else {
        (file_path.clone(), vec![])
    };

    println!("Запуск: {}", resolved.display());

    let mut cmd = Command::new(&resolved);
    if !args.is_empty() {
        cmd.args(args);
    }

    match cmd.spawn() {
        Ok(child) => unsafe {
            let handle = HANDLE(child.as_raw_handle() as isize);
            if let Err(e) = SetProcessAffinityMask(handle, affinity_mask) {
                eprintln!("Не удалось установить маску affinity: {:?}", e);
            }
        },
        Err(e) => {
            eprintln!("Ошибка запуска: {:?}", e);
        }
    }
}

fn resolve_lnk_target_with_args(lnk_path: &PathBuf) -> Option<(PathBuf, Vec<String>)> {
    Lnk::try_from(lnk_path.as_path()).ok().and_then(|link| {
        let path = link.link_info.local_base_path.clone().map(PathBuf::from)?;
        let args = link.string_data.command_line_arguments.unwrap_or_default();
        let split_args = shlex::split(&args).unwrap_or_else(|| vec![args]);
        Some((path, split_args))
    })
}
