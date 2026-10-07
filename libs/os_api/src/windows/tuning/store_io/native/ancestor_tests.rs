//! Owned ancestor mutation characterization at the production relative-open seam.
use super::*;
use crate::windows::tuning::TuningTemporaryDirectory;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    fs::{self, OpenOptions},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
};
use windows::Win32::{
    Foundation::CloseHandle,
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    Storage::FileSystem::{DELETE, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_WRITE_ATTRIBUTES},
    System::{
        IO::DeviceIoControl,
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

const ANCESTOR: &str = "fixture-ancestor";
const ORIGINAL: &str = "fixture-original-sentinel";
const CREATED: &str = "fixture-created-child";
const EXTERNAL: &str = "fixture-external-sentinel";
const ORIGINAL_BYTES: &[u8] = b"original ancestor sentinel";
const EXTERNAL_BYTES: &[u8] = b"external sentinel must remain exact";
const CREATED_BYTES: &[u8] = b"original retained directory only";
const LIMIT: usize = 256;
const MOUNT_POINT: u32 = 0xa000_0003;
const SET_REPARSE: u32 = 0x0009_00a4;
const DELETE_REPARSE: u32 = 0x0009_00ac;

#[derive(PartialEq, Eq)]
struct Artifact {
    id: ObjectId,
    bytes: Vec<u8>,
}
#[derive(PartialEq, Eq)]
struct Snapshot {
    root: ObjectId,
    names: BTreeSet<String>,
    files: Vec<(String, Artifact)>,
}
struct Evidence {
    entered: bool,
    mutation: Result<()>,
    operation: Result<(ObjectId, Disposition, Vec<u8>)>,
    external_before: Snapshot,
    external_after: Snapshot,
    original: Snapshot,
    parent_id: ObjectId,
    original_id: Option<ObjectId>,
}

fn units(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}
fn io_failure(operation: &'static str, error: std::io::Error) -> TuningNativeError {
    code_error(operation, error.raw_os_error().unwrap_or(-1) as u32)
}
fn metadata(file: &File, directory: bool, allow_reparse: bool) -> Result<ObjectId> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(raw(file), &mut info) }
        .map_err(|e| native_error("owned ancestor fixture identity", e))?;
    if unsafe { GetFileType(raw(file)) } != FILE_TYPE_DISK
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != directory
        || (!allow_reparse && info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(TuningNativeError::UnsafeStore(
            "owned ancestor fixture type",
        ));
    }
    Ok(ObjectId {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}
fn open_owned(path: &Path, access: u32) -> Result<File> {
    let text = wide(path.as_os_str())?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(text.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|e| native_error("owned ancestor fixture open", e))?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}
fn artifact(path: &Path) -> Result<Artifact> {
    let mut file = open_owned(
        path,
        (FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE).0,
    )?;
    let id = metadata(&file, false, false)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_failure("owned ancestor fixture read", e))?;
    if bytes.len() > LIMIT || metadata(&file, false, false)? != id {
        return Err(TuningNativeError::UnsafeStore(
            "owned ancestor fixture bounds",
        ));
    }
    Ok(Artifact { id, bytes })
}
fn snapshot(path: &Path, expected: ObjectId) -> Result<Snapshot> {
    let pin = open_owned(path, FILE_READ_ATTRIBUTES.0)?;
    if metadata(&pin, true, false)? != expected {
        return Err(TuningNativeError::UnsafeStore(
            "owned ancestor fixture changed",
        ));
    }
    let mut names = BTreeSet::new();
    let mut files = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| io_failure("owned ancestor fixture listing", e))? {
        if names.len() >= 4 {
            return Err(TuningNativeError::UnsafeStore(
                "owned ancestor fixture listing bounds",
            ));
        }
        let entry = entry.map_err(|e| io_failure("owned ancestor fixture entry", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| TuningNativeError::UnsafeStore("owned ancestor fixture name"))?;
        if ![ORIGINAL, CREATED, EXTERNAL].contains(&name.as_str()) {
            return Err(TuningNativeError::UnsafeStore(
                "owned ancestor fixture unknown entry",
            ));
        }
        files.push((name.clone(), artifact(&path.join(&name))?));
        names.insert(name);
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    if metadata(&pin, true, false)? != expected {
        return Err(TuningNativeError::UnsafeStore(
            "owned ancestor fixture changed",
        ));
    }
    Ok(Snapshot {
        root: expected,
        names,
        files,
    })
}
fn remove_owned(path: &Path, expected: ObjectId, directory: bool) -> Result<()> {
    let file = open_owned(path, (DELETE | FILE_READ_ATTRIBUTES).0)?;
    if metadata(&file, directory, false)? != expected {
        return Err(TuningNativeError::UnsafeStore(
            "owned ancestor cleanup changed",
        ));
    }
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            raw(&file),
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|e| native_error("owned ancestor exact cleanup", e))?;
    drop(file);
    Ok(())
}

// One fixed mount-point handle guard; no caller-created native authority.
struct Junction {
    file: File,
    id: ObjectId,
    installed: Cell<bool>,
}
impl Junction {
    fn new(path: &Path, expected: ObjectId) -> Result<Self> {
        let file = open_owned(path, (FILE_WRITE_ATTRIBUTES | FILE_READ_ATTRIBUTES).0)?;
        if metadata(&file, true, false)? != expected {
            return Err(TuningNativeError::UnsafeStore(
                "owned mutation handle changed",
            ));
        }
        Ok(Self {
            file,
            id: expected,
            installed: Cell::new(false),
        })
    }
    fn set(&self, target: &TuningTemporaryDirectory) -> Result<()> {
        if metadata(&self.file, true, false)? != self.id
            || target.path().parent() != Some(std::env::temp_dir().as_path())
            || !target.path().file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("cpu-affinity-tuning-test-")
            })
        {
            return Err(TuningNativeError::UnsafeStore("owned junction boundary"));
        }
        let target_file = open_owned(target.path(), FILE_READ_ATTRIBUTES.0)?;
        let (volume, index) = target.original_identity();
        if metadata(&target_file, true, false)? != (ObjectId { volume, index }) {
            return Err(TuningNativeError::UnsafeStore(
                "owned junction target changed",
            ));
        }
        let print: Vec<u16> = target.path().as_os_str().encode_wide().collect();
        let substitute: Vec<u16> = "\\??\\"
            .encode_utf16()
            .chain(print.iter().copied())
            .collect();
        let length = 16 + (substitute.len() + 1 + print.len() + 1) * 2;
        if length > 16 * 1024 {
            return Err(TuningNativeError::UnsafeStore(
                "owned junction buffer bounds",
            ));
        }
        let mut aligned = vec![0_u32; length.div_ceil(4)];
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(aligned.as_mut_ptr().cast::<u8>(), aligned.len() * 4)
        };
        bytes[0..4].copy_from_slice(&MOUNT_POINT.to_le_bytes());
        bytes[4..6].copy_from_slice(&((length - 8) as u16).to_le_bytes());
        bytes[10..12].copy_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
        bytes[12..14].copy_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
        bytes[14..16].copy_from_slice(&((print.len() * 2) as u16).to_le_bytes());
        for (i, unit) in substitute
            .into_iter()
            .chain([0])
            .chain(print)
            .chain([0])
            .enumerate()
        {
            bytes[16 + i * 2..18 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
        let mut returned = 0;
        unsafe {
            DeviceIoControl(
                raw(&self.file),
                SET_REPARSE,
                Some(aligned.as_ptr().cast()),
                length as u32,
                None,
                0,
                Some(&mut returned),
                None,
            )
        }
        .map_err(|e| native_error("owned WRITE_ATTRIBUTES junction setup", e))?;
        self.installed.set(true);
        let mut observed = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(raw(&self.file), &mut observed) }
            .map_err(|e| native_error("owned installed junction query", e))?;
        if metadata(&self.file, true, true)? != self.id
            || observed.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
        {
            return Err(TuningNativeError::UnsafeStore(
                "owned junction object changed",
            ));
        }
        Ok(())
    }
    fn restore(&self) -> Result<()> {
        if !self.installed.replace(false) {
            return Ok(());
        }
        if metadata(&self.file, true, true)? != self.id {
            return Err(TuningNativeError::UnsafeStore("owned tag cleanup changed"));
        }
        let request = [MOUNT_POINT, 0_u32];
        let mut returned = 0;
        unsafe {
            DeviceIoControl(
                raw(&self.file),
                DELETE_REPARSE,
                Some(request.as_ptr().cast()),
                size_of_val(&request) as u32,
                None,
                0,
                Some(&mut returned),
                None,
            )
        }
        .map_err(|e| native_error("owned exact junction tag cleanup", e))?;
        if metadata(&self.file, true, false)? != self.id {
            return Err(TuningNativeError::UnsafeStore("owned tag cleanup changed"));
        }
        Ok(())
    }
}
impl Drop for Junction {
    fn drop(&mut self) {
        // Early failure may remove only this exact installed tag, never recurse.
        let _ = self.restore();
    }
}

fn report_elevation() -> Result<()> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| native_error("owned fixture token query", e))?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    unsafe {
        let _ = CloseHandle(token);
    }
    queried.map_err(|e| native_error("owned fixture token elevation", e))?;
    if returned as usize != size_of::<TOKEN_ELEVATION>() || elevation.TokenIsElevated > 1 {
        return Err(TuningNativeError::UnsafeStore("owned fixture token shape"));
    }
    eprintln!(
        "ancestor_fixture_token={}",
        if elevation.TokenIsElevated == 0 {
            "standard"
        } else {
            "elevated"
        }
    );
    Ok(())
}

fn characterize(nonempty: bool) -> Result<Evidence> {
    report_elevation()?;
    let root = TuningTemporaryDirectory::new()?;
    let external = TuningTemporaryDirectory::new()?;
    let sid = crate::windows::OS::tuning_current_token_identity()?
        .sid()
        .to_owned();
    let descriptor = Rc::new(Descriptor::new(Some(&sid))?);
    let policy = Rc::new(SecurityPolicy {
        test_sid: Some(sid),
    });
    let base = Directory::bootstrap(root.path(), descriptor, policy, Some(true))?;
    let (volume, index) = root.original_identity();
    if base.id() != (ObjectId { volume, index }) {
        return Err(TuningNativeError::UnsafeStore(
            "owned production root changed",
        ));
    }
    let (node, disposition) = base.open(
        &units(ANCESTOR),
        Kind::Directory,
        OpenMode::CreateNew,
        Sharing::Retained,
        true,
        Some(false),
    )?;
    if disposition != Disposition::Created {
        return Err(TuningNativeError::UnsafeStore(
            "owned ancestor creation disposition",
        ));
    }
    let parent = node.into_directory()?;
    let parent_id = parent.id();
    let mut original_id = None;
    let original_pin = if nonempty {
        let (mut node, _) = parent.open(
            &units(ORIGINAL),
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Retained,
            false,
            Some(true),
        )?;
        node.write_synced(ORIGINAL_BYTES, LIMIT)?;
        original_id = Some(node.id());
        // A writable production Node owns DELETE access. Close it before a
        // second no-delete-sharing read open; keep the sentinel pinned read-only.
        drop(node);
        let (mut readonly, disposition) = parent.open(
            &units(ORIGINAL),
            Kind::File,
            OpenMode::Existing,
            Sharing::Retained,
            true,
            Some(true),
        )?;
        if disposition != Disposition::Opened
            || Some(readonly.id()) != original_id
            || readonly.read(LIMIT)? != ORIGINAL_BYTES
        {
            return Err(TuningNativeError::UnsafeStore(
                "owned original sentinel changed",
            ));
        }
        Some(readonly)
    } else {
        None
    };
    let mut outside = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(external.path().join(EXTERNAL))
        .map_err(|e| io_failure("owned external sentinel create", e))?;
    outside
        .write_all(EXTERNAL_BYTES)
        .map_err(|e| io_failure("owned external sentinel write", e))?;
    outside
        .sync_all()
        .map_err(|e| io_failure("owned external sentinel sync", e))?;
    drop(outside);
    let (volume, index) = external.original_identity();
    let external_id = ObjectId { volume, index };
    let external_before = snapshot(external.path(), external_id)?;
    let parent_path: PathBuf = root.path().join(ANCESTOR);
    let junction = Junction::new(&parent_path, parent_id)?;
    let entered = Cell::new(false);
    let mutation = RefCell::new(None);
    let opened = parent.open_after_parent_check(
        &units(if nonempty { ORIGINAL } else { CREATED }),
        Kind::File,
        if nonempty {
            OpenMode::Existing
        } else {
            OpenMode::CreateNew
        },
        (Sharing::Retained, nonempty),
        Some(true),
        || {
            entered.set(true);
            let result = junction.set(&external);
            *mutation.borrow_mut() = Some(result.clone());
            if nonempty { Ok(()) } else { result }
        },
    );
    match mutation.borrow().as_ref() {
        Some(Ok(())) => eprintln!("ancestor_mountpoint_result=installed"),
        Some(Err(TuningNativeError::Denied { code, .. } | TuningNativeError::Io { code, .. })) => {
            eprintln!("ancestor_mountpoint_result=native_error code={code}");
        }
        Some(Err(_)) => eprintln!("ancestor_mountpoint_result=refused"),
        None => eprintln!("ancestor_mountpoint_result=not_reached"),
    }
    let operation = opened.and_then(|(mut node, disposition)| {
        if !nonempty {
            node.write_synced(CREATED_BYTES, LIMIT)?;
        }
        Ok((node.id(), disposition, node.read(LIMIT)?))
    });
    // Sample every external object before tag removal or exact fixture cleanup.
    let external_after = snapshot(external.path(), external_id);
    let restored = junction.restore();
    drop(junction);
    let original = restored.and_then(|()| snapshot(&parent_path, parent_id));
    drop(original_pin);
    drop(parent);
    drop(base);
    if let Ok(observed) = &original {
        for (name, file) in &observed.files {
            remove_owned(&parent_path.join(name), file.id, false)?;
        }
        remove_owned(&parent_path, parent_id, true)?;
    }
    root.cleanup()?;
    external.cleanup()?;
    Ok(Evidence {
        entered: entered.get(),
        mutation: mutation.into_inner().ok_or(TuningNativeError::UnsafeStore(
            "owned parent hook not reached",
        ))?,
        operation,
        external_before,
        external_after: external_after?,
        original: original?,
        parent_id,
        original_id,
    })
}

#[test]
fn production_empty_ancestor_junction_race_never_mutates_external_fixture() {
    let evidence =
        characterize(false).expect("empty owned production ancestor characterization setup");
    assert!(
        evidence.entered,
        "final production parent/native boundary was reached"
    );
    evidence
        .mutation
        .expect("actual standard-token WRITE_ATTRIBUTES mount-point setup required");
    assert!(
        evidence.external_before == evidence.external_after,
        "external root ID/names/file IDs/full bytes retained before cleanup"
    );
    assert!(
        evidence.original.root == evidence.parent_id,
        "original ancestor identity retained after exact tag cleanup"
    );
    match evidence.operation {
        Ok((id, disposition, bytes)) => {
            assert_eq!(disposition, Disposition::Created);
            assert!(
                bytes == CREATED_BYTES,
                "returned original file has exact fixture bytes"
            );
            assert!(
                evidence.original.files.len() == 1
                    && evidence.original.files[0].0 == CREATED
                    && evidence.original.files[0].1.id == id
                    && evidence.original.files[0].1.bytes == CREATED_BYTES,
                "successful descent is the exact original object, never the external target"
            );
        }
        Err(
            TuningNativeError::UnsafeStore(_)
            | TuningNativeError::Denied { .. }
            | TuningNativeError::Io { .. }
            | TuningNativeError::Busy,
        ) => {}
        Err(_) => panic!("unsupported or unrelated failure is not native reparse characterization"),
    }
}

#[test]
fn production_nonempty_ancestor_rejects_junction_and_retains_original_traversal() {
    let evidence =
        characterize(true).expect("nonempty owned production ancestor characterization setup");
    assert!(
        evidence.entered,
        "final production parent/native boundary was reached"
    );
    assert!(
        matches!(
            evidence.mutation,
            Err(TuningNativeError::Io { code: 145, .. })
        ),
        "actual nonempty-directory rejection required; privilege/support denial is no evidence"
    );
    assert!(
        evidence.external_before == evidence.external_after,
        "external root ID/names/file IDs/full bytes retained before cleanup"
    );
    let (id, disposition, bytes) = evidence
        .operation
        .expect("nonempty original ancestor traversal succeeds");
    assert_eq!(disposition, Disposition::Opened);
    assert!(
        Some(id) == evidence.original_id,
        "retained original sentinel ID matches actual read"
    );
    assert!(
        bytes == ORIGINAL_BYTES,
        "retained original sentinel full bytes match actual read"
    );
    assert!(
        evidence.original.root == evidence.parent_id
            && evidence.original.names == BTreeSet::from([ORIGINAL.to_owned()])
            && evidence.original.files.len() == 1
            && evidence.original.files[0].1.id == id
            && evidence.original.files[0].1.bytes == ORIGINAL_BYTES,
        "original non-reparse ancestor and sentinel survive attempted mutation"
    );
}
