//! Read-only Windows characterization of a fixed in-memory placeholder ACE.
//! No descriptor is applied to a file, registry key, token or canonical store.

use super::{
    native_token,
    registry_policy::{KEY_ALL, KEY_READ, KEY_WRITE, Scope, SidClass},
    security_descriptor,
};
use std::mem::{offset_of, size_of};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE, HLOCAL, LocalFree},
        Security::{
            AccessCheck,
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            CreatePrivateObjectSecurity, DestroyPrivateObjectSecurity, DuplicateTokenEx,
            GENERIC_MAPPING, GetSecurityDescriptorLength, GetTokenInformation, LUID_AND_ATTRIBUTES,
            PRIVILEGE_SET, PSECURITY_DESCRIPTOR, SecurityImpersonation, TOKEN_DUPLICATE,
            TOKEN_QUERY, TOKEN_USER, TokenImpersonation, TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
    core::{BOOL, HRESULT, PCWSTR},
};

const MAX_DESCRIPTOR_BYTES: usize = 65_536;
const MAX_PRIVILEGES: usize = 64;
const SYSTEM_SID: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
const CREATOR_OWNER_SID: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0];
const EVERYONE_SID: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];
const REGISTRY_MAPPING: GENERIC_MAPPING = GENERIC_MAPPING {
    GenericRead: KEY_READ,
    GenericWrite: KEY_WRITE,
    GenericExecute: KEY_READ,
    GenericAll: KEY_ALL,
};

struct TokenHandle(HANDLE);
impl TokenHandle {
    fn current_impersonation() -> Self {
        let mut primary = HANDLE::default();
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut primary,
            )
        };
        assert!(opened.is_ok(), "current token query/duplicate open failed");
        let primary = Self(primary);
        assert!(!primary.0.is_invalid(), "current token handle is invalid");

        let mut duplicate = HANDLE::default();
        let duplicated = unsafe {
            DuplicateTokenEx(
                primary.0,
                TOKEN_QUERY,
                None,
                SecurityImpersonation,
                TokenImpersonation,
                &mut duplicate,
            )
        };
        assert!(
            duplicated.is_ok(),
            "actual impersonation token duplicate failed"
        );
        let duplicate = Self(duplicate);
        assert!(
            !duplicate.0.is_invalid(),
            "duplicate token handle is invalid"
        );
        duplicate.assert_non_system_user();
        duplicate
    }

    fn assert_non_system_user(&self) {
        let header = size_of::<TOKEN_USER>();
        let mut required = 0;
        let sizing = unsafe { GetTokenInformation(self.0, TokenUser, None, 0, &mut required) };
        assert!(
            sizing.err().map(|error| error.code())
                == Some(HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0)),
            "actual client user sizing did not return its bounded query contract"
        );
        let length = native_token::variable_buffer_size(required, header)
            .expect("actual client user buffer is unsupported");
        let mut words = vec![0u64; length.div_ceil(size_of::<u64>())];
        let mut returned = 0;
        let queried = unsafe {
            GetTokenInformation(
                self.0,
                TokenUser,
                Some(words.as_mut_ptr().cast()),
                required,
                &mut returned,
            )
        };
        assert!(queried.is_ok(), "actual client user query failed");
        native_token::returned_buffer_size(length, returned, header)
            .expect("actual client user buffer size changed");
        // The full initialized header and exact returned byte count are checked
        // before copying its pointer; bounded_sid_bytes never dereferences it.
        let user = unsafe { std::ptr::read_unaligned(words.as_ptr().cast::<TOKEN_USER>()) };
        let bytes = unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), length) };
        let sid = native_token::bounded_sid_bytes(
            bytes,
            header,
            words.as_ptr() as usize,
            user.User.Sid.0 as usize,
        )
        .expect("actual client user SID is unsupported");
        // The fixed SY owner must not be the client; otherwise implicit owner
        // rights could invalidate the intended CO-only negative control.
        assert!(
            sid != SYSTEM_SID,
            "characterization requires a non-System client"
        );
    }
}
impl Drop for TokenHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct ConvertedDescriptor {
    pointer: PSECURITY_DESCRIPTOR,
    length: usize,
}
impl ConvertedDescriptor {
    fn fixed(sddl: PCWSTR) -> Self {
        let mut pointer = PSECURITY_DESCRIPTOR::default();
        let mut length = 0;
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl,
                SDDL_REVISION_1,
                &mut pointer,
                Some(&mut length),
            )
        };
        assert!(
            converted.is_ok(),
            "fixed in-memory descriptor conversion failed"
        );
        // Own the successful SDK allocation before any shape rejection.
        let descriptor = Self {
            pointer,
            length: length as usize,
        };
        descriptor.assert_bounds();
        descriptor
    }

    fn assert_bounds(&self) {
        assert!(
            !self.pointer.0.is_null(),
            "descriptor allocation is missing"
        );
        assert!(
            (self.pointer.0 as usize).is_multiple_of(4)
                && (20..=MAX_DESCRIPTOR_BYTES).contains(&self.length),
            "descriptor allocation is outside fixed bounds"
        );
    }

    fn bytes(&self) -> &[u8] {
        // Only a successful converter allocation with checked length is used.
        unsafe { std::slice::from_raw_parts(self.pointer.0.cast(), self.length) }
    }

    fn assert_single_parent_ace(&self, expected_sid: &[u8; 12]) {
        let bytes = self.bytes();
        let facts = security_descriptor::parse_descriptor(bytes, Scope::Ancestor)
            .expect("fixed parent descriptor does not parse");
        assert_eq!(facts.owner, SidClass::System);
        assert_eq!(facts.dacl.len(), 1);
        assert!(facts.labels.is_empty());
        assert_eq!(facts.dacl[0].kind, 0);
        assert_eq!(facts.dacl[0].flags, 2);
        assert_eq!(facts.dacl[0].mask, KEY_ALL);

        let group = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        assert!(
            bytes.get(group..group + SYSTEM_SID.len()) == Some(SYSTEM_SID.as_slice()),
            "fixed parent group differs"
        );
        let dacl = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let sid = dacl + 8 + 8;
        assert!(
            bytes.get(sid..sid + expected_sid.len()) == Some(expected_sid.as_slice()),
            "fixed parent trustee differs"
        );
    }
}
impl Drop for ConvertedDescriptor {
    fn drop(&mut self) {
        if !self.pointer.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.pointer.0)));
            }
        }
    }
}

#[repr(C)]
struct PrivilegeBuffer {
    count: u32,
    control: u32,
    privileges: [LUID_AND_ATTRIBUTES; MAX_PRIVILEGES],
}

fn access(token: &TokenHandle, descriptor: &ConvertedDescriptor, requested: u32) -> (bool, u32) {
    assert_eq!(
        requested & 0xf000_0000,
        0,
        "request must contain concrete key rights"
    );
    let mut privileges = PrivilegeBuffer {
        count: 0,
        control: 0,
        privileges: [LUID_AND_ATTRIBUTES::default(); MAX_PRIVILEGES],
    };
    let capacity = size_of::<PrivilegeBuffer>() as u32;
    let mut returned = capacity;
    let mut granted = 0;
    let mut status = BOOL::default();
    let checked = unsafe {
        AccessCheck(
            descriptor.pointer,
            token.0,
            requested,
            &REGISTRY_MAPPING,
            Some((&mut privileges as *mut PrivilegeBuffer).cast::<PRIVILEGE_SET>()),
            &mut returned,
            &mut granted,
            &mut status,
        )
    };
    assert!(checked.is_ok(), "bounded native AccessCheck failed");
    assert!(
        (offset_of!(PrivilegeBuffer, privileges) as u32..=capacity).contains(&returned)
            && privileges.count as usize <= MAX_PRIVILEGES
            && offset_of!(PrivilegeBuffer, privileges)
                + privileges.count as usize * size_of::<LUID_AND_ATTRIBUTES>()
                <= returned as usize,
        "native privilege output is outside fixed bounds"
    );
    (status.as_bool(), granted)
}

#[test]
fn native_creator_owner_ci_placeholder_does_not_grant_parent_key_mutation() {
    let token = TokenHandle::current_impersonation();
    let placeholder = ConvertedDescriptor::fixed(windows::core::w!("O:SYG:SYD:(A;CI;KA;;;CO)"));
    let everyone = ConvertedDescriptor::fixed(windows::core::w!("O:SYG:SYD:(A;CI;KA;;;WD)"));
    placeholder.assert_single_parent_ace(&CREATOR_OWNER_SID);
    everyone.assert_single_parent_ace(&EVERYONE_SID);

    // Identical real client and concrete requests make failure of the negative
    // control meaningful: the positive control must grant each same request.
    for requested in [0x0002, 0x0004, 0x0020, 0x0001_0000] {
        let (allowed, granted) = access(&token, &everyone, requested);
        assert!(
            allowed && granted == requested,
            "Everyone positive control did not grant key mutation"
        );
        let (allowed, granted) = access(&token, &placeholder, requested);
        assert!(
            !allowed && granted == 0,
            "CreatorOwner placeholder granted parent key mutation"
        );
    }
    // There was no impersonation of this thread, privilege adjustment, object
    // creation or applied descriptor. These are private access-check decisions.
}

struct DerivedDescriptor {
    pointer: PSECURITY_DESCRIPTOR,
    length: usize,
}
impl DerivedDescriptor {
    fn protected_child(
        parent: &ConvertedDescriptor,
        requested: &ConvertedDescriptor,
        token: &TokenHandle,
    ) -> Self {
        let mut pointer = PSECURITY_DESCRIPTOR::default();
        let derived = unsafe {
            CreatePrivateObjectSecurity(
                Some(parent.pointer),
                Some(requested.pointer),
                &mut pointer,
                true,
                Some(token.0),
                &REGISTRY_MAPPING,
            )
        };
        assert!(
            derived.is_ok(),
            "in-memory protected child derivation failed"
        );
        let mut descriptor = Self { pointer, length: 0 };
        assert!(
            !descriptor.pointer.0.is_null() && (descriptor.pointer.0 as usize).is_multiple_of(4),
            "derived descriptor allocation is invalid"
        );
        descriptor.length = unsafe { GetSecurityDescriptorLength(descriptor.pointer) } as usize;
        assert!(
            (20..=MAX_DESCRIPTOR_BYTES).contains(&descriptor.length),
            "derived descriptor allocation is outside fixed bounds"
        );
        descriptor
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.pointer.0.cast(), self.length) }
    }
}
impl Drop for DerivedDescriptor {
    fn drop(&mut self) {
        if !self.pointer.0.is_null() {
            unsafe {
                let _ = DestroyPrivateObjectSecurity(std::ptr::addr_of_mut!(self.pointer));
            }
        }
    }
}

#[test]
fn native_protected_child_derivation_excludes_parent_creator_owner_placeholder() {
    let token = TokenHandle::current_impersonation();
    let parent = ConvertedDescriptor::fixed(windows::core::w!("O:SYG:SYD:(A;CI;KA;;;CO)"));
    parent.assert_single_parent_ace(&CREATOR_OWNER_SID);
    // Owner/group default from the actual token. This is a protected DACL
    // derivation, never proof of canonical owner, High label or key security.
    let requested = ConvertedDescriptor::fixed(windows::core::w!("D:P(A;;KA;;;SY)(A;;KA;;;BA)"));
    let child = DerivedDescriptor::protected_child(&parent, &requested, &token);
    let facts = security_descriptor::parse_descriptor(child.bytes(), Scope::Managed)
        .expect("derived bounded protected child does not parse");
    assert!(facts.dacl_present && facts.dacl_protected);
    assert_eq!(facts.dacl.len(), 2);
    for ace in &facts.dacl {
        assert_eq!(ace.kind, 0);
        assert_eq!(ace.flags, 0);
        assert_eq!(ace.mask, KEY_ALL);
    }
    assert_eq!(
        facts
            .dacl
            .iter()
            .filter(|ace| ace.sid == SidClass::System)
            .count(),
        1
    );
    assert_eq!(
        facts
            .dacl
            .iter()
            .filter(|ace| ace.sid == SidClass::Administrators)
            .count(),
        1
    );
    // Both exact trusted grants exhaust the child DACL: no inherited generic
    // placeholder or mapped actual-user ACE survives protected creation.
}
