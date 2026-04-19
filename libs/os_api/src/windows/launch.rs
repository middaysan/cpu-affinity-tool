use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::ptr::null_mut;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, PROCESS_INFORMATION, ResumeThread, STARTUPINFOW,
    SetPriorityClass, SetProcessAffinityMask,
};
use windows::Win32::UI::Shell::{ApplicationActivationManager, IApplicationActivationManager};
use windows::core::{PCWSTR, PWSTR};

use crate::PriorityClass;

use super::OS;
use super::common::{ComGuard, HandleGuard, OsError, to_wide_z_str, transform_to_win_priority};

pub(super) fn quote_arg_windows(arg: &str) -> String {
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
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(ch);
                backslashes = 0;
            }
        }
    }

    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

pub(super) fn build_command_line(exe: &PathBuf, args: &[String]) -> String {
    let exe_s = exe.to_string_lossy();
    let mut parts = Vec::with_capacity(1 + args.len());
    parts.push(quote_arg_windows(&exe_s));
    for arg in args {
        parts.push(quote_arg_windows(arg));
    }
    parts.join(" ")
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
    let handle = HANDLE(child.as_raw_handle());
    unsafe { SetProcessAffinityMask(handle, mask) }
        .map_err(|e| format!("SetProcessAffinityMask failed: {}", e))
}

#[allow(dead_code)]
fn set_priority(child: &Child, priority: PriorityClass) -> Result<(), String> {
    let handle = HANDLE(child.as_raw_handle());
    unsafe { SetPriorityClass(handle, transform_to_win_priority(priority)) }
        .map_err(|e| format!("SetPriorityClass failed: {}", e))
}

impl OS {
    pub fn run(
        file_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
    ) -> Result<u32, String> {
        let mut mask = 0usize;
        for &core in cores {
            let bit = 1usize
                .checked_shl(core as u32)
                .ok_or_else(|| format!("core index {} out of range for affinity mask", core))?;
            mask |= bit;
        }

        if mask == 0 {
            return Err("affinity mask is empty".into());
        }

        (|| unsafe {
            let cmdline = build_command_line(&file_path, &args);
            let mut cmd_w = to_wide_z_str(&cmdline);

            let mut si: STARTUPINFOW = std::mem::zeroed();
            si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;

            let mut pi: PROCESS_INFORMATION = std::mem::zeroed();

            CreateProcessW(
                PCWSTR(null_mut()),
                Some(PWSTR(cmd_w.as_mut_ptr())),
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
            let _pg = HandleGuard(process);
            let _tg = HandleGuard(thread);

            SetProcessAffinityMask(process, mask)?;
            SetPriorityClass(process, transform_to_win_priority(priority))?;

            let _ = ResumeThread(thread);

            Ok(pi.dwProcessId)
        })()
        .map_err(|e: OsError| format!("run {:?} failed: {}", file_path, e))
    }

    pub fn activate_application(aumid: &str) -> Result<u32, String> {
        (|| unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(OsError::Win)?;
            let _com = ComGuard;

            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_INPROC_SERVER)?;

            let aumid_w = to_wide_z_str(aumid);
            let empty_args = [0u16];
            let process_id = manager.ActivateApplication(
                PCWSTR(aumid_w.as_ptr()),
                PCWSTR(empty_args.as_ptr()),
                windows::Win32::UI::Shell::ACTIVATEOPTIONS(0),
            )?;

            Ok(process_id)
        })()
        .map_err(|e: OsError| format!("activate_application {aumid:?} failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{build_command_line, quote_arg_windows};
    use crate::windows::shell::split_windows_args;

    #[test]
    fn test_quote_arg_windows_handles_embedded_quotes() {
        let arg = r#"say "hello""#;
        let quoted = quote_arg_windows(arg);
        let exe = PathBuf::from(r"C:\Tools\app.exe");
        let cmdline = format!("{} {}", quote_arg_windows(&exe.to_string_lossy()), quoted);
        let split = split_windows_args(&cmdline);
        assert_eq!(
            split,
            vec![exe.to_string_lossy().to_string(), arg.to_string()]
        );
    }

    #[test]
    fn test_build_command_line_roundtrips_spaces_quotes_and_trailing_backslashes() {
        let exe = PathBuf::from(r"C:\Program Files\Test App\app.exe");
        let args = vec![
            "simple".to_string(),
            "two words".to_string(),
            "embedded \"quote\"".to_string(),
            r#"C:\Path With Space\folder\"#.to_string(),
        ];

        let cmdline = build_command_line(&exe, &args);
        let split = split_windows_args(&cmdline);

        let mut expected = vec![exe.to_string_lossy().to_string()];
        expected.extend(args);
        assert_eq!(split, expected);
    }
}
