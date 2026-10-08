use super::store_io::native::RelativeAncestorCapability;
use super::{native_error, wide};
use crate::tuning::TuningNativeError;
use std::{ffi::c_void, mem::size_of, ptr};
use windows::Win32::{
    Foundation::{HANDLE, HLOCAL, LocalFree},
    Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
    },
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        INHERIT_ONLY_ACE, LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
    },
};
use windows::core::{PCWSTR, PWSTR};

const SYSTEM: &str = "S-1-5-18";
const ADMINISTRATORS: &str = "S-1-5-32-544";
pub(super) const TRUSTED_INSTALLER: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
const DANGEROUS_PARENT_RIGHTS: u32 = 0x000d_0152 | 0x1000_0000 | 0x4000_0000;

fn untrusted_grant_is_unsafe(managed: bool, mask: u32) -> bool {
    managed || mask & DANGEROUS_PARENT_RIGHTS != 0
}
fn relative_ancestor_grant_is_unsafe(mask: u32) -> bool {
    // Only actual bound relative ancestors may admit metadata/create grants.
    // Delete, owner/DACL changes and both raw generic bits remain refused.
    // Native ACLs may map generic requests to indistinguishable concrete rights.
    mask & (DANGEROUS_PARENT_RIGHTS & !0x112) != 0
}

pub(super) struct LocalAllocation(pub *mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0)));
        }
    }
}

pub(super) struct Descriptor {
    allocation: LocalAllocation,
}
fn config_sddl(sid: &str) -> String {
    format!("O:{sid}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{sid})")
}
#[cfg(test)]
fn config_characterization_sddl(sid: &str) -> String {
    format!("{}(A;;FR;;;SY)", config_sddl(sid))
}
impl Descriptor {
    pub fn new(test_sid: Option<&str>) -> Result<Self, TuningNativeError> {
        let sddl = match test_sid {
            Some(sid) => format!("O:{sid}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{sid})"),
            None => "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)S:(ML;OICI;NW;;;HI)".to_owned(),
        };
        Self::from_sddl(&sddl)
    }
    pub(super) fn for_config(
        principal: &super::identity::ConfigPrincipal,
    ) -> Result<Self, TuningNativeError> {
        Self::from_sddl(&config_sddl(principal.sid()))
    }
    #[cfg(test)]
    pub(super) fn for_config_characterization(
        principal: &super::identity::ConfigPrincipal,
    ) -> Result<Self, TuningNativeError> {
        principal.verify()?;
        Self::from_sddl(&config_characterization_sddl(principal.sid()))
    }
    fn from_sddl(sddl: &str) -> Result<Self, TuningNativeError> {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let value = wide(std::ffi::OsStr::new(sddl))?;
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(value.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|e| native_error("security descriptor", e))?;
        Ok(Self {
            allocation: LocalAllocation(descriptor.0),
        })
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.allocation.0,
            bInheritHandle: false.into(),
        }
    }
}

#[derive(Clone)]
pub(super) struct SecurityPolicy {
    pub test_sid: Option<String>,
}
impl SecurityPolicy {
    pub fn validate_relative_ancestor(
        &self,
        capability: &RelativeAncestorCapability<'_>,
    ) -> Result<(), TuningNativeError> {
        capability.verify_object()?;
        self.validate_with_grants(
            capability.handle(),
            false,
            relative_ancestor_grant_is_unsafe,
        )?;
        capability.verify_object()
    }
    fn trusted(&self, sid: &str, managed: bool) -> bool {
        sid == SYSTEM
            || sid == ADMINISTRATORS
            || (!managed && sid == TRUSTED_INSTALLER)
            || self.test_sid.as_deref() == Some(sid)
    }

    pub fn validate(&self, handle: HANDLE, managed: bool) -> Result<(), TuningNativeError> {
        self.validate_with_grants(handle, managed, |mask| {
            untrusted_grant_is_unsafe(managed, mask)
        })
    }

    fn validate_with_grants(
        &self,
        handle: HANDLE,
        managed: bool,
        unsafe_grant: impl Fn(u32) -> bool,
    ) -> Result<(), TuningNativeError> {
        let mut owner = PSID::default();
        let mut dacl = ptr::null_mut();
        let mut sacl = ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let info = OWNER_SECURITY_INFORMATION
            | DACL_SECURITY_INFORMATION
            | if managed && self.test_sid.is_none() {
                LABEL_SECURITY_INFORMATION
            } else {
                windows::Win32::Security::OBJECT_SECURITY_INFORMATION(0)
            };
        let code = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                info,
                Some(&mut owner),
                None,
                Some(&mut dacl),
                Some(&mut sacl),
                Some(&mut descriptor),
            )
        };
        if code.0 != 0 {
            return Err(super::code_error("GetSecurityInfo", code.0));
        }
        let _allocation = LocalAllocation(descriptor.0);
        if !self.trusted(&sid_string(owner)?, managed) {
            return Err(TuningNativeError::UnsafeStore("untrusted object owner"));
        }
        if dacl.is_null() {
            return Err(TuningNativeError::UnsafeStore("missing DACL"));
        }
        let mut control = 0;
        let mut revision = 0;
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }
            .map_err(|e| native_error("GetSecurityDescriptorControl", e))?;
        if managed && control & SE_DACL_PROTECTED.0 == 0 {
            return Err(TuningNativeError::UnsafeStore("unprotected managed DACL"));
        }
        let entries = acl_entries(dacl)?;
        if entries.is_empty() {
            return Err(TuningNativeError::UnsafeStore("empty DACL"));
        }
        let mut required_full = false;
        for entry in entries {
            if entry.flags & INHERIT_ONLY_ACE.0 as u8 != 0 {
                continue;
            }
            // Ordinary deny ACEs cannot grant access; unknown/conditional ACEs are not admitted.
            if entry.kind == 1 {
                continue;
            }
            if entry.kind != 0 {
                return Err(TuningNativeError::Unsupported("complex storage DACL"));
            }
            let trusted = self.trusted(&entry.sid, managed);
            if unsafe_grant(entry.mask) && !trusted {
                return Err(TuningNativeError::UnsafeStore(
                    "untrusted storage mutation grant",
                ));
            }
            let required_sid = self.test_sid.as_deref().unwrap_or(ADMINISTRATORS);
            if entry.sid == required_sid && entry.mask & 0x001f_01ff == 0x001f_01ff {
                required_full = true;
            }
        }
        if managed && !required_full {
            return Err(TuningNativeError::UnsafeStore("missing authority grant"));
        }
        if managed && self.test_sid.is_none() {
            let labels = acl_entries(sacl)?;
            if !high_label_ok(&labels) {
                return Err(TuningNativeError::UnsafeStore(
                    "missing high integrity no-write-up",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Ace {
    pub kind: u8,
    pub flags: u8,
    pub mask: u32,
    pub sid: String,
}
pub(super) fn config_security(handle: HANDLE) -> Result<(String, Vec<Ace>), TuningNativeError> {
    let mut owner = PSID::default();
    let mut dacl = ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let code = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            Some(&mut descriptor),
        )
    };
    if code.0 != 0 {
        return Err(super::code_error("config GetSecurityInfo", code.0));
    }
    let _allocation = LocalAllocation(descriptor.0);
    Ok((sid_string(owner)?, acl_entries(dacl)?))
}
fn high_label_ok(labels: &[Ace]) -> bool {
    labels.len() == 1
        && labels[0].kind == 17
        && labels[0].sid == "S-1-16-12288"
        && labels[0].mask == 1
        && labels[0].flags & INHERIT_ONLY_ACE.0 as u8 == 0
}
fn acl_entries(acl: *const ACL) -> Result<Vec<Ace>, TuningNativeError> {
    if acl.is_null() {
        return Err(TuningNativeError::UnsafeStore("missing ACL"));
    }
    let mut info = ACL_SIZE_INFORMATION::default();
    unsafe {
        GetAclInformation(
            acl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    }
    .map_err(|e| native_error("GetAclInformation", e))?;
    if info.AceCount > 256 || info.AclBytesInUse > 65536 {
        return Err(TuningNativeError::Unsupported("oversized storage ACL"));
    }
    let mut entries = Vec::new();
    for index in 0..info.AceCount {
        let mut ace = ptr::null_mut();
        unsafe { GetAce(acl, index, &mut ace) }.map_err(|e| native_error("GetAce", e))?;
        let header = unsafe { ptr::read_unaligned(ace.cast::<ACE_HEADER>()) };
        if !matches!(header.AceType, 0 | 1 | 17) {
            return Err(TuningNativeError::Unsupported("complex storage ACE"));
        }
        if header.AceSize < 16 {
            return Err(TuningNativeError::UnsafeStore("malformed ACE"));
        }
        let allowed = unsafe { ptr::read_unaligned(ace.cast::<ACCESS_ALLOWED_ACE>()) };
        let sid_ptr = unsafe { ace.cast::<u8>().add(8) };
        let sid_length = 8 + unsafe { *sid_ptr.add(1) } as usize * 4;
        if sid_length + 8 > header.AceSize as usize {
            return Err(TuningNativeError::UnsafeStore("malformed ACE SID"));
        }
        entries.push(Ace {
            kind: header.AceType,
            flags: header.AceFlags,
            mask: allowed.Mask,
            sid: sid_string(PSID(sid_ptr.cast()))?,
        });
    }
    Ok(entries)
}

pub(super) fn sid_string(sid: PSID) -> Result<String, TuningNativeError> {
    let mut value = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut value) }
        .map_err(|e| native_error("ConvertSidToStringSidW", e))?;
    let _allocation = LocalAllocation(value.0.cast());
    unsafe { value.to_string() }
        .map_err(|_| TuningNativeError::Unsupported("invalid native SID string"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn p1b_dacl_characterization_creation_is_distinct_by_one_trusted_read_ace() {
        const PRINCIPAL: &str = "S-1-5-21-1-2-3-1001";
        let base = config_sddl(PRINCIPAL);
        let stage = config_characterization_sddl(PRINCIPAL);
        assert_ne!(base, stage);
        assert_eq!(stage.strip_suffix("(A;;FR;;;SY)"), Some(base.as_str()));
    }
    #[test]
    fn raw_generic_and_destructive_relative_ancestor_grants_are_refused() {
        for mask in [0x40, 0x10000, 0x40000, 0x80000, 0x10000000, 0x40000000] {
            assert!(
                relative_ancestor_grant_is_unsafe(mask),
                "raw mask={mask:#x}"
            );
        }
    }
    #[test]
    fn ordinary_ancestor_grants_that_can_set_reparse_data_are_refused() {
        // Directory FILE_ADD_FILE has the same access bit as FILE_WRITE_DATA.
        // The documented reparse access check admits either this bit or
        // FILE_WRITE_ATTRIBUTES; sharing flags do not supply an alternative proof.
        for mask in [0x0000_0002, 0x0000_0100, 0x0000_0116] {
            assert!(untrusted_grant_is_unsafe(false, mask), "mask={mask:#x}");
        }
    }
    #[test]
    fn ancestor_read_and_subdirectory_create_grants_keep_their_prior_policy() {
        for mask in [0x0000_0001, 0x0000_0004, 0x0012_00a9, 0x8000_0000] {
            assert!(!untrusted_grant_is_unsafe(false, mask), "mask={mask:#x}");
            assert!(untrusted_grant_is_unsafe(true, mask));
        }
        for mask in [
            0x0001_0000,
            0x0004_0000,
            0x0008_0000,
            0x1000_0000,
            0x4000_0000,
        ] {
            assert!(untrusted_grant_is_unsafe(false, mask));
        }
    }
    #[test]
    fn inheritance_only_integrity_label_does_not_protect_this_object() {
        let label = |flags| Ace {
            kind: 17,
            flags,
            mask: 1,
            sid: "S-1-16-12288".to_owned(),
        };
        assert!(high_label_ok(&[label(0)]));
        assert!(!high_label_ok(&[label(INHERIT_ONLY_ACE.0 as u8)]));
    }
}
