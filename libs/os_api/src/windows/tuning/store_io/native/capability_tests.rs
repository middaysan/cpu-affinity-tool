//! Actual owned descriptor/handle observations, never canonical store evidence.
use super::*;
use crate::windows::tuning::{
    security::{LocalAllocation, sid_string},
    test_support::TuningTemporaryDirectory,
};
use std::{fs::OpenOptions, path::PathBuf};
use windows::Win32::{
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, PSECURITY_DESCRIPTOR, PSID,
        SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{CreateDirectoryW, DELETE},
};

struct Fixture {
    children: Vec<(PathBuf, ObjectId, Kind)>,
    descriptor: Rc<Descriptor>,
    policy: Rc<SecurityPolicy>,
    root: TuningTemporaryDirectory,
}
impl Fixture {
    fn new() -> Self {
        let root = TuningTemporaryDirectory::new().expect("owned capability root setup");
        let sid = crate::windows::OS::tuning_current_token_identity()
            .expect("actual fixture token identity")
            .sid()
            .to_owned();
        Self {
            children: Vec::new(),
            descriptor: Rc::new(Descriptor::new(Some(&sid)).expect("owned fixture descriptor")),
            policy: Rc::new(SecurityPolicy {
                test_sid: Some(sid),
            }),
            root,
        }
    }
    fn directory(&self, scope: Option<bool>) -> Directory {
        let directory = Directory::bootstrap(
            self.root.path(),
            Rc::clone(&self.descriptor),
            Rc::clone(&self.policy),
            scope,
        )
        .expect("actual owned bootstrap handle");
        let (volume, index) = self.root.original_identity();
        assert!(
            directory.id() == ObjectId { volume, index },
            "actual original root ID"
        );
        directory
    }
    fn create(&mut self, name: &str, kind: Kind, ordinary_mask: Option<u32>) -> ObjectId {
        assert!(component(&name.encode_utf16().collect::<Vec<_>>()).is_ok());
        let path = self.root.path().join(name);
        if kind == Kind::Directory {
            let sid = self.policy.test_sid.as_deref().unwrap();
            let ordinary = ordinary_mask
                .map(|mask| format!("(A;;0x{mask:x};;;WD)"))
                .unwrap_or_default();
            let text = format!("O:{sid}D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;{sid}){ordinary}");
            let mut raw_descriptor = PSECURITY_DESCRIPTOR::default();
            let sddl = wide(std::ffi::OsStr::new(&text)).unwrap();
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(sddl.as_ptr()),
                    SDDL_REVISION_1,
                    &mut raw_descriptor,
                    None,
                )
            }
            .expect("owned descriptor conversion");
            let allocation = LocalAllocation(raw_descriptor.0);
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: allocation.0,
                bInheritHandle: false.into(),
            };
            let text = wide(path.as_os_str()).unwrap();
            unsafe { CreateDirectoryW(PCWSTR(text.as_ptr()), Some(&attributes)) }
                .expect("owned descriptor directory setup");
        } else {
            assert!(ordinary_mask.is_none());
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .expect("owned file setup");
        }
        let file = fixture_open(&path, false);
        let id = identity(&file, kind).expect("actual created fixture ID/type");
        self.children.push((path, id, kind));
        id
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Test Nodes/root pins are declared after Fixture and therefore close
        // first on unwind. Remove only captured flat objects through exact IDs.
        for (path, expected, kind) in self.children.iter().rev() {
            let Ok(file) = fixture_try_open(path, true) else {
                continue;
            };
            if identity(&file, *kind).ok() != Some(*expected) {
                continue;
            }
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            let _ = unsafe {
                SetFileInformationByHandle(
                    raw(&file),
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            };
        }
    }
}
fn fixture_try_open(path: &Path, delete: bool) -> Result<File> {
    let name = wide(path.as_os_str())?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            (FILE_READ_ATTRIBUTES | READ_CONTROL).0 | if delete { DELETE.0 } else { 0 },
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|e| native_error("owned capability fixture open", e))?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}
fn fixture_open(path: &Path, delete: bool) -> File {
    fixture_try_open(path, delete).expect("actual owned fixture handle")
}
fn name(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}
/// Independent characterization of the one fixed Everyone ACE in this owned
/// fixture. Only its mask may be printed; no descriptor, SID or path is emitted.
fn fixture_observed_everyone_mask(file: &File) -> u32 {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let mut dacl = std::ptr::null_mut();
    let code = unsafe {
        GetSecurityInfo(
            raw(file),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            Some(&mut descriptor),
        )
    };
    assert_eq!(code.0, 0, "owned fixture SD-mask query");
    let _allocation = LocalAllocation(descriptor.0);
    assert!(!dacl.is_null(), "owned fixture mask requires DACL");
    let mut info = ACL_SIZE_INFORMATION::default();
    unsafe {
        GetAclInformation(
            dacl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    }
    .expect("owned fixture ACL-size query");
    assert!(info.AceCount <= 8 && info.AclBytesInUse <= 65536);
    let mut observed = None;
    for index in 0..info.AceCount {
        let mut ace = std::ptr::null_mut();
        unsafe { GetAce(dacl, index, &mut ace) }.expect("owned fixture ACE query");
        let header = unsafe { std::ptr::read_unaligned(ace.cast::<ACE_HEADER>()) };
        assert!(
            header.AceType == 0 && header.AceSize >= 16,
            "owned fixed allow ACE"
        );
        let sid = unsafe { ace.cast::<u8>().add(8) };
        let sid_length = 8 + usize::from(unsafe { *sid.add(1) }) * 4;
        assert!(
            sid_length + 8 <= usize::from(header.AceSize),
            "owned ACE SID extent"
        );
        if sid_string(PSID(sid.cast())).expect("owned fixed ACE trustee") == "S-1-1-0" {
            assert!(observed.is_none(), "one fixed Everyone ACE");
            observed =
                Some(unsafe { std::ptr::read_unaligned(ace.cast::<ACCESS_ALLOWED_ACE>()) }.Mask);
        }
    }
    observed.expect("one fixed Everyone ACE mask")
}

#[test]
fn actual_relative_ancestor_0116_is_accepted_only_through_bound_capability() {
    let mut fixture = Fixture::new();
    let expected = fixture.create("ordinary-metadata", Kind::Directory, Some(0x116));
    let root = fixture.directory(None);
    let independent = fixture_open(&fixture.root.path().join("ordinary-metadata"), false);
    assert!(
        matches!(
            fixture.policy.validate(raw(&independent), false),
            Err(TuningNativeError::UnsafeStore(
                "untrusted storage mutation grant"
            ))
        ),
        "strict ancestor policy must stay closed for 0x116"
    );
    assert!(
        matches!(
            fixture.policy.validate(raw(&independent), true),
            Err(TuningNativeError::UnsafeStore(
                "untrusted storage mutation grant"
            ))
        ),
        "managed policy must stay closed for 0x116"
    );
    let result = root.open(
        &name("ordinary-metadata"),
        Kind::Directory,
        OpenMode::Existing,
        Sharing::Retained,
        true,
        Some(false),
    );
    assert!(
        result.is_ok(),
        "actual relative ancestor 0x116 capability admission"
    );
    let (node, disposition) = result.unwrap();
    assert_eq!(disposition, Disposition::Opened);
    assert!(node.id() == expected, "bound actual ancestor ID");
    let capability = node
        .relative_ancestor_capability()
        .expect("actual bound capability");
    fixture
        .policy
        .validate_relative_ancestor(&capability)
        .expect("bound relative policy");
}

#[test]
fn actual_capability_eligibility_refuses_bootstrap_managed_file_and_delete_sharing() {
    let mut fixture = Fixture::new();
    for value in ["retained", "managed", "renameable", "unscoped"] {
        fixture.create(value, Kind::Directory, None);
    }
    fixture.create("file", Kind::File, None);
    let root = fixture.directory(Some(false));
    assert!(
        root.node.relative_ancestor_capability().is_err(),
        "bootstrap cannot mint"
    );
    for (value, kind, sharing, scope, eligible) in [
        (
            "retained",
            Kind::Directory,
            Sharing::Retained,
            Some(false),
            true,
        ),
        (
            "managed",
            Kind::Directory,
            Sharing::Retained,
            Some(true),
            false,
        ),
        (
            "renameable",
            Kind::Directory,
            Sharing::Renameable,
            Some(false),
            false,
        ),
        ("unscoped", Kind::Directory, Sharing::Retained, None, false),
        ("file", Kind::File, Sharing::Retained, Some(false), false),
    ] {
        let (node, _) = root
            .open(&name(value), kind, OpenMode::Existing, sharing, true, scope)
            .expect("actual capability role setup");
        assert_eq!(
            node.relative_ancestor_capability().is_ok(),
            eligible,
            "actual role eligibility"
        );
    }
}

#[test]
fn actual_relative_ancestor_forbidden_grants_remain_refused() {
    let mut fixture = Fixture::new();
    let masks = [0x40, 0x10000, 0x40000, 0x80000, 0x10000000];
    for (index, mask) in masks.into_iter().enumerate() {
        fixture.create(&format!("forbidden-{index}"), Kind::Directory, Some(mask));
    }
    let root = fixture.directory(None);
    for (index, requested_mask) in masks.into_iter().enumerate() {
        let independent = fixture_open(
            &fixture.root.path().join(format!("forbidden-{index}")),
            false,
        );
        let observed_mask = fixture_observed_everyone_mask(&independent);
        let expected_observed = if requested_mask == 0x10000000 {
            0x001f01ff
        } else {
            requested_mask
        };
        assert_eq!(
            observed_mask, expected_observed,
            "owned effective dangerous grant mask"
        );
        let result = root.open(
            &name(&format!("forbidden-{index}")),
            Kind::Directory,
            OpenMode::Existing,
            Sharing::Retained,
            true,
            Some(false),
        );
        assert!(
            matches!(
                result,
                Err(TuningNativeError::UnsafeStore(
                    "untrusted storage mutation grant"
                ))
            ),
            "forbidden ordinary ancestor grant must refuse: index={index}, requested={:#010x}, observed={observed_mask:#010x}",
            requested_mask
        );
    }
    assert_eq!(root.entries().unwrap().len(), masks.len());
}

#[test]
fn actual_mapped_generic_write_is_a_concrete_relative_metadata_grant() {
    let mut fixture = Fixture::new();
    let expected = fixture.create("mapped-generic-write", Kind::Directory, Some(0x40000000));
    let root = fixture.directory(None);
    let independent = fixture_open(&fixture.root.path().join("mapped-generic-write"), false);
    // Windows maps the requested generic spelling in the object's DACL. The
    // stored concrete rights contain no request-origin provenance.
    assert_eq!(
        fixture_observed_everyone_mask(&independent),
        0x00120116,
        "actual stored mapped GENERIC_WRITE rights"
    );
    for managed in [false, true] {
        assert!(
            matches!(
                fixture.policy.validate(raw(&independent), managed),
                Err(TuningNativeError::UnsafeStore(
                    "untrusted storage mutation grant"
                ))
            ),
            "strict and Managed validation must refuse mapped metadata grant"
        );
    }
    let (node, disposition) = root
        .open(
            &name("mapped-generic-write"),
            Kind::Directory,
            OpenMode::Existing,
            Sharing::Retained,
            true,
            Some(false),
        )
        .expect("actual mapped metadata rights through bound relative entry");
    assert_eq!(disposition, Disposition::Opened);
    assert!(
        node.id() == expected,
        "same full object under mapped rights"
    );
    let capability = node
        .relative_ancestor_capability()
        .expect("actual mapped-grant capability");
    fixture
        .policy
        .validate_relative_ancestor(&capability)
        .expect("bound effective-rights policy");
    assert!(
        identity(&independent, Kind::Directory).unwrap() == expected,
        "independent actual handle still denotes the original object"
    );
}
