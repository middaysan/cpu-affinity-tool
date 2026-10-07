//! Real owned NTFS fixtures; these observations never grant canonical authority.
use super::*;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read as IoRead, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
};

const OLD: &[u8] = b"old fixture journal: exact original bytes";
const NEW: &[u8] = b"new fixture journal: exact resulting bytes!";
const FOREIGN: &[u8] = b"foreign owned sentinel must survive";
const TIMEOUT: Duration = Duration::from_secs(10);
const CHILD_TEST: &str = "windows::tuning::relative_publication::native_tests::publication_child";
const ROOT_ENV: &str = "CPU_AFFINITY_PUBLICATION_FIXTURE_ROOT";
const ID_ENV: &str = "CPU_AFFINITY_PUBLICATION_FIXTURE_ID";
const ROLE_ENV: &str = "CPU_AFFINITY_PUBLICATION_FIXTURE_ROLE";
const READY: &str = "PUBLICATION_FIXTURE_READY";
const LINE_LIMIT: usize = 32;
const LINE_BYTES: usize = 512;

#[derive(Clone, PartialEq, Eq)]
struct Artifact {
    id: (u32, u64),
    bytes: Vec<u8>,
}
fn id(file: &File) -> (u32, u64) {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .expect("owned artifact identity query");
    (
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    )
}
fn create(root: &Path, name: &str, bytes: &[u8]) -> Artifact {
    assert!(matches!(
        Path::new(name).components().next(),
        Some(std::path::Component::Normal(_))
    ));
    assert_eq!(Path::new(name).components().count(), 1);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .expect("new owned artifact setup");
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    Artifact {
        id: id(&file),
        bytes: bytes.to_vec(),
    }
}
fn artifact(root: &Path, name: &str) -> Artifact {
    let mut file = File::open(root.join(name)).expect("existing owned artifact");
    assert!(file.metadata().unwrap().len() <= (CAPACITY + 1) as u64);
    let mut bytes = Vec::new();
    (&mut file)
        .take((CAPACITY + 1) as u64)
        .read_to_end(&mut bytes)
        .unwrap();
    Artifact {
        id: id(&file),
        bytes,
    }
}
fn names(root: &Path) -> BTreeSet<String> {
    let entries: Vec<_> = fs::read_dir(root).unwrap().collect();
    assert!(entries.len() <= 12, "bounded owned fixture listing");
    entries
        .into_iter()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect()
}
fn expect_names(root: &Path, expected: &[&str]) {
    assert_eq!(
        names(root),
        expected.iter().map(|name| (*name).to_owned()).collect()
    );
}
fn same(root: &Path, name: &str, expected: &Artifact) {
    assert!(
        artifact(root, name) == *expected,
        "exact owned artifact identity and bytes retained"
    );
}
fn refused_artifacts<T>(result: Result<T>) {
    assert!(
        matches!(
            result,
            Err(TuningNativeError::UnsafeStore(
                "relative publication artifacts"
            ))
        ),
        "interrupted or unknown artifacts must refuse clean admission"
    );
}
fn busy(root: &TuningTemporaryDirectory) {
    assert!(
        matches!(
            PublicationOwner::fixture(root),
            Err(TuningNativeError::Busy)
        ),
        "the original publication context must exclude a replacement owner"
    );
}

#[test]
fn native_initial_and_replacement_publish_verify_exact_objects() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let mut owner = PublicationOwner::fixture(&root).unwrap();
    assert_eq!(owner.read().unwrap(), Read::Absent);
    owner.publish(OLD).unwrap();
    let old = artifact(root.path(), JOURNAL);
    assert!(old.bytes == OLD);
    let phases = Arc::new(Mutex::new(Vec::new()));
    let trace = phases.clone();
    let staged = Arc::new(Mutex::new(None));
    let expected_stage = staged.clone();
    let observed_root = root.path().to_owned();
    let original = old.clone();
    owner.set_boundary_hook(move |phase| {
        let mut trace = trace.lock().unwrap();
        match phase {
            Boundary::StageSynced => {
                expect_names(&observed_root, &[LEASE, JOURNAL, STAGE]);
                same(&observed_root, JOURNAL, &original);
                let stage = artifact(&observed_root, STAGE);
                assert!(stage.bytes == NEW);
                *expected_stage.lock().unwrap() = Some(stage);
            }
            Boundary::OldMoved => {
                expect_names(&observed_root, &[LEASE, PREVIOUS, STAGE]);
                same(&observed_root, PREVIOUS, &original);
            }
            Boundary::NewMoved | Boundary::Verified => {
                expect_names(&observed_root, &[LEASE, JOURNAL, PREVIOUS]);
                same(&observed_root, PREVIOUS, &original);
                same(
                    &observed_root,
                    JOURNAL,
                    expected_stage.lock().unwrap().as_ref().unwrap(),
                );
            }
            Boundary::OldDeleted => {
                expect_names(&observed_root, &[LEASE, JOURNAL]);
                same(
                    &observed_root,
                    JOURNAL,
                    expected_stage.lock().unwrap().as_ref().unwrap(),
                );
            }
        }
        trace.push(phase);
        Ok(())
    });
    owner.publish(NEW).unwrap();
    assert_eq!(
        *phases.lock().unwrap(),
        [
            Boundary::StageSynced,
            Boundary::OldMoved,
            Boundary::NewMoved,
            Boundary::Verified,
            Boundary::OldDeleted
        ]
    );
    let new = artifact(root.path(), JOURNAL);
    assert!(
        Some(new.clone()) == *staged.lock().unwrap(),
        "published object equals the exact synced stage"
    );
    assert!(
        new.id != old.id,
        "replacement must be the staged object, not the original"
    );
    assert_eq!(owner.read().unwrap(), Read::Present(NEW.to_vec()));
    assert_eq!(owner.state(), OwnerState::Ready);
    drop(owner);
    let mut fresh = PublicationOwner::fixture(&root).unwrap();
    assert_eq!(fresh.read().unwrap(), Read::Present(NEW.to_vec()));
    same(root.path(), JOURNAL, &new);
}

#[test]
fn native_orphan_and_unknown_files_never_become_absent_or_initializable() {
    for initial in [
        vec![STAGE],
        vec![PREVIOUS, STAGE],
        vec![JOURNAL, PREVIOUS],
        vec![JOURNAL, STAGE],
        vec!["fixture-unknown"],
    ] {
        let root = TuningTemporaryDirectory::new().unwrap();
        let originals: Vec<_> = initial
            .iter()
            .map(|name| (*name, create(root.path(), name, FOREIGN)))
            .collect();
        let mut reader = PublicationOwner::fixture(&root).unwrap();
        refused_artifacts(reader.read());
        drop(reader);
        let mut initializer = PublicationOwner::fixture(&root).unwrap();
        refused_artifacts(initializer.publish(NEW));
        for (name, original) in originals {
            same(root.path(), name, &original);
        }
        let mut expected = initial;
        expected.push(LEASE);
        expect_names(root.path(), &expected);
    }
}

#[test]
fn native_terminal_fault_poison_retains_evidence_and_lease_until_drop() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let old = create(root.path(), JOURNAL, OLD);
    let reached = Arc::new(Mutex::new(false));
    let signal = reached.clone();
    let mut owner = PublicationOwner::fixture(&root).unwrap();
    owner.set_boundary_hook(move |phase| {
        if phase == Boundary::StageSynced {
            *signal.lock().unwrap() = true;
            return Err(TuningNativeError::Io {
                operation: "fixture terminal boundary",
                code: 5,
            });
        }
        Ok(())
    });
    let result = owner.publish(NEW);
    assert!(
        *reached.lock().unwrap(),
        "fault follows the real synced stage boundary"
    );
    assert_eq!(
        result,
        Err(TuningNativeError::Io {
            operation: "fixture terminal boundary",
            code: 5
        })
    );
    assert_eq!(owner.state(), OwnerState::Poisoned);
    let stage = artifact(root.path(), STAGE);
    same(root.path(), JOURNAL, &old);
    busy(&root);
    assert_eq!(owner.read(), Err(TuningNativeError::Poisoned));
    assert_eq!(owner.publish(OLD), Err(TuningNativeError::Poisoned));
    same(root.path(), STAGE, &stage);
    drop(owner);
    let mut fresh = PublicationOwner::fixture(&root).unwrap();
    refused_artifacts(fresh.read());
    same(root.path(), JOURNAL, &old);
    same(root.path(), STAGE, &stage);
}

#[test]
fn native_collision_at_each_rename_preserves_foreign_and_owned_evidence() {
    for destination in [PREVIOUS, JOURNAL] {
        let root = TuningTemporaryDirectory::new().unwrap();
        let old = create(root.path(), JOURNAL, OLD);
        let observed_root = root.path().to_owned();
        let captured = Arc::new(Mutex::new(None));
        let signal = captured.clone();
        let mut owner = PublicationOwner::fixture(&root).unwrap();
        owner.set_boundary_hook(move |phase| {
            let stop = if destination == PREVIOUS {
                Boundary::StageSynced
            } else {
                Boundary::OldMoved
            };
            if phase == stop {
                *signal.lock().unwrap() = Some(create(&observed_root, destination, FOREIGN));
            }
            Ok(())
        });
        let result = owner.publish(NEW);
        let collision = captured
            .lock()
            .unwrap()
            .clone()
            .expect("collision was introduced at real boundary");
        assert!(
            matches!(result, Err(TuningNativeError::Io { code: 183, .. })),
            "no-replace collision must remain a native collision"
        );
        assert_eq!(owner.state(), OwnerState::Poisoned);
        same(root.path(), destination, &collision);
        same(
            root.path(),
            if destination == PREVIOUS {
                JOURNAL
            } else {
                PREVIOUS
            },
            &old,
        );
        assert!(artifact(root.path(), STAGE).bytes == NEW);
        busy(&root);
    }
}

#[test]
fn native_source_name_substitution_is_refused_before_corresponding_rename() {
    for source in [JOURNAL, STAGE] {
        let root = TuningTemporaryDirectory::new().unwrap();
        let old = create(root.path(), JOURNAL, OLD);
        let observed_root = root.path().to_owned();
        let captured = Arc::new(Mutex::new(None));
        let signal = captured.clone();
        let mut owner = PublicationOwner::fixture(&root).unwrap();
        owner.set_boundary_hook(move |phase| {
            let stop = if source == JOURNAL {
                Boundary::StageSynced
            } else {
                Boundary::OldMoved
            };
            if phase == stop {
                let original = artifact(&observed_root, source);
                fs::rename(
                    observed_root.join(source),
                    observed_root.join("fixture-moved-source"),
                )
                .unwrap();
                let replacement = create(&observed_root, source, FOREIGN);
                *signal.lock().unwrap() = Some((original, replacement));
            }
            Ok(())
        });
        let result = owner.publish(NEW);
        let (original, foreign) = captured
            .lock()
            .unwrap()
            .clone()
            .expect("actual source substitution reached");
        assert!(
            result.is_err(),
            "a substituted source name must not be published"
        );
        assert_eq!(owner.state(), OwnerState::Poisoned);
        same(root.path(), source, &foreign);
        same(root.path(), "fixture-moved-source", &original);
        if source == JOURNAL {
            assert!(
                !root.path().join(PREVIOUS).exists(),
                "old rename is stopped before mutation"
            );
            assert!(original == old);
        } else {
            same(root.path(), PREVIOUS, &old);
            assert!(
                !root.path().join(JOURNAL).exists(),
                "stage rename is stopped before mutation"
            );
        }
    }
}

#[test]
fn native_old_cleanup_deletes_only_its_retained_source_handle() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let old = create(root.path(), JOURNAL, OLD);
    let observed_root = root.path().to_owned();
    let captured = Arc::new(Mutex::new(None));
    let signal = captured.clone();
    let mut owner = PublicationOwner::fixture(&root).unwrap();
    owner.set_boundary_hook(move |phase| {
        if phase == Boundary::Verified {
            same(&observed_root, PREVIOUS, &old);
            fs::rename(
                observed_root.join(PREVIOUS),
                observed_root.join("fixture-original-old"),
            )
            .unwrap();
            let foreign = create(&observed_root, PREVIOUS, FOREIGN);
            *signal.lock().unwrap() = Some((foreign, artifact(&observed_root, JOURNAL)));
        }
        Ok(())
    });
    let result = owner.publish(NEW);
    let (foreign, new) = captured
        .lock()
        .unwrap()
        .clone()
        .expect("verified boundary before actual old cleanup");
    assert!(
        result.is_err(),
        "remaining foreign artifact prevents a clean terminal result"
    );
    assert!(
        !root.path().join("fixture-original-old").exists(),
        "exact retained old object must be deleted"
    );
    same(root.path(), PREVIOUS, &foreign);
    same(root.path(), JOURNAL, &new);
    assert!(new.bytes == NEW);
    assert_eq!(owner.state(), OwnerState::Poisoned);
}

#[test]
fn native_verified_journal_identity_substitution_preserves_old_before_cleanup() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let old = create(root.path(), JOURNAL, OLD);
    let observed_root = root.path().to_owned();
    let captured = Arc::new(Mutex::new(None));
    let signal = captured.clone();
    let mut owner = PublicationOwner::fixture(&root).unwrap();
    owner.set_boundary_hook(move |phase| {
        if phase == Boundary::Verified {
            let stage = artifact(&observed_root, JOURNAL);
            assert!(stage.bytes == NEW);
            fs::rename(
                observed_root.join(JOURNAL),
                observed_root.join("fixture-published-stage"),
            )
            .unwrap();
            let foreign = create(&observed_root, JOURNAL, NEW);
            assert!(
                foreign.id != stage.id,
                "substitution uses a different actual object"
            );
            *signal.lock().unwrap() = Some((stage, foreign));
        }
        Ok(())
    });
    let result = owner.publish(NEW);
    let (stage, foreign) = captured
        .lock()
        .unwrap()
        .clone()
        .expect("same-byte journal substitution follows real verified boundary");
    assert!(
        result.is_err(),
        "equal bytes cannot replace exact published object proof"
    );
    assert_eq!(owner.state(), OwnerState::Poisoned);
    same(root.path(), PREVIOUS, &old);
    same(root.path(), JOURNAL, &foreign);
    same(root.path(), "fixture-published-stage", &stage);
    expect_names(
        root.path(),
        &[LEASE, PREVIOUS, JOURNAL, "fixture-published-stage"],
    );
    busy(&root);
}

#[test]
fn native_root_pin_refuses_namespace_replacement_and_preserves_foreign_root() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let foreign_root = TuningTemporaryDirectory::new().unwrap();
    let foreign = create(foreign_root.path(), JOURNAL, FOREIGN);
    let vacant = TuningTemporaryDirectory::new().unwrap();
    let destination = vacant.path().to_owned();
    vacant.cleanup().unwrap();
    let mut owner = PublicationOwner::fixture(&root).unwrap();
    let attempted = fs::rename(root.path(), &destination);
    if attempted.is_ok() {
        fs::rename(&destination, root.path())
            .expect("restore exact owned root after failed pin characterization");
    }
    assert!(attempted.is_err(), "retained root denies delete sharing");
    owner.publish(NEW).unwrap();
    assert!(artifact(root.path(), JOURNAL).bytes == NEW);
    same(foreign_root.path(), JOURNAL, &foreign);
    expect_names(foreign_root.path(), &[JOURNAL]);
}

#[test]
fn native_capacity_refusal_preserves_existing_bytes_and_allows_valid_publication() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let old = create(root.path(), JOURNAL, OLD);
    let mut owner = PublicationOwner::fixture(&root).unwrap();
    assert_eq!(
        owner.publish(&vec![0x71; CAPACITY + 1]),
        Err(TuningNativeError::Capacity {
            size: (CAPACITY + 1) as u64,
            limit: CAPACITY,
        })
    );
    same(root.path(), JOURNAL, &old);
    expect_names(root.path(), &[LEASE, JOURNAL]);
    assert_eq!(owner.state(), OwnerState::Ready);
    owner.publish(NEW).unwrap();
    assert_eq!(owner.read().unwrap(), Read::Present(NEW.to_vec()));
    drop(owner);

    let oversized_root = TuningTemporaryDirectory::new().unwrap();
    let oversized = create(oversized_root.path(), JOURNAL, &vec![0x71; CAPACITY + 1]);
    let mut reader = PublicationOwner::fixture(&oversized_root).unwrap();
    assert_eq!(
        reader.read(),
        Err(TuningNativeError::UnsafeStore("relative content bounds"))
    );
    same(oversized_root.path(), JOURNAL, &oversized);
    expect_names(oversized_root.path(), &[LEASE, JOURNAL]);
}

#[test]
fn native_borrowed_fixture_selector_requires_original_id_and_owned_root_shape() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let (volume, index) = root.original_identity();
    assert!(matches!(
        PublicationOwner::fixture_observed_root(root.path(), (volume, index ^ 1)),
        Err(TuningNativeError::UnsafeStore(
            "relative fixture root identity"
        ))
    ));
    for invalid in [
        root.path().join("fixture-nested"),
        std::env::temp_dir().join("fixture-unowned-selection"),
    ] {
        assert!(matches!(
            PublicationOwner::fixture_observed_root(&invalid, (volume, index)),
            Err(TuningNativeError::UnsafeStore(
                "owned temporary root required"
            ))
        ));
    }
    expect_names(root.path(), &[]);
}

struct Worker {
    release: mpsc::SyncSender<()>,
    thread: Option<JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.release.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[test]
fn native_barrier_timeout_retains_same_worker_owner_until_terminal_release() {
    let root = TuningTemporaryDirectory::new().unwrap();
    create(root.path(), JOURNAL, OLD);
    let path = root.path().to_owned();
    let identity = root.original_identity();
    let deadline = Instant::now() + TIMEOUT;
    let (entered, entering) = mpsc::sync_channel(1);
    let (release, releasing) = mpsc::sync_channel(1);
    let (finished, finishing) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let mut owner = PublicationOwner::fixture_observed_root(&path, identity).unwrap();
        owner.set_boundary_hook(move |phase| {
            if phase == Boundary::StageSynced {
                entered.send(()).unwrap();
                releasing
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .expect("explicit same-worker boundary release");
            }
            Ok(())
        });
        let result = owner.publish(NEW);
        drop(owner);
        finished.send(result).unwrap();
    });
    let worker = Worker {
        release,
        thread: Some(handle),
    };
    entering
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .expect("actual synced stage entered on original worker");
    assert!(matches!(
        finishing.recv_timeout(Duration::ZERO),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    busy(&root);
    expect_names(root.path(), &[LEASE, JOURNAL, STAGE]);
    worker.release.send(()).unwrap();
    assert_eq!(
        finishing
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap(),
        Ok(())
    );
    let mut fresh = PublicationOwner::fixture(&root).unwrap();
    assert_eq!(fresh.read().unwrap(), Read::Present(NEW.to_vec()));
}

struct OwnedChild {
    child: Child,
    lines: mpsc::Receiver<String>,
    reader: Option<JoinHandle<()>>,
    deadline: Instant,
}
impl OwnedChild {
    fn spawn(root: &TuningTemporaryDirectory, role: &str) -> Self {
        let deadline = Instant::now() + TIMEOUT;
        let (volume, index) = root.original_identity();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(ROOT_ENV, root.path())
            .env(ID_ENV, format!("{volume}:{index}"))
            .env(ROLE_ENV, role)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .expect("owned publication child setup");
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::sync_channel(LINE_LIMIT);
        let reader = thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            for _ in 0..LINE_LIMIT {
                let mut line = String::new();
                let result = (&mut input)
                    .take((LINE_BYTES + 1) as u64)
                    .read_line(&mut line);
                if !matches!(result, Ok(1..)) || line.len() > LINE_BYTES || !line.ends_with('\n') {
                    break;
                }
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            reader: Some(reader),
            deadline,
        }
    }
    fn wait_ready(&self) {
        for _ in 0..LINE_LIMIT {
            let line = self
                .lines
                .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
                .expect("owned child failed to reach required real publication boundary");
            if line.trim() == READY {
                return;
            }
        }
        panic!("owned child protocol line limit");
    }
    fn terminate(&mut self) {
        assert!(
            self.child.try_wait().unwrap().is_none(),
            "child retains original context until forced kill"
        );
        self.child.kill().unwrap();
        assert!(!self.child.wait().unwrap().success());
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
fn park_at_boundary() -> ! {
    println!("\n{READY}");
    std::io::stdout().flush().unwrap();
    let mutex = Mutex::new(());
    let signal = Condvar::new();
    let mut guard = mutex.lock().unwrap();
    loop {
        guard = signal.wait(guard).unwrap();
    }
}
fn boundary_role(boundary: Boundary) -> &'static str {
    match boundary {
        Boundary::StageSynced => "stage-synced",
        Boundary::OldMoved => "old-moved",
        Boundary::NewMoved => "new-moved",
        Boundary::Verified => "verified",
        Boundary::OldDeleted => "old-deleted",
    }
}

#[test]
fn native_forced_child_boundaries_preserve_ids_bytes_and_block_orphan_initialization() {
    let cases = [
        (true, Boundary::StageSynced),
        (true, Boundary::OldMoved),
        (true, Boundary::NewMoved),
        (true, Boundary::Verified),
        (true, Boundary::OldDeleted),
        (false, Boundary::StageSynced),
        (false, Boundary::NewMoved),
    ];
    for (replacement, boundary) in cases {
        let root = TuningTemporaryDirectory::new().unwrap();
        let old = replacement.then(|| create(root.path(), JOURNAL, OLD));
        let mut child = OwnedChild::spawn(&root, boundary_role(boundary));
        child.wait_ready();
        busy(&root);
        let expected: Vec<_> = match (replacement, boundary) {
            (true, Boundary::StageSynced) => vec![LEASE, JOURNAL, STAGE],
            (true, Boundary::OldMoved) => vec![LEASE, PREVIOUS, STAGE],
            (true, Boundary::NewMoved | Boundary::Verified) => vec![LEASE, JOURNAL, PREVIOUS],
            (_, Boundary::StageSynced) => vec![LEASE, STAGE],
            _ => vec![LEASE, JOURNAL],
        };
        expect_names(root.path(), &expected);
        let preserved: Vec<_> = expected
            .iter()
            .filter(|name| **name != LEASE)
            .map(|name| (*name, artifact(root.path(), name)))
            .collect();
        let resulting_name = if matches!(boundary, Boundary::StageSynced | Boundary::OldMoved) {
            STAGE
        } else {
            JOURNAL
        };
        let resulting = artifact(root.path(), resulting_name);
        assert!(
            resulting.bytes == NEW,
            "synced staged bytes survive each forced boundary"
        );
        if let Some(old) = &old {
            same(
                root.path(),
                if matches!(boundary, Boundary::StageSynced | Boundary::OldDeleted) {
                    JOURNAL
                } else {
                    PREVIOUS
                },
                if boundary == Boundary::OldDeleted {
                    &resulting
                } else {
                    old
                },
            );
            assert!(
                resulting.id != old.id,
                "stage and original identities are independent"
            );
        }
        child.terminate();
        let clean =
            boundary == Boundary::OldDeleted || (!replacement && boundary == Boundary::NewMoved);
        let mut fresh = PublicationOwner::fixture(&root).unwrap();
        if clean {
            assert_eq!(fresh.read().unwrap(), Read::Present(NEW.to_vec()));
        } else {
            refused_artifacts(fresh.read());
        }
        drop(fresh);
        if !clean {
            let mut initializer = PublicationOwner::fixture(&root).unwrap();
            refused_artifacts(initializer.publish(OLD));
        }
        expect_names(root.path(), &expected);
        for (name, original) in preserved {
            same(root.path(), name, &original);
        }
    }
}

#[test]
fn native_forced_child_before_and_after_clean_publication_releases_lease_only_on_exit() {
    for role in ["before", "after"] {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut child = OwnedChild::spawn(&root, role);
        child.wait_ready();
        busy(&root);
        expect_names(
            root.path(),
            if role == "before" {
                &[LEASE]
            } else {
                &[LEASE, JOURNAL]
            },
        );
        child.terminate();
        let mut fresh = PublicationOwner::fixture(&root).unwrap();
        assert_eq!(
            fresh.read().unwrap(),
            if role == "before" {
                Read::Absent
            } else {
                Read::Present(NEW.to_vec())
            }
        );
    }
}

#[test]
fn native_synthetic_pending_retains_whole_child_context_after_wrapper_drop() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let mut child = OwnedChild::spawn(&root, "pending");
    child.wait_ready();
    expect_names(root.path(), &[LEASE]);
    busy(&root);
    child.terminate();
    let _fresh = PublicationOwner::fixture(&root).unwrap();
    // This is synthetic ownership retention, not an observed SDK pending I/O.
}

#[test]
fn publication_child() {
    let Some(role) = std::env::var_os(ROLE_ENV) else {
        return;
    };
    let role = role.to_str().expect("fixed child role");
    let path = std::env::var_os(ROOT_ENV).expect("parent-owned fixture root");
    assert!(path.len() <= 1024, "bounded parent-owned root input");
    let raw_id = std::env::var(ID_ENV).expect("captured parent-owned root identity");
    assert!(raw_id.len() <= 32, "bounded parent identity input");
    let (volume, index) = raw_id.split_once(':').expect("fixed root identity shape");
    let identity = (
        volume.parse::<u32>().unwrap(),
        index.parse::<u64>().unwrap(),
    );
    let mut owner =
        PublicationOwner::fixture_observed_root(&PathBuf::from(path), identity).unwrap();
    match role {
        "pending" => {
            assert_eq!(
                owner.inject_pending_after_lease_for_test(),
                Err(TuningNativeError::UnsafeStore(
                    "relative pending completion"
                ))
            );
            assert_eq!(owner.state(), OwnerState::Unresolved);
            assert_eq!(owner.read(), Err(TuningNativeError::Poisoned));
            assert_eq!(owner.publish(NEW), Err(TuningNativeError::Poisoned));
            assert_eq!(owner.state(), OwnerState::Unresolved);
            drop(owner);
            park_at_boundary();
        }
        "before" => park_at_boundary(),
        "after" => {
            owner.publish(NEW).unwrap();
            park_at_boundary();
        }
        "stage-synced" | "old-moved" | "new-moved" | "verified" | "old-deleted" => {
            let role = role.to_owned();
            owner.set_boundary_hook(move |phase| {
                if boundary_role(phase) == role {
                    park_at_boundary();
                }
                Ok(())
            });
            owner.publish(NEW).unwrap();
            panic!("desired real publication boundary was not reached");
        }
        _ => panic!("unsupported fixed publication child role"),
    }
}
