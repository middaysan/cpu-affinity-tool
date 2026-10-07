//! Retained actual canonical context; no caller facts or fixture constructor.
use super::{
    UnknownReason,
    driver::{DriverFailure, DriverResult},
    native_plumbing::admit_retained_token,
    native_token::{RetainedCurrentToken, TokenSnapshot},
};
use crate::{
    tuning::{TuningMachineIdentity, TuningStore, TuningStoreAuthority},
    windows::OS,
};

pub(super) struct NativeContext {
    token: RetainedCurrentToken,
    expected_token: TokenSnapshot,
    expected_machine: TuningMachineIdentity,
}
impl NativeContext {
    pub fn capture(store: &mut TuningStore) -> DriverResult<Self> {
        native64_build()?;
        require_canonical(store)?;
        store.validate_authority().map_err(DriverFailure::Native)?;
        let token = RetainedCurrentToken::open_current()?;
        let expected_token = token.snapshot()?;
        // Denial and uncertain effective attributes stop before hardware.
        admit_retained_token(&expected_token)?;
        let expected_machine = OS::tuning_machine_identity().map_err(DriverFailure::Native)?;
        let context = Self {
            token,
            expected_token,
            expected_machine,
        };
        context.verify(store)?;
        Ok(context)
    }
    pub fn verify(&self, store: &mut TuningStore) -> DriverResult<()> {
        native64_build()?;
        require_canonical(store)?;
        store.validate_authority().map_err(DriverFailure::Native)?;
        self.verify_token()?;
        let actual = OS::tuning_machine_identity().map_err(DriverFailure::Native)?;
        if actual != self.expected_machine {
            return Err(DriverFailure::Unknown(UnknownReason::Context));
        }
        self.verify_token()?;
        store.validate_authority().map_err(DriverFailure::Native)
    }
    fn verify_token(&self) -> DriverResult<()> {
        let actual = self.token.snapshot()?;
        admit_retained_token(&actual)?;
        if actual != self.expected_token {
            return Err(DriverFailure::Unknown(UnknownReason::Context));
        }
        Ok(())
    }
}
impl std::fmt::Debug for NativeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeContext { native_authority: opaque }")
    }
}
fn native64_build() -> DriverResult<()> {
    // An x86_64 PE64 image cannot run on 32-bit Windows. ARM64 x64 emulation
    // remains an unmeasured acceptance context, not a native64 view fabrication.
    if cfg!(all(target_arch = "x86_64", target_pointer_width = "64")) {
        Ok(())
    } else {
        Err(DriverFailure::Unknown(UnknownReason::Context))
    }
}
fn require_canonical(store: &TuningStore) -> DriverResult<()> {
    if store.authority() == TuningStoreAuthority::Canonical {
        Ok(())
    } else {
        Err(DriverFailure::Unknown(UnknownReason::Context))
    }
}
