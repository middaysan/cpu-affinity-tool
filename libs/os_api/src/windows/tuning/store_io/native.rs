//! Owned handle-relative NTFS operations for the production metadata store.
//! Only bootstrap selects a path. Descendants have no pathname fallback.
use super::super::{
    code_error,
    config_source::ConfigSourcePolicy,
    native_error,
    security::{Descriptor, SecurityPolicy},
    wide,
};
use super::{
    CALL_LIMIT, Disposition, ENUM_BYTES, Entry, Kind, NativeProgress, OpenMode, OpenPlan, Result,
    Sharing, component, config_read_plan, disposition, io_progress, observed_attributes, open_plan,
    parse_records, retain_nonterminal,
};
use crate::tuning::TuningNativeError;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    mem::{offset_of, size_of, size_of_val},
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::Path,
    rc::Rc,
};
use windows::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_RENAME_INFORMATION, FileRenameInformation, NtCreateFile, NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::{
            GetLastError, HANDLE, NTSTATUS, OBJECT_ATTRIBUTE_FLAGS, RtlNtStatusToDosError,
            UNICODE_STRING,
        },
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
            FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
            FILE_TRAVERSE, FILE_TYPE_DISK, FileDispositionInfo, FileIdBothDirectoryInfo,
            FileIdBothDirectoryRestartInfo, FileIdInfo, GetDriveTypeW, GetFileInformationByHandle,
            GetFileInformationByHandleEx, GetFileType, GetFinalPathNameByHandleW, OPEN_EXISTING,
            READ_CONTROL, SYNCHRONIZE, SetFileInformationByHandle, VOLUME_NAME_GUID,
        },
        System::IO::IO_STATUS_BLOCK,
    },
    core::{PCWSTR, PWSTR},
};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::windows::tuning) struct ObjectId {
    pub volume: u32,
    pub index: u64,
}
impl std::fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoreObjectId(..)")
    }
}
pub(in crate::windows::tuning) struct Node {
    file: File,
    id: ObjectId,
    kind: Kind,
    descriptor: Rc<Descriptor>,
    policy: NativePolicy,
    scope: Option<bool>,
    // Present only after the actual relative NtCreateFile used this exact plan.
    // Bootstrap handles cannot produce relative-ancestor evidence.
    relative_plan: Option<OpenPlan>,
}
#[derive(Clone)]
enum NativePolicy {
    Canonical(Rc<SecurityPolicy>),
    Config(Rc<ConfigSourcePolicy>),
}
impl NativePolicy {
    fn validate(&self, handle: HANDLE, managed: bool) -> Result<()> {
        match self {
            Self::Canonical(policy) => policy.validate(handle, managed),
            Self::Config(policy) => policy.validate(handle, managed),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::windows::tuning) struct FullObjectId {
    pub volume: u64,
    pub id: [u8; 16],
}
impl Node {
    pub(in crate::windows::tuning) fn full_id(&self) -> Result<FullObjectId> {
        self.verify()?;
        full_identity(&self.file)
    }
    pub(in crate::windows::tuning) fn config_dacl(
        &self,
    ) -> Result<Vec<super::super::security::Ace>> {
        self.verify()?;
        if !matches!(self.policy, NativePolicy::Config(_)) {
            return Err(TuningNativeError::Unsupported(
                "config native policy required",
            ));
        }
        super::super::security::config_security(raw(&self.file)).map(|(_, entries)| entries)
    }
    /// Only configuration publication checks its retained old object after the
    /// final leaf has independently been verified. A retired object is not a
    /// current qualified source and cannot supply a new receipt.
    pub(in crate::windows::tuning) fn config_retired_bytes(
        &mut self,
        expected: FullObjectId,
        limit: usize,
    ) -> Result<Vec<u8>> {
        if !matches!(self.policy, NativePolicy::Config(_)) || self.kind != Kind::File {
            return Err(TuningNativeError::Unsupported("config retired object"));
        }
        let old_id = self.id;
        read_file(&mut self.file, limit, |file| {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe { GetFileInformationByHandle(raw(file), &mut info) }
                .map_err(|e| native_error("config retired identity", e))?;
            observed_attributes(info.dwFileAttributes, Kind::File)?;
            if unsafe { GetFileType(raw(file)) } != FILE_TYPE_DISK
                || info.nNumberOfLinks > 1
                || info.dwVolumeSerialNumber != old_id.volume
                || ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
                    != old_id.index
                || full_identity(file)? != expected
            {
                return Err(TuningNativeError::UnsafeStore(
                    "config retired object changed",
                ));
            }
            Ok(())
        })
    }
}
#[cfg(any(test, feature = "tuning-test-controls"))]
impl Node {
    /// Duplicate only this already owned and verified object. No raw handle or
    /// caller identity can construct a lab deletion receipt.
    pub(in crate::windows::tuning) fn clone_for_lab(&self) -> Result<Self> {
        if self.kind != Kind::File {
            return Err(TuningNativeError::UnsafeStore("lab owned file required"));
        }
        self.verify()?;
        let file = self
            .file
            .try_clone()
            .map_err(|e| io_error("lab owned handle", e))?;
        let node = Self {
            file,
            id: self.id,
            kind: self.kind,
            descriptor: Rc::clone(&self.descriptor),
            policy: self.policy.clone(),
            scope: self.scope,
            relative_plan: self.relative_plan.as_ref().map(|plan| OpenPlan {
                desired: plan.desired,
                share: plan.share,
                disposition: plan.disposition,
                options: plan.options,
                attributes: plan.attributes,
            }),
        };
        node.verify()?;
        Ok(node)
    }
    pub(in crate::windows::tuning) fn scalar_io(&self) -> Result<TuningLabScalarIo> {
        self.verify()?;
        if self.kind != Kind::File || self.scope != Some(true) {
            return Err(TuningNativeError::UnsafeStore("owned scalar file required"));
        }
        Ok(TuningLabScalarIo {
            file: std::sync::Mutex::new(
                self.file
                    .try_clone()
                    .map_err(|e| io_error("owned scalar handle", e))?,
            ),
            id: self.id,
            policy: match &self.policy {
                NativePolicy::Canonical(policy) => (**policy).clone(),
                NativePolicy::Config(_) => {
                    return Err(TuningNativeError::Unsupported(
                        "config is not lab authority",
                    ));
                }
            },
            creator: std::thread::current().id(),
        })
    }
}
/// Sealed IO for one actual created four-byte lab object. No raw/path constructor.
#[cfg(any(test, feature = "tuning-test-controls"))]
pub struct TuningLabScalarIo {
    file: std::sync::Mutex<File>,
    id: ObjectId,
    policy: SecurityPolicy,
    creator: std::thread::ThreadId,
}
#[cfg(any(test, feature = "tuning-test-controls"))]
impl std::fmt::Debug for TuningLabScalarIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningLabScalarIo(redacted)")
    }
}
#[cfg(any(test, feature = "tuning-test-controls"))]
impl TuningLabScalarIo {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, File>> {
        if std::thread::current().id() != self.creator {
            return Err(TuningNativeError::UnsafeStore("owned scalar worker"));
        }
        self.file
            .lock()
            .map_err(|_| TuningNativeError::UnsafeStore("owned scalar cursor"))
    }
    pub fn read(&self) -> Result<i32> {
        let mut file = self.lock()?;
        let bytes = read_file(&mut file, 4, |file| {
            verify_file(file, self.id, &self.policy, Some(true))
        })?;
        Ok(i32::from_le_bytes(bytes.try_into().map_err(|_| {
            TuningNativeError::UnsafeStore("owned scalar length")
        })?))
    }
    pub fn write(&self, value: i32) -> Result<()> {
        if !matches!(value, 7 | 12) {
            return Err(TuningNativeError::UnsafeStore("owned scalar value"));
        }
        let mut file = self.lock()?;
        verify_file(&file, self.id, &self.policy, Some(true))?;
        if file
            .metadata()
            .map_err(|e| io_error("owned scalar length", e))?
            .len()
            != 4
        {
            return Err(TuningNativeError::UnsafeStore("owned scalar length"));
        }
        write_file(&mut file, &value.to_le_bytes(), 4, |file| {
            verify_file(file, self.id, &self.policy, Some(true))
        })
    }
}
/// Borrowed proof of this exact retained relative ancestor. No raw constructor,
/// cloning, or independent caller-selected handle can enter the security seam.
pub(in crate::windows::tuning) struct RelativeAncestorCapability<'a> {
    node: &'a Node,
}
impl RelativeAncestorCapability<'_> {
    pub(in crate::windows::tuning) fn verify_object(&self) -> Result<()> {
        self.node.verify_identity()
    }
    pub(in crate::windows::tuning) fn handle(&self) -> HANDLE {
        raw(&self.node.file)
    }
}
pub(in crate::windows::tuning) struct Directory {
    node: Node,
}
/// Every native-used create input and both outputs share one stable allocation.
/// An ambiguous result parks this entire request and the outer opening/store.
struct CreateRequest {
    handle: HANDLE,
    io: IO_STATUS_BLOCK,
    name: Vec<u16>,
    unicode: UNICODE_STRING,
    attributes: OBJECT_ATTRIBUTES,
    descriptor: Rc<Descriptor>,
}
impl Directory {
    pub(in crate::windows::tuning) fn bootstrap(
        path: &Path,
        descriptor: Rc<Descriptor>,
        policy: Rc<SecurityPolicy>,
        scope: Option<bool>,
    ) -> Result<Self> {
        Self::bootstrap_policy(path, descriptor, NativePolicy::Canonical(policy), scope)
    }
    fn bootstrap_policy(
        path: &Path,
        descriptor: Rc<Descriptor>,
        policy: NativePolicy,
        scope: Option<bool>,
    ) -> Result<Self> {
        let text = wide(path.as_os_str())?;
        let handle = unsafe {
            CreateFileW(
                PCWSTR(text.as_ptr()),
                (FILE_LIST_DIRECTORY
                    | FILE_TRAVERSE
                    | FILE_READ_ATTRIBUTES
                    | READ_CONTROL
                    | SYNCHRONIZE)
                    .0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(|e| native_error("store bootstrap directory", e))?;
        let file = unsafe { File::from_raw_handle(handle.0) };
        let id = identity(&file, Kind::Directory)?;
        super::super::store::validate_volume(&file)?;
        let node = Node {
            file,
            id,
            kind: Kind::Directory,
            descriptor,
            policy,
            scope,
            relative_plan: None,
        };
        node.verify()?;
        Ok(Self { node })
    }
    /// The OS KnownFolder selects the drive. Global and current DOS roots must
    /// denote the same full GUID volume root and actual object. Descent retains
    /// only the global handle; a SUBST suffix or ambiguous alias never admits.
    pub(in crate::windows::tuning) fn canonical_bootstrap(
        letter: u8,
        descriptor: Rc<Descriptor>,
        policy: Rc<SecurityPolicy>,
    ) -> Result<Self> {
        Self::root_bootstrap(letter, descriptor, NativePolicy::Canonical(policy))
    }
    pub(in crate::windows::tuning) fn config_bootstrap(
        letter: u8,
        descriptor: Rc<Descriptor>,
        policy: Rc<ConfigSourcePolicy>,
    ) -> Result<Self> {
        Self::root_bootstrap(letter, descriptor, NativePolicy::Config(policy))
    }
    fn root_bootstrap(
        letter: u8,
        descriptor: Rc<Descriptor>,
        policy: NativePolicy,
    ) -> Result<Self> {
        if !letter.is_ascii_alphabetic() {
            return Err(TuningNativeError::UnsafeStore("store global volume root"));
        }
        let letter = char::from(letter.to_ascii_uppercase());
        let global_path = format!("\\\\?\\GLOBALROOT\\Global??\\{letter}:\\");
        let local_path = format!("{letter}:\\");
        let global = Self::bootstrap_policy(
            Path::new(&global_path),
            Rc::clone(&descriptor),
            policy.clone(),
            Some(false),
        )?;
        let local =
            Self::bootstrap_policy(Path::new(&local_path), descriptor, policy, Some(false))?;
        global.verify()?;
        local.verify()?;
        let global_guid = final_guid_root(&global.node.file)?;
        let local_guid = final_guid_root(&local.node.file)?;
        let guid_wide: Vec<_> = global_guid.iter().copied().chain([0]).collect();
        // DRIVE_FIXED is the documented GetDriveType return value 3.
        // Classify the captured GUID root, never a mutable DOS drive alias.
        let fixed = unsafe { GetDriveTypeW(PCWSTR(guid_wide.as_ptr())) } == 3;
        global.verify()?;
        local.verify()?;
        let global_after = final_guid_root(&global.node.file)?;
        let local_after = final_guid_root(&local.node.file)?;
        if global_after != global_guid || local_after != local_guid {
            return Err(TuningNativeError::UnsafeStore(
                "store global volume root changed",
            ));
        }
        super::global_root_proof(
            &global_after,
            &local_after,
            global.node.id == local.node.id,
            fixed,
            true,
        )?;
        global.verify()?;
        local.verify()?;
        Ok(global)
    }
    pub(in crate::windows::tuning) fn id(&self) -> ObjectId {
        self.node.id
    }
    pub(in crate::windows::tuning) fn verify(&self) -> Result<()> {
        self.node.verify()
    }
    pub(in crate::windows::tuning) fn full_id(&self) -> Result<FullObjectId> {
        self.verify()?;
        full_identity(&self.node.file)
    }
    pub(in crate::windows::tuning) fn guid_path(&self) -> Result<Vec<u16>> {
        self.verify()?;
        final_guid_root(&self.node.file)
    }
    pub(in crate::windows::tuning) fn config_leaf(
        &self,
        name: &[u16],
        publish_old: bool,
    ) -> Result<Node> {
        if !matches!(self.node.policy, NativePolicy::Config(_)) {
            return Err(TuningNativeError::Unsupported(
                "config native policy required",
            ));
        }
        self.open_inner(
            name,
            Kind::File,
            OpenMode::Existing,
            (Sharing::Renameable, true, Some(publish_old)),
            Some(true),
            || Ok(()),
        )
        .map(|(node, _)| node)
    }
    /// Characterization-only safe descriptor variation at creation. It changes
    /// no existing ACL and cannot select a caller-supplied policy/descriptor.
    #[cfg(test)]
    pub(in crate::windows::tuning) fn create_config_dacl_characterization_stage(
        &mut self,
    ) -> Result<(Node, Vec<u16>)> {
        let descriptor = match &self.node.policy {
            NativePolicy::Config(policy) => Rc::new(policy.characterization_descriptor()?),
            _ => {
                return Err(TuningNativeError::Unsupported(
                    "config characterization policy",
                ));
            }
        };
        let name: Vec<_> = ".state.json.tuning-dacl-characterization"
            .encode_utf16()
            .collect();
        let original = std::mem::replace(&mut self.node.descriptor, descriptor);
        let result = self.open(
            &name,
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Renameable,
            false,
            Some(true),
        );
        self.node.descriptor = original;
        result.map(|(node, _)| (node, name))
    }
    pub(in crate::windows::tuning) fn open(
        &self,
        name: &[u16],
        kind: Kind,
        mode: OpenMode,
        sharing: Sharing,
        read_only: bool,
        scope: Option<bool>,
    ) -> Result<(Node, Disposition)> {
        self.open_inner(name, kind, mode, (sharing, read_only, None), scope, || {
            Ok(())
        })
    }
    #[cfg(test)]
    fn open_after_parent_check(
        &self,
        name: &[u16],
        kind: Kind,
        mode: OpenMode,
        access: (Sharing, bool),
        scope: Option<bool>,
        after_parent_check: impl FnOnce() -> Result<()>,
    ) -> Result<(Node, Disposition)> {
        self.open_inner(
            name,
            kind,
            mode,
            (access.0, access.1, None),
            scope,
            after_parent_check,
        )
    }
    fn open_inner(
        &self,
        name: &[u16],
        kind: Kind,
        mode: OpenMode,
        access: (Sharing, bool, Option<bool>),
        scope: Option<bool>,
        after_parent_check: impl FnOnce() -> Result<()>,
    ) -> Result<(Node, Disposition)> {
        component(name)?;
        self.verify()?;
        let (sharing, read_only, config_old) = access;
        let plan = if let Some(old) = config_old {
            config_read_plan(old)?
        } else {
            open_plan(kind, mode, sharing, read_only)?
        };
        let bytes = u16::try_from(size_of_val(name))
            .map_err(|_| TuningNativeError::UnsafeStore("store relative component"))?;
        let mut request = Box::new(CreateRequest {
            handle: HANDLE::default(),
            io: IO_STATUS_BLOCK::default(),
            name: name.to_vec(),
            unicode: UNICODE_STRING::default(),
            attributes: OBJECT_ATTRIBUTES::default(),
            descriptor: Rc::clone(&self.node.descriptor),
        });
        request.unicode = UNICODE_STRING {
            Length: bytes,
            MaximumLength: bytes,
            Buffer: PWSTR(request.name.as_mut_ptr()),
        };
        request.attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: raw(&self.node.file),
            ObjectName: &request.unicode,
            Attributes: OBJECT_ATTRIBUTE_FLAGS(plan.attributes),
            SecurityDescriptor: request.descriptor.attributes().lpSecurityDescriptor.cast(),
            ..Default::default()
        };
        self.verify()?;
        after_parent_check()?;
        let r = &mut *request;
        let status = unsafe {
            NtCreateFile(
                &mut r.handle,
                plan.desired,
                &r.attributes,
                &mut r.io,
                None,
                FILE_ATTRIBUTE_NORMAL,
                plan.share,
                plan.disposition,
                plan.options,
                None,
                0,
            )
        };
        match super::call_progress(status.0) {
            NativeProgress::TerminalFailure => {
                return Err(nt_error("store relative open", status.0));
            }
            NativeProgress::RetainSameWorker => retain_nonterminal(request, || {}),
            NativeProgress::InspectCompletedOutput => {}
        }
        if io_progress(unsafe { request.io.Anonymous.Status.0 })
            != NativeProgress::InspectCompletedOutput
        {
            retain_nonterminal(request, || {});
        }
        if request.handle.is_invalid() {
            return Err(TuningNativeError::UnsafeStore("store returned handle"));
        }
        let file = unsafe { File::from_raw_handle(request.handle.0) };
        let opened = disposition(request.io.Information, mode)?;
        let id = identity(&file, kind)?;
        let node = Node {
            file,
            id,
            kind,
            descriptor: Rc::clone(&self.node.descriptor),
            policy: self.node.policy.clone(),
            scope,
            relative_plan: Some(plan),
        };
        node.verify()?;
        self.verify()?;
        if id.volume != self.node.id.volume {
            return Err(TuningNativeError::UnsafeStore("store relative volume"));
        }
        Ok((node, opened))
    }
    pub(in crate::windows::tuning) fn entries(&self) -> Result<Vec<Entry>> {
        self.verify()?;
        let mut storage = vec![0_u64; ENUM_BYTES / size_of::<u64>()];
        let mut entries = Vec::new();
        for call in 0..CALL_LIMIT {
            self.verify()?;
            storage.fill(0);
            let queried = unsafe {
                GetFileInformationByHandleEx(
                    raw(&self.node.file),
                    if call == 0 {
                        FileIdBothDirectoryRestartInfo
                    } else {
                        FileIdBothDirectoryInfo
                    },
                    storage.as_mut_ptr().cast(),
                    ENUM_BYTES as u32,
                )
            };
            if let Err(error) = queried {
                let error = native_error("store relative enumeration", error);
                if matches!(error, TuningNativeError::Io { code: 18, .. }) {
                    self.verify()?;
                    return Ok(entries);
                }
                return Err(error);
            }
            // Win32 returns no byte count. The parser sees the exact original
            // aligned zeroed capacity, never an invented returned length.
            let bytes =
                unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), ENUM_BYTES) };
            entries.extend(parse_records(bytes, entries.len())?);
            self.verify()?;
        }
        Err(TuningNativeError::UnsafeStore("store enumeration bounds"))
    }
    pub(in crate::windows::tuning) fn entry_matches(
        &self,
        entry: &Entry,
        node: &Node,
    ) -> Result<bool> {
        self.verify()?;
        node.verify()?;
        Ok(self.node.id.volume == node.id.volume
            && entry.file_id as u64 == node.id.index
            && entry.attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
            && (entry.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0)
                == (node.kind == Kind::Directory))
    }
}
impl Node {
    pub(in crate::windows::tuning) fn id(&self) -> ObjectId {
        self.id
    }
    pub(in crate::windows::tuning) fn into_directory(self) -> Result<Directory> {
        if self.kind != Kind::Directory {
            return Err(TuningNativeError::UnsafeStore("store directory kind"));
        }
        self.verify()?;
        Ok(Directory { node: self })
    }
    pub(in crate::windows::tuning) fn verify(&self) -> Result<()> {
        self.verify_identity()?;
        if let Some(managed) = self.scope {
            if let NativePolicy::Canonical(policy) = &self.policy
                && !managed
                && self.has_relative_ancestor_plan()
            {
                let capability = self.relative_ancestor_capability()?;
                policy.validate_relative_ancestor(&capability)?;
            } else {
                self.policy.validate(raw(&self.file), managed)?;
            }
        }
        Ok(())
    }
    fn verify_identity(&self) -> Result<()> {
        if identity(&self.file, self.kind)? != self.id {
            return Err(TuningNativeError::UnsafeStore("store held object changed"));
        }
        Ok(())
    }
    fn relative_ancestor_capability(&self) -> Result<RelativeAncestorCapability<'_>> {
        let invalid = || TuningNativeError::UnsafeStore("relative ancestor capability");
        if !self.has_relative_ancestor_plan() {
            return Err(invalid());
        }
        self.verify_identity()?;
        Ok(RelativeAncestorCapability { node: self })
    }
    fn has_relative_ancestor_plan(&self) -> bool {
        let Some(plan) = &self.relative_plan else {
            return false;
        };
        let Ok(expected) = open_plan(Kind::Directory, OpenMode::Existing, Sharing::Retained, true)
        else {
            return false;
        };
        self.kind == Kind::Directory
            && self.scope == Some(false)
            && plan.desired == expected.desired
            && plan.share == expected.share
            && plan.options == expected.options
            && plan.attributes == expected.attributes
    }
    pub(in crate::windows::tuning) fn read(&mut self, limit: usize) -> Result<Vec<u8>> {
        self.verify()?;
        if self.kind != Kind::File {
            return Err(TuningNativeError::UnsafeStore("store file kind"));
        }
        let id = self.id;
        let policy = self.policy.clone();
        let scope = self.scope;
        read_file(&mut self.file, limit, |file| {
            verify_native_file(file, id, &policy, scope)
        })
    }
    pub(in crate::windows::tuning) fn write_synced(
        &mut self,
        bytes: &[u8],
        limit: usize,
    ) -> Result<()> {
        if bytes.len() > limit {
            return Err(TuningNativeError::Capacity {
                size: bytes.len() as u64,
                limit,
            });
        }
        self.verify()?;
        if self.kind != Kind::File {
            return Err(TuningNativeError::UnsafeStore("store file kind"));
        }
        let id = self.id;
        let policy = self.policy.clone();
        let scope = self.scope;
        write_file(&mut self.file, bytes, limit, |file| {
            verify_native_file(file, id, &policy, scope)
        })
    }
    pub(in crate::windows::tuning) fn rename_no_replace(
        &self,
        target: &Directory,
        name: &[u16],
    ) -> Result<()> {
        component(name)?;
        self.verify()?;
        target.verify()?;
        let length = size_of::<FILE_RENAME_INFORMATION>()
            .checked_add(size_of_val(name))
            .and_then(|n| n.checked_add(size_of::<u16>()))
            .ok_or(TuningNativeError::UnsafeStore("store rename bounds"))?;
        let mut storage = vec![0_u64; length.div_ceil(size_of::<u64>())];
        let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = raw(&target.node.file);
            (*info).FileNameLength = size_of_val(name) as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                storage
                    .as_mut_ptr()
                    .cast::<u8>()
                    .add(offset_of!(FILE_RENAME_INFORMATION, FileName))
                    .cast::<u16>(),
                name.len(),
            );
        }
        self.verify()?;
        target.verify()?;
        let mut request = Box::new((IO_STATUS_BLOCK::default(), storage));
        let (io, input) = &mut *request;
        let status = unsafe {
            NtSetInformationFile(
                raw(&self.file),
                io,
                input.as_ptr().cast(),
                length as u32,
                FileRenameInformation,
            )
        };
        match super::call_progress(status.0) {
            NativeProgress::TerminalFailure => {
                return Err(nt_error("store relative rename", status.0));
            }
            NativeProgress::RetainSameWorker => retain_nonterminal(request, || {}),
            NativeProgress::InspectCompletedOutput => {}
        }
        if io_progress(unsafe { request.0.Anonymous.Status.0 })
            != NativeProgress::InspectCompletedOutput
        {
            retain_nonterminal(request, || {});
        }
        if request.0.Information != 0 {
            return Err(TuningNativeError::UnsafeStore("store rename completion"));
        }
        self.verify()?;
        target.verify()
    }
    pub(in crate::windows::tuning) fn delete_owned(self) -> Result<()> {
        self.verify()?;
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        unsafe {
            SetFileInformationByHandle(
                raw(&self.file),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        }
        .map_err(|e| native_error("store exact old handle cleanup", e))?;
        drop(self);
        Ok(())
    }
}
fn identity(file: &File, kind: Kind) -> Result<ObjectId> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(raw(file), &mut info) }
        .map_err(|e| native_error("store relative identity", e))?;
    observed_attributes(info.dwFileAttributes, kind)?;
    if unsafe { GetFileType(raw(file)) } != FILE_TYPE_DISK
        || (kind == Kind::File && info.nNumberOfLinks != 1)
    {
        return Err(TuningNativeError::UnsafeStore(
            "unexpected object type/reparse/hardlink",
        ));
    }
    Ok(ObjectId {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}
fn full_identity(file: &File) -> Result<FullObjectId> {
    let mut info = FILE_ID_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            raw(file),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(|e| native_error("config full object ID", e))?;
    if info.FileId.Identifier == [0; 16] {
        return Err(TuningNativeError::Unsupported("config full ID unavailable"));
    }
    Ok(FullObjectId {
        volume: info.VolumeSerialNumber,
        id: info.FileId.Identifier,
    })
}
fn final_guid_root(file: &File) -> Result<Vec<u16>> {
    let mut path = [0_u16; 1024];
    let len = unsafe { GetFinalPathNameByHandleW(raw(file), &mut path, VOLUME_NAME_GUID) };
    if len == 0 {
        return Err(code_error(
            "store final volume root",
            unsafe { GetLastError() }.0,
        ));
    }
    if len as usize >= path.len() {
        return Err(TuningNativeError::UnsafeStore(
            "store final volume root bounds",
        ));
    }
    Ok(path[..len as usize].to_vec())
}
fn raw(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}
fn nt_error(operation: &'static str, status: i32) -> TuningNativeError {
    code_error(operation, unsafe {
        RtlNtStatusToDosError(NTSTATUS(status))
    })
}
fn io_error(operation: &'static str, error: std::io::Error) -> TuningNativeError {
    code_error(operation, error.raw_os_error().unwrap_or(-1) as u32)
}

#[cfg(any(test, feature = "tuning-test-controls"))]
fn verify_file(
    file: &File,
    id: ObjectId,
    policy: &SecurityPolicy,
    scope: Option<bool>,
) -> Result<()> {
    if identity(file, Kind::File)? != id {
        return Err(TuningNativeError::UnsafeStore("store held object changed"));
    }
    if let Some(managed) = scope {
        policy.validate(raw(file), managed)?;
    }
    Ok(())
}
fn verify_native_file(
    file: &File,
    id: ObjectId,
    policy: &NativePolicy,
    scope: Option<bool>,
) -> Result<()> {
    if identity(file, Kind::File)? != id {
        return Err(TuningNativeError::UnsafeStore("store held object changed"));
    }
    if let Some(managed) = scope {
        policy.validate(raw(file), managed)?;
    }
    Ok(())
}
fn read_file(
    file: &mut File,
    limit: usize,
    verify: impl Fn(&File) -> Result<()>,
) -> Result<Vec<u8>> {
    verify(file)?;
    let size = file
        .metadata()
        .map_err(|e| io_error("store relative size", e))?
        .len();
    if size > limit as u64 {
        return Err(TuningNativeError::Capacity { size, limit });
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_error("store relative read position", e))?;
    let mut bytes = Vec::with_capacity(size as usize);
    (&mut *file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error("store relative read", e))?;
    verify(file)?;
    if bytes.len() as u64 != size
        || file
            .metadata()
            .map_err(|e| io_error("store relative size", e))?
            .len()
            != size
    {
        return Err(TuningNativeError::UnsafeStore(
            "store relative length changed",
        ));
    }
    Ok(bytes)
}

fn write_file(
    file: &mut File,
    bytes: &[u8],
    limit: usize,
    verify: impl Fn(&File) -> Result<()>,
) -> Result<()> {
    if bytes.len() > limit {
        return Err(TuningNativeError::Capacity {
            size: bytes.len() as u64,
            limit,
        });
    }
    verify(file)?;
    file.set_len(0)
        .map_err(|e| io_error("store relative truncate", e))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_error("store relative write position", e))?;
    verify(file)?;
    file.write_all(bytes)
        .map_err(|e| io_error("store relative write", e))?;
    verify(file)?;
    file.sync_all()
        .map_err(|e| io_error("store relative sync", e))?;
    verify(file)
}

#[cfg(test)]
#[path = "native/ancestor_tests.rs"]
mod ancestor_tests;
#[cfg(test)]
#[path = "native/capability_tests.rs"]
mod capability_tests;
