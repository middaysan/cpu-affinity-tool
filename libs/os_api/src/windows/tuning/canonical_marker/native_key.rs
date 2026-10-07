//! Fixed native64 namespace and one-attempt marker capability; no caller root/path.
use super::{
    UnknownReason, capsule,
    driver::View,
    driver::{Component, Disposition, DriverFailure, DriverResult},
    native_plumbing::component_plan,
    primitives::Key,
    registry_descriptor::ManagedDescriptor,
};
use std::cell::Cell;

/// Constructed solely from an actual successful Marker/New SDK disposition.
/// The attempted flag is set before the only valid-shape native Set call.
struct NewMarker {
    attempted: Cell<bool>,
}

/// One worker-owned key and actual creation provenance; no caller constructor.
pub(super) struct NativeKey {
    key: Key,
    component: Component,
    disposition: Disposition,
    new_marker: Option<NewMarker>,
}
impl NativeKey {
    /// Fixed native64/OpenLink relative open. No caller name or global root.
    pub fn open(
        parent: Option<&Self>,
        component: Component,
        view: View,
    ) -> DriverResult<Option<Self>> {
        use windows::Win32::System::Registry::{
            HKEY, HKEY_LOCAL_MACHINE, REG_OPTION_OPEN_LINK, REG_SAM_FLAGS, RegOpenKeyExW,
        };
        let plan = component_plan(parent.map(Self::component), component, view)?;
        let mut handle = HKEY::default();
        let code = unsafe {
            RegOpenKeyExW(
                parent.map_or(HKEY_LOCAL_MACHINE, |key| key.key.raw()),
                plan.wide(),
                Some(REG_OPTION_OPEN_LINK.0),
                REG_SAM_FLAGS(plan.open_mask()),
                &mut handle,
            )
        };
        if code.0 == 2 {
            return Ok(None);
        }
        if code.0 != 0 {
            return Err(DriverFailure::Native(super::super::code_error(
                "fixed canonical component open",
                code.0,
            )));
        }
        if handle.0.is_null() {
            return Err(DriverFailure::Unknown(UnknownReason::Context));
        }
        // SAFETY: successful fixed SDK open transferred one owned key handle.
        let key = unsafe { Key::from_owned(handle) };
        refuse_symbolic_link_header(&key)?;
        Ok(Some(Self {
            key,
            component,
            disposition: Disposition::Existing,
            new_marker: None,
        }))
    }
    /// The caller verifies actual context immediately before this operation.
    /// Planner validation precedes every SDK call; descriptor owner lives here.
    pub fn create(
        parent: &Self,
        component: Component,
        volatile: bool,
        view: View,
        descriptor: &ManagedDescriptor,
    ) -> DriverResult<Self> {
        use windows::Win32::System::Registry::{
            HKEY, REG_CREATE_KEY_DISPOSITION, REG_CREATED_NEW_KEY, REG_OPENED_EXISTING_KEY,
            REG_OPTION_NON_VOLATILE, REG_OPTION_VOLATILE, REG_SAM_FLAGS, RegCreateKeyExW,
        };
        let plan = component_plan(Some(parent.component), component, view)?;
        let access = plan.validate_creation(volatile)?;
        let attrs = descriptor.attributes();
        let mut handle = HKEY::default();
        let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
        // RegCreateKeyEx documents no OPEN_LINK create option. Strict parent
        // checks plus current-namespace replay authenticate the result.
        let code = unsafe {
            RegCreateKeyExW(
                parent.key.raw(),
                plan.wide(),
                None,
                windows::core::PCWSTR::null(),
                if volatile {
                    REG_OPTION_VOLATILE
                } else {
                    REG_OPTION_NON_VOLATILE
                },
                REG_SAM_FLAGS(access),
                Some(&attrs),
                &mut handle,
                Some(&mut disposition),
            )
        };
        if code.0 != 0 {
            return Err(DriverFailure::Native(super::super::code_error(
                "fixed canonical component create",
                code.0,
            )));
        }
        if handle.0.is_null() {
            return Err(DriverFailure::Unknown(UnknownReason::Context));
        }
        // SAFETY: a successful fixed create/open returned exclusive ownership.
        let key = unsafe { Key::from_owned(handle) };
        let disposition = match disposition {
            REG_CREATED_NEW_KEY => Disposition::New,
            REG_OPENED_EXISTING_KEY => Disposition::Existing,
            _ => return Err(DriverFailure::Unknown(UnknownReason::Context)),
        };
        refuse_symbolic_link_header(&key)?;
        Ok(Self {
            key,
            component,
            disposition,
            new_marker: (component == Component::Marker && disposition == Disposition::New).then(
                || NewMarker {
                    attempted: Cell::new(false),
                },
            ),
        })
    }
    pub fn key(&self) -> &Key {
        &self.key
    }
    pub fn component(&self) -> Component {
        self.component
    }
    pub fn disposition(&self) -> Disposition {
        self.disposition
    }
}
impl std::fmt::Debug for NativeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeKey { native_handle: opaque, authority: private }")
    }
}

/// Malformed shape performs zero Set and leaves a fresh attempt unused.
/// Existing keys can never overwrite any bytes. A valid native attempt consumes
/// NewMarker before the API call, including a denied/deleted-handle failure.
pub(super) fn set_new_marker_once(key: &NativeKey, bytes: &[u8; 56]) -> DriverResult<()> {
    use windows::Win32::System::Registry::{REG_BINARY, RegSetValueExW};
    // Malformed shape never consumes the sole valid native attempt.
    capsule::decode(capsule::BINARY_TYPE, bytes).map_err(DriverFailure::Unknown)?;
    if key.component != Component::Marker || key.disposition != Disposition::New {
        return Err(DriverFailure::Unknown(UnknownReason::Initialization));
    }
    let marker = key
        .new_marker
        .as_ref()
        .ok_or(DriverFailure::Unknown(UnknownReason::Initialization))?;
    // Consume before the SDK call. Neither a failed write nor a dropped caller
    // can retry through this same actual new-key capability.
    if marker.attempted.replace(true) {
        return Err(DriverFailure::Unknown(UnknownReason::Initialization));
    }
    let code = unsafe {
        RegSetValueExW(
            key.key.raw(),
            windows::core::w!("identity-v1"),
            None,
            REG_BINARY,
            Some(bytes),
        )
    };
    if code.0 != 0 {
        return Err(DriverFailure::Native(super::super::code_error(
            "candidate new marker Set",
            code.0,
        )));
    }
    Ok(())
}

/// Fixed SymbolicLinkValue header only: missing is clean; every presence,
/// native error or unknown size/type is Unknown. No allocation/target read.
pub(super) fn refuse_symbolic_link_header(key: &Key) -> DriverResult<()> {
    use windows::Win32::System::Registry::{REG_VALUE_TYPE, RegQueryValueExW};
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0;
    let code = unsafe {
        RegQueryValueExW(
            key.raw(),
            windows::core::w!("SymbolicLinkValue"),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if code.0 == 2 {
        Ok(())
    } else {
        // Every presence/type/size or failed query refuses the key. The link
        // target and value data are never inspected, allocated or followed.
        Err(DriverFailure::Unknown(UnknownReason::Context))
    }
}

/// Shape-only fixture setup, with no canonical context/security/observation.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) enum FixtureMarkerAccess {
    ReadOnly,
    ReadWrite,
}

/// The actual SDK operation captures New/Existing; callers supply neither a
/// disposition nor a capability. Parent must be their owned process appkey.
#[cfg(test)]
pub(super) fn create_fixture_marker(
    parent: &Key,
    access: FixtureMarkerAccess,
) -> DriverResult<NativeKey> {
    use windows::Win32::System::Registry::{
        HKEY, KEY_ALL_ACCESS, KEY_READ, KEY_WOW64_64KEY, REG_CREATE_KEY_DISPOSITION,
        REG_CREATED_NEW_KEY, REG_OPENED_EXISTING_KEY, REG_OPTION_VOLATILE, RegCreateKeyExW,
    };
    let mut handle = HKEY::default();
    let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
    let code = unsafe {
        RegCreateKeyExW(
            parent.raw(),
            windows::core::w!("ContinuityV1"),
            None,
            windows::core::PCWSTR::null(),
            REG_OPTION_VOLATILE,
            match access {
                FixtureMarkerAccess::ReadOnly => KEY_READ,
                FixtureMarkerAccess::ReadWrite => KEY_ALL_ACCESS,
            } | KEY_WOW64_64KEY,
            None,
            &mut handle,
            Some(&mut disposition),
        )
    };
    if code.0 != 0 {
        // Failure output is undefined; never close it as an owned key.
        return Err(DriverFailure::Native(super::super::code_error(
            "owned fixture marker create",
            code.0,
        )));
    }
    if handle.0.is_null() {
        return Err(DriverFailure::Unknown(UnknownReason::Initialization));
    }
    // SAFETY: successful fixed relative SDK creation returned exclusive ownership.
    let key = unsafe { Key::from_owned(handle) };
    let disposition = match disposition {
        REG_CREATED_NEW_KEY => Disposition::New,
        REG_OPENED_EXISTING_KEY => Disposition::Existing,
        _ => return Err(DriverFailure::Unknown(UnknownReason::Initialization)),
    };
    Ok(NativeKey {
        key,
        component: Component::Marker,
        disposition,
        new_marker: (disposition == Disposition::New).then(|| NewMarker {
            attempted: Cell::new(false),
        }),
    })
}
