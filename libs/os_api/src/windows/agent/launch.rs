use super::{
    super::{
        common::{to_wide_z_str, transform_to_win_priority},
        launch::build_command_line,
        self_affinity,
    },
    identity::{self, handle},
};
use std::{
    collections::BTreeMap,
    ffi::c_void,
    fs::File,
    os::windows::io::{FromRawHandle, OwnedHandle},
    path::PathBuf,
};
use windows::{
    Win32::{
        Foundation::{FILETIME, HANDLE, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0},
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::{
            Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock},
            Pipes::CreatePipe,
            Threading::*,
        },
    },
    core::{PCWSTR, PWSTR},
};

#[derive(Clone)]
pub struct LaunchSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub cpus: Option<Vec<usize>>,
    pub priority: crate::PriorityClass,
    pub elevated: bool,
    pub capture: bool,
    pub environment: BTreeMap<String, String>,
}
pub struct ManagedProcess {
    pub pid: u32,
    pub creation_token: u64,
    pub elevated: bool,
    process: OwnedHandle,
    thread: Option<OwnedHandle>,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
}
impl ManagedProcess {
    /// Readers may be installed before resume. Dropping an unresumed child terminates it.
    pub fn resume(&mut self) -> Result<(), String> {
        let Some(thread) = &self.thread else {
            return Ok(());
        };
        if unsafe { ResumeThread(handle(thread)) } == u32::MAX {
            return Err(format!(
                "Cannot resume created PID {}: {}",
                self.pid,
                windows::core::Error::from_thread()
            ));
        }
        self.thread = None;
        Ok(())
    }
    pub fn exit_code(&self) -> Result<Option<u32>, String> {
        let wait = unsafe { WaitForSingleObject(handle(&self.process), 0) };
        if wait == windows::Win32::Foundation::WAIT_FAILED {
            return Err(windows::core::Error::from_thread().to_string());
        }
        if wait != WAIT_OBJECT_0 {
            return Ok(None);
        }
        let mut code = 0;
        unsafe { GetExitCodeProcess(handle(&self.process), &mut code) }
            .map_err(|e| e.to_string())?;
        Ok(Some(code))
    }
    pub fn terminate(&self) -> Result<(), String> {
        unsafe { TerminateProcess(handle(&self.process), 1) }.map_err(|e| e.to_string())
    }
    pub fn request_close(&self) -> Result<bool, String> {
        super::process::request_close(self.pid)
    }
}
impl Drop for ManagedProcess {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.terminate();
        }
    }
}
pub(super) fn requested_mask(cpus: Option<&[usize]>) -> Result<Option<usize>, String> {
    cpus.map(|cpus| {
        if cpus.is_empty() {
            return Err("CPU mask cannot be empty".into());
        }
        cpus.iter().try_fold(0usize, |mask, cpu| {
            1usize
                .checked_shl(u32::try_from(*cpu).unwrap_or(u32::MAX))
                .map(|bit| mask | bit)
                .ok_or_else(|| "CPU index is outside the representable processor group".into())
        })
    })
    .transpose()
}
fn pipe() -> Result<(File, OwnedHandle), String> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe { CreatePipe(&mut read, &mut write, Some(&attributes), 65536) }
        .map_err(|e| e.to_string())?;
    let read = unsafe { File::from_raw_handle(read.0) };
    let write = unsafe { OwnedHandle::from_raw_handle(write.0) };
    use std::os::windows::io::AsRawHandle;
    unsafe {
        windows::Win32::Foundation::SetHandleInformation(
            HANDLE(read.as_raw_handle()),
            HANDLE_FLAG_INHERIT.0,
            Default::default(),
        )
    }
    .map_err(|e| e.to_string())?;
    Ok((read, write))
}
fn null_handle(write: bool) -> Result<OwnedHandle, String> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let nul = to_wide_z_str("NUL");
    let handle = unsafe {
        CreateFileW(
            PCWSTR(nul.as_ptr()),
            if write {
                FILE_GENERIC_WRITE.0
            } else {
                FILE_GENERIC_READ.0
            },
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            Some(&attributes),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|e| e.to_string())?;
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}
struct Attributes {
    _buffer: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn handles(handles: &[HANDLE], parent: Option<&HANDLE>) -> Result<Self, String> {
        let count = if parent.is_some() { 2 } else { 1 };
        let mut bytes = 0;
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, count, None, &mut bytes);
        }
        if bytes == 0 || bytes > 65536 {
            return Err("Invalid process attribute size".into());
        }
        let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let pointer = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr().cast());
        unsafe { InitializeProcThreadAttributeList(Some(pointer), count, None, &mut bytes) }
            .map_err(|e| e.to_string())?;
        let attributes = Self {
            _buffer: buffer,
            pointer,
        };
        unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                std::mem::size_of_val(handles),
                None,
                None,
            )
        }
        .map_err(|e| e.to_string())?;
        if let Some(parent) = parent {
            unsafe {
                UpdateProcThreadAttribute(
                    pointer,
                    0,
                    PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize,
                    Some((parent as *const HANDLE).cast()),
                    size_of::<HANDLE>(),
                    None,
                    None,
                )
            }
            .map_err(|e| e.to_string())?;
        }
        Ok(attributes)
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}
struct ParentHandles<'a> {
    parent: Option<&'a OwnedHandle>,
    handles: Vec<HANDLE>,
}
impl<'a> ParentHandles<'a> {
    fn new(parent: Option<&'a OwnedHandle>, source: &[HANDLE]) -> Result<Self, String> {
        let mut result = Self {
            parent,
            handles: Vec::new(),
        };
        for source in source {
            let duplicated = if let Some(parent) = parent {
                let mut destination = HANDLE::default();
                unsafe {
                    windows::Win32::Foundation::DuplicateHandle(
                        GetCurrentProcess(),
                        *source,
                        handle(parent),
                        &mut destination,
                        0,
                        true,
                        windows::Win32::Foundation::DUPLICATE_SAME_ACCESS,
                    )
                }
                .map_err(|e| e.to_string())?;
                destination
            } else {
                *source
            };
            result.handles.push(duplicated);
        }
        Ok(result)
    }
}
impl Drop for ParentHandles<'_> {
    fn drop(&mut self) {
        if let Some(parent) = self.parent {
            for source in &self.handles {
                unsafe {
                    let mut local = HANDLE::default();
                    if windows::Win32::Foundation::DuplicateHandle(
                        handle(parent),
                        *source,
                        GetCurrentProcess(),
                        &mut local,
                        0,
                        false,
                        windows::Win32::Foundation::DUPLICATE_CLOSE_SOURCE
                            | windows::Win32::Foundation::DUPLICATE_SAME_ACCESS,
                    )
                    .is_ok()
                    {
                        let _ = windows::Win32::Foundation::CloseHandle(local);
                    }
                }
            }
        }
    }
}
struct Environment(*mut c_void);
impl Drop for Environment {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyEnvironmentBlock(self.0);
        }
    }
}
fn environment(token: HANDLE, patch: &BTreeMap<String, String>) -> Result<Vec<u16>, String> {
    if patch.len() > 32
        || patch.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 128
                || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                || value.contains('\0')
                || value.len() > 8192
        })
    {
        return Err("Invalid bounded environment overrides".into());
    }
    let mut block = std::ptr::null_mut();
    unsafe { CreateEnvironmentBlock(&mut block, Some(token), false) }.map_err(|e| e.to_string())?;
    let block = Environment(block);
    let mut variables = BTreeMap::new();
    let mut hidden = Vec::new();
    let mut offset = 0;
    loop {
        if offset >= 65536 {
            return Err("User environment exceeds launch budget".into());
        }
        let start = offset;
        unsafe {
            while *(block.0.cast::<u16>().add(offset)) != 0 {
                offset += 1;
                if offset >= 65536 {
                    return Err("User environment exceeds launch budget".into());
                }
            }
        }
        if offset == start {
            break;
        }
        let text = String::from_utf16(unsafe {
            std::slice::from_raw_parts(block.0.cast::<u16>().add(start), offset - start)
        })
        .map_err(|_| "Invalid Unicode user environment")?;
        if text.starts_with('=') {
            hidden.push(text);
        } else if let Some((key, value)) = text.split_once('=') {
            variables.insert(key.to_uppercase(), (key.to_owned(), value.to_owned()));
        }
        offset += 1;
    }
    for (key, value) in patch {
        variables.insert(key.to_uppercase(), (key.clone(), value.clone()));
    }
    let mut result = Vec::new();
    for text in hidden.into_iter().chain(
        variables
            .into_values()
            .map(|(key, value)| format!("{key}={value}")),
    ) {
        result.extend(text.encode_utf16());
        result.push(0);
    }
    result.push(0);
    if result.len() > 65536 {
        return Err("Environment exceeds 64 Ki UTF-16 units".into());
    }
    Ok(result)
}
/// Creates and verifies a suspended exact instance. The caller installs optional output readers, then calls resume.
pub fn launch_executable(spec: &LaunchSpec) -> Result<ManagedProcess, String> {
    if !spec.executable.is_absolute()
        || !spec
            .executable
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        || spec.executable.to_string_lossy().contains('\0')
        || spec.args.len() > 128
        || spec.args.iter().any(|arg| arg.contains('\0'))
        || spec
            .cwd
            .as_ref()
            .is_some_and(|cwd| !cwd.is_absolute() || cwd.to_string_lossy().contains('\0'))
    {
        return Err("Explicit absolute EXE/cwd and bounded NUL-free arguments are required".into());
    }
    let mask = requested_mask(spec.cpus.as_deref())?.or(self_affinity::original_mask());
    let context = identity::launch_context(spec.elevated)?;
    let parent = context.shell.as_ref().filter(|_| context.inherit_shell);
    let env = environment(handle(&context.token), &spec.environment)?;
    let mut command = to_wide_z_str(&build_command_line(&spec.executable, &spec.args));
    if command.len() > 32767 {
        return Err("Windows command line exceeds 32767 UTF-16 units".into());
    }
    let executable = to_wide_z_str(&spec.executable.to_string_lossy());
    let cwd = spec
        .cwd
        .as_ref()
        .map(|path| to_wide_z_str(&path.to_string_lossy()));
    let stdin = null_handle(false)?;
    let (stdout, stdout_write) = if spec.capture {
        let (read, write) = pipe()?;
        (Some(read), write)
    } else {
        (None, null_handle(true)?)
    };
    let (stderr, stderr_write) = if spec.capture {
        let (read, write) = pipe()?;
        (Some(read), write)
    } else {
        (None, null_handle(true)?)
    };
    let inherited = ParentHandles::new(
        parent,
        &[handle(&stdin), handle(&stdout_write), handle(&stderr_write)],
    )?;
    let parent_handle = parent.map(handle);
    let attributes = Attributes::handles(&inherited.handles, parent_handle.as_ref())?;
    let mut desktop = to_wide_z_str("winsta0\\default");
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.lpDesktop = PWSTR(desktop.as_mut_ptr());
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = inherited.handles[0];
    startup.StartupInfo.hStdOutput = inherited.handles[1];
    startup.StartupInfo.hStdError = inherited.handles[2];
    startup.lpAttributeList = attributes.pointer;
    let mut process = PROCESS_INFORMATION::default();
    let flags = CREATE_SUSPENDED
        | CREATE_UNICODE_ENVIRONMENT
        | CREATE_NO_WINDOW
        | EXTENDED_STARTUPINFO_PRESENT
        | transform_to_win_priority(spec.priority);
    let cwd = PCWSTR(
        cwd.as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr()),
    );
    let result = unsafe {
        CreateProcessW(
            PCWSTR(executable.as_ptr()),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            true,
            flags,
            Some(env.as_ptr().cast()),
            cwd,
            &startup.StartupInfo,
            &mut process,
        )
    };
    result.map_err(|e| {
        format!("Requested launch privilege could not be provided; no fallback was used: {e}")
    })?;
    let mut child = ManagedProcess {
        pid: process.dwProcessId,
        creation_token: 0,
        elevated: spec.elevated,
        process: unsafe { OwnedHandle::from_raw_handle(process.hProcess.0) },
        thread: Some(unsafe { OwnedHandle::from_raw_handle(process.hThread.0) }),
        stdout,
        stderr,
    };
    let verify: Result<(), String> = (|| unsafe {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        GetProcessTimes(
            handle(&child.process),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .map_err(|e| e.to_string())?;
        child.creation_token =
            ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
        let actual_token = identity::open_token(handle(&child.process))?;
        if identity::inspect(handle(&actual_token))? != context.identity {
            return Err("Created process token differs from the requested logon".into());
        }
        if let Some(mask) = mask {
            SetProcessAffinityMask(handle(&child.process), mask).map_err(|e| e.to_string())?;
            let mut actual = 0;
            let mut system = 0;
            GetProcessAffinityMask(handle(&child.process), &mut actual, &mut system)
                .map_err(|e| e.to_string())?;
            if actual != mask {
                return Err("Created process affinity failed readback".into());
            }
        }
        if GetPriorityClass(handle(&child.process)) != transform_to_win_priority(spec.priority).0 {
            return Err("Created process priority failed readback".into());
        }
        Ok(())
    })();
    if let Err(error) = verify {
        let cleanup = child.terminate();
        return Err(format!(
            "Created PID {} failed verification: {error}; suspended-child cleanup: {cleanup:?}",
            child.pid
        ));
    }
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_mask_rejects_empty_and_unrepresentable_cpu_sets() {
        assert!(requested_mask(Some(&[])).is_err());
        assert!(requested_mask(Some(&[usize::BITS as usize])).is_err());
        assert_eq!(requested_mask(Some(&[1, 3, 1])).unwrap(), Some(10));
        assert_eq!(requested_mask(None).unwrap(), None);
    }
    #[test]
    #[ignore = "Requires an unrestricted interactive Windows session; use the isolated native acceptance runner"]
    fn native_managed_child_captures_output_and_retains_exact_exit_identity() {
        use std::io::Read;
        let executable = std::env::current_exe().unwrap();
        let mut child = launch_executable(&LaunchSpec {
            executable,
            args: vec![
                "--exact".into(),
                "windows::agent::launch::tests::output_probe_child".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: None,
            cpus: None,
            priority: crate::PriorityClass::Normal,
            elevated: false,
            capture: true,
            environment: BTreeMap::from([("CPU_AFFINITY_MANAGED_OUTPUT_PROBE".into(), "1".into())]),
        })
        .unwrap();
        assert!(child.pid > 0 && child.creation_token > 0);
        assert!(
            super::super::ExactProcess::open(
                child.pid,
                child.creation_token + 1,
                true,
                false,
                false
            )
            .is_err()
        );
        let exact =
            super::super::ExactProcess::open(child.pid, child.creation_token, true, false, false)
                .unwrap();
        exact
            .set_priority(crate::PriorityClass::BelowNormal)
            .unwrap();
        assert_eq!(
            exact.state().unwrap().priority,
            crate::PriorityClass::BelowNormal
        );
        assert!(exact.set_priority(crate::PriorityClass::Realtime).is_err());
        exact.set_priority(crate::PriorityClass::Normal).unwrap();
        let original = exact.state().unwrap().affinity;
        let cpu = original.trailing_zeros() as usize;
        exact.set_affinity(&[cpu]).unwrap();
        assert_eq!(exact.state().unwrap().affinity, 1usize << cpu);
        child.resume().unwrap();
        let mut stdout = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut stdout)
            .unwrap();
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(
            stdout.contains("stdout-probe"),
            "stdout={stdout:?}; stderr={stderr:?}; exit={:?}",
            child.exit_code()
        );
        assert!(stderr.contains("stderr-probe"));
        for _ in 0..100 {
            if let Some(code) = child.exit_code().unwrap() {
                assert_eq!(code, 7);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("Controlled child did not exit");
    }
    #[test]
    #[ignore = "Controlled child fixture, run only by the managed-launch test"]
    fn output_probe_child() {
        if std::env::var("CPU_AFFINITY_MANAGED_OUTPUT_PROBE").as_deref() != Ok("1") {
            return;
        }
        println!("stdout-probe");
        eprintln!("stderr-probe");
        std::process::exit(7);
    }
    #[test]
    #[ignore = "Isolated native acceptance; changes only the test process scheduling and restores it"]
    fn native_child_does_not_inherit_own_cpu19_or_below_normal() {
        use super::super::super::OS;
        struct Restore {
            mask: usize,
            priority: crate::PriorityClass,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = OS::set_process_affinity_by_pid(std::process::id(), self.mask);
                let _ = OS::set_current_process_priority(self.priority);
            }
        }
        let restore = Restore {
            mask: OS::get_process_affinity(std::process::id()).unwrap(),
            priority: OS::get_process_priority(std::process::id()).unwrap(),
        };
        OS::pin_current_process_to_last_cpu().unwrap();
        OS::set_current_process_priority(crate::PriorityClass::BelowNormal).unwrap();
        let mut child = launch_executable(&LaunchSpec {
            executable: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "windows::agent::launch::tests::output_probe_child".into(),
                "--ignored".into(),
            ],
            cwd: None,
            cpus: None,
            priority: crate::PriorityClass::Normal,
            elevated: false,
            capture: false,
            environment: Default::default(),
        })
        .unwrap();
        assert_eq!(OS::get_process_affinity(child.pid).unwrap(), restore.mask);
        assert_eq!(
            OS::get_process_priority(child.pid).unwrap(),
            crate::PriorityClass::Normal
        );
        assert!(!child.elevated);
        child.resume().unwrap();
    }
}
