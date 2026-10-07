//! Test-only characterization of fixed, handle-relative NTFS operations.
//! No canonical admission, arbitrary root, production policy or path fallback.
use super::{
    code_error, native_error, security::Descriptor, test_support::TuningTemporaryDirectory, wide,
};
use crate::tuning::TuningNativeError;
use std::{
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    mem::{offset_of, size_of, size_of_val},
    os::windows::io::{AsRawHandle, FromRawHandle},
    rc::Rc,
};
use windows::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
            FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
            FileRenameInformation, NtCreateFile, NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::{
            HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, RtlNtStatusToDosError,
            UNICODE_STRING,
        },
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
            FILE_GENERIC_WRITE, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_TYPE_DISK,
            FILE_WRITE_ATTRIBUTES, FileDispositionInfo, FileIdBothDirectoryInfo,
            FileIdBothDirectoryRestartInfo, GetFileInformationByHandle,
            GetFileInformationByHandleEx, GetFileType, OPEN_EXISTING, READ_CONTROL, SYNCHRONIZE,
            SetFileInformationByHandle,
        },
        System::IO::IO_STATUS_BLOCK,
    },
    core::{PCWSTR, PWSTR},
};

pub(super) const NAME_UNITS: usize = 64;
pub(super) const ENUM_BYTES: usize = 64 * 1024;
pub(super) const ENTRY_LIMIT: usize = 32;
pub(super) const CALL_LIMIT: usize = 16;
pub(super) const CONTENT_BYTES: usize = 4096;
pub(super) type Result<T> = std::result::Result<T, TuningNativeError>;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct RelativeName {
    units: [u16; NAME_UNITS],
    len: u16,
}
impl RelativeName {
    pub(super) fn new(value: &str) -> Result<Self> {
        let units: Vec<_> = value.encode_utf16().take(NAME_UNITS + 1).collect();
        Self::from_units(&units)
    }
    pub(super) fn from_units(value: &[u16]) -> Result<Self> {
        let invalid = || TuningNativeError::UnsafeStore("relative name");
        if value.is_empty() || value.len() > NAME_UNITS {
            return Err(invalid());
        }
        let text = String::from_utf16(value).map_err(|_| invalid())?;
        if text == "."
            || text == ".."
            || text.ends_with('.')
            || text.ends_with(' ')
            || text.chars().any(|c| {
                c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '|' | '<' | '>' | '"')
            })
        {
            return Err(invalid());
        }
        let stem = text
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches([' ', '.'])
            .to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|&prefix| {
                stem.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            });
        if device {
            return Err(invalid());
        }
        let mut units = [0; NAME_UNITS];
        units[..value.len()].copy_from_slice(value);
        Ok(Self {
            units,
            len: value.len() as u16,
        })
    }
    pub(super) fn units(&self) -> &[u16] {
        &self.units[..usize::from(self.len)]
    }
    /// Independent fixture shape, deliberately restricted to a fixed prefix.
    /// Never a native handle or admission producer. Desired name tests use new.
    pub(super) fn fixture(value: &str) -> Result<Self> {
        if !value.starts_with("fixture-")
            || value.len() > NAME_UNITS
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(TuningNativeError::UnsafeStore("relative fixture name"));
        }
        let mut units = [0; NAME_UNITS];
        for (target, source) in units.iter_mut().zip(value.bytes()) {
            *target = u16::from(source);
        }
        Ok(Self {
            units,
            len: value.len() as u16,
        })
    }
}
impl fmt::Debug for RelativeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelativeName(..)")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Directory,
    File,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OpenMode {
    Existing,
    CreateNew,
    OpenOrCreate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disposition {
    Opened,
    Created,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sharing {
    Retained,
    Renameable,
    ExclusiveLease,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ObjectId {
    volume: u32,
    index: u64,
}
impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ObjectId(..)")
    }
}

pub(super) struct Node {
    file: File,
    id: ObjectId,
    kind: Kind,
    disposition: Disposition,
    // The descriptor owner also keeps this actual native object !Send.
    descriptor: Rc<Descriptor>,
}
impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelativeNode(..)")
    }
}
pub(super) struct Directory {
    node: Node,
}
impl fmt::Debug for Directory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelativeDirectory(..)")
    }
}

impl Directory {
    /// Setup only: selects the already owned TempRoot and retains a real handle.
    /// There is no public/raw handle constructor or caller-selected root path.
    pub(super) fn fixture(root: &TuningTemporaryDirectory) -> Result<Self> {
        let sid = crate::windows::OS::tuning_current_token_identity()?
            .sid()
            .to_owned();
        let descriptor = Rc::new(Descriptor::new(Some(&sid))?);
        let text = wide(root.path().as_os_str())?;
        let handle = unsafe {
            CreateFileW(
                PCWSTR(text.as_ptr()),
                // The base TempRoot is never renamed/deleted by this helper.
                // Omitting DELETE permits independent setup while this pin is
                // retained. Desired child handles have their own fixed rights.
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(|e| native_error("relative owned fixture root", e))?;
        let file = unsafe { File::from_raw_handle(handle.0) };
        super::store::validate_volume(&file)?;
        let id = fixture_identity(&file)?;
        require_original_root(root, &file)?;
        Ok(Self {
            node: Node {
                file,
                id,
                kind: Kind::Directory,
                disposition: Disposition::Opened,
                descriptor,
            },
        })
    }
    pub(super) fn as_node(&self) -> &Node {
        &self.node
    }
    /// Independent setup of an already existing directly owned fixture child.
    /// No create/overwrite/repair, no desired relative-operation fallback.
    pub(super) fn fixture_child(
        root: &TuningTemporaryDirectory,
        name: &RelativeName,
        kind: Kind,
        sharing: Sharing,
    ) -> Result<Node> {
        fixture_child(root, name, kind, sharing, false)
    }
    pub(super) fn fixture_child_read_only(
        root: &TuningTemporaryDirectory,
        name: &RelativeName,
        kind: Kind,
    ) -> Result<Node> {
        fixture_child(root, name, kind, Sharing::Renameable, true)
    }
    pub(super) fn open(
        &self,
        name: &RelativeName,
        kind: Kind,
        mode: OpenMode,
        sharing: Sharing,
    ) -> Result<Node> {
        self.open_native(name, kind, mode, sharing, false, || Ok(()))
    }
    /// Same boundary as open; a private test hook runs after actual parent
    /// validation immediately before the SDK. It receives no authority.
    pub(super) fn open_after_parent_check<F: FnOnce() -> Result<()>>(
        &self,
        name: &RelativeName,
        kind: Kind,
        mode: OpenMode,
        sharing: Sharing,
        hook: F,
    ) -> Result<Node> {
        self.open_native(name, kind, mode, sharing, false, hook)
    }
    pub(super) fn open_read_only_file(&self, name: &RelativeName) -> Result<Node> {
        self.open_native(
            name,
            Kind::File,
            OpenMode::Existing,
            Sharing::Renameable,
            true,
            || Ok(()),
        )
    }
    pub(super) fn enumerate(&self) -> Result<Vec<Entry>> {
        self.node.verify()?;
        let mut storage = vec![0_u64; ENUM_BYTES / size_of::<u64>()];
        let mut entries = Vec::new();
        for calls in 1..=CALL_LIMIT {
            enumeration_budget(calls, entries.len())?;
            self.node.verify()?;
            storage.fill(0);
            let class = if calls == 1 {
                FileIdBothDirectoryRestartInfo
            } else {
                FileIdBothDirectoryInfo
            };
            let queried = unsafe {
                GetFileInformationByHandleEx(
                    raw(&self.node.file),
                    class,
                    storage.as_mut_ptr().cast(),
                    ENUM_BYTES as u32,
                )
            };
            if let Err(error) = queried {
                let error = native_error("relative directory enumeration", error);
                if matches!(error, TuningNativeError::Io { code: 18, .. }) {
                    self.node.verify()?;
                    return Ok(entries);
                }
                return Err(error);
            }
            // GetFileInformationByHandleEx returns no byte count. The parser
            // sees precisely the original aligned, zeroed capacity.
            let bytes =
                unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), ENUM_BYTES) };
            entries.extend(parse_records(bytes, entries.len())?);
            self.node.verify()?;
        }
        Err(TuningNativeError::UnsafeStore(
            "relative enumeration bounds",
        ))
    }
    fn open_native<F: FnOnce() -> Result<()>>(
        &self,
        name: &RelativeName,
        kind: Kind,
        mode: OpenMode,
        sharing: Sharing,
        read_only: bool,
        hook: F,
    ) -> Result<Node> {
        RelativeName::from_units(name.units())?;
        self.node.verify()?;
        let byte_length = u16::try_from(size_of_val(name.units()))
            .map_err(|_| TuningNativeError::UnsafeStore("relative name"))?;
        let unicode = UNICODE_STRING {
            Length: byte_length,
            MaximumLength: byte_length,
            Buffer: PWSTR(name.units().as_ptr().cast_mut()),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: raw(&self.node.file),
            ObjectName: &unicode,
            Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
            SecurityDescriptor: self
                .node
                .descriptor
                .attributes()
                .lpSecurityDescriptor
                .cast(),
            ..Default::default()
        };
        let desired = match kind {
            Kind::Directory => {
                FILE_LIST_DIRECTORY
                    | FILE_TRAVERSE
                    | FILE_READ_ATTRIBUTES
                    | FILE_WRITE_ATTRIBUTES
                    | READ_CONTROL
                    | SYNCHRONIZE
                    | DELETE
            }
            Kind::File => {
                FILE_GENERIC_READ
                    | SYNCHRONIZE
                    | if read_only {
                        windows::Win32::Storage::FileSystem::FILE_ACCESS_RIGHTS(0)
                    } else {
                        FILE_GENERIC_WRITE | DELETE
                    }
            }
        };
        let share = share_mode(sharing);
        let disposition = match mode {
            OpenMode::Existing => FILE_OPEN,
            OpenMode::CreateNew => FILE_CREATE,
            OpenMode::OpenOrCreate => FILE_OPEN_IF,
        };
        let options = FILE_SYNCHRONOUS_IO_NONALERT
            | FILE_OPEN_REPARSE_POINT
            | if kind == Kind::Directory {
                FILE_DIRECTORY_FILE
            } else {
                FILE_NON_DIRECTORY_FILE
            };
        // Both SDK output locations remain stable for the entire native request.
        let mut output = Box::new((HANDLE::default(), IO_STATUS_BLOCK::default()));
        // This test hook deliberately characterizes a parent reparse change
        // after the check. No path fallback or second namespace selection exists.
        hook()?;
        let (handle, io) = &mut *output;
        let status = unsafe {
            NtCreateFile(
                handle,
                desired,
                &attributes,
                io,
                None,
                FILE_ATTRIBUTE_NORMAL,
                share,
                disposition,
                options,
                None,
                0,
            )
        };
        // Pending bypasses all SDK output reads. Its entire output allocation
        // and any subsequently delivered handle remain retained until process
        // exit; no cancellation completion is inferred. Completed NT success
        // enters File RAII before disposition, type and parent rejection.
        inspect_completion_output(status.0, output, move |output| {
            if status.0 < 0 {
                return Err(nt_error("relative NtCreateFile", status.0));
            }
            if output.0.is_invalid() {
                return Err(TuningNativeError::UnsafeStore("relative returned handle"));
            }
            let file = unsafe { File::from_raw_handle(output.0.0) };
            let disposition = completion(
                status.0,
                unsafe { output.1.Anonymous.Status.0 },
                output.1.Information,
                mode,
            )?;
            let id = object_identity(&file, kind)?;
            self.node.verify()?;
            if id.volume != self.node.id.volume {
                return Err(TuningNativeError::UnsafeStore("relative object volume"));
            }
            Ok(Node {
                file,
                id,
                kind,
                disposition,
                descriptor: Rc::clone(&self.node.descriptor),
            })
        })
    }
}
impl Node {
    pub(super) fn id(&self) -> ObjectId {
        self.id
    }
    pub(super) fn kind(&self) -> Kind {
        self.kind
    }
    pub(super) fn disposition(&self) -> Disposition {
        self.disposition
    }
    pub(super) fn into_directory(self) -> Result<Directory> {
        if self.kind != Kind::Directory {
            return Err(TuningNativeError::UnsafeStore("relative object kind"));
        }
        Ok(Directory { node: self })
    }
    pub(super) fn write(&mut self, data: &[u8]) -> Result<()> {
        if data.len() > CONTENT_BYTES {
            return Err(TuningNativeError::UnsafeStore("relative content bounds"));
        }
        self.verify_file()?;
        self.file
            .set_len(0)
            .map_err(|e| io_error("relative truncate", e))?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|e| io_error("relative write position", e))?;
        self.verify_file()?;
        self.file
            .write_all(data)
            .map_err(|e| io_error("relative write", e))?;
        self.verify_file()?;
        self.file
            .sync_all()
            .map_err(|e| io_error("relative file sync", e))?;
        self.verify_file()
    }
    pub(super) fn read(&mut self) -> Result<Vec<u8>> {
        self.verify_file()?;
        if self
            .file
            .metadata()
            .map_err(|e| io_error("relative content size", e))?
            .len()
            > CONTENT_BYTES as u64
        {
            return Err(TuningNativeError::UnsafeStore("relative content bounds"));
        }
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|e| io_error("relative read position", e))?;
        let mut bytes = Vec::with_capacity(CONTENT_BYTES + 1);
        (&mut self.file)
            .take(CONTENT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| io_error("relative read", e))?;
        if bytes.len() > CONTENT_BYTES {
            return Err(TuningNativeError::UnsafeStore("relative content bounds"));
        }
        self.verify_file()?;
        Ok(bytes)
    }
    pub(super) fn rename_no_replace(
        &self,
        destination: &Directory,
        name: &RelativeName,
    ) -> Result<()> {
        RelativeName::from_units(name.units())?;
        self.verify()?;
        destination.node.verify()?;
        let (mut storage, length) = rename_buffer(name)?;
        let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        // The flexible record uses the generated SDK offsets and aligned zeroed
        // storage. FileNameLength excludes NUL; no replace/ex flags are set.
        unsafe {
            (*info).RootDirectory = raw(&destination.node.file);
        }
        self.verify()?;
        destination.node.verify()?;
        // Keep native output and the entire input record together. An unexpected
        // pending request retains both buffers rather than returning freed memory.
        let mut request = Box::new((IO_STATUS_BLOCK::default(), storage));
        let (io, storage) = &mut *request;
        let status = unsafe {
            NtSetInformationFile(
                raw(&self.file),
                io,
                storage.as_ptr().cast(),
                length as u32,
                FileRenameInformation,
            )
        };
        inspect_completion_output(status.0, request, |request| {
            set_completion(
                status.0,
                unsafe { request.0.Anonymous.Status.0 },
                request.0.Information,
            )?;
            self.verify()?;
            destination.node.verify()
        })
    }
    pub(super) fn delete_owned(self) -> Result<()> {
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
        .map_err(|e| native_error("relative exact-source deletion", e))?;
        drop(self);
        Ok(())
    }
    fn verify(&self) -> Result<()> {
        if object_identity(&self.file, self.kind)? != self.id {
            return Err(TuningNativeError::UnsafeStore("relative object identity"));
        }
        Ok(())
    }
    fn verify_file(&self) -> Result<()> {
        if self.kind != Kind::File {
            return Err(TuningNativeError::UnsafeStore("relative object kind"));
        }
        self.verify()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) name: RelativeName,
    pub(super) file_id: i64,
    pub(super) attributes: u32,
}
impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelativeEntry(..)")
    }
}
/// Strict completion of a synchronous actual call and its IO_STATUS_BLOCK.
pub(super) fn completion(
    status: i32,
    io_status: i32,
    information: usize,
    mode: OpenMode,
) -> Result<Disposition> {
    if status < 0 {
        return Err(nt_error("relative NtCreateFile", status));
    }
    if io_status < 0 {
        return Err(nt_error("relative NtCreate completion", io_status));
    }
    if status != 0 || io_status != 0 {
        return Err(TuningNativeError::UnsafeStore("relative completion"));
    }
    match (information, mode) {
        (1, OpenMode::Existing | OpenMode::OpenOrCreate) => Ok(Disposition::Opened),
        (2, OpenMode::CreateNew | OpenMode::OpenOrCreate) => Ok(Disposition::Created),
        _ => Err(TuningNativeError::UnsafeStore("relative completion")),
    }
}
/// Parser sees original zeroed capacity, because this Win32 API has no returned
/// byte count. Only exact dot metadata may be omitted; no unknown-name filter.
pub(super) fn parse_records(buffer: &[u8], already_seen: usize) -> Result<Vec<Entry>> {
    let invalid = || TuningNativeError::UnsafeStore("relative directory records");
    let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    if buffer.len() < header || buffer.len() > ENUM_BYTES || already_seen > ENTRY_LIMIT {
        return Err(invalid());
    }
    let mut result = Vec::new();
    let mut cursor = 0_usize;
    for _ in 0..ENTRY_LIMIT + 2 {
        let header_end = cursor.checked_add(header).ok_or_else(invalid)?;
        if !cursor.is_multiple_of(8) || header_end > buffer.len() {
            return Err(invalid());
        }
        let field32 = |offset: usize| -> u32 {
            u32::from_le_bytes(
                buffer[cursor + offset..cursor + offset + 4]
                    .try_into()
                    .expect("bounded SDK record field"),
            )
        };
        let next = field32(offset_of!(FILE_ID_BOTH_DIR_INFO, NextEntryOffset)) as usize;
        let length = field32(offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength)) as usize;
        let short_length = buffer[cursor + offset_of!(FILE_ID_BOTH_DIR_INFO, ShortNameLength)];
        if length == 0 || !length.is_multiple_of(2) || length > NAME_UNITS * 2
            // Negative SDK i8 lengths are >=128 in this exact raw byte view.
            || short_length > 24 || !short_length.is_multiple_of(2)
        {
            return Err(invalid());
        }
        let end = header_end.checked_add(length).ok_or_else(invalid)?;
        if end > buffer.len() {
            return Err(invalid());
        }
        let units: Vec<_> = buffer[header_end..end]
            .chunks_exact(2)
            .map(|part| u16::from_le_bytes([part[0], part[1]]))
            .collect();
        if units != [b'.' as u16] && units != [b'.' as u16, b'.' as u16] {
            let name = RelativeName::from_units(&units).map_err(|_| invalid())?;
            let count = already_seen
                .checked_add(result.len())
                .and_then(|n| n.checked_add(1))
                .ok_or_else(invalid)?;
            if count > ENTRY_LIMIT {
                return Err(invalid());
            }
            let id_offset = cursor + offset_of!(FILE_ID_BOTH_DIR_INFO, FileId);
            let file_id = i64::from_le_bytes(
                buffer[id_offset..id_offset + 8]
                    .try_into()
                    .expect("bounded SDK record ID"),
            );
            result.push(Entry {
                name,
                file_id,
                attributes: field32(offset_of!(FILE_ID_BOTH_DIR_INFO, FileAttributes)),
            });
        }
        if next == 0 {
            return Ok(result);
        }
        let following = cursor.checked_add(next).ok_or_else(invalid)?;
        if !next.is_multiple_of(8) || following < end || following >= buffer.len() {
            return Err(invalid());
        }
        cursor = following;
    }
    Err(invalid())
}
pub(super) fn enumeration_budget(calls: usize, entries: usize) -> Result<()> {
    if calls > CALL_LIMIT || entries > ENTRY_LIMIT {
        return Err(TuningNativeError::UnsafeStore(
            "relative enumeration bounds",
        ));
    }
    Ok(())
}
fn raw(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}
fn share_mode(sharing: Sharing) -> windows::Win32::Storage::FileSystem::FILE_SHARE_MODE {
    match sharing {
        Sharing::Retained => FILE_SHARE_READ | FILE_SHARE_WRITE,
        Sharing::Renameable => FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        Sharing::ExclusiveLease => windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
    }
}
fn nt_error(operation: &'static str, status: i32) -> TuningNativeError {
    code_error(operation, unsafe {
        RtlNtStatusToDosError(NTSTATUS(status))
    })
}
fn io_error(operation: &'static str, error: std::io::Error) -> TuningNativeError {
    error.raw_os_error().map_or(
        TuningNativeError::UnsafeStore("relative I/O error"),
        |code| code_error(operation, code as u32),
    )
}

/// Supplied extent covers the SDK size, exact name bytes and a zero WCHAR tail.
/// FileNameLength excludes the terminator and padding.
fn rename_buffer_extent(units: usize) -> Result<usize> {
    if units == 0 || units > NAME_UNITS {
        return Err(TuningNativeError::UnsafeStore("relative name"));
    }
    units
        .checked_mul(size_of::<u16>())
        .and_then(|bytes| size_of::<FILE_RENAME_INFORMATION>().checked_add(bytes))
        .and_then(|bytes| bytes.checked_add(size_of::<u16>()))
        .ok_or(TuningNativeError::UnsafeStore("relative name"))
}
fn rename_buffer(name: &RelativeName) -> Result<(Vec<u64>, usize)> {
    let length = rename_buffer_extent(name.units().len())?;
    let mut storage = vec![0_u64; length.div_ceil(size_of::<u64>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).FileNameLength = size_of_val(name.units()) as u32;
        std::ptr::copy_nonoverlapping(
            name.units().as_ptr(),
            storage
                .as_mut_ptr()
                .cast::<u8>()
                .add(offset_of!(FILE_RENAME_INFORMATION, FileName))
                .cast::<u16>(),
            name.units().len(),
        );
    }
    Ok((storage, length))
}
/// Unexpected pending is never admitted. Native cancellation can still write
/// this output, so retain its allocation indefinitely without inspecting it.
/// This cfg(test)-only retention is no production reaper or cancellation policy.
/// Any handle delivered to retained output is left for process-exit cleanup.
fn inspect_completion_output<T: 'static, U>(
    status: i32,
    output: Box<T>,
    inspect: impl FnOnce(&T) -> Result<U>,
) -> Result<U> {
    if status == 0x103 {
        let _ = Box::into_raw(output);
        return Err(TuningNativeError::UnsafeStore(
            "relative pending completion",
        ));
    }
    inspect(&output)
}
fn set_completion(status: i32, io_status: i32, information: usize) -> Result<()> {
    if status < 0 {
        return Err(nt_error("relative no-replace rename", status));
    }
    if io_status < 0 {
        return Err(nt_error("relative rename completion", io_status));
    }
    if status != 0 || io_status != 0 || information != 0 {
        return Err(TuningNativeError::UnsafeStore("relative rename completion"));
    }
    Ok(())
}
fn fixture_identity(file: &File) -> Result<ObjectId> {
    object_identity(file, Kind::Directory)
}
fn object_identity(file: &File, kind: Kind) -> Result<ObjectId> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|e| native_error("relative fixture identity", e))?;
    if unsafe { GetFileType(HANDLE(file.as_raw_handle())) } != FILE_TYPE_DISK
        || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != (kind == Kind::Directory)
        || (kind == Kind::File && info.nNumberOfLinks != 1)
    {
        return Err(TuningNativeError::UnsafeStore(
            "relative fixture object type",
        ));
    }
    Ok(ObjectId {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

fn require_original_root(root: &TuningTemporaryDirectory, file: &File) -> Result<()> {
    let observed = fixture_identity(file)?;
    let (volume, index) = root.original_identity();
    if observed != (ObjectId { volume, index }) {
        return Err(TuningNativeError::UnsafeStore(
            "relative fixture root identity",
        ));
    }
    Ok(())
}
fn fixture_child(
    root: &TuningTemporaryDirectory,
    name: &RelativeName,
    kind: Kind,
    sharing: Sharing,
    read_only: bool,
) -> Result<Node> {
    // The fixture name helper is intentionally separate from the desired name
    // validator. Refuse any private shape not produced by the fixed subset.
    let component = String::from_utf16(name.units())
        .map_err(|_| TuningNativeError::UnsafeStore("relative fixture name"))?;
    RelativeName::fixture(&component)?;
    let retained = Directory::fixture(root)?;
    require_original_root(root, &retained.node.file)?;
    let text = wide(root.path().join(component).as_os_str())?;
    let share = match sharing {
        Sharing::Retained => FILE_SHARE_READ | FILE_SHARE_WRITE,
        Sharing::Renameable => FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        Sharing::ExclusiveLease => windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
    };
    let handle = unsafe {
        CreateFileW(
            PCWSTR(text.as_ptr()),
            FILE_GENERIC_READ.0
                | if read_only {
                    0
                } else {
                    FILE_GENERIC_WRITE.0 | DELETE.0
                },
            share,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT
                | if kind == Kind::Directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0)
                },
            None,
        )
    }
    .map_err(|e| native_error("relative existing fixture child", e))?;
    let file = unsafe { File::from_raw_handle(handle.0) };
    let id = object_identity(&file, kind)?;
    require_original_root(root, &retained.node.file)?;
    Ok(Node {
        file,
        id,
        kind,
        disposition: Disposition::Opened,
        descriptor: retained.node.descriptor,
    })
}

#[path = "relative_io/native_tests.rs"]
mod native_tests;
#[path = "relative_io/pure_tests.rs"]
mod pure_tests;
#[path = "relative_io/regression_tests.rs"]
mod regression_tests;
