use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_BROKEN_PIPE,
    ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_NO_DATA,
    ERROR_OPERATION_ABORTED, ERROR_PATH_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    GetLastError, HANDLE, HLOCAL, LocalFree, WAIT_TIMEOUT, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, RevertToSelf, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_NONE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_EFFECTIVE_ONLY, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResultEx, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientSessionId,
    GetNamedPipeServerSessionId, ImpersonateNamedPipeClient, NAMED_PIPE_MODE,
    PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
    SetNamedPipeHandleState,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThread,
    OpenMutexW, OpenProcessToken, OpenThreadToken, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows::core::{PCWSTR, PWSTR};

use super::OS;
use super::common::{HandleGuard, to_wide_z_str};

const MAX_FRAME_BYTES: usize = 4096;
const PIPE_TIMEOUT_MS: u32 = 5000;
const PIPE_MAX_INSTANCES: u32 = 1;
const SERVER_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const SERVER_READ_TIMEOUT: Duration = Duration::from_secs(1);
const CANCEL_COMPLETION_TIMEOUT_MS: u32 = 1000;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const OPEN_CLIENT_TOKEN_AS_SELF: bool = true;

pub type LocalIpcWake = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct LocalIpcEndpoint {
    pub pipe_name: String,
    pub mutex_name: String,
}

pub struct LocalIpcRequest {
    pub request: Vec<u8>,
    pub response_tx: Sender<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalIpcClientError {
    NoServer,
    ServerNotReady,
    Timeout,
    SecurityRejected(String),
    Io(String),
}

pub struct LocalIpcGuard {
    _handle: HandleGuard,
}

pub struct LocalIpcServer {
    endpoint: LocalIpcEndpoint,
    request_rx: Receiver<LocalIpcRequest>,
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LocalIpcServer {
    pub fn try_recv(&self) -> Result<LocalIpcRequest, mpsc::TryRecvError> {
        self.request_rx.try_recv()
    }
}

impl Drop for LocalIpcServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = OS::send_local_ipc_request(&self.endpoint, b"shutdown", Duration::from_millis(50));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl OS {
    pub fn local_ipc_endpoint() -> Result<LocalIpcEndpoint, String> {
        let sid = current_user_sid_string()?;
        let session_id = current_session_id()?;
        let exe = std::env::current_exe()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|_| "cpu-affinity-tool".to_string());
        let identity = stable_hash(&format!("{sid}:{session_id}:{exe}"));

        Ok(LocalIpcEndpoint {
            pipe_name: format!(r"\\.\pipe\cpu-affinity-tool-{identity}"),
            mutex_name: format!(r"Local\cpu-affinity-tool-{identity}-primary"),
        })
    }

    pub fn try_claim_local_ipc_primary_guard(
        endpoint: &LocalIpcEndpoint,
    ) -> Result<Option<LocalIpcGuard>, String> {
        let mut security = SecurityAttributes::new_for_current_user()?;
        let name = to_wide_z_str(&endpoint.mutex_name);
        let handle =
            unsafe { CreateMutexW(Some(security.as_mut_ptr()), true, PCWSTR(name.as_ptr())) }
                .map_err(|err| format!("failed to create primary guard mutex: {err}"))?;

        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already_exists {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Ok(None)
        } else {
            Ok(Some(LocalIpcGuard {
                _handle: HandleGuard(handle),
            }))
        }
    }

    pub fn local_ipc_primary_guard_exists(endpoint: &LocalIpcEndpoint) -> Result<bool, String> {
        let name = to_wide_z_str(&endpoint.mutex_name);
        match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) } {
            Ok(handle) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    pub fn start_local_ipc_server(endpoint: &LocalIpcEndpoint) -> Result<LocalIpcServer, String> {
        Self::start_local_ipc_server_with_wake(endpoint, None)
    }

    pub fn start_local_ipc_server_with_wake(
        endpoint: &LocalIpcEndpoint,
        wake: Option<LocalIpcWake>,
    ) -> Result<LocalIpcServer, String> {
        let (request_tx, request_rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let endpoint = endpoint.clone();
        let thread_endpoint = endpoint.clone();
        let initial_pipe = create_server_pipe(&endpoint)?;
        let initial_pipe_handle = initial_pipe.0.0 as isize;
        std::mem::forget(initial_pipe);

        let thread = match thread::Builder::new()
            .name("cpu-affinity-tool-ipc".to_string())
            .spawn(move || {
                let initial_pipe = HandleGuard(HANDLE(initial_pipe_handle as *mut c_void));
                server_loop(
                    thread_endpoint,
                    request_tx,
                    thread_shutdown,
                    initial_pipe,
                    wake,
                );
            }) {
            Ok(thread) => thread,
            Err(err) => {
                unsafe {
                    let _ = CloseHandle(HANDLE(initial_pipe_handle as *mut c_void));
                }
                return Err(format!("failed to spawn local IPC server: {err}"));
            }
        };

        Ok(LocalIpcServer {
            endpoint,
            request_rx,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn send_local_ipc_request(
        endpoint: &LocalIpcEndpoint,
        request: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, LocalIpcClientError> {
        if request.len() > MAX_FRAME_BYTES {
            return Err(LocalIpcClientError::Io(
                "request frame is too large".to_string(),
            ));
        }

        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(LocalIpcClientError::Timeout);
            }
            match send_local_ipc_request_once(
                endpoint,
                request,
                deadline.saturating_duration_since(now),
            ) {
                Err(LocalIpcClientError::ServerNotReady) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                result => return result,
            }
        }
    }
}

fn server_loop(
    _endpoint: LocalIpcEndpoint,
    request_tx: Sender<LocalIpcRequest>,
    shutdown: Arc<AtomicBool>,
    listener: HandleGuard,
    wake: Option<LocalIpcWake>,
) {
    while !shutdown.load(Ordering::SeqCst) {
        match connect_pipe(listener.0, &shutdown) {
            Ok(()) => {}
            Err(PipeIoError::Shutdown) => break,
            Err(_) => {
                unsafe {
                    let _ = DisconnectNamedPipe(listener.0);
                }
                continue;
            }
        }

        if !same_session_client(listener.0) {
            unsafe {
                let _ = DisconnectNamedPipe(listener.0);
            }
            continue;
        }

        let mut buffer = vec![0u8; MAX_FRAME_BYTES];
        let bytes_read = match read_pipe(
            listener.0,
            &mut buffer,
            SERVER_READ_TIMEOUT,
            Some(&shutdown),
        ) {
            Ok(bytes_read) => bytes_read,
            Err(PipeIoError::Shutdown) => {
                unsafe {
                    let _ = DisconnectNamedPipe(listener.0);
                }
                break;
            }
            Err(_) => {
                unsafe {
                    let _ = DisconnectNamedPipe(listener.0);
                }
                continue;
            }
        };

        if bytes_read == 0 {
            unsafe {
                let _ = DisconnectNamedPipe(listener.0);
            }
            continue;
        }

        if !same_user_client(listener.0) {
            unsafe {
                let _ = DisconnectNamedPipe(listener.0);
            }
            continue;
        }

        buffer.truncate(bytes_read as usize);
        if shutdown.load(Ordering::SeqCst) {
            unsafe {
                let _ = DisconnectNamedPipe(listener.0);
            }
            break;
        }

        let (response_tx, response_rx) = mpsc::channel();
        if request_tx
            .send(LocalIpcRequest {
                request: buffer,
                response_tx,
            })
            .is_err()
        {
            break;
        }
        if let Some(wake) = &wake {
            wake();
        }

        let Some(response) = receive_response(&response_rx, &shutdown) else {
            unsafe {
                let _ = DisconnectNamedPipe(listener.0);
            }
            break;
        };
        let _ = write_pipe(
            listener.0,
            response.as_slice(),
            RESPONSE_TIMEOUT,
            Some(&shutdown),
        );
        unsafe {
            let _ = DisconnectNamedPipe(listener.0);
        }
    }
}

fn receive_response(response_rx: &Receiver<Vec<u8>>, shutdown: &AtomicBool) -> Option<Vec<u8>> {
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return None;
        }

        let now = Instant::now();
        if now >= deadline {
            return Some(b"{\"version\":1,\"code\":\"timeout\"}".to_vec());
        }

        let wait = std::cmp::min(Duration::from_millis(25), deadline - now);
        match response_rx.recv_timeout(wait) {
            Ok(response) => return Some(response),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Some(b"{\"version\":1,\"code\":\"timeout\"}".to_vec());
            }
        }
    }
}

fn create_server_pipe(endpoint: &LocalIpcEndpoint) -> Result<HandleGuard, String> {
    let mut security = SecurityAttributes::new_for_current_user()?;
    let name = to_wide_z_str(&endpoint.pipe_name);
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            server_open_mode(),
            server_pipe_mode(),
            PIPE_MAX_INSTANCES,
            MAX_FRAME_BYTES as u32,
            MAX_FRAME_BYTES as u32,
            PIPE_TIMEOUT_MS,
            Some(security.as_mut_ptr()),
        )
    };

    if handle.is_invalid() {
        Err(format!("failed to create local IPC pipe: {:?}", unsafe {
            GetLastError()
        }))
    } else {
        Ok(HandleGuard(handle))
    }
}

fn server_open_mode() -> FILE_FLAGS_AND_ATTRIBUTES {
    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE
}

fn server_pipe_mode() -> NAMED_PIPE_MODE {
    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS
}

fn client_open_flags() -> FILE_FLAGS_AND_ATTRIBUTES {
    FILE_ATTRIBUTE_NORMAL
        | FILE_FLAG_OVERLAPPED
        | SECURITY_IDENTIFICATION
        | SECURITY_EFFECTIVE_ONLY
        | SECURITY_SQOS_PRESENT
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PipeIoError {
    Timeout,
    Shutdown,
    BrokenPipe,
    FrameTooLarge,
    Io(String),
}

struct OverlappedOperation {
    overlapped: Box<OVERLAPPED>,
    event: HandleGuard,
}

impl OverlappedOperation {
    fn new() -> Result<Self, PipeIoError> {
        let event = unsafe { CreateEventW(None, true, false, None) }
            .map_err(|err| PipeIoError::Io(format!("failed to create IPC event: {err}")))?;
        let event = HandleGuard(event);
        let mut overlapped = Box::<OVERLAPPED>::default();
        overlapped.hEvent = event.0;
        Ok(Self { overlapped, event })
    }

    fn as_mut_ptr(&mut self) -> *mut OVERLAPPED {
        &mut *self.overlapped
    }

    fn finish_now(&mut self, handle: HANDLE) -> Result<u32, PipeIoError> {
        let mut bytes = 0u32;
        unsafe { GetOverlappedResultEx(handle, &*self.overlapped, &mut bytes, 0, false) }
            .map_err(|_| pipe_io_error("overlapped IPC operation failed"))?;
        Ok(bytes)
    }

    fn wait(
        self,
        handle: HANDLE,
        timeout: Duration,
        shutdown: Option<&AtomicBool>,
    ) -> Result<u32, PipeIoError> {
        let deadline = Instant::now() + timeout;
        loop {
            if shutdown.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                return self.cancel_and_finish(handle, PipeIoError::Shutdown);
            }

            let now = Instant::now();
            if now >= deadline {
                return self.cancel_and_finish(handle, PipeIoError::Timeout);
            }

            let wait = std::cmp::min(Duration::from_millis(25), deadline - now);
            let mut bytes = 0u32;
            match unsafe {
                GetOverlappedResultEx(
                    handle,
                    &*self.overlapped,
                    &mut bytes,
                    wait.as_millis().try_into().unwrap_or(u32::MAX),
                    false,
                )
            } {
                Ok(()) => return Ok(bytes),
                Err(_) => {
                    let err = unsafe { GetLastError() };
                    if is_wait_timeout(err) {
                        continue;
                    }
                    return Err(pipe_io_error_from_code(
                        err,
                        "overlapped IPC operation failed",
                    ));
                }
            }
        }
    }

    fn cancel_and_finish(self, handle: HANDLE, result: PipeIoError) -> Result<u32, PipeIoError> {
        let this = self;
        let _ = unsafe { CancelIoEx(handle, Some(&*this.overlapped)) };
        let mut bytes = 0u32;
        match unsafe {
            GetOverlappedResultEx(
                handle,
                &*this.overlapped,
                &mut bytes,
                CANCEL_COMPLETION_TIMEOUT_MS,
                false,
            )
        } {
            Ok(()) => Err(result),
            Err(_) => {
                let err = unsafe { GetLastError() };
                if err == ERROR_OPERATION_ABORTED || is_wait_timeout(err) {
                    if is_wait_timeout(err) {
                        let _ = Box::leak(this.overlapped);
                        std::mem::forget(this.event);
                    }
                    Err(result)
                } else {
                    Err(pipe_io_error_from_code(
                        err,
                        "failed to cancel overlapped IPC operation",
                    ))
                }
            }
        }
    }
}

fn connect_pipe(handle: HANDLE, shutdown: &AtomicBool) -> Result<(), PipeIoError> {
    let mut operation = OverlappedOperation::new()?;
    match unsafe { ConnectNamedPipe(handle, Some(operation.as_mut_ptr())) } {
        Ok(()) => operation.finish_now(handle).map(|_| ()),
        Err(_) => match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => Ok(()),
            ERROR_IO_PENDING => operation
                .wait(handle, SERVER_CONNECT_TIMEOUT, Some(shutdown))
                .map(|_| ()),
            err => Err(pipe_io_error_from_code(err, "failed to connect IPC pipe")),
        },
    }
}

fn read_pipe(
    handle: HANDLE,
    buffer: &mut [u8],
    timeout: Duration,
    shutdown: Option<&AtomicBool>,
) -> Result<u32, PipeIoError> {
    let mut operation = OverlappedOperation::new()?;
    match unsafe { ReadFile(handle, Some(buffer), None, Some(operation.as_mut_ptr())) } {
        Ok(()) => operation.finish_now(handle),
        Err(_) => match unsafe { GetLastError() } {
            ERROR_IO_PENDING => operation.wait(handle, timeout, shutdown),
            err => Err(pipe_io_error_from_code(err, "failed to read IPC pipe")),
        },
    }
}

fn write_pipe(
    handle: HANDLE,
    bytes: &[u8],
    timeout: Duration,
    shutdown: Option<&AtomicBool>,
) -> Result<(), PipeIoError> {
    let mut operation = OverlappedOperation::new()?;
    let expected = bytes.len() as u32;
    let written =
        match unsafe { WriteFile(handle, Some(bytes), None, Some(operation.as_mut_ptr())) } {
            Ok(()) => operation.finish_now(handle)?,
            Err(_) => match unsafe { GetLastError() } {
                ERROR_IO_PENDING => operation.wait(handle, timeout, shutdown)?,
                err => return Err(pipe_io_error_from_code(err, "failed to write IPC pipe")),
            },
        };

    if written == expected {
        Ok(())
    } else {
        Err(PipeIoError::Io(format!(
            "short IPC pipe write: wrote {written} of {expected} bytes"
        )))
    }
}

fn pipe_io_error(context: &str) -> PipeIoError {
    pipe_io_error_from_code(unsafe { GetLastError() }, context)
}

fn pipe_io_error_from_code(err: WIN32_ERROR, context: &str) -> PipeIoError {
    match err {
        ERROR_BROKEN_PIPE | ERROR_NO_DATA | ERROR_OPERATION_ABORTED => PipeIoError::BrokenPipe,
        ERROR_MORE_DATA => PipeIoError::FrameTooLarge,
        _ if is_wait_timeout(err) => PipeIoError::Timeout,
        _ => PipeIoError::Io(format!("{context}: {err:?}")),
    }
}

fn is_wait_timeout(err: WIN32_ERROR) -> bool {
    err.0 == WAIT_TIMEOUT.0
}

fn same_session_client(handle: HANDLE) -> bool {
    let Ok(current_session) = current_session_id() else {
        return false;
    };
    let mut client_session = 0u32;
    unsafe { GetNamedPipeClientSessionId(handle, &mut client_session) }.is_ok()
        && client_session == current_session
}

fn same_user_client(handle: HANDLE) -> bool {
    let Ok(current_sid) = current_user_sid_string() else {
        return false;
    };
    if unsafe { ImpersonateNamedPipeClient(handle) }.is_err() {
        return false;
    }

    let _revert = ImpersonationRevertGuard;
    let mut token = HANDLE::default();
    if unsafe {
        OpenThreadToken(
            GetCurrentThread(),
            TOKEN_QUERY,
            OPEN_CLIENT_TOKEN_AS_SELF,
            &mut token,
        )
    }
    .is_err()
    {
        return false;
    }

    let token = HandleGuard(token);
    token_user_sid_string(token.0).is_ok_and(|client_sid| client_sid == current_sid)
}

fn send_local_ipc_request_once(
    endpoint: &LocalIpcEndpoint,
    request: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, LocalIpcClientError> {
    let name = to_wide_z_str(&endpoint.pipe_name);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            client_open_flags(),
            None,
        )
    };

    let handle = match handle {
        Ok(handle) => HandleGuard(handle),
        Err(_) => {
            return match unsafe { GetLastError() } {
                ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Err(LocalIpcClientError::NoServer),
                err => Err(client_open_error_from_code(
                    err,
                    format!("failed to open local IPC pipe: {err:?}"),
                )),
            };
        }
    };

    if !same_session_server(handle.0) {
        return Err(LocalIpcClientError::SecurityRejected(
            "server session did not match current session".to_string(),
        ));
    }

    let mut mode = PIPE_READMODE_MESSAGE;
    let _ = unsafe { SetNamedPipeHandleState(handle.0, Some(&mut mode), None, None) };

    let started = Instant::now();
    write_pipe(handle.0, request, timeout, None).map_err(client_error_from_pipe_io)?;
    let elapsed = started.elapsed();
    let remaining = match timeout.checked_sub(elapsed) {
        Some(remaining) if !remaining.is_zero() => remaining,
        _ => return Err(LocalIpcClientError::Timeout),
    };

    let mut buffer = vec![0u8; MAX_FRAME_BYTES];
    let bytes_read = read_pipe(handle.0, &mut buffer, remaining, None)
        .map_err(client_response_error_from_pipe_io)?;
    buffer.truncate(bytes_read as usize);

    Ok(buffer)
}

fn client_error_from_pipe_io(err: PipeIoError) -> LocalIpcClientError {
    match err {
        PipeIoError::Timeout => LocalIpcClientError::Timeout,
        PipeIoError::BrokenPipe => LocalIpcClientError::ServerNotReady,
        PipeIoError::FrameTooLarge => {
            LocalIpcClientError::Io("local IPC transport frame exceeded read limit".to_string())
        }
        PipeIoError::Shutdown => LocalIpcClientError::ServerNotReady,
        PipeIoError::Io(message) => LocalIpcClientError::Io(message),
    }
}

fn client_open_error_from_code(err: WIN32_ERROR, detail: String) -> LocalIpcClientError {
    match err {
        ERROR_PIPE_BUSY | ERROR_NO_DATA | ERROR_BROKEN_PIPE => LocalIpcClientError::ServerNotReady,
        ERROR_ACCESS_DENIED => LocalIpcClientError::SecurityRejected(
            "access denied while opening local IPC pipe".to_string(),
        ),
        _ => LocalIpcClientError::Io(detail),
    }
}

fn client_response_error_from_pipe_io(err: PipeIoError) -> LocalIpcClientError {
    match err {
        PipeIoError::BrokenPipe => LocalIpcClientError::SecurityRejected(
            "local IPC server closed the pipe before sending a response".to_string(),
        ),
        other => client_error_from_pipe_io(other),
    }
}

fn same_session_server(handle: HANDLE) -> bool {
    let Ok(current_session) = current_session_id() else {
        return false;
    };
    let mut server_session = 0u32;
    unsafe { GetNamedPipeServerSessionId(handle, &mut server_session) }.is_ok()
        && server_session == current_session
}

fn current_session_id() -> Result<u32, String> {
    let mut session_id = 0u32;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id) }
        .map_err(|err| format!("failed to resolve current session id: {err}"))?;
    Ok(session_id)
}

fn current_user_sid_string() -> Result<String, String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|err| format!("failed to open process token: {err}"))?;
    let token = HandleGuard(token);

    token_user_sid_string(token.0)
}

fn token_user_sid_string(token: HANDLE) -> Result<String, String> {
    let mut needed = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    if needed == 0 {
        return Err("failed to query token user size".to_string());
    }

    let mut buffer = vec![0u8; needed as usize];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr() as *mut c_void),
            needed,
            &mut needed,
        )
    }
    .map_err(|err| format!("failed to query token user: {err}"))?;

    let token_user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let mut sid_string = PWSTR::null();
    unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string) }
        .map_err(|err| format!("failed to stringify user SID: {err}"))?;

    let result = pwstr_to_string(sid_string);
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sid_string.0 as *mut c_void)));
    }
    result.ok_or_else(|| "failed to decode user SID".to_string())
}

struct ImpersonationRevertGuard;

impl Drop for ImpersonationRevertGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = RevertToSelf();
        }
    }
}

fn pwstr_to_string(value: PWSTR) -> Option<String> {
    if value.is_null() {
        return None;
    }

    let mut len = 0usize;
    unsafe {
        while *value.0.add(len) != 0 {
            len += 1;
        }
        Some(String::from_utf16_lossy(std::slice::from_raw_parts(
            value.0, len,
        )))
    }
}

struct SecurityAttributes {
    descriptor: PSECURITY_DESCRIPTOR,
    attrs: SECURITY_ATTRIBUTES,
}

impl SecurityAttributes {
    fn new_for_current_user() -> Result<Self, String> {
        let sid = current_user_sid_string()?;
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
        let wide = to_wide_z(OsStr::new(&sddl));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|err| format!("failed to build IPC security descriptor: {err}"))?;

        Ok(Self {
            descriptor,
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            },
        })
    }

    fn as_mut_ptr(&mut self) -> *mut SECURITY_ATTRIBUTES {
        &mut self.attrs
    }
}

impl Drop for SecurityAttributes {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.descriptor.0)));
        }
    }
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn to_wide_z(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::time::SystemTime;

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn unique_endpoint(label: &str) -> LocalIpcEndpoint {
        let id = NEXT_TEST_ID.fetch_add(1, AtomicOrdering::SeqCst);
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("cpu-affinity-tool-test-{label}-{pid}-{id}-{nanos}");
        LocalIpcEndpoint {
            pipe_name: format!(r"\\.\pipe\{name}"),
            mutex_name: format!(r"Local\{name}-primary"),
        }
    }

    fn recv_request(server: &LocalIpcServer, timeout: Duration) -> LocalIpcRequest {
        let deadline = Instant::now() + timeout;
        loop {
            match server.try_recv() {
                Ok(request) => return request,
                Err(mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("failed to receive IPC request before timeout: {err:?}"),
            }
        }
    }

    fn create_raw_pipe(
        endpoint: &LocalIpcEndpoint,
    ) -> Result<HandleGuard, windows::Win32::Foundation::WIN32_ERROR> {
        create_raw_pipe_with_instances(endpoint, 2)
    }

    fn create_raw_pipe_with_instances(
        endpoint: &LocalIpcEndpoint,
        max_instances: u32,
    ) -> Result<HandleGuard, windows::Win32::Foundation::WIN32_ERROR> {
        create_raw_pipe_with_security(endpoint, None, max_instances)
    }

    fn create_raw_pipe_with_security(
        endpoint: &LocalIpcEndpoint,
        security: Option<*const SECURITY_ATTRIBUTES>,
        max_instances: u32,
    ) -> Result<HandleGuard, windows::Win32::Foundation::WIN32_ERROR> {
        create_raw_pipe_with_security_and_first_instance(endpoint, security, max_instances, true)
    }

    fn create_raw_pipe_with_security_and_first_instance(
        endpoint: &LocalIpcEndpoint,
        security: Option<*const SECURITY_ATTRIBUTES>,
        max_instances: u32,
        first_instance: bool,
    ) -> Result<HandleGuard, windows::Win32::Foundation::WIN32_ERROR> {
        let name = to_wide_z_str(&endpoint.pipe_name);
        let mut open_mode = PIPE_ACCESS_DUPLEX;
        if first_instance {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                open_mode,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                max_instances,
                MAX_FRAME_BYTES as u32,
                MAX_FRAME_BYTES as u32,
                PIPE_TIMEOUT_MS,
                security,
            )
        };

        if handle.is_invalid() {
            Err(unsafe { GetLastError() })
        } else {
            Ok(HandleGuard(handle))
        }
    }

    fn create_raw_non_first_pipe(
        endpoint: &LocalIpcEndpoint,
    ) -> Result<HandleGuard, windows::Win32::Foundation::WIN32_ERROR> {
        create_raw_pipe_with_security_and_first_instance(endpoint, None, 2, false)
    }

    fn open_raw_client(endpoint: &LocalIpcEndpoint) -> Result<HandleGuard, LocalIpcClientError> {
        open_raw_client_with_flags(endpoint, client_open_flags())
    }

    fn open_raw_client_with_flags(
        endpoint: &LocalIpcEndpoint,
        flags: FILE_FLAGS_AND_ATTRIBUTES,
    ) -> Result<HandleGuard, LocalIpcClientError> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let name = to_wide_z_str(&endpoint.pipe_name);
            match unsafe {
                CreateFileW(
                    PCWSTR(name.as_ptr()),
                    (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    flags,
                    None,
                )
            } {
                Ok(handle) => return Ok(HandleGuard(handle)),
                Err(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => {
                    return Err(LocalIpcClientError::Io(format!(
                        "failed to open raw client pipe: {err}"
                    )));
                }
            }
        }
    }

    struct TestSecurityAttributes {
        descriptor: PSECURITY_DESCRIPTOR,
        attrs: SECURITY_ATTRIBUTES,
    }

    impl TestSecurityAttributes {
        fn from_sddl(sddl: &str) -> Self {
            let wide = to_wide_z(OsStr::new(sddl));
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(wide.as_ptr()),
                    1,
                    &mut descriptor,
                    None,
                )
            }
            .expect("test SDDL should be valid");

            Self {
                descriptor,
                attrs: SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor.0,
                    bInheritHandle: false.into(),
                },
            }
        }

        fn as_ptr(&mut self) -> *const SECURITY_ATTRIBUTES {
            &self.attrs
        }
    }

    impl Drop for TestSecurityAttributes {
        fn drop(&mut self) {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.descriptor.0)));
            }
        }
    }

    #[test]
    fn local_ipc_primary_guard_excludes_second_owner() {
        let endpoint = unique_endpoint("guard");
        let first = OS::try_claim_local_ipc_primary_guard(&endpoint)
            .expect("first guard claim should not fail")
            .expect("first guard claim should win");

        let second = OS::try_claim_local_ipc_primary_guard(&endpoint)
            .expect("second guard claim should not fail");
        assert!(second.is_none(), "second guard claim must not win");

        drop(first);
        let third = OS::try_claim_local_ipc_primary_guard(&endpoint)
            .expect("third guard claim should not fail");
        assert!(
            third.is_some(),
            "guard should be claimable again after first owner drops"
        );
    }

    #[test]
    fn start_local_ipc_server_fails_when_first_pipe_instance_is_already_owned() {
        let endpoint = unique_endpoint("preowned");
        let _squatter = create_raw_pipe(&endpoint).expect("test pipe should be created");

        let result = OS::start_local_ipc_server(&endpoint);

        assert!(
            result.is_err(),
            "server startup must fail before reporting ready when first pipe ownership is unavailable"
        );
    }

    #[test]
    fn local_ipc_round_trips_one_request_and_response() {
        let endpoint = unique_endpoint("roundtrip");
        let server = OS::start_local_ipc_server(&endpoint).expect("server should start");
        let client_endpoint = endpoint.clone();
        let client = thread::spawn(move || {
            OS::send_local_ipc_request(&client_endpoint, b"ping", Duration::from_secs(2))
        });

        let request = recv_request(&server, Duration::from_secs(2));
        assert_eq!(request.request, b"ping");
        request
            .response_tx
            .send(b"pong".to_vec())
            .expect("response should send");

        let response = client
            .join()
            .expect("client thread should not panic")
            .expect("client request should succeed");
        assert_eq!(response, b"pong");
    }

    #[test]
    fn local_ipc_wake_callback_runs_after_request_is_enqueued() {
        let endpoint = unique_endpoint("wake");
        let (wake_tx, wake_rx) = mpsc::channel();
        let wake = Arc::new(move || {
            let _ = wake_tx.send(());
        });
        let server = OS::start_local_ipc_server_with_wake(&endpoint, Some(wake))
            .expect("server should start");
        let client_endpoint = endpoint.clone();
        let client = thread::spawn(move || {
            OS::send_local_ipc_request(&client_endpoint, b"ping", Duration::from_secs(2))
        });

        let request = recv_request(&server, Duration::from_secs(2));
        wake_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("server should wake the owner after enqueueing a request");
        request
            .response_tx
            .send(b"pong".to_vec())
            .expect("response should send");
        client
            .join()
            .expect("client should not panic")
            .expect("client should succeed");
    }

    #[test]
    fn local_ipc_request_to_absent_server_returns_no_server() {
        let endpoint = unique_endpoint("absent");

        let result = OS::send_local_ipc_request(&endpoint, b"ping", Duration::from_millis(50));

        assert!(matches!(result, Err(LocalIpcClientError::NoServer)));
    }

    #[test]
    fn local_ipc_oversized_request_is_rejected_before_transport_open() {
        let endpoint = unique_endpoint("oversized");
        let request = vec![0u8; MAX_FRAME_BYTES + 1];

        let result = OS::send_local_ipc_request(&endpoint, &request, Duration::from_millis(50));

        assert!(
            matches!(result, Err(LocalIpcClientError::Io(message)) if message.contains("too large"))
        );
    }

    #[test]
    fn oversized_transport_frame_is_not_dispatched() {
        let endpoint = unique_endpoint("transport-oversized");
        let server = OS::start_local_ipc_server(&endpoint).expect("server should start");
        let client = open_raw_client_with_flags(
            &endpoint,
            FILE_ATTRIBUTE_NORMAL
                | SECURITY_IDENTIFICATION
                | SECURITY_EFFECTIVE_ONLY
                | SECURITY_SQOS_PRESENT,
        )
        .expect("raw client should connect");
        let oversized = vec![b'x'; MAX_FRAME_BYTES + 1];
        let mut bytes_written = 0u32;
        let _ = unsafe {
            WriteFile(
                client.0,
                Some(oversized.as_slice()),
                Some(&mut bytes_written),
                None,
            )
        };

        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            match server.try_recv() {
                Ok(request) => panic!(
                    "oversized transport frame should not be dispatched: {} bytes",
                    request.request.len()
                ),
                Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(10)),
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
    }

    #[test]
    fn pipe_busy_open_error_maps_to_server_not_ready() {
        assert_eq!(
            client_open_error_from_code(ERROR_PIPE_BUSY, "busy".to_string()),
            LocalIpcClientError::ServerNotReady
        );
    }

    #[test]
    fn local_ipc_client_timeout_is_bounded_after_connection() {
        let endpoint = unique_endpoint("client-timeout");
        let server = OS::start_local_ipc_server(&endpoint).expect("server should start");
        let client_endpoint = endpoint.clone();
        let started = Instant::now();

        let client = thread::spawn(move || {
            OS::send_local_ipc_request(&client_endpoint, b"ping", Duration::from_millis(300))
        });
        let request = recv_request(&server, Duration::from_secs(2));
        assert_eq!(request.request, b"ping");

        let result = client.join().expect("client thread should not panic");

        assert!(matches!(result, Err(LocalIpcClientError::Timeout)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "client timeout should not be stretched by server response wait"
        );
        drop(request);
    }

    #[test]
    fn dropping_server_with_silent_same_user_client_is_bounded() {
        let endpoint = unique_endpoint("silent-client");
        let server = OS::start_local_ipc_server(&endpoint).expect("server should start");
        let silent_client = open_raw_client(&endpoint).expect("silent client should connect");
        let (done_tx, done_rx) = mpsc::channel();

        let dropper = thread::spawn(move || {
            drop(server);
            let _ = done_tx.send(());
        });

        match done_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(()) => {
                drop(silent_client);
                dropper.join().expect("dropper should not panic");
            }
            Err(err) => {
                drop(silent_client);
                let _ = done_rx.recv_timeout(Duration::from_secs(2));
                let _ = dropper.join();
                panic!("server drop should not wait indefinitely for silent client: {err}");
            }
        }
    }

    #[test]
    fn pipe_name_remains_owned_after_request_completes() {
        let endpoint = unique_endpoint("continuous-owner");
        let server = OS::start_local_ipc_server(&endpoint).expect("server should start");
        let client_endpoint = endpoint.clone();
        let client = thread::spawn(move || {
            OS::send_local_ipc_request(&client_endpoint, b"ping", Duration::from_secs(2))
        });
        let request = recv_request(&server, Duration::from_secs(2));
        request
            .response_tx
            .send(b"pong".to_vec())
            .expect("response should send");
        client
            .join()
            .expect("client should not panic")
            .expect("client should succeed");

        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            match create_raw_pipe(&endpoint) {
                Ok(pipe) => {
                    drop(pipe);
                    panic!("pipe name became available to a first-instance squatter");
                }
                Err(_) => thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    #[test]
    fn additional_non_first_server_instance_is_denied_while_idle() {
        let endpoint = unique_endpoint("no-extra-instance");
        let _server = OS::start_local_ipc_server(&endpoint).expect("server should start");

        match create_raw_non_first_pipe(&endpoint) {
            Ok(pipe) => {
                drop(pipe);
                panic!("additional non-first pipe server instance should be denied");
            }
            Err(_) => {}
        }
    }

    #[test]
    fn access_denied_pipe_open_maps_to_security_rejected() {
        if current_user_sid_string().as_deref() == Ok("S-1-5-18") {
            return;
        }

        let endpoint = unique_endpoint("access-denied");
        let mut security = TestSecurityAttributes::from_sddl("D:P(A;;GA;;;SY)");
        let _pipe = create_raw_pipe_with_security(&endpoint, Some(security.as_ptr()), 2)
            .expect("restrictive test pipe should be created");

        let result = OS::send_local_ipc_request(&endpoint, b"ping", Duration::from_millis(50));

        assert!(matches!(
            result,
            Err(LocalIpcClientError::SecurityRejected(_))
        ));
    }

    #[test]
    fn ipc_security_and_lifecycle_flags_are_locked_down() {
        assert_ne!(
            server_open_mode().0 & FILE_FLAG_FIRST_PIPE_INSTANCE.0,
            0,
            "initial listener must prove first pipe ownership"
        );
        assert_eq!(
            PIPE_MAX_INSTANCES, 1,
            "server must not allow additional named-pipe server instances"
        );
        assert_ne!(
            server_open_mode().0 & FILE_FLAG_OVERLAPPED.0,
            0,
            "server pipe handles must use overlapped I/O"
        );
        assert_ne!(
            server_pipe_mode().0 & PIPE_REJECT_REMOTE_CLIENTS.0,
            0,
            "server pipe must reject remote clients"
        );
        assert_ne!(
            client_open_flags().0 & FILE_FLAG_OVERLAPPED.0,
            0,
            "client pipe handle must use overlapped I/O"
        );
        assert_ne!(
            client_open_flags().0 & SECURITY_SQOS_PRESENT.0,
            0,
            "client must explicitly set pipe SQOS"
        );
        assert_ne!(
            client_open_flags().0 & SECURITY_IDENTIFICATION.0,
            0,
            "client SQOS should limit server impersonation to identification"
        );
        assert!(
            OPEN_CLIENT_TOKEN_AS_SELF,
            "OpenThreadToken must use OpenAsSelf for SecurityIdentification clients"
        );
    }
}
