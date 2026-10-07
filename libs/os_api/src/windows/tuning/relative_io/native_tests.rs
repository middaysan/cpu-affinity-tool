//! Owned native fixtures exercise relative primitives without canonical authority.
use super::*;
use std::{
    cell::Cell,
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    mem::size_of,
    os::windows::{ffi::OsStrExt, io::AsRawHandle, process::CommandExt},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        FILE_DISPOSITION_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FileDispositionInfo,
        SetFileInformationByHandle,
    },
    System::IO::DeviceIoControl,
};

struct Tracked {
    path: PathBuf,
    id: (u32, u64),
}
struct Fixture {
    root: TuningTemporaryDirectory,
    tracked: Vec<Tracked>,
}
impl Fixture {
    fn new() -> Self {
        Self {
            root: TuningTemporaryDirectory::new().expect("owned NTFS fixture setup"),
            tracked: Vec::new(),
        }
    }
    fn path(&self, relative: &str) -> PathBuf {
        let relative = Path::new(relative);
        assert!(
            relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        );
        self.root.path().join(relative)
    }
    fn file(&mut self, relative: &str, bytes: &[u8]) {
        let path = self.path(relative);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        self.tracked.push(Tracked {
            path,
            id: identity(&file),
        });
    }
    fn directory(&mut self, relative: &str) {
        let path = self.path(relative);
        fs::create_dir(&path).unwrap();
        let file = fixture_file(&path, false).unwrap();
        self.tracked.push(Tracked {
            path,
            id: identity(&file),
        });
    }
    fn remember(&mut self, relative: &str, node: &Node) {
        self.tracked.push(Tracked {
            path: self.path(relative),
            id: identity(&node.file),
        });
    }
    fn moved(&mut self, old: &str, new: &str) {
        let old = self.path(old);
        let new = self.path(new);
        for tracked in &mut self.tracked {
            if let Ok(suffix) = tracked.path.strip_prefix(&old) {
                tracked.path = new.join(suffix);
            }
        }
    }
    fn rename_setup(&mut self, old: &str, new: &str) {
        fs::rename(self.path(old), self.path(new)).unwrap();
        self.moved(old, new);
    }
    fn child(&self, value: &str, kind: Kind, sharing: Sharing) -> Node {
        Directory::fixture_child(&self.root, &name(value), kind, sharing)
            .expect("actual existing fixture child setup")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Reopen only bounded fixture names, authenticate the captured object,
        // then delete that exact handle. Never recurse or follow a leaf junction.
        for tracked in self.tracked.iter().rev() {
            let Ok(file) = fixture_file(&tracked.path, true) else {
                continue;
            };
            if try_identity(&file).is_ok_and(|observed| observed == tracked.id) {
                let _ = remove_exact(file);
            }
        }
        // TempRoot subsequently authenticates and removes its original root.
    }
}
fn name(value: &str) -> RelativeName {
    RelativeName::fixture(value).expect("fixed fixture name shape")
}
fn identity(file: &File) -> (u32, u64) {
    try_identity(file).expect("owned fixture identity query")
}
fn try_identity(file: &File) -> Result<(u32, u64)> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|error| native_error("owned fixture identity query", error))?;
    Ok((
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}
fn node_matches(node: &Node, expected: (u32, u64)) -> bool {
    let id = node.id();
    (id.volume, id.index) == expected && identity(&node.file) == expected
}
fn fixture_file(path: &Path, delete: bool) -> Result<File> {
    let text = wide(path.as_os_str())?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(text.as_ptr()),
            FILE_READ_ATTRIBUTES.0 | if delete { DELETE.0 } else { 0 },
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|error| native_error("owned fixture metadata handle", error))?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}
fn remove_exact(file: File) -> Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|error| native_error("owned fixture exact handle cleanup", error))?;
    drop(file);
    Ok(())
}
fn listing(path: &Path) -> BTreeSet<String> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect()
}
fn entry_names(entries: &[Entry]) -> BTreeSet<String> {
    entries
        .iter()
        .map(|entry| String::from_utf16(entry.name.units()).unwrap())
        .collect()
}
fn set_owned_junction(file: &File, target: &Path) -> Result<()> {
    // Verified SDK FSCTL_SET_REPARSE_POINT and IO_REPARSE_TAG_MOUNT_POINT.
    const SET_REPARSE: u32 = 0x0009_00a4;
    const MOUNT_POINT: u32 = 0xa000_0003;
    assert!(
        target.parent() == Some(std::env::temp_dir().as_path()),
        "owned target is directly under temporary directory"
    );
    assert!(
        target
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cpu-affinity-tuning-test-")
    );
    let print: Vec<u16> = target.as_os_str().encode_wide().collect();
    let substitute: Vec<u16> = "\\??\\"
        .encode_utf16()
        .chain(print.iter().copied())
        .collect();
    let path_units = substitute.len() + 1 + print.len() + 1;
    let length = 16 + path_units * 2;
    assert!(length <= 16 * 1024);
    let mut aligned = vec![0u32; length.div_ceil(4)];
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(aligned.as_mut_ptr().cast::<u8>(), aligned.len() * 4)
    };
    bytes[0..4].copy_from_slice(&MOUNT_POINT.to_le_bytes());
    bytes[4..6].copy_from_slice(&((length - 8) as u16).to_le_bytes());
    bytes[10..12].copy_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
    bytes[12..14].copy_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
    bytes[14..16].copy_from_slice(&((print.len() * 2) as u16).to_le_bytes());
    for (index, unit) in substitute
        .iter()
        .copied()
        .chain([0])
        .chain(print)
        .chain([0])
        .enumerate()
    {
        bytes[16 + index * 2..18 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    let mut returned = 0;
    unsafe {
        DeviceIoControl(
            HANDLE(file.as_raw_handle()),
            SET_REPARSE,
            Some(aligned.as_ptr().cast()),
            length as u32,
            None,
            0,
            Some(&mut returned),
            None,
        )
    }
    .map_err(|error| native_error("owned fixture junction setup", error))
}

#[test]
fn native_open_uses_actual_dispositions_and_preserves_existing_bytes() {
    let mut fixture = Fixture::new();
    fixture.file("fixture-existing", b"original exact bytes\0");
    let expected = identity(&fixture_file(&fixture.path("fixture-existing"), false).unwrap());
    let directory = Directory::fixture(&fixture.root).unwrap();
    for mode in [OpenMode::Existing, OpenMode::OpenOrCreate] {
        let mut opened = directory
            .open(
                &name("fixture-existing"),
                Kind::File,
                mode,
                Sharing::Retained,
            )
            .unwrap();
        assert_eq!(opened.disposition(), Disposition::Opened);
        assert_eq!(opened.kind(), Kind::File);
        assert!(
            node_matches(&opened, expected),
            "existing object identity is retained"
        );
        assert_eq!(opened.read().unwrap(), b"original exact bytes\0");
    }
    for (label, mode) in [
        ("fixture-created", OpenMode::CreateNew),
        ("fixture-open-or-create", OpenMode::OpenOrCreate),
    ] {
        let mut created = directory
            .open(&name(label), Kind::File, mode, Sharing::Retained)
            .unwrap();
        fixture.remember(label, &created);
        assert_eq!(created.disposition(), Disposition::Created);
        created.write(b"new exact payload\0").unwrap();
        assert_eq!(created.read().unwrap(), b"new exact payload\0");
        drop(created);
        assert_eq!(
            fs::read(fixture.path(label)).unwrap(),
            b"new exact payload\0"
        );
    }
    let child = directory
        .open(
            &name("fixture-created-directory"),
            Kind::Directory,
            OpenMode::CreateNew,
            Sharing::Retained,
        )
        .unwrap();
    fixture.remember("fixture-created-directory", &child);
    assert_eq!(child.disposition(), Disposition::Created);
    assert_eq!(child.kind(), Kind::Directory);
    let expected = identity(&child.file);
    let child = child.into_directory().unwrap();
    assert!(child.enumerate().unwrap().is_empty());
    drop(child);
    let opened = directory
        .open(
            &name("fixture-created-directory"),
            Kind::Directory,
            OpenMode::Existing,
            Sharing::Retained,
        )
        .unwrap();
    assert_eq!(opened.disposition(), Disposition::Opened);
    assert!(
        node_matches(&opened, expected),
        "directory incarnation is retained"
    );
}

#[test]
fn native_created_directory_supports_relative_descendants_without_root_fallback() {
    let mut fixture = Fixture::new();
    let mut foreign = Fixture::new();
    fixture.file("fixture-sentinel", b"outer root sentinel");
    foreign.file("fixture-created-parent", b"foreign root conflict");
    let foreign_names = listing(foreign.root.path());
    let root = Directory::fixture(&fixture.root).unwrap();

    let parent = root
        .open(
            &name("fixture-created-parent"),
            Kind::Directory,
            OpenMode::CreateNew,
            Sharing::Retained,
        )
        .unwrap();
    fixture.remember("fixture-created-parent", &parent);
    assert_eq!(parent.disposition(), Disposition::Created);
    let parent = parent.into_directory().unwrap();

    let mut file = parent
        .open(
            &name("fixture-nested-file"),
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Retained,
        )
        .unwrap();
    fixture.remember("fixture-created-parent/fixture-nested-file", &file);
    assert_eq!(file.disposition(), Disposition::Created);
    file.write(b"exact nested bytes\0").unwrap();
    assert_eq!(file.read().unwrap(), b"exact nested bytes\0");
    drop(file);

    let subdirectory = parent
        .open(
            &name("fixture-subdirectory"),
            Kind::Directory,
            OpenMode::CreateNew,
            Sharing::Retained,
        )
        .unwrap();
    fixture.remember("fixture-created-parent/fixture-subdirectory", &subdirectory);
    assert_eq!(subdirectory.disposition(), Disposition::Created);
    let subdirectory = subdirectory.into_directory().unwrap();
    assert!(subdirectory.enumerate().unwrap().is_empty());
    let mut descendant = subdirectory
        .open(
            &name("fixture-descendant"),
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Retained,
        )
        .unwrap();
    fixture.remember(
        "fixture-created-parent/fixture-subdirectory/fixture-descendant",
        &descendant,
    );
    descendant.write(b"exact descendant bytes").unwrap();
    assert_eq!(descendant.read().unwrap(), b"exact descendant bytes");
    drop(descendant);
    assert_eq!(
        entry_names(&subdirectory.enumerate().unwrap()),
        BTreeSet::from(["fixture-descendant".into()])
    );
    assert_eq!(
        entry_names(&parent.enumerate().unwrap()),
        BTreeSet::from(["fixture-nested-file".into(), "fixture-subdirectory".into()])
    );
    assert_eq!(
        entry_names(&root.enumerate().unwrap()),
        BTreeSet::from(["fixture-sentinel".into(), "fixture-created-parent".into()])
    );

    assert_eq!(
        fs::read(fixture.path("fixture-created-parent/fixture-nested-file")).unwrap(),
        b"exact nested bytes\0"
    );
    assert_eq!(
        fs::read(fixture.path("fixture-created-parent/fixture-subdirectory/fixture-descendant"))
            .unwrap(),
        b"exact descendant bytes"
    );
    assert_eq!(
        fs::read(fixture.path("fixture-sentinel")).unwrap(),
        b"outer root sentinel"
    );
    assert_eq!(listing(foreign.root.path()), foreign_names);
    assert_eq!(
        fs::read(foreign.path("fixture-created-parent")).unwrap(),
        b"foreign root conflict"
    );
}

#[test]
fn native_hardlink_is_refused_without_changing_either_owned_name() {
    let mut fixture = Fixture::new();
    fixture.file("fixture-source", b"both links retain these bytes");
    let directory = Directory::fixture(&fixture.root).unwrap();
    let opened = fixture.child("fixture-source", Kind::File, Sharing::Retained);
    drop(opened);
    fs::hard_link(
        fixture.path("fixture-source"),
        fixture.path("fixture-alias"),
    )
    .unwrap();
    let alias = fixture_file(&fixture.path("fixture-alias"), false).unwrap();
    fixture.tracked.push(Tracked {
        path: fixture.path("fixture-alias"),
        id: identity(&alias),
    });
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(alias.as_raw_handle()), &mut info) }.unwrap();
    assert_eq!(info.nNumberOfLinks, 2, "fixture has two actual links");
    assert!(matches!(
        directory.open(
            &name("fixture-source"),
            Kind::File,
            OpenMode::Existing,
            Sharing::Retained
        ),
        Err(TuningNativeError::UnsafeStore(_))
    ));
    for label in ["fixture-source", "fixture-alias"] {
        assert_eq!(
            fs::read(fixture.path(label)).unwrap(),
            b"both links retain these bytes"
        );
    }
}

#[test]
fn native_actual_file_cannot_become_a_directory_parent() {
    let mut fixture = Fixture::new();
    fixture.file("fixture-file", b"file cannot be a parent");
    let node = fixture.child("fixture-file", Kind::File, Sharing::Retained);
    assert!(matches!(
        node.into_directory(),
        Err(TuningNativeError::UnsafeStore("relative object kind"))
    ));
    assert_eq!(
        fs::read(fixture.path("fixture-file")).unwrap(),
        b"file cannot be a parent"
    );
}

#[test]
fn native_collision_missing_and_type_refusals_do_not_mutate_objects() {
    let mut fixture = Fixture::new();
    fixture.file("fixture-file", b"retained");
    fixture.directory("fixture-directory");
    let directory = Directory::fixture(&fixture.root).unwrap();
    let opened = directory
        .open(
            &name("fixture-file"),
            Kind::File,
            OpenMode::Existing,
            Sharing::Retained,
        )
        .unwrap();
    drop(opened);
    assert!(
        directory
            .open(
                &name("fixture-file"),
                Kind::File,
                OpenMode::CreateNew,
                Sharing::Retained
            )
            .is_err()
    );
    assert!(
        directory
            .open(
                &name("fixture-missing"),
                Kind::File,
                OpenMode::Existing,
                Sharing::Retained
            )
            .is_err()
    );
    assert!(
        directory
            .open(
                &name("fixture-file"),
                Kind::Directory,
                OpenMode::Existing,
                Sharing::Retained
            )
            .is_err()
    );
    assert!(
        directory
            .open(
                &name("fixture-directory"),
                Kind::File,
                OpenMode::Existing,
                Sharing::Retained
            )
            .is_err()
    );
    assert_eq!(fs::read(fixture.path("fixture-file")).unwrap(), b"retained");
    assert_eq!(
        listing(fixture.root.path()),
        BTreeSet::from(["fixture-file".into(), "fixture-directory".into()])
    );
}

#[test]
fn native_exclusive_lease_contends_and_reopens_only_after_owner_drop() {
    let mut fixture = Fixture::new();
    fixture.file("fixture-lease", b"existing lease bytes");
    let directory = Directory::fixture(&fixture.root).unwrap();
    let lease = directory
        .open(
            &name("fixture-lease"),
            Kind::File,
            OpenMode::OpenOrCreate,
            Sharing::ExclusiveLease,
        )
        .unwrap();
    let original = identity(&lease.file);
    assert!(matches!(
        directory.open(
            &name("fixture-lease"),
            Kind::File,
            OpenMode::Existing,
            Sharing::ExclusiveLease
        ),
        Err(TuningNativeError::Busy)
    ));
    drop(lease);
    let reopened = directory
        .open(
            &name("fixture-lease"),
            Kind::File,
            OpenMode::OpenOrCreate,
            Sharing::ExclusiveLease,
        )
        .unwrap();
    assert_eq!(reopened.disposition(), Disposition::Opened);
    assert!(
        node_matches(&reopened, original),
        "the same lease object reopens"
    );
    drop(reopened);
    assert_eq!(
        fs::read(fixture.path("fixture-lease")).unwrap(),
        b"existing lease bytes"
    );
}

#[test]
fn native_enumeration_includes_unknown_names_and_enforces_total_limit() {
    let mut fixture = Fixture::new();
    fixture.file("external-evidence", b"unknown retained bytes");
    for index in 1..ENTRY_LIMIT {
        fixture.file(&format!("fixture-entry-{index}"), b"owned entry");
    }
    let directory = Directory::fixture(&fixture.root).unwrap();
    let entries = directory.enumerate().unwrap();
    assert_eq!(entries.len(), ENTRY_LIMIT);
    assert_eq!(entry_names(&entries), listing(fixture.root.path()));
    assert_eq!(
        entry_names(&directory.enumerate().unwrap()),
        entry_names(&entries)
    );
    fixture.file("fixture-entry-over-limit", b"extra retained bytes");
    assert!(directory.enumerate().is_err());
    assert_eq!(
        fs::read(fixture.path("external-evidence")).unwrap(),
        b"unknown retained bytes"
    );
    assert_eq!(listing(fixture.root.path()).len(), ENTRY_LIMIT + 1);
}

#[test]
fn native_rename_no_replace_preserves_collision_and_source_identity() {
    let mut fixture = Fixture::new();
    fixture.file("fixture-source", b"source data");
    fixture.file("fixture-target", b"foreign target data");
    let directory = Directory::fixture(&fixture.root).unwrap();
    let source = fixture.child("fixture-source", Kind::File, Sharing::Retained);
    let original = identity(&source.file);
    let target_id = identity(&fixture_file(&fixture.path("fixture-target"), false).unwrap());
    assert!(matches!(
        source.rename_no_replace(&directory, &name("fixture-target")),
        Err(TuningNativeError::Io { code: 80 | 183, .. })
    ));
    assert_eq!(
        fs::read(fixture.path("fixture-source")).unwrap(),
        b"source data"
    );
    assert_eq!(
        fs::read(fixture.path("fixture-target")).unwrap(),
        b"foreign target data"
    );
    assert!(
        identity(&fixture_file(&fixture.path("fixture-target"), false).unwrap()) == target_id,
        "collision preserves target identity"
    );
    source
        .rename_no_replace(&directory, &name("fixture-renamed"))
        .unwrap();
    fixture.moved("fixture-source", "fixture-renamed");
    assert!(!fixture.path("fixture-source").exists());
    assert!(
        identity(&fixture_file(&fixture.path("fixture-renamed"), false).unwrap()) == original,
        "rename retains original object"
    );
    assert_eq!(
        fs::read(fixture.path("fixture-renamed")).unwrap(),
        b"source data"
    );
}

#[test]
fn native_delete_uses_source_handle_after_old_name_is_replaced() {
    let mut fixture = Fixture::new();
    fixture.file("fixture-source", b"original source");
    let source = fixture.child("fixture-source", Kind::File, Sharing::Renameable);
    fixture.rename_setup("fixture-source", "fixture-moved");
    fixture.file("fixture-source", b"replacement must survive");
    fixture.file("fixture-sentinel", b"sentinel must survive");
    source.delete_owned().unwrap();
    assert!(!fixture.path("fixture-moved").exists());
    assert_eq!(
        fs::read(fixture.path("fixture-source")).unwrap(),
        b"replacement must survive"
    );
    assert_eq!(
        fs::read(fixture.path("fixture-sentinel")).unwrap(),
        b"sentinel must survive"
    );
}

#[test]
fn native_content_bounds_refuse_oversized_read_and_write_without_mutation() {
    let mut fixture = Fixture::new();
    let oversized = vec![0x6b; 4097];
    let baseline = b"retained exact baseline\0";
    fixture.file("fixture-oversized", &oversized);
    fixture.file("fixture-small", baseline);
    let mut large = fixture.child("fixture-oversized", Kind::File, Sharing::Retained);
    let mut small = fixture.child("fixture-small", Kind::File, Sharing::Retained);
    small.file.seek(SeekFrom::Start(3)).unwrap();

    // Reach both authentic retained-handle seams before either result assertion.
    let read = large.read();
    let write = small.write(&oversized);
    let position = small.file.stream_position().unwrap();
    let large_bytes = fs::read(fixture.path("fixture-oversized")).unwrap();
    let small_bytes = fs::read(fixture.path("fixture-small")).unwrap();

    assert_eq!(large_bytes, oversized);
    assert_eq!(small_bytes, baseline);
    assert_eq!(
        position, 3,
        "rejected write does not seek the retained handle"
    );
    assert_eq!(
        read,
        Err(TuningNativeError::UnsafeStore("relative content bounds"))
    );
    assert_eq!(
        write,
        Err(TuningNativeError::UnsafeStore("relative content bounds"))
    );
}

#[test]
fn native_read_only_write_rename_and_delete_fail_without_path_fallback() {
    let mut fixture = Fixture::new();
    fixture.file("fixture-read-only", b"retained read-only bytes");
    let mut node =
        Directory::fixture_child_read_only(&fixture.root, &name("fixture-read-only"), Kind::File)
            .unwrap();
    assert_eq!(node.read().unwrap(), b"retained read-only bytes");
    assert!(matches!(
        node.write(b"replacement"),
        Err(TuningNativeError::Denied { code: 5, .. })
    ));
    let directory = Directory::fixture(&fixture.root).unwrap();
    assert!(matches!(
        node.rename_no_replace(&directory, &name("fixture-renamed")),
        Err(TuningNativeError::Denied { code: 5, .. })
    ));
    assert!(!fixture.path("fixture-renamed").exists());
    assert!(matches!(
        node.delete_owned(),
        Err(TuningNativeError::Denied { code: 5, .. })
    ));
    assert_eq!(
        fs::read(fixture.path("fixture-read-only")).unwrap(),
        b"retained read-only bytes"
    );
    let mut opened = directory
        .open_read_only_file(&name("fixture-read-only"))
        .unwrap();
    assert_eq!(opened.read().unwrap(), b"retained read-only bytes");
    assert!(matches!(
        opened.write(b"replacement"),
        Err(TuningNativeError::Denied { code: 5, .. })
    ));
    assert_eq!(
        fs::read(fixture.path("fixture-read-only")).unwrap(),
        b"retained read-only bytes"
    );
}

#[test]
fn native_retained_parent_ignores_replaced_namespace_for_all_relative_operations() {
    let mut fixture = Fixture::new();
    fixture.directory("fixture-held");
    let held = fixture
        .child("fixture-held", Kind::Directory, Sharing::Renameable)
        .into_directory()
        .unwrap();
    let original = identity(&held.as_node().file);
    fixture.rename_setup("fixture-held", "fixture-moved");
    fixture.directory("fixture-held");
    fixture.file("fixture-held/fixture-sentinel", b"replacement sentinel");
    let replacement_names = listing(&fixture.path("fixture-held"));
    let mut created = held
        .open(
            &name("fixture-new"),
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Retained,
        )
        .unwrap();
    fixture.remember("fixture-moved/fixture-new", &created);
    created.write(b"original held object").unwrap();
    drop(created);
    let mut opened = held
        .open(
            &name("fixture-new"),
            Kind::File,
            OpenMode::Existing,
            Sharing::Retained,
        )
        .unwrap();
    assert_eq!(opened.read().unwrap(), b"original held object");
    assert_eq!(
        entry_names(&held.enumerate().unwrap()),
        BTreeSet::from(["fixture-new".into()])
    );
    opened
        .rename_no_replace(&held, &name("fixture-renamed"))
        .unwrap();
    fixture.moved("fixture-moved/fixture-new", "fixture-moved/fixture-renamed");
    drop(opened);
    assert!(
        identity(&held.as_node().file) == original,
        "retained parent identity stays exact"
    );
    assert_eq!(
        fs::read(fixture.path("fixture-moved/fixture-renamed")).unwrap(),
        b"original held object"
    );
    assert_eq!(
        fs::read(fixture.path("fixture-held/fixture-sentinel")).unwrap(),
        b"replacement sentinel"
    );
    assert_eq!(listing(&fixture.path("fixture-held")), replacement_names);
}

#[test]
fn native_parent_junction_race_never_writes_into_foreign_fixture() {
    let mut fixture = Fixture::new();
    let mut target = Fixture::new();
    fixture.directory("fixture-parent");
    target.file("fixture-sentinel", b"foreign sentinel");
    let parent = fixture
        .child("fixture-parent", Kind::Directory, Sharing::Renameable)
        .into_directory()
        .unwrap();
    assert!(listing(&fixture.path("fixture-parent")).is_empty());
    let target_names = listing(target.root.path());
    let entered = Cell::new(false);
    let result = parent.open_after_parent_check(
        &name("fixture-race-write"),
        Kind::File,
        OpenMode::CreateNew,
        Sharing::Retained,
        || {
            set_owned_junction(&parent.as_node().file, target.root.path())
                .expect("owned parent junction setup");
            entered.set(true);
            Ok(())
        },
    );
    let mut returned = result.ok();
    let write = returned
        .as_mut()
        .map(|node| node.write(b"held parent only"));
    // Capture evidence before cleanup: a redirected creation must remain a
    // failure even though cleanup subsequently removes that exact returned object.
    let foreign_bytes = fs::read(target.path("fixture-sentinel")).unwrap();
    let foreign_names = listing(target.root.path());
    if let Some(node) = returned {
        remove_exact(node.file).expect("exact returned race object cleanup");
    }
    assert!(
        entered.get(),
        "the actual before-native-call race boundary was reached"
    );
    assert_eq!(foreign_bytes, b"foreign sentinel");
    assert_eq!(foreign_names, target_names);
    if let Some(write) = write {
        write.unwrap();
    }
}

#[test]
fn native_existing_and_dangling_leaf_junctions_never_follow_or_create_targets() {
    for dangling in [false, true] {
        let mut fixture = Fixture::new();
        let mut target = Fixture::new();
        fixture.directory("fixture-link");
        let target_path = target.root.path().to_owned();
        if !dangling {
            target.file("fixture-sentinel", b"foreign target");
        }
        let link = fixture.child("fixture-link", Kind::Directory, Sharing::Renameable);
        if dangling {
            drop(target);
        }
        set_owned_junction(&link.file, &target_path).unwrap();
        drop(link);
        let directory = Directory::fixture(&fixture.root).unwrap();
        for mode in [OpenMode::Existing, OpenMode::OpenOrCreate] {
            assert!(
                directory
                    .open(
                        &name("fixture-link"),
                        Kind::Directory,
                        mode,
                        Sharing::Retained
                    )
                    .is_err()
            );
        }
        let entries = directory.enumerate().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0);
        if dangling {
            assert!(!target_path.exists());
        } else {
            assert_eq!(
                fs::read(target_path.join("fixture-sentinel")).unwrap(),
                b"foreign target"
            );
            assert_eq!(
                listing(&target_path),
                BTreeSet::from(["fixture-sentinel".into()])
            );
        }
    }
}

#[test]
fn native_replaced_temp_root_is_refused_before_fixture_authority() {
    let original = TuningTemporaryDirectory::new().unwrap();
    let original_handle = fixture_file(original.path(), true).unwrap();
    assert!(
        identity(&original_handle) == original.original_identity(),
        "original root captured"
    );
    let destination = TuningTemporaryDirectory::new().unwrap();
    let moved_path = destination.path().to_owned();
    destination.cleanup().unwrap();
    fs::rename(original.path(), &moved_path).unwrap();
    let replacement = TuningTemporaryDirectory::try_new_at(original.path().to_owned()).unwrap();
    let result = Directory::fixture(&original);
    let refused = result.is_err();
    drop(result);
    assert!(
        identity(&original_handle) == original.original_identity(),
        "held moved root remains owned"
    );
    remove_exact(original_handle).unwrap();
    assert!(!moved_path.exists());
    drop(original);
    replacement.cleanup().unwrap();
    assert!(
        refused,
        "a different root incarnation cannot become fixture authority"
    );
}

const CWD_CHILD: &str = "CPU_AFFINITY_RELATIVE_CWD_CHILD";
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
#[test]
fn native_relative_operations_ignore_an_owned_child_process_current_directory() {
    let mut cwd = Fixture::new();
    cwd.file("fixture-sentinel", b"current directory sentinel");
    cwd.file(
        "fixture-cwd-target",
        b"conflicting current directory target",
    );
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "windows::tuning::relative_io::native_tests::cwd_child_characterization",
                "--nocapture",
            ])
            .env(CWD_CHILD, "1")
            .current_dir(cwd.root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            panic!("owned CWD child exceeded the shared deadline");
        }
        std::thread::yield_now();
    };
    assert!(
        status.success(),
        "owned CWD child must complete relative operations"
    );
    assert_eq!(
        fs::read(cwd.path("fixture-sentinel")).unwrap(),
        b"current directory sentinel"
    );
    assert_eq!(
        listing(cwd.root.path()),
        BTreeSet::from(["fixture-sentinel".into(), "fixture-cwd-target".into()])
    );
    assert_eq!(
        fs::read(cwd.path("fixture-cwd-target")).unwrap(),
        b"conflicting current directory target"
    );
}
#[test]
fn cwd_child_characterization() {
    if std::env::var_os(CWD_CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let mut fixture = Fixture::new();
    let directory = Directory::fixture(&fixture.root).unwrap();
    let mut node = directory
        .open(
            &name("fixture-cwd-target"),
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Retained,
        )
        .unwrap();
    fixture.remember("fixture-cwd-target", &node);
    node.write(b"own child root").unwrap();
    assert_eq!(node.read().unwrap(), b"own child root");
    node.rename_no_replace(&directory, &name("fixture-cwd-renamed"))
        .unwrap();
    fixture.moved("fixture-cwd-target", "fixture-cwd-renamed");
    assert_eq!(
        entry_names(&directory.enumerate().unwrap()),
        BTreeSet::from(["fixture-cwd-renamed".into()])
    );
    node.delete_owned().unwrap();
    assert!(directory.enumerate().unwrap().is_empty());
}
