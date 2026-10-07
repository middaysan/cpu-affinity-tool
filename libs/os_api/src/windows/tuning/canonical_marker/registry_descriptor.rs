//! Fixed requested registry descriptor; not proof of any object security.
use super::{
    UnknownReason,
    driver::{DriverFailure, DriverResult},
    registry_policy::{self, Scope},
    security_descriptor,
};
use std::{marker::PhantomData, mem::size_of, rc::Rc};
use windows::Win32::{
    Foundation::{HLOCAL, LocalFree},
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    },
};

/// Owns only the fixed converter allocation; no caller SDDL/handle/constructor.
pub(super) struct ManagedDescriptor {
    pointer: PSECURITY_DESCRIPTOR,
    length: usize,
    _worker_only: PhantomData<Rc<()>>,
}
impl ManagedDescriptor {
    /// Fixed BA owner, protected SY/BA key-all grants and an effective High label.
    /// Tests convert/parse the requested shape, but never apply it to an object.
    pub fn new() -> DriverResult<Self> {
        let mut pointer = PSECURITY_DESCRIPTOR::default();
        let mut length = 0;
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                windows::core::w!("O:BAD:P(A;;KA;;;SY)(A;;KA;;;BA)S:(ML;;NW;;;HI)"),
                SDDL_REVISION_1,
                &mut pointer,
                Some(&mut length),
            )
        }
        .map_err(|error| {
            // Failed conversion does not define an owned output allocation.
            DriverFailure::Native(super::super::native_error(
                "fixed requested registry descriptor",
                error,
            ))
        })?;
        // Own the successful allocation before every possible rejection below.
        let descriptor = Self {
            pointer,
            length: length as usize,
            _worker_only: PhantomData,
        };
        registry_policy::descriptor_buffer_size(length).map_err(DriverFailure::Unknown)?;
        if descriptor.pointer.0.is_null() || !(descriptor.pointer.0 as usize).is_multiple_of(4) {
            return Err(DriverFailure::Unknown(UnknownReason::UnsafeSecurity));
        }
        let facts = security_descriptor::parse_descriptor(descriptor.bytes(), Scope::Managed)
            .map_err(DriverFailure::Unknown)?;
        registry_policy::validate(&facts, Scope::Managed).map_err(DriverFailure::Unknown)?;
        Ok(descriptor)
    }
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: only the fixed native converter constructor can create this
        // object, after checking its returned pointer and bounded byte length.
        unsafe { std::slice::from_raw_parts(self.pointer.0.cast(), self.length) }
    }
    /// Borrowed descriptor pointer: keep this owner alive throughout the SDK
    /// call. Requested shape never replaces post-create native security replay.
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.pointer.0,
            bInheritHandle: false.into(),
        }
    }
}
impl std::fmt::Debug for ManagedDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedDescriptor { requested_shape: redacted }")
    }
}
impl Drop for ManagedDescriptor {
    fn drop(&mut self) {
        if !self.pointer.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.pointer.0)));
            }
        }
    }
}
