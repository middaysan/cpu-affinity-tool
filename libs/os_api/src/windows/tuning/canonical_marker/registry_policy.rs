//! Registry-specific shape policy. File-object masks/validation are untouched.
use super::UnknownReason;
use std::ops::Range;

pub(super) const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;
pub(super) const MAX_ACES: u32 = 256;
pub(super) const MAX_SID_BYTES: usize = 68;
pub(super) const RELATIVE_HEADER_BYTES: usize = 20;
pub(super) const KEY_ALL: u32 = 0x000f_003f;
pub(super) const KEY_READ: u32 = 0x0002_0019;
pub(super) const KEY_WRITE: u32 = 0x0002_0006;
pub(super) const DANGEROUS_PARENT: u32 = 0x000d_0026;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidClass {
    System,
    Administrators,
    TrustedInstaller,
    HighIntegrity,
    CreatorOwner,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Ancestor,
    Managed,
}
#[derive(Clone, Debug)]
pub(super) struct AceFacts {
    pub kind: u8,
    pub flags: u8,
    pub mask: u32,
    pub sid: SidClass,
}
#[derive(Clone, Debug)]
pub(super) struct DescriptorFacts {
    pub owner: SidClass,
    pub dacl_present: bool,
    pub dacl_protected: bool,
    pub dacl: Vec<AceFacts>,
    pub labels: Vec<AceFacts>,
}
pub(super) fn validate(facts: &DescriptorFacts, scope: Scope) -> Result<(), UnknownReason> {
    let owner_trusted = matches!(facts.owner, SidClass::System | SidClass::Administrators)
        || (scope == Scope::Ancestor && facts.owner == SidClass::TrustedInstaller);
    if !owner_trusted
        || !facts.dacl_present
        || facts.dacl.is_empty()
        || facts.dacl.len() > MAX_ACES as usize
        || (scope == Scope::Managed && !facts.dacl_protected)
    {
        return Err(UnknownReason::UnsafeSecurity);
    }
    let mut system_full = false;
    let mut administrators_full = false;
    for ace in &facts.dacl {
        // Deny membership/order and unknown/conditional ACEs stay unknown;
        // a matching allow cannot be counted as an effective full grant.
        if ace.kind != 0 || ace.flags & !0x1f != 0 {
            return Err(UnknownReason::UnsafeSecurity);
        }
        let mask = registry_mask(ace.mask)?;
        if ace.sid == SidClass::CreatorOwner {
            // The exact observed inheritance template is not a token grant.
            // Managed children must independently retain their protected DACL.
            if scope != Scope::Ancestor || ace.flags != 2 || ace.mask != KEY_ALL {
                return Err(UnknownReason::UnsafeSecurity);
            }
            continue;
        }
        let trusted = matches!(ace.sid, SidClass::System | SidClass::Administrators)
            || (scope == Scope::Ancestor && ace.sid == SidClass::TrustedInstaller);
        if !trusted && (scope == Scope::Managed || mask & DANGEROUS_PARENT != 0) {
            return Err(UnknownReason::UnsafeSecurity);
        }
        if ace.flags & 8 == 0 && mask & KEY_ALL == KEY_ALL {
            system_full |= ace.sid == SidClass::System;
            administrators_full |= ace.sid == SidClass::Administrators;
        }
    }
    if scope == Scope::Managed {
        if !system_full || !administrators_full || facts.labels.len() != 1 {
            return Err(UnknownReason::UnsafeSecurity);
        }
        let label = &facts.labels[0];
        if label.kind != 17
            || label.sid != SidClass::HighIntegrity
            || label.mask != 1
            || label.flags & 8 != 0
            || label.flags & !0x1f != 0
        {
            return Err(UnknownReason::UnsafeSecurity);
        }
    }
    Ok(())
}
fn registry_mask(mask: u32) -> Result<u32, UnknownReason> {
    const GENERIC: u32 = 0xf000_0000;
    if mask & !(GENERIC | KEY_ALL) != 0 {
        return Err(UnknownReason::UnsafeSecurity);
    }
    let mut mapped = mask & !GENERIC;
    if mask & 0x1000_0000 != 0 {
        mapped |= KEY_ALL;
    }
    if mask & 0x4000_0000 != 0 {
        mapped |= KEY_WRITE;
    }
    if mask & (0x8000_0000 | 0x2000_0000) != 0 {
        mapped |= KEY_READ;
    }
    Ok(mapped)
}
pub(super) fn descriptor_buffer_size(bytes: u32) -> Result<usize, UnknownReason> {
    let bytes = bytes as usize;
    if (RELATIVE_HEADER_BYTES..=MAX_DESCRIPTOR_BYTES).contains(&bytes) {
        Ok(bytes)
    } else {
        Err(UnknownReason::QueryBound)
    }
}
pub(super) fn acl_walk_bounds(bytes: u32, count: u32) -> Result<(), UnknownReason> {
    if bytes < 8
        || bytes as usize > MAX_DESCRIPTOR_BYTES
        || count > MAX_ACES
        || count > (bytes - 8) / 16
    {
        Err(UnknownReason::QueryBound)
    } else {
        Ok(())
    }
}
pub(super) fn sid_read_size(subauthorities: u8, available: usize) -> Result<usize, UnknownReason> {
    let length = 8 + subauthorities as usize * 4;
    if length <= MAX_SID_BYTES && length <= available {
        Ok(length)
    } else {
        Err(UnknownReason::QueryBound)
    }
}
pub(super) fn relative_range(
    total: usize,
    offset: usize,
    size: usize,
) -> Result<Range<usize>, UnknownReason> {
    let end = offset.checked_add(size).ok_or(UnknownReason::QueryBound)?;
    if total > MAX_DESCRIPTOR_BYTES || offset < RELATIVE_HEADER_BYTES || size == 0 || end > total {
        Err(UnknownReason::QueryBound)
    } else {
        Ok(offset..end)
    }
}
