//! Production store operations on actual owned IsolatedTest NTFS fixtures.
//! No canonical factory, global directory, settings or fabricated authority.
use super::super::test_support::TuningTemporaryDirectory;
use super::{TuningPublishPoint, TuningStore};
use crate::tuning::{MAX_TUNING_JOURNAL_BYTES, TuningNativeError, TuningStoreAuthority};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
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

const JOURNAL: &str = "journal-v1.json";
const OLD: &[u8] = b"actual isolated production store original";
const NEW: &[u8] = b"actual isolated production store replacement";
const STAGE: &str = "journal-v1.staging";
const PREVIOUS: &str = "journal-v1.previous";
const LEASE: &str = "writer-v1.lock";
const POINTS: [TuningPublishPoint; 5] = [
    TuningPublishPoint::AfterStageSync,
    TuningPublishPoint::AfterOldMove,
    TuningPublishPoint::AfterPublish,
    TuningPublishPoint::BeforeBackupCleanup,
    TuningPublishPoint::AfterBackupCleanup,
];

#[derive(Clone, PartialEq, Eq)]
struct Artifact {
    id: (u32, u64),
    bytes: Vec<u8>,
}
fn artifact(root: &Path, name: &str) -> Artifact {
    let mut file = File::open(root.join(name)).expect("existing owned artifact");
    assert!(file.metadata().unwrap().len() <= (MAX_TUNING_JOURNAL_BYTES + 1) as u64);
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .expect("owned artifact identity query");
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_TUNING_JOURNAL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .unwrap();
    Artifact {
        id: (
            info.dwVolumeSerialNumber,
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ),
        bytes,
    }
}
fn store(root: &TuningTemporaryDirectory) -> TuningStore {
    let store = TuningStore::open_test_directory(root.path(), MAX_TUNING_JOURNAL_BYTES)
        .expect("actual protected isolated store and lease");
    assert_eq!(store.authority(), TuningStoreAuthority::IsolatedTest);
    store
}
fn names(root: &Path) -> BTreeSet<String> {
    let entries: Vec<_> = fs::read_dir(root).unwrap().collect();
    assert!(
        entries.len() <= 8,
        "bounded owned migration fixture inspection"
    );
    entries
        .into_iter()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect()
}
fn expected_names(point: TuningPublishPoint) -> &'static [&'static str] {
    match point {
        TuningPublishPoint::AfterStageSync => &[LEASE, JOURNAL, STAGE],
        TuningPublishPoint::AfterOldMove => &[LEASE, PREVIOUS, STAGE],
        TuningPublishPoint::AfterPublish | TuningPublishPoint::BeforeBackupCleanup => {
            &[LEASE, JOURNAL, PREVIOUS]
        }
        TuningPublishPoint::AfterBackupCleanup => &[LEASE, JOURNAL],
    }
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
        "exact migration artifact ID and bytes retained"
    );
}
fn create_artifact(root: &Path, name: &str, bytes: &[u8]) -> Artifact {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    artifact(root, name)
}
fn blocked_fresh(root: &TuningTemporaryDirectory) {
    match TuningStore::open_test_directory(root.path(), MAX_TUNING_JOURNAL_BYTES) {
        Err(TuningNativeError::UnsafeStore(_)) => {}
        Ok(mut writer) => {
            assert!(
                matches!(writer.read(), Err(TuningNativeError::UnsafeStore(_))),
                "interrupted artifact cannot become clean absence"
            );
            assert!(
                matches!(
                    writer.publish(OLD),
                    Err(TuningNativeError::UnsafeStore(_) | TuningNativeError::Poisoned)
                ),
                "interrupted artifact cannot be initialized or repaired"
            );
        }
        _ => panic!("fresh interrupted store must refuse retained artifacts"),
    }
}
fn busy(root: &TuningTemporaryDirectory) {
    assert!(
        matches!(
            TuningStore::open_test_directory(root.path(), MAX_TUNING_JOURNAL_BYTES),
            Err(TuningNativeError::Busy)
        ),
        "original worker still owns the actual store lease"
    );
}

#[test]
fn production_store_faults_at_all_five_real_two_rename_boundaries() {
    for point in POINTS {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut writer = store(&root);
        writer.publish(OLD).unwrap();
        let old = artifact(root.path(), JOURNAL);
        let reached = Arc::new(Mutex::new(false));
        let observed = reached.clone();
        writer.set_publication_hook(move |current| {
            if current == point {
                *observed.lock().unwrap() = true;
                return Err(TuningNativeError::Io {
                    operation: "fixture migration boundary",
                    code: 5,
                });
            }
            Ok(())
        });
        let result = writer.publish(NEW);
        assert!(
            *reached.lock().unwrap(),
            "fault follows the requested real store operation"
        );
        assert_eq!(
            result,
            Err(TuningNativeError::Io {
                operation: "fixture migration boundary",
                code: 5
            })
        );
        expect_names(root.path(), expected_names(point));
        if point == TuningPublishPoint::AfterStageSync {
            same(root.path(), JOURNAL, &old);
        } else if point != TuningPublishPoint::AfterBackupCleanup {
            same(root.path(), PREVIOUS, &old);
        }
        let new_name = if matches!(
            point,
            TuningPublishPoint::AfterStageSync | TuningPublishPoint::AfterOldMove
        ) {
            STAGE
        } else {
            JOURNAL
        };
        let new = artifact(root.path(), new_name);
        assert!(
            new.bytes == NEW && new.id != old.id,
            "new bytes live in the distinct staged object"
        );
        busy(&root);
        assert_eq!(writer.publish(OLD), Err(TuningNativeError::Poisoned));
        drop(writer);
        if point == TuningPublishPoint::AfterBackupCleanup {
            let mut fresh = store(&root);
            assert!(fresh.read().unwrap().as_deref() == Some(NEW));
        } else {
            blocked_fresh(&root);
        }
        same(root.path(), new_name, &new);
        expect_names(root.path(), expected_names(point));
    }
}

#[test]
fn production_store_source_name_substitution_prevents_each_corresponding_rename() {
    for source in [JOURNAL, STAGE] {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut writer = store(&root);
        writer.publish(OLD).unwrap();
        let old = artifact(root.path(), JOURNAL);
        let path = root.path().to_owned();
        let captured = Arc::new(Mutex::new(None));
        let observe = captured.clone();
        writer.set_publication_hook(move |point| {
            let stop = if source == JOURNAL {
                TuningPublishPoint::AfterStageSync
            } else {
                TuningPublishPoint::AfterOldMove
            };
            if point == stop {
                let original = artifact(&path, source);
                fs::rename(path.join(source), path.join("fixture-moved-source")).unwrap();
                let foreign = create_artifact(&path, source, &original.bytes);
                assert!(
                    foreign.id != original.id && foreign.bytes == original.bytes,
                    "substituted source has equal bytes but a different actual ID"
                );
                *observe.lock().unwrap() = Some((original, foreign));
            }
            Ok(())
        });
        let result = writer.publish(NEW);
        let (original, foreign) = captured
            .lock()
            .unwrap()
            .clone()
            .expect("source substitution follows the real corresponding rename boundary");
        assert!(
            result.is_err(),
            "substituted source name must stop its native rename"
        );
        same(root.path(), source, &foreign);
        same(root.path(), "fixture-moved-source", &original);
        if source == JOURNAL {
            assert!(original == old);
            assert!(!root.path().join(PREVIOUS).exists());
            expect_names(
                root.path(),
                &[LEASE, JOURNAL, STAGE, "fixture-moved-source"],
            );
        } else {
            same(root.path(), PREVIOUS, &old);
            assert!(!root.path().join(JOURNAL).exists());
            expect_names(
                root.path(),
                &[LEASE, PREVIOUS, STAGE, "fixture-moved-source"],
            );
        }
        busy(&root);
        assert_eq!(writer.publish(OLD), Err(TuningNativeError::Poisoned));
    }
}

#[test]
fn production_store_journal_collision_preserves_foreign_name_before_second_rename() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let mut writer = store(&root);
    writer.publish(OLD).unwrap();
    let old = artifact(root.path(), JOURNAL);
    let path = root.path().to_owned();
    let captured = Arc::new(Mutex::new(None));
    let observe = captured.clone();
    writer.set_publication_hook(move |point| {
        if point == TuningPublishPoint::AfterOldMove {
            let stage = artifact(&path, STAGE);
            let foreign = create_artifact(&path, JOURNAL, b"foreign journal collision");
            *observe.lock().unwrap() = Some((stage, foreign));
        }
        Ok(())
    });
    let result = writer.publish(NEW);
    let (stage, foreign) = captured
        .lock()
        .unwrap()
        .clone()
        .expect("actual old rename precedes journal collision");
    assert!(
        matches!(result, Err(TuningNativeError::Io { code: 183, .. })),
        "second rename refuses replacement with the actual native collision"
    );
    same(root.path(), PREVIOUS, &old);
    same(root.path(), STAGE, &stage);
    same(root.path(), JOURNAL, &foreign);
    expect_names(root.path(), &[LEASE, PREVIOUS, STAGE, JOURNAL]);
    busy(&root);
    assert_eq!(writer.publish(OLD), Err(TuningNativeError::Poisoned));
}

#[test]
fn production_store_read_present_refuses_same_byte_different_id_late_substitution() {
    for orphan in [false, true] {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut writer = store(&root);
        writer.publish(OLD).unwrap();
        let old = artifact(root.path(), JOURNAL);
        let path = root.path().to_owned();
        let captured = Arc::new(Mutex::new(None));
        let observe = captured.clone();
        writer.set_after_read_snapshot_hook(move || {
            let introduced = if orphan {
                create_artifact(&path, STAGE, b"late orphan beside unchanged journal")
            } else {
                fs::rename(path.join(JOURNAL), path.join("fixture-read-original")).unwrap();
                create_artifact(&path, JOURNAL, OLD)
            };
            *observe.lock().unwrap() = Some(introduced);
        });
        let result = writer.read();
        let introduced = captured
            .lock()
            .unwrap()
            .clone()
            .expect("actual present snapshot precedes namespace change");
        if orphan {
            same(root.path(), JOURNAL, &old);
            same(root.path(), STAGE, &introduced);
            expect_names(root.path(), &[LEASE, JOURNAL, STAGE]);
        } else {
            assert!(introduced.id != old.id && introduced.bytes == old.bytes);
            same(root.path(), JOURNAL, &introduced);
            same(root.path(), "fixture-read-original", &old);
        }
        assert!(
            matches!(result, Err(TuningNativeError::UnsafeStore(_))),
            "public present read must replay both journal identity and clean artifacts"
        );
        busy(&root);
    }
}

#[test]
fn production_store_read_absent_refuses_a_late_journal_or_orphan() {
    for name in [JOURNAL, STAGE] {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut writer = store(&root);
        let path = root.path().to_owned();
        let captured = Arc::new(Mutex::new(None));
        let observe = captured.clone();
        writer.set_after_read_snapshot_hook(move || {
            *observe.lock().unwrap() =
                Some(create_artifact(&path, name, b"late retained evidence"));
        });
        let result = writer.read();
        let introduced = captured
            .lock()
            .unwrap()
            .clone()
            .expect("actual absent snapshot precedes late entry");
        same(root.path(), name, &introduced);
        expect_names(root.path(), &[LEASE, name]);
        assert!(
            matches!(result, Err(TuningNativeError::UnsafeStore(_))),
            "late journal or staging evidence must invalidate the observed absence"
        );
        busy(&root);
    }
}

#[test]
fn production_store_cleanup_rechecks_journal_name_after_actual_old_read() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let mut writer = store(&root);
    writer.publish(OLD).unwrap();
    let old = artifact(root.path(), JOURNAL);
    let path = root.path().to_owned();
    let captured = Arc::new(Mutex::new(None));
    let observe = captured.clone();
    let expected_old = old.clone();
    writer.set_after_previous_read_hook(move || {
        same(&path, PREVIOUS, &expected_old);
        let stage = artifact(&path, JOURNAL);
        fs::rename(path.join(JOURNAL), path.join("fixture-after-old-read")).unwrap();
        let foreign = create_artifact(&path, JOURNAL, NEW);
        assert!(foreign.id != stage.id && foreign.bytes == stage.bytes);
        *observe.lock().unwrap() = Some((stage, foreign));
    });
    let result = writer.publish(NEW);
    let (stage, foreign) = captured
        .lock()
        .unwrap()
        .clone()
        .expect("actual previous-byte read precedes late journal swap");
    assert!(
        root.path().join(PREVIOUS).exists(),
        "fresh journal-name proof must precede old-object deletion after the intervening read"
    );
    same(root.path(), PREVIOUS, &old);
    same(root.path(), JOURNAL, &foreign);
    same(root.path(), "fixture-after-old-read", &stage);
    assert!(matches!(result, Err(TuningNativeError::UnsafeStore(_))));
    busy(&root);
    assert_eq!(writer.publish(OLD), Err(TuningNativeError::Poisoned));
}

#[test]
fn production_store_same_bytes_journal_substitution_stops_old_cleanup() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let mut writer = store(&root);
    writer.publish(OLD).unwrap();
    let old = artifact(root.path(), JOURNAL);
    let path = root.path().to_owned();
    let captured = Arc::new(Mutex::new(None));
    let observe = captured.clone();
    writer.set_publication_hook(move |point| {
        if point == TuningPublishPoint::BeforeBackupCleanup {
            let stage = artifact(&path, JOURNAL);
            fs::rename(path.join(JOURNAL), path.join("fixture-published-stage")).unwrap();
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path.join(JOURNAL))
                .unwrap();
            file.write_all(NEW).unwrap();
            file.sync_all().unwrap();
            drop(file);
            let foreign = artifact(&path, JOURNAL);
            assert!(
                stage.bytes == foreign.bytes && stage.id != foreign.id,
                "same bytes belong to different actual objects"
            );
            *observe.lock().unwrap() = Some((stage, foreign));
        }
        Ok(())
    });
    let result = writer.publish(NEW);
    let (stage, foreign) = captured
        .lock()
        .unwrap()
        .clone()
        .expect("actual verified-name substitution reached");
    assert!(
        result.is_err(),
        "new name must retain the exact stage ID before old cleanup"
    );
    same(root.path(), PREVIOUS, &old);
    same(root.path(), JOURNAL, &foreign);
    same(root.path(), "fixture-published-stage", &stage);
    busy(&root);
    assert_eq!(writer.publish(OLD), Err(TuningNativeError::Poisoned));
}

const CHILD_TEST: &str = "windows::tuning::store::native_migration_tests::migration_child";
const ROOT_ENV: &str = "CPU_AFFINITY_STORE_MIGRATION_ROOT";
const ID_ENV: &str = "CPU_AFFINITY_STORE_MIGRATION_ID";
const ROLE_ENV: &str = "CPU_AFFINITY_STORE_MIGRATION_ROLE";
const READY: &str = "STORE_MIGRATION_BOUNDARY_READY";
const TIMEOUT: Duration = Duration::from_secs(10);
const LINE_LIMIT: usize = 32;
const LINE_BYTES: usize = 512;
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
            .unwrap();
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
                .expect("actual child did not enter required real migration boundary");
            if line.trim() == READY {
                return;
            }
        }
        panic!("bounded migration child line limit");
    }
    fn terminate(&mut self) {
        assert!(
            self.child.try_wait().unwrap().is_none(),
            "actual worker remains held before forced kill"
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
    let condition = Condvar::new();
    let mut guard = mutex.lock().unwrap();
    loop {
        guard = condition.wait(guard).unwrap();
    }
}
fn point_role(point: TuningPublishPoint) -> &'static str {
    match point {
        TuningPublishPoint::AfterStageSync => "stage-synced",
        TuningPublishPoint::AfterOldMove => "old-moved",
        TuningPublishPoint::AfterPublish => "new-moved",
        TuningPublishPoint::BeforeBackupCleanup => "verified",
        TuningPublishPoint::AfterBackupCleanup => "old-deleted",
    }
}

#[test]
fn production_store_forced_child_preserves_all_five_real_artifact_states() {
    for point in POINTS {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut initial = store(&root);
        initial.publish(OLD).unwrap();
        let old = artifact(root.path(), JOURNAL);
        drop(initial);
        let mut child = OwnedChild::spawn(&root, point_role(point));
        child.wait_ready();
        busy(&root);
        expect_names(root.path(), expected_names(point));
        // The old production stage stream is intentionally exclusive while
        // its worker is paused. Inspect full bytes/IDs only after exact kill
        // and wait; live lease exclusion and names were checked above.
        child.terminate();
        let retained: Vec<_> = expected_names(point)
            .iter()
            .filter(|name| **name != LEASE)
            .map(|name| (*name, artifact(root.path(), name)))
            .collect();
        let new_name = if matches!(
            point,
            TuningPublishPoint::AfterStageSync | TuningPublishPoint::AfterOldMove
        ) {
            STAGE
        } else {
            JOURNAL
        };
        let new = artifact(root.path(), new_name);
        assert!(new.bytes == NEW && new.id != old.id);
        if point == TuningPublishPoint::AfterStageSync {
            same(root.path(), JOURNAL, &old);
        } else if point != TuningPublishPoint::AfterBackupCleanup {
            same(root.path(), PREVIOUS, &old);
        }
        if point == TuningPublishPoint::AfterBackupCleanup {
            let mut fresh = store(&root);
            assert!(fresh.read().unwrap().as_deref() == Some(NEW));
        } else {
            blocked_fresh(&root);
        }
        for (name, original) in retained {
            same(root.path(), name, &original);
        }
        expect_names(root.path(), expected_names(point));
    }
}

fn retain_role(role: &str) {
    let root = TuningTemporaryDirectory::new().unwrap();
    let mut child = OwnedChild::spawn(&root, role);
    child.wait_ready();
    expect_names(root.path(), &[LEASE]);
    busy(&root);
    child.terminate();
    let mut fresh = store(&root);
    assert!(matches!(fresh.read(), Ok(None)));
    // These are injected nonterminal fixtures, not observed SDK pending.
}
#[test]
fn production_store_nonterminal_worker_keeps_lease_after_caller_drop() {
    retain_role("nonterminal-store");
}
#[test]
fn production_store_nonterminal_opening_keeps_lease_after_caller_drop() {
    retain_role("nonterminal-opening");
}

fn retain_child_worker(path: PathBuf, opening: bool) -> ! {
    let deadline = Instant::now() + TIMEOUT;
    let (entered, entering) = mpsc::sync_channel(1);
    let (release, releasing) = mpsc::sync_channel(1);
    let (terminal, terminal_result) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let entered = move || {
            entered.send(()).unwrap();
            releasing
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("explicit retained-worker entry release");
        };
        let _unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if opening {
                TuningStore::retain_opening_for_test(&path, MAX_TUNING_JOURNAL_BYTES, entered);
            } else {
                let mut writer =
                    TuningStore::open_test_directory(&path, MAX_TUNING_JOURNAL_BYTES).unwrap();
                assert_eq!(writer.authority(), TuningStoreAuthority::IsolatedTest);
                writer.retain_nonterminal_for_test(entered);
            }
        }));
        // A returned/unwound test worker has already dropped its actual store
        // or partial opening. It must not be hidden by an alive controller.
        terminal.send(()).unwrap();
    });
    entering
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .expect("actual worker acquired its lease before retained entry");
    assert!(matches!(
        terminal_result.recv_timeout(Duration::ZERO),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    drop(worker); // Caller/controller Drop detaches the same actual worker.
    release.send(()).unwrap();
    assert!(
        matches!(
            terminal_result.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "nonterminal worker must not unwind or release authority"
    );
    println!("\n{READY}");
    std::io::stdout().flush().unwrap();
    // Keep monitoring the worker after READY. A late terminal/unwind closes
    // this child instead of masking worker release behind an infinite park.
    let _unexpected_terminal = terminal_result.recv();
    panic!("retained worker unexpectedly reached terminal or disconnected");
}

#[test]
fn migration_child() {
    let Some(role) = std::env::var_os(ROLE_ENV) else {
        return;
    };
    let role = role.to_str().expect("finite migration child role");
    let path = std::env::var_os(ROOT_ENV).expect("parent-owned existing root");
    assert!(path.len() <= 1024, "bounded parent root input");
    let raw_id = std::env::var(ID_ENV).expect("original parent root ID");
    assert!(raw_id.len() <= 32);
    let (volume, index) = raw_id.split_once(':').expect("fixed original ID shape");
    let original = (
        volume.parse::<u32>().unwrap(),
        index.parse::<u64>().unwrap(),
    );
    let path = PathBuf::from(path);
    // Existing-only native setup verifies the parent's actual root incarnation;
    // it creates neither a TempRoot cleanup owner nor public store authority.
    let proof =
        super::super::relative_io::Directory::fixture_observed_root(&path, original).unwrap();
    drop(proof);
    if matches!(role, "nonterminal-store" | "nonterminal-opening") {
        retain_child_worker(path, role == "nonterminal-opening");
    }
    let point = POINTS
        .into_iter()
        .find(|point| point_role(*point) == role)
        .expect("fixed migration boundary");
    let mut writer = TuningStore::open_test_directory(&path, MAX_TUNING_JOURNAL_BYTES).unwrap();
    assert_eq!(writer.authority(), TuningStoreAuthority::IsolatedTest);
    writer.set_publication_hook(move |current| {
        if current == point {
            park_at_boundary();
        }
        Ok(())
    });
    writer.publish(NEW).unwrap();
    panic!("requested real migration boundary was not observed");
}

#[test]
fn production_store_preserves_real_four_mib_publication_boundaries() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let mut writer = store(&root);
    assert!(matches!(writer.read(), Ok(None)));
    let smaller = vec![0x35; MAX_TUNING_JOURNAL_BYTES - 1];
    writer.publish(&smaller).unwrap();
    assert!(writer.read().unwrap().as_deref() == Some(smaller.as_slice()));
    let original = artifact(root.path(), JOURNAL);

    let mut exact = vec![0x36; MAX_TUNING_JOURNAL_BYTES];
    *exact.last_mut().unwrap() = 0x79;
    writer.publish(&exact).unwrap();
    assert!(writer.read().unwrap().as_deref() == Some(exact.as_slice()));
    let published = artifact(root.path(), JOURNAL);
    assert!(
        published.bytes == exact,
        "all four MiB including the final byte are retained"
    );
    assert!(
        published.id != original.id,
        "replacement is the actual staged object"
    );

    assert_eq!(
        writer.publish(&vec![0x37; MAX_TUNING_JOURNAL_BYTES + 1]),
        Err(TuningNativeError::Capacity {
            size: (MAX_TUNING_JOURNAL_BYTES + 1) as u64,
            limit: MAX_TUNING_JOURNAL_BYTES,
        })
    );
    assert!(
        artifact(root.path(), JOURNAL) == published,
        "oversized input preserves the original full bytes and object identity"
    );
    assert!(writer.read().unwrap().as_deref() == Some(exact.as_slice()));
    drop(writer);
    let mut reopened = store(&root);
    assert!(reopened.read().unwrap().as_deref() == Some(exact.as_slice()));
    assert!(artifact(root.path(), JOURNAL) == published);
}

#[test]
fn production_store_preserves_oversized_existing_four_mib_evidence() {
    let root = TuningTemporaryDirectory::new().unwrap();
    let mut writer = store(&root);
    writer.publish(OLD).unwrap();
    let oversized = vec![0x42; MAX_TUNING_JOURNAL_BYTES + 1];
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(root.path().join(JOURNAL))
        .unwrap();
    file.write_all(&oversized).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let retained = artifact(root.path(), JOURNAL);
    assert!(matches!(writer.read(), Err(TuningNativeError::Capacity {
        size, limit: MAX_TUNING_JOURNAL_BYTES,
    }) if size == (MAX_TUNING_JOURNAL_BYTES + 1) as u64));
    assert!(
        matches!(writer.publish(OLD), Err(TuningNativeError::Capacity {
        size, limit: MAX_TUNING_JOURNAL_BYTES,
    }) if size == (MAX_TUNING_JOURNAL_BYTES + 1) as u64)
    );
    assert!(
        artifact(root.path(), JOURNAL) == retained,
        "oversized stored evidence is neither overwritten nor reinitialized"
    );
}
