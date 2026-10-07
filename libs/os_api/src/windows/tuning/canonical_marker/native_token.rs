//! Private current-primary-token reader. No machine/store/registry authority.
use super::{
    UnknownReason,
    driver::{DriverFailure, DriverResult},
};
use crate::tuning::TuningTokenIdentity;
use std::{marker::PhantomData, mem::size_of, rc::Rc};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE, LUID},
        Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL,
            TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER, TokenElevation, TokenIntegrityLevel,
            TokenPrimary, TokenSessionId, TokenStatistics, TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
    core::HRESULT,
};

const MAX_TOKEN_BYTES: usize = 65_536;
const MAX_SID_BYTES: usize = 68;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Integrity {
    Low,
    Medium,
    High,
    System,
    Unknown,
}

/// Immutable output of the retained native handle; no caller constructor.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct TokenSnapshot {
    identity: TuningTokenIdentity,
    elevated: bool,
    integrity: Integrity,
    integrity_rid: u32,
    integrity_attributes: u32,
    token_id: u64,
    modified_id: u64,
}
impl TokenSnapshot {
    #[cfg(test)]
    pub fn identity(&self) -> &TuningTokenIdentity {
        &self.identity
    }
    pub fn elevated(&self) -> bool {
        self.elevated
    }
    pub fn integrity(&self) -> Integrity {
        self.integrity
    }
    #[cfg(test)]
    pub fn integrity_rid(&self) -> u32 {
        self.integrity_rid
    }
    #[cfg(test)]
    pub fn integrity_attributes(&self) -> u32 {
        self.integrity_attributes
    }
    #[cfg(test)]
    pub fn token_id(&self) -> u64 {
        self.token_id
    }
    #[cfg(test)]
    pub fn modified_id(&self) -> u64 {
        self.modified_id
    }
}
impl std::fmt::Debug for TokenSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenSnapshot { provenance: redacted }")
    }
}

/// One real TOKEN_QUERY primary handle, opened/read/closed on one worker.
pub(super) struct RetainedCurrentToken {
    handle: HANDLE,
    token_id: u64,
    _worker_only: PhantomData<Rc<()>>,
}
impl RetainedCurrentToken {
    pub fn open_current() -> DriverResult<Self> {
        let handle = CurrentPrimaryHandle::open()?;
        let initial = query_stamp(handle.0, true)?;
        if !initial.primary || initial.token_id == 0 || initial.modified_id == 0 {
            return context_unknown();
        }
        let token = Self {
            handle: handle.0,
            token_id: initial.token_id,
            _worker_only: PhantomData,
        };
        // Transfer the real handle only after the initial native checks succeed.
        std::mem::forget(handle);
        Ok(token)
    }
    pub fn snapshot(&self) -> DriverResult<TokenSnapshot> {
        let before = query_stamp(self.handle, true)?;

        let user = query_variable(self.handle, TokenUser, size_of::<TOKEN_USER>())?;
        // SAFETY: query_variable checked the complete initialized header and
        // returned byte count; read_unaligned copies it without following Sid.
        let user_header =
            unsafe { std::ptr::read_unaligned(user.words.as_ptr().cast::<TOKEN_USER>()) };
        let sid = sid_text(bounded_sid_bytes(
            user.bytes(),
            size_of::<TOKEN_USER>(),
            user.words.as_ptr() as usize,
            user_header.User.Sid.0 as usize,
        )?)?;

        let session_id: u32 = query_fixed(self.handle, TokenSessionId)?;
        let elevation: TOKEN_ELEVATION = query_fixed(self.handle, TokenElevation)?;
        let elevated = elevation_value(elevation.TokenIsElevated)?;
        let mandatory = query_variable(
            self.handle,
            TokenIntegrityLevel,
            size_of::<TOKEN_MANDATORY_LABEL>(),
        )?;
        // SAFETY: the bounded returned buffer contains this entire header.
        // The embedded Sid pointer is validated as an offset before any use.
        let label = unsafe {
            std::ptr::read_unaligned(mandatory.words.as_ptr().cast::<TOKEN_MANDATORY_LABEL>())
        };
        let integrity = integrity_facts(
            bounded_sid_bytes(
                mandatory.bytes(),
                size_of::<TOKEN_MANDATORY_LABEL>(),
                mandatory.words.as_ptr() as usize,
                label.Label.Sid.0 as usize,
            )?,
            label.Label.Attributes,
        )?;

        let after = query_stamp(self.handle, true)?;
        // This second handle contributes provenance only. Every tuple field
        // above and AuthenticationId below comes from the retained handle.
        let current = CurrentPrimaryHandle::open()?;
        let current_primary = query_stamp(current.0, false)?;
        token_coherence(self.token_id, before, after, current_primary)?;
        Ok(TokenSnapshot {
            identity: TuningTokenIdentity {
                sid,
                session_id,
                authentication_id: before.authentication_id,
            },
            elevated,
            integrity: integrity.class,
            integrity_rid: integrity.rid,
            integrity_attributes: integrity.attributes,
            token_id: before.token_id,
            modified_id: before.modified_id,
        })
    }
}
impl std::fmt::Debug for RetainedCurrentToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RetainedCurrentToken { native_handle: opaque }")
    }
}
impl Drop for RetainedCurrentToken {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

// Pure malformed-native-data decisions issue only private shape results.
pub(super) fn variable_buffer_size(required: u32, header: usize) -> DriverResult<usize> {
    let required = required as usize;
    if header == 0 || header > MAX_TOKEN_BYTES || required < header || required > MAX_TOKEN_BYTES {
        return bounds_unknown();
    }
    Ok(required)
}
pub(super) fn returned_buffer_size(
    capacity: usize,
    returned: u32,
    header: usize,
) -> DriverResult<usize> {
    let size = variable_buffer_size(returned, header)?;
    if capacity != size {
        return bounds_unknown();
    }
    Ok(size)
}
pub(super) fn bounded_sid_bytes(
    bytes: &[u8],
    header: usize,
    base: usize,
    pointer: usize,
) -> DriverResult<&[u8]> {
    if header == 0
        || header > bytes.len()
        || bytes.len() > MAX_TOKEN_BYTES
        || !pointer.is_multiple_of(4)
    {
        return bounds_unknown();
    }
    let backing_end = base.checked_add(bytes.len()).ok_or(bounds_failure())?;
    let offset = pointer.checked_sub(base).ok_or(bounds_failure())?;
    if offset < header || !offset.is_multiple_of(4) {
        return bounds_unknown();
    }
    let sid_bytes = bytes.get(offset..).ok_or(bounds_failure())?;
    let length = sid_length(sid_bytes)?;
    let sid_end = pointer.checked_add(length).ok_or(bounds_failure())?;
    if sid_end > backing_end {
        return bounds_unknown();
    }
    sid_bytes.get(..length).ok_or(bounds_failure())
}
pub(super) fn sid_text(bytes: &[u8]) -> DriverResult<String> {
    if sid_length(bytes)? != bytes.len() {
        return bounds_unknown();
    }
    let authority = sid_authority(bytes);
    let mut text = if authority < (1u64 << 32) {
        format!("S-1-{authority}")
    } else {
        format!("S-1-0x{authority:012x}")
    };
    for bytes in bytes[8..].as_chunks::<4>().0 {
        let subauthority = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        text.push_str(&format!("-{subauthority}"));
    }
    Ok(text)
}
pub(super) fn fixed_field_size(expected: usize, returned: u32) -> DriverResult<()> {
    if expected == 0 || expected > MAX_TOKEN_BYTES || expected != returned as usize {
        return bounds_unknown();
    }
    Ok(())
}
pub(super) fn elevation_value(value: u32) -> DriverResult<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => context_unknown(),
    }
}
pub(super) fn authentication_id(low: u32, high: i32) -> DriverResult<u64> {
    let full = ((high as u32 as u64) << 32) | low as u64;
    if full == 0 {
        return context_unknown();
    }
    Ok(full)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IntegrityFacts {
    pub class: Integrity,
    pub rid: u32,
    pub attributes: u32,
}
pub(super) fn integrity_facts(sid: &[u8], attributes: u32) -> DriverResult<IntegrityFacts> {
    // Effective mandatory label only; optional/unknown flags confer no authority.
    if attributes != 0x60
        || sid_length(sid).map_err(|_| DriverFailure::Unknown(UnknownReason::Context))? != 12
        || sid.len() != 12
        || sid_authority(sid) != 16
    {
        return context_unknown();
    }
    let rid = u32::from_le_bytes([sid[8], sid[9], sid[10], sid[11]]);
    let class = match rid {
        0x1000 => Integrity::Low,
        0x2000 | 0x2100 => Integrity::Medium,
        0x3000 => Integrity::High,
        0x4000 => Integrity::System,
        _ => Integrity::Unknown,
    };
    Ok(IntegrityFacts {
        class,
        rid,
        attributes,
    })
}

/// Statistics shape only: construction cannot create a native guard/snapshot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct TokenStamp {
    pub token_id: u64,
    pub modified_id: u64,
    pub authentication_id: u64,
    pub primary: bool,
}
impl std::fmt::Debug for TokenStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenStamp { shape_only: true, provenance: redacted }")
    }
}
/// Original-handle statistics bracket all tuple reads. A fresh primary handle
/// contributes TokenId/ModifiedId provenance only, never joined tuple fields.
pub(super) fn token_coherence(
    expected_token_id: u64,
    before: TokenStamp,
    after: TokenStamp,
    current_primary: TokenStamp,
) -> DriverResult<()> {
    if expected_token_id == 0
        || before.token_id != expected_token_id
        || before.modified_id == 0
        || before.authentication_id == 0
        || !before.primary
        || before != after
        || current_primary.token_id != before.token_id
        || current_primary.modified_id != before.modified_id
        || !current_primary.primary
    {
        return context_unknown();
    }
    Ok(())
}

fn bounds_failure() -> DriverFailure {
    DriverFailure::Unknown(UnknownReason::QueryBound)
}
fn bounds_unknown<T>() -> DriverResult<T> {
    Err(bounds_failure())
}
fn context_unknown<T>() -> DriverResult<T> {
    Err(DriverFailure::Unknown(UnknownReason::Context))
}
fn sid_length(bytes: &[u8]) -> DriverResult<usize> {
    if bytes.len() < 8 || bytes[0] != 1 || !(1..=15).contains(&bytes[1]) {
        return bounds_unknown();
    }
    let length = 8usize
        .checked_add(
            4usize
                .checked_mul(bytes[1] as usize)
                .ok_or(bounds_failure())?,
        )
        .ok_or(bounds_failure())?;
    if length > MAX_SID_BYTES || length > bytes.len() {
        return bounds_unknown();
    }
    Ok(length)
}
// Called only after sid_length checked the complete header.
fn sid_authority(bytes: &[u8]) -> u64 {
    u64::from_be_bytes([
        0, 0, bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}
fn native_failure(operation: &'static str, error: windows::core::Error) -> DriverFailure {
    DriverFailure::Native(super::super::native_error(operation, error))
}

struct CurrentPrimaryHandle(HANDLE);
impl CurrentPrimaryHandle {
    fn open() -> DriverResult<Self> {
        let mut handle = HANDLE::default();
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) }
            .map_err(|error| native_failure("candidate OpenProcessToken", error))?;
        if handle.is_invalid() {
            return context_unknown();
        }
        Ok(Self(handle))
    }
}
impl Drop for CurrentPrimaryHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

// These initialized native POD outputs are private to this leaf; no caller
// supplies a type, class, handle or output pointer to the native producer.
fn query_fixed<T: Default>(handle: HANDLE, class: TOKEN_INFORMATION_CLASS) -> DriverResult<T> {
    let capacity = size_of::<T>();
    fixed_field_size(capacity, capacity.try_into().map_err(|_| bounds_failure())?)?;
    let mut value = T::default();
    let mut returned = 0;
    unsafe {
        GetTokenInformation(
            handle,
            class,
            Some((&mut value as *mut T).cast()),
            capacity as u32,
            &mut returned,
        )
    }
    .map_err(|error| native_failure("candidate fixed token query", error))?;
    fixed_field_size(capacity, returned)?;
    Ok(value)
}

struct TokenBytes {
    words: Vec<usize>,
    length: usize,
}
impl TokenBytes {
    fn bytes(&self) -> &[u8] {
        // SAFETY: initialized aligned words own at least length bounded bytes.
        // Native output never enlarges this slice into alignment padding.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast(), self.length) }
    }
}
fn query_variable(
    handle: HANDLE,
    class: TOKEN_INFORMATION_CLASS,
    header: usize,
) -> DriverResult<TokenBytes> {
    let mut required = 0;
    match unsafe { GetTokenInformation(handle, class, None, 0, &mut required) } {
        Err(error) if error.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => {}
        Err(error) => return Err(native_failure("candidate token size query", error)),
        Ok(()) => return bounds_unknown(),
    }
    let capacity = variable_buffer_size(required, header)?;
    let mut words = vec![0usize; capacity.div_ceil(size_of::<usize>())];
    let mut returned = 0;
    match unsafe {
        GetTokenInformation(
            handle,
            class,
            Some(words.as_mut_ptr().cast()),
            required,
            &mut returned,
        )
    } {
        Err(error) if error.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => {
            return bounds_unknown();
        }
        Err(error) => return Err(native_failure("candidate bounded token query", error)),
        Ok(()) => {}
    }
    let length = returned_buffer_size(capacity, returned, header)?;
    Ok(TokenBytes { words, length })
}
fn luid_bits(value: LUID) -> u64 {
    ((value.HighPart as u32 as u64) << 32) | value.LowPart as u64
}
fn query_stamp(handle: HANDLE, require_authentication: bool) -> DriverResult<TokenStamp> {
    let stats: TOKEN_STATISTICS = query_fixed(handle, TokenStatistics)?;
    Ok(TokenStamp {
        token_id: luid_bits(stats.TokenId),
        modified_id: luid_bits(stats.ModifiedId),
        authentication_id: if require_authentication {
            authentication_id(
                stats.AuthenticationId.LowPart,
                stats.AuthenticationId.HighPart,
            )?
        } else {
            // Fresh primary contributes only TokenId/ModifiedId/type provenance.
            luid_bits(stats.AuthenticationId)
        },
        primary: stats.TokenType == TokenPrimary,
    })
}
