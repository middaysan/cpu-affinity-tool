use crate::logic::{
    Backend, Elevation, Failure, Presence, ProbeStore, TokenContext, classify_integrity_sid,
};
use os_api::tuning::{
    MAX_TUNING_JOURNAL_BYTES, TuningNativeError, TuningStore, TuningStoreAuthority,
};
use std::{mem::size_of, ptr};
use windows::Win32::{
    Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE},
    Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenElevation,
        TokenIntegrityLevel,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

const MAX_TOKEN_BYTES: usize = 64 * 1024;
// winnt.h SE_GROUP_INTEGRITY. Keep the helper within the existing Security
// feature boundary rather than enabling an unrelated Windows API namespace.
const INTEGRITY_GROUP_ATTRIBUTE: u32 = 0x0000_0020;
struct TokenGuard(HANDLE);
impl Drop for TokenGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
fn native_failure(operation: &'static str, error: windows::core::Error) -> Failure {
    let code = error.code().0 as u32 & 0xffff;
    if code == 5 {
        TuningNativeError::Denied { operation, code }.into()
    } else {
        TuningNativeError::Io { operation, code }.into()
    }
}
fn unsupported() -> Failure {
    TuningNativeError::Unsupported("unrecognized token preflight data").into()
}
fn token_context() -> Result<TokenContext, Failure> {
    let mut raw = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) }
        .map_err(|error| native_failure("OpenProcessToken preflight", error))?;
    let token = TokenGuard(raw);
    let mut elevated = TOKEN_ELEVATION::default();
    let mut returned = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&mut elevated as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    }
    .map_err(|error| native_failure("TokenElevation preflight", error))?;
    if returned as usize != size_of::<TOKEN_ELEVATION>() {
        return Err(unsupported());
    }
    let elevation = match elevated.TokenIsElevated {
        0 => Elevation::Standard,
        1 => Elevation::Elevated,
        _ => return Err(unsupported()),
    };
    let mut required = 0;
    let sizing =
        unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, None, 0, &mut required) };
    match sizing {
        Err(error)
            if error.code() == windows::core::HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => {}
        Err(error) => return Err(native_failure("TokenIntegrityLevel size preflight", error)),
        Ok(()) => return Err(unsupported()),
    }
    let capacity = required as usize;
    if !(size_of::<TOKEN_MANDATORY_LABEL>()..=MAX_TOKEN_BYTES).contains(&capacity) {
        return Err(unsupported());
    }
    // usize allocation supplies alignment for the native structure and keeps
    // the embedded SID alive until every checked read has completed.
    let mut storage = vec![0usize; capacity.div_ceil(size_of::<usize>())];
    returned = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            Some(storage.as_mut_ptr().cast()),
            required,
            &mut returned,
        )
    }
    .map_err(|error| native_failure("TokenIntegrityLevel preflight", error))?;
    let length = returned as usize;
    if length < size_of::<TOKEN_MANDATORY_LABEL>() || length > capacity {
        return Err(unsupported());
    }
    let label = unsafe { ptr::read_unaligned(storage.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
    if label.Label.Attributes & INTEGRITY_GROUP_ATTRIBUTE == 0 {
        return Err(unsupported());
    }
    let base = storage.as_ptr() as usize;
    let offset = (label.Label.Sid.0 as usize)
        .checked_sub(base)
        .ok_or_else(unsupported)?;
    let end = offset
        .checked_add(12)
        .filter(|end| *end <= length)
        .ok_or_else(unsupported)?;
    if offset < size_of::<TOKEN_MANDATORY_LABEL>() {
        return Err(unsupported());
    }
    let bytes = unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), length) };
    let integrity = classify_integrity_sid(&bytes[offset..end]);
    Ok(TokenContext {
        elevation,
        integrity,
    })
}

pub struct NativeBackend;
pub struct NativeStore(TuningStore);
impl Backend for NativeBackend {
    type Store = NativeStore;
    fn token_context(&mut self) -> Result<TokenContext, Failure> {
        token_context()
    }
    fn open_canonical(&mut self) -> Result<Self::Store, Failure> {
        TuningStore::open_canonical(MAX_TUNING_JOURNAL_BYTES)
            .map(NativeStore)
            .map_err(Into::into)
    }
}
impl ProbeStore for NativeStore {
    fn canonical(&self) -> bool {
        self.0.authority() == TuningStoreAuthority::Canonical
    }
    fn validate(&mut self) -> Result<(), Failure> {
        self.0.validate_authority().map_err(Into::into)
    }
    fn presence(&mut self) -> Result<Presence, Failure> {
        match self.0.read() {
            Ok(None) => Ok(Presence::Absent),
            Ok(Some(bytes)) => Ok(Presence::PresentUninterpreted(bytes.len() as u64)),
            Err(TuningNativeError::Capacity { size, .. }) => Ok(Presence::Oversized(size)),
            Err(error) => Err(error.into()),
        }
    }
}
