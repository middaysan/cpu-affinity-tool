//! Shared private HKEY queries/probe. No root opener or public authority.
use super::{
    UnknownReason, capsule,
    driver::{CapsuleShape, Counts, Disposition, DriverFailure, DriverResult, ValueHeader, View},
    registry_policy,
};
use std::{marker::PhantomData, rc::Rc};
use windows::{
    Win32::{
        Security::{
            Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
            DACL_SECURITY_INFORMATION, LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR,
        },
        System::Registry::{
            HKEY, KEY_READ, REG_CREATE_KEY_DISPOSITION, REG_CREATED_NEW_KEY,
            REG_OPENED_EXISTING_KEY, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_VALUE_TYPE,
            RegCloseKey, RegCreateKeyExW, RegGetKeySecurity, RegQueryInfoKeyW, RegQueryValueExW,
        },
    },
    core::PCWSTR,
};

/// One owned HKEY, used and closed on the same worker. Never an authority cast.
pub(super) struct Key {
    handle: HKEY,
    _worker_only: PhantomData<Rc<()>>,
}
impl Key {
    /// # Safety
    /// The handle is a valid newly owned key, with no other closing owner.
    /// Fixtures may pass only their process-private appkey handles.
    pub unsafe fn from_owned(handle: HKEY) -> Self {
        Self {
            handle,
            _worker_only: PhantomData,
        }
    }
    pub fn raw(&self) -> HKEY {
        self.handle
    }
}
impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key { native_handle: opaque }")
    }
}
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.handle);
        }
    }
}

/// Fixed single component, CSPRNG in the native route; no caller path accepted.
pub(super) struct PrivateProbeName {
    text: [u16; 71],
}
impl PrivateProbeName {
    pub fn random() -> DriverResult<Self> {
        let mut nonce = [0; 32];
        unsafe { BCryptGenRandom(None, &mut nonce, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
            .ok()
            .map_err(|error| {
                DriverFailure::Native(super::super::native_error("private probe nonce", error))
            })?;
        Self::from_nonce(nonce)
    }
    fn from_nonce(nonce: [u8; 32]) -> DriverResult<Self> {
        if nonce == [0; 32] {
            return Err(DriverFailure::Unknown(UnknownReason::Initialization));
        }
        let mut text = [0; 71];
        for (index, byte) in b"probe-".iter().enumerate() {
            text[index] = *byte as u16;
        }
        for (index, byte) in nonce.into_iter().enumerate() {
            let hex = b"0123456789abcdef";
            text[6 + index * 2] = hex[(byte >> 4) as usize] as u16;
            text[7 + index * 2] = hex[(byte & 15) as usize] as u16;
        }
        Ok(Self { text })
    }
    #[cfg(test)]
    pub fn for_fixture(nonce: [u8; 32]) -> DriverResult<Self> {
        Self::from_nonce(nonce)
    }
    pub fn wide(&self) -> &[u16; 71] {
        &self.text
    }
}
impl std::fmt::Debug for PrivateProbeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateProbeName { value: redacted }")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProbeOutcome {
    pub native_code: u32,
    /// Present only after a successful create/open; never accepted as volatility.
    pub disposition: Option<Disposition>,
}

pub(super) fn capsule_header(key: &Key) -> DriverResult<Option<ValueHeader>> {
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0;
    let code = unsafe {
        RegQueryValueExW(
            key.raw(),
            windows::core::w!("identity-v1"),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    }
    .0;
    if code == 2 {
        return Ok(None);
    }
    native_success("query private capsule header", code)?;
    Ok(Some(ValueHeader { kind: kind.0, size }))
}
pub(super) fn capsule_data(key: &Key, buffer: &mut [u8; 56]) -> DriverResult<ValueHeader> {
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = capsule::CAPSULE_BYTES as u32;
    let code = unsafe {
        RegQueryValueExW(
            key.raw(),
            windows::core::w!("identity-v1"),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        )
    }
    .0;
    if code == 234 {
        return Err(DriverFailure::Unknown(UnknownReason::QueryBound));
    }
    native_success("query private fixed capsule", code)?;
    capsule::fixed_read_size(kind.0, size).map_err(DriverFailure::Unknown)?;
    Ok(ValueHeader { kind: kind.0, size })
}
pub(super) fn read_capsule(key: &Key) -> DriverResult<CapsuleShape> {
    let header =
        capsule_header(key)?.ok_or(DriverFailure::Unknown(UnknownReason::MalformedCapsule))?;
    capsule::fixed_read_size(header.kind, header.size).map_err(DriverFailure::Unknown)?;
    let mut bytes = [0; 56];
    let repeated = capsule_data(key, &mut bytes)?;
    capsule::fixed_read_size(repeated.kind, repeated.size).map_err(DriverFailure::Unknown)?;
    if repeated != header {
        return Err(DriverFailure::Unknown(UnknownReason::QueryBound));
    }
    capsule::decode(repeated.kind, &bytes).map_err(DriverFailure::Unknown)
}
pub(super) fn marker_counts(key: &Key) -> DriverResult<Counts> {
    let mut values = 0;
    let mut subkeys = 0;
    let code = unsafe {
        RegQueryInfoKeyW(
            key.raw(),
            None,
            None,
            None,
            Some(&mut subkeys),
            None,
            None,
            Some(&mut values),
            None,
            None,
            None,
            None,
        )
    }
    .0;
    native_success("query private marker counts", code)?;
    Ok(Counts { values, subkeys })
}

/// One aligned, bounded native-query buffer. Bytes are private shape evidence.
pub(super) struct DescriptorBytes {
    buffer: Vec<u32>,
    length: usize,
}
impl DescriptorBytes {
    pub fn bytes(&self) -> &[u8] {
        // Construction is private: the returned length fits this initialized
        // buffer, and every bit pattern is valid when viewed as bytes.
        unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast(), self.length) }
    }
}
impl std::fmt::Debug for DescriptorBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DescriptorBytes { shape_only: true, value: redacted }")
    }
}
/// OWNER + DACL + LABEL under READ_CONTROL only. Bound first size to64 KiB,
/// allocate one aligned buffer, then query once without growth or fallback.
pub(super) fn security_descriptor_bytes(key: &Key) -> DriverResult<DescriptorBytes> {
    let information =
        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
    let mut required = 0;
    let code = unsafe { RegGetKeySecurity(key.raw(), information, None, &mut required) }.0;
    if code != 122 {
        if code == 0 {
            return Err(DriverFailure::Unknown(UnknownReason::QueryBound));
        }
        return Err(DriverFailure::Native(super::super::code_error(
            "size private key security",
            code,
        )));
    }
    let capacity =
        registry_policy::descriptor_buffer_size(required).map_err(DriverFailure::Unknown)?;
    let mut buffer = vec![0u32; capacity.div_ceil(std::mem::size_of::<u32>())];
    // Keep the original requested byte capacity, even if the aligned allocation
    // has up to three padding bytes. Never grow or retry a changing descriptor.
    let mut returned = required;
    let code = unsafe {
        RegGetKeySecurity(
            key.raw(),
            information,
            Some(PSECURITY_DESCRIPTOR(buffer.as_mut_ptr().cast())),
            &mut returned,
        )
    }
    .0;
    if code == 122 {
        return Err(DriverFailure::Unknown(UnknownReason::QueryBound));
    }
    native_success("read private key security", code)?;
    let length = descriptor_returned_size(capacity, returned)?;
    Ok(DescriptorBytes { buffer, length })
}

pub(super) fn descriptor_returned_size(capacity: usize, returned: u32) -> DriverResult<usize> {
    let length =
        registry_policy::descriptor_buffer_size(returned).map_err(DriverFailure::Unknown)?;
    if length != capacity {
        Err(DriverFailure::Unknown(UnknownReason::QueryBound))
    } else {
        Ok(length)
    }
}
/// Requests one nonvolatile child in Native64. Close its returned handle only;
/// a successful or collided child remains metadata evidence, never deleted.
pub(super) fn probe_nonvolatile_child_once(
    key: &Key,
    view: View,
    name: &PrivateProbeName,
) -> DriverResult<ProbeOutcome> {
    let mut child = HKEY::default();
    let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
    let code = unsafe {
        RegCreateKeyExW(
            key.raw(),
            PCWSTR(name.wide().as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | REG_SAM_FLAGS(view.flag()),
            None,
            &mut child,
            Some(&mut disposition),
        )
    }
    .0;
    if code != 0 {
        // Failure does not define an owned result handle; do not close it.
        return Ok(ProbeOutcome {
            native_code: code,
            disposition: None,
        });
    }
    if child.0.is_null() {
        return Err(DriverFailure::Unknown(UnknownReason::Initialization));
    }
    // SAFETY: successful create/open returned exactly one newly owned handle.
    let child = unsafe { Key::from_owned(child) };
    let disposition = match disposition {
        REG_CREATED_NEW_KEY => Disposition::New,
        REG_OPENED_EXISTING_KEY => Disposition::Existing,
        _ => return Err(DriverFailure::Unknown(UnknownReason::Initialization)),
    };
    // Close only the result handle. The key/value artifact remains untouched,
    // including an unknown successful/collided nonvolatile child.
    drop(child);
    Ok(ProbeOutcome {
        native_code: 0,
        disposition: Some(disposition),
    })
}

fn native_success(operation: &'static str, code: u32) -> DriverResult<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(DriverFailure::Native(super::super::code_error(
            operation, code,
        )))
    }
}
