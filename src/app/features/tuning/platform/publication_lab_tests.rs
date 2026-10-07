//! Explicit ignored canonical publication lab. Default tests use shape/isolated fixtures only.
use super::{
    contract::*,
    coordinator::CoordinatorSnapshot,
    journal::{JournalDocument, JournalLoad, JournalPhase, JournalStamp, MAX_JOURNAL_BYTES},
};
use os_api::tuning::{
    TuningLabArtifactKind as ArtifactKind, TuningLabSnapshot, TuningNativeError,
    TuningPublishPoint, TuningStore, TuningStoreAuthority, TuningTemporaryDirectory,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabCase {
    Success,
    Fault(TuningPublishPoint),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostFailure {
    Failed,
    RollbackUnresolved,
}
#[derive(Clone, Copy)]
enum HostResult {
    VerifiedRolledBack,
    FaultRolledBack,
    Pending,
    Refused,
    Failed,
    TimedOut,
    WorkerFailed,
    RollbackUnresolved,
}
impl HostResult {
    fn label(self) -> &'static str {
        match self {
            Self::VerifiedRolledBack => "verified_rolled_back",
            Self::FaultRolledBack => "fault_rolled_back",
            Self::Pending => "pending",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::WorkerFailed => "worker_failed",
            Self::RollbackUnresolved => "rollback_unresolved",
        }
    }
}
struct HostOutcome {
    result: HostResult,
    verified: bool,
    rollback_verified: bool,
}
fn host_selection(value: Option<&str>, marker: Option<&str>) -> Option<LabCase> {
    if marker != Some("restore-verified-absent-only") {
        return None;
    }
    case_selection(value)
}
fn host_guarded_bytes(
    authority: TuningStoreAuthority,
    opened: bool,
    first: &Result<Option<Vec<u8>>, TuningNativeError>,
    machine: &MachineIdentity,
    document: &JournalDocument,
) -> Result<Vec<u8>, ()> {
    guarded_bytes(authority, opened, first, machine, document)
}
fn host_report_line(result: HostResult, terminal: bool, verified: bool, rollback: bool) -> String {
    let result = result.label();
    format!("\n{{\"schema\":1,\"result\":\"{result}\",\"worker_terminal\":{terminal},\"verified\":{verified},\"rollback_verified\":{rollback}}}\n")
}
fn rollback_protocol(
    store: &mut TuningStore,
    old: &JournalDocument,
    new: &JournalDocument,
    case: LabCase,
    verify_machine: impl FnMut() -> Result<(), ()>,
) -> Result<bool, HostFailure> {
    let proof = protocol(store, old, new, case, verify_machine);
    // Once armed, every controlled terminal outcome attempts the same cap's
    // rollback; no early protocol return bypasses cleanup.
    store
        .lab_rollback()
        .map_err(|_| HostFailure::RollbackUnresolved)?;
    proof.map_err(|_| HostFailure::Failed)
}
#[test]
fn host_publication_selection_is_explicit_finite_and_preserves_vm_selection() {
    for value in [
        "success",
        "fault-stage",
        "fault-old-move",
        "fault-publish",
        "fault-before-cleanup",
        "fault-after-cleanup",
    ] {
        assert!(host_selection(Some(value), Some("restore-verified-absent-only")).is_some());
        assert!(host_selection(Some(value), Some("fresh-disposable-windows-snapshot")).is_none());
    }
    for value in [None, Some(""), Some("success "), Some("arbitrary")] {
        assert!(host_selection(value, Some("restore-verified-absent-only")).is_none());
    }
    assert!(host_selection(Some("success"), None).is_none());
    assert!(selection(Some("success"), Some("restore-verified-absent-only")).is_none());
}
#[test]
fn host_publication_guard_reuses_empty_codec_without_fixture_authority() {
    let machine = shape_machine();
    let document = empty_document(machine.clone(), 1);
    assert!(host_guarded_bytes(
        TuningStoreAuthority::Canonical,
        true,
        &Ok(None),
        &machine,
        &document
    )
    .is_ok());
    let mut calls = 0;
    for (authority, opened, first) in [
        (TuningStoreAuthority::IsolatedTest, true, Ok(None)),
        (TuningStoreAuthority::Canonical, false, Ok(None)),
        (TuningStoreAuthority::Canonical, true, Ok(Some(Vec::new()))),
        (
            TuningStoreAuthority::Canonical,
            true,
            Err(TuningNativeError::Poisoned),
        ),
    ] {
        if host_guarded_bytes(authority, opened, &first, &machine, &document).is_ok() {
            calls += 1;
        }
    }
    assert_eq!(calls, 0);
}
#[test]
fn host_publication_report_is_fresh_line_five_fixed_fields_and_bounded() {
    let line = host_report_line(HostResult::VerifiedRolledBack, true, true, true);
    assert!(line.starts_with('\n'), "libtest prefix cannot swallow JSON");
    let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert_eq!(value["rollback_verified"], true);
    assert_eq!(value["schema"], 1);
    assert!(line.len() < 256);
}
#[test]
fn host_publication_isolated_protocol_rolls_back_all_terminal_paths() {
    let machine = MachineIdentity {
        id: MachineId::new([0x43; 16]).unwrap(),
    };
    let old = empty_document(machine.clone(), 1);
    let new = empty_document(machine, 2);
    for case in [
        LabCase::Success,
        LabCase::Fault(TuningPublishPoint::AfterStageSync),
        LabCase::Fault(TuningPublishPoint::AfterOldMove),
        LabCase::Fault(TuningPublishPoint::AfterPublish),
        LabCase::Fault(TuningPublishPoint::BeforeBackupCleanup),
        LabCase::Fault(TuningPublishPoint::AfterBackupCleanup),
    ] {
        let root = TuningTemporaryDirectory::new().unwrap();
        drop(TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap());
        let mut store = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
        let initial = store.lab_snapshot().unwrap();
        assert!(!initial.opened_canonical(), "fixture stays isolated");
        store.lab_arm_rollback().unwrap();
        let result = rollback_protocol(&mut store, &old, &new, case, || Ok(()));
        let after = store.lab_snapshot().unwrap();
        assert!(
            after.artifact(ArtifactKind::Journal).is_none()
                && after.artifact(ArtifactKind::Stage).is_none()
                && after.artifact(ArtifactKind::Previous).is_none(),
            "terminal lab always attempts exact rollback"
        );
        assert!(
            initial.root_id() == after.root_id() && initial.lease_id() == after.lease_id(),
            "original metadata retained"
        );
        drop(store);
        let mut reopened =
            TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
        assert!(matches!(reopened.read(), Ok(None)));
        drop(reopened);
        root.cleanup().unwrap();
        assert_eq!(result, Ok(case != LabCase::Success));
    }
}
#[test]
fn host_publication_protocol_machine_refusal_after_first_publish_still_rolls_back() {
    let root = TuningTemporaryDirectory::new().unwrap();
    drop(TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap());
    let mut store = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
    store.lab_arm_rollback().unwrap();
    let machine = MachineIdentity {
        id: MachineId::new([0x43; 16]).unwrap(),
    };
    let mut calls = 0;
    let result = rollback_protocol(
        &mut store,
        &empty_document(machine.clone(), 1),
        &empty_document(machine, 2),
        LabCase::Success,
        || {
            calls += 1;
            if calls == 2 {
                Err(())
            } else {
                Ok(())
            }
        },
    );
    assert_eq!(calls, 2);
    assert!(
        store
            .lab_snapshot()
            .unwrap()
            .artifact(ArtifactKind::Journal)
            .is_none(),
        "rollback after controlled terminal protocol refusal"
    );
    assert_eq!(result, Err(HostFailure::Failed));
    drop(store);
    root.cleanup().unwrap();
}
#[test]
fn host_publication_timeout_keeps_armed_cap_and_lease_until_same_worker_rollback() {
    use std::{sync::mpsc, time::Duration};
    let root = TuningTemporaryDirectory::new().unwrap();
    let bootstrap = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
    let initial = bootstrap.lab_snapshot().unwrap();
    drop(bootstrap);
    let path = root.path().to_owned();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let controller = std::thread::spawn(move || {
        let result = wait_worker(
            move || {
                let mut store = TuningStore::open_test_directory(&path, MAX_JOURNAL_BYTES).unwrap();
                store.lab_arm_rollback().unwrap();
                store.publish(b"owned timed-out fixture bytes").unwrap();
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                store.lab_rollback().unwrap();
                let snapshot = store.lab_snapshot().unwrap();
                assert!(snapshot.artifact(ArtifactKind::Journal).is_none());
                drop(store);
                true
            },
            Duration::ZERO,
            || (),
        );
        result_tx.send(result).unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(
        result_rx.recv_timeout(Duration::ZERO),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(matches!(
        TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES),
        Err(TuningNativeError::Busy)
    ));
    release_tx.send(()).unwrap();
    let result = result_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    controller.join().unwrap();
    assert_eq!(
        result,
        Ok((true, true)),
        "late exact rollback keeps timeout sticky"
    );
    let mut reopened = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
    assert!(matches!(reopened.read(), Ok(None)));
    let final_state = reopened.lab_snapshot().unwrap();
    assert!(
        initial.root_id() == final_state.root_id() && initial.lease_id() == final_state.lease_id(),
        "same original metadata after actual worker terminal"
    );
    drop(reopened);
    root.cleanup().unwrap();
}
fn selection(value: Option<&str>, snapshot: Option<&str>) -> Option<LabCase> {
    if snapshot != Some("fresh-disposable-windows-snapshot") {
        return None;
    }
    case_selection(value)
}
fn case_selection(value: Option<&str>) -> Option<LabCase> {
    Some(match value? {
        "success" => LabCase::Success,
        "fault-stage" => LabCase::Fault(TuningPublishPoint::AfterStageSync),
        "fault-old-move" => LabCase::Fault(TuningPublishPoint::AfterOldMove),
        "fault-publish" => LabCase::Fault(TuningPublishPoint::AfterPublish),
        "fault-before-cleanup" => LabCase::Fault(TuningPublishPoint::BeforeBackupCleanup),
        "fault-after-cleanup" => LabCase::Fault(TuningPublishPoint::AfterBackupCleanup),
        _ => return None,
    })
}
fn empty_document(machine: MachineIdentity, revision: u64) -> JournalDocument {
    JournalDocument::capture(
        machine,
        revision,
        JournalStamp::Recovery,
        JournalPhase::Recovery,
        &CoordinatorSnapshot::default(),
    )
    .unwrap()
}
fn guarded_bytes(
    authority: TuningStoreAuthority,
    fresh: bool,
    first: &Result<Option<Vec<u8>>, TuningNativeError>,
    machine: &MachineIdentity,
    document: &JournalDocument,
) -> Result<Vec<u8>, ()> {
    if authority != TuningStoreAuthority::Canonical
        || !fresh
        || !matches!(first, Ok(None))
        || document.machine != *machine
        || !matches!(
            machine.id.source(),
            MachineIdSource::Tpm | MachineIdSource::Uefi
        )
        || !document.sources.is_empty()
        || !document.owners.is_empty()
        || !document.resources.is_empty()
        || document.restoration_effect.is_some()
        || document.stamp != JournalStamp::Recovery
        || document.phase != JournalPhase::Recovery
    {
        return Err(());
    }
    let bytes = document.to_bytes().map_err(|_| ())?;
    if !matches!(
        JournalDocument::load(bytes.clone(), machine),
        JournalLoad::Known(_)
    ) {
        return Err(());
    }
    Ok(bytes)
}
fn shape_machine() -> MachineIdentity {
    // Codec/decision shape only; this is never supplied to a canonical store.
    MachineIdentity {
        id: MachineId::observed(MachineIdSource::Tpm, vec![0x63; 27]).unwrap(),
    }
}
#[test]
fn publication_lab_guard_accepts_only_valid_empty_machine_bound_record_shape() {
    let machine = shape_machine();
    let document = empty_document(machine.clone(), 1);
    let bytes = guarded_bytes(
        TuningStoreAuthority::Canonical,
        true,
        &Ok(None),
        &machine,
        &document,
    )
    .unwrap();
    let JournalLoad::Known(loaded) = JournalDocument::load(bytes, &machine) else {
        panic!("known empty codec record");
    };
    assert!(
        loaded == document,
        "exact machine/empty record codec roundtrip"
    );
}
#[test]
fn publication_lab_guard_refusals_issue_zero_publish_for_unknown_present_or_bad_document() {
    let machine = shape_machine();
    let original = empty_document(machine.clone(), 1);
    let mut calls = 0;
    for first in [
        Ok(Some(Vec::new())),
        Ok(Some(original.to_bytes().unwrap())),
        Ok(Some(b"corrupt retained record".to_vec())),
        Err(TuningNativeError::Poisoned),
        Err(TuningNativeError::Capacity {
            size: (MAX_JOURNAL_BYTES + 1) as u64,
            limit: MAX_JOURNAL_BYTES,
        }),
    ] {
        if guarded_bytes(
            TuningStoreAuthority::Canonical,
            true,
            &first,
            &machine,
            &original,
        )
        .is_ok()
        {
            calls += 1;
        }
    }
    for field in 0..6 {
        let mut document = original.clone();
        match field {
            0 => document.format += 1,
            1 => document.revision = 0,
            2 => document.restoration_effect = Some(Effect::RestartRequired),
            3 => {
                document.machine = MachineIdentity {
                    id: MachineId::observed(MachineIdSource::Uefi, vec![0x63; 27]).unwrap(),
                }
            }
            4 => {
                document.machine = MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                }
            }
            _ => {
                document.phase = JournalPhase::BeforeWrite {
                    step: 0,
                    effect: Effect::Immediate,
                }
            }
        }
        if guarded_bytes(
            TuningStoreAuthority::Canonical,
            true,
            &Ok(None),
            &machine,
            &document,
        )
        .is_ok()
        {
            calls += 1;
        }
    }
    for (authority, fresh) in [
        (TuningStoreAuthority::IsolatedTest, true),
        (TuningStoreAuthority::Canonical, false),
    ] {
        if guarded_bytes(authority, fresh, &Ok(None), &machine, &original).is_ok() {
            calls += 1;
        }
    }
    let mut nonempty = original.clone();
    nonempty.sources.push(super::journal::JournalSource {
        snapshot: super::codec::ModuleEnvelope {
            feature: FeatureId::new("publication_fixture").unwrap(),
            kind: super::codec::EnvelopeKind::Snapshot,
            version: SchemaVersion::new(1).unwrap(),
            target: Some(TargetIdentity::Machine(machine.clone())),
            payload: serde_json::Value::Null,
        },
        config: None,
    });
    assert!(
        nonempty.to_bytes().is_ok(),
        "valid codec shape with retained source"
    );
    if guarded_bytes(
        TuningStoreAuthority::Canonical,
        true,
        &Ok(None),
        &machine,
        &nonempty,
    )
    .is_ok()
    {
        calls += 1;
    }
    assert_eq!(calls, 0);
}
#[test]
fn publication_lab_selection_requires_explicit_disposable_snapshot_and_finite_case() {
    assert_eq!(selection(None, None), None);
    assert_eq!(selection(Some("success"), None), None);
    for invalid in ["", "Success", "success --root override", "fault", "cleanup"] {
        assert_eq!(
            selection(Some(invalid), Some("fresh-disposable-windows-snapshot")),
            None
        );
    }
    assert_eq!(
        selection(Some("success"), Some("fresh-disposable-windows-snapshot")),
        Some(LabCase::Success)
    );
    assert!(matches!(
        selection(
            Some("fault-stage"),
            Some("fresh-disposable-windows-snapshot")
        ),
        Some(LabCase::Fault(_))
    ));
}

fn protocol(
    store: &mut TuningStore,
    old: &JournalDocument,
    new: &JournalDocument,
    case: LabCase,
    mut verify_machine: impl FnMut() -> Result<(), ()>,
) -> Result<bool, ()> {
    let old_bytes = old.to_bytes().map_err(|_| ())?;
    let new_bytes = new.to_bytes().map_err(|_| ())?;
    if old.machine != new.machine
        || old.revision != 1
        || new.revision != 2
        || !matches!(store.read(), Ok(None))
    {
        return Err(());
    }
    verify_machine()?;
    store.validate_authority().map_err(|_| ())?;
    store.publish(&old_bytes).map_err(|_| ())?;
    let original = store.lab_snapshot().map_err(|_| ())?;
    let old_artifact = original.artifact(ArtifactKind::Journal).ok_or(())?;
    if old_artifact.bytes() != old_bytes
        || original.last_synced_stage_id() != Some(old_artifact.id())
    {
        return Err(());
    }
    verify_machine()?;
    store.validate_authority().map_err(|_| ())?;
    if let LabCase::Fault(point) = case {
        store.fail_publication_at(point);
    }
    let result = store.publish(&new_bytes);
    match case {
        LabCase::Success => result.map_err(|_| ())?,
        LabCase::Fault(_) => {
            if !matches!(
                result,
                Err(TuningNativeError::Io {
                    operation: "injected publication failure",
                    code: 0
                })
            ) {
                return Err(());
            }
        }
    }
    let final_state = store.lab_snapshot().map_err(|_| ())?;
    if original.root_id() != final_state.root_id() || original.lease_id() != final_state.lease_id()
    {
        return Err(());
    }
    let (old_kind, new_kind) = match case {
        LabCase::Fault(TuningPublishPoint::AfterStageSync) => {
            (Some(ArtifactKind::Journal), ArtifactKind::Stage)
        }
        LabCase::Fault(TuningPublishPoint::AfterOldMove) => {
            (Some(ArtifactKind::Previous), ArtifactKind::Stage)
        }
        LabCase::Fault(
            TuningPublishPoint::AfterPublish | TuningPublishPoint::BeforeBackupCleanup,
        ) => (Some(ArtifactKind::Previous), ArtifactKind::Journal),
        _ => (None, ArtifactKind::Journal),
    };
    for kind in [
        ArtifactKind::Journal,
        ArtifactKind::Stage,
        ArtifactKind::Previous,
    ] {
        let actual = final_state.artifact(kind);
        if Some(kind) == old_kind {
            let actual = actual.ok_or(())?;
            if actual.id() != old_artifact.id() || actual.bytes() != old_bytes {
                return Err(());
            }
        } else if kind == new_kind {
            let actual = actual.ok_or(())?;
            if actual.id() == old_artifact.id()
                || actual.bytes() != new_bytes
                || final_state.last_synced_stage_id() != Some(actual.id())
            {
                return Err(());
            }
        } else if actual.is_some() {
            return Err(());
        }
    }
    if case == LabCase::Success {
        if store.read().map_err(|_| ())?.as_deref() != Some(new_bytes.as_slice()) {
            return Err(());
        }
        Ok(false)
    } else {
        if !matches!(store.read(), Err(TuningNativeError::Poisoned))
            || !matches!(store.publish(&new_bytes), Err(TuningNativeError::Poisoned))
        {
            return Err(());
        }
        Ok(true)
    }
}
fn reopen_matches(
    opened: Result<TuningStore, TuningNativeError>,
    before: &TuningLabSnapshot,
    new: &JournalDocument,
    case: LabCase,
) -> Result<(), ()> {
    if !matches!(
        case,
        LabCase::Success | LabCase::Fault(TuningPublishPoint::AfterBackupCleanup)
    ) {
        // The actual constructor refuses retained incomplete artifacts before a store can exist.
        return if matches!(
            opened,
            Err(TuningNativeError::UnsafeStore(
                "pending or unknown journal artifact"
            ))
        ) {
            Ok(())
        } else {
            Err(())
        };
    }
    let mut store = opened.map_err(|_| ())?;
    let expected = new.to_bytes().map_err(|_| ())?;
    if store.read().map_err(|_| ())?.as_deref() != Some(expected.as_slice()) {
        return Err(());
    }
    let after = store.lab_snapshot().map_err(|_| ())?;
    if before.root_id() != after.root_id() || before.lease_id() != after.lease_id() {
        return Err(());
    }
    for kind in [
        ArtifactKind::Journal,
        ArtifactKind::Stage,
        ArtifactKind::Previous,
    ] {
        match (before.artifact(kind), after.artifact(kind)) {
            (None, None) => (),
            (Some(a), Some(b)) if a.id() == b.id() && a.bytes() == b.bytes() => (),
            _ => return Err(()),
        }
    }
    Ok(())
}
#[test]
fn publication_lab_isolated_protocol_reuses_empty_codec_and_five_native_faults() {
    let machine = MachineIdentity {
        id: MachineId::new([0x71; 16]).unwrap(),
    };
    let old = empty_document(machine.clone(), 1);
    let new = empty_document(machine, 2);
    for case in [
        LabCase::Success,
        LabCase::Fault(TuningPublishPoint::AfterStageSync),
        LabCase::Fault(TuningPublishPoint::AfterOldMove),
        LabCase::Fault(TuningPublishPoint::AfterPublish),
        LabCase::Fault(TuningPublishPoint::BeforeBackupCleanup),
        LabCase::Fault(TuningPublishPoint::AfterBackupCleanup),
    ] {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut store = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
        assert_eq!(store.authority(), TuningStoreAuthority::IsolatedTest);
        assert!(store.read().unwrap().is_none());
        let accepted = guarded_bytes(
            store.authority(),
            store.lab_snapshot().unwrap().freshly_created_canonical(),
            &Ok(None),
            &old.machine,
            &old,
        );
        assert!(
            accepted.is_err(),
            "fixture cannot issue canonical publication admission"
        );
        // Fixture closure supplies no machine authority. Canonical runs re-query actual native identity.
        let result = protocol(&mut store, &old, &new, case, || Ok(()));
        let before = store.lab_snapshot().unwrap();
        assert!(matches!(
            TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES),
            Err(TuningNativeError::Busy)
        ));
        drop(store);
        let reopened_result = reopen_matches(
            TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES),
            &before,
            &new,
            case,
        );
        root.cleanup().unwrap();
        assert_eq!(result, Ok(case != LabCase::Success));
        assert_eq!(reopened_result, Ok(()));
    }
}
fn wait_worker<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    timeout: std::time::Duration,
    on_timeout: impl FnOnce(),
) -> Result<(T, bool), ()> {
    use std::sync::mpsc;
    let (send, receive) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let value = work();
        let _ = send.send(value);
    });
    let observed = match receive.recv_timeout(timeout) {
        Ok(value) => Ok((value, false)),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            on_timeout();
            // Retain this exact worker and its frames until real terminal completion.
            receive.recv().map(|value| (value, true)).map_err(|_| ())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
    };
    worker.join().map_err(|_| ())?;
    observed
}
#[test]
fn publication_lab_timeout_retains_same_actual_fixture_worker_until_terminal() {
    use std::{sync::mpsc, time::Duration};
    let root = TuningTemporaryDirectory::new().unwrap();
    let path = root.path().to_owned();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let controller = std::thread::spawn(move || {
        let result = wait_worker(
            move || {
                let mut store = TuningStore::open_test_directory(&path, MAX_JOURNAL_BYTES).unwrap();
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                store.validate_authority().unwrap();
                drop(store);
                true
            },
            Duration::ZERO,
            || (),
        );
        result_tx.send(result).unwrap();
    });
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("actual worker entered");
    assert!(matches!(
        result_rx.recv_timeout(Duration::ZERO),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(matches!(
        TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES),
        Err(TuningNativeError::Busy)
    ));
    release_tx.send(()).unwrap();
    let result = result_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    controller.join().unwrap();
    let reopened = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
    drop(reopened);
    root.cleanup().unwrap();
    assert_eq!(
        result,
        Ok((true, true)),
        "late terminal retains sticky timeout"
    );
}
fn canonical_run(case: LabCase) -> Result<bool, ()> {
    let mut store = TuningStore::open_canonical(MAX_JOURNAL_BYTES).map_err(|_| ())?;
    let first = store.read();
    if !matches!(first, Ok(None)) {
        return Err(());
    }
    let initial = store.lab_snapshot().map_err(|_| ())?;
    if !initial.freshly_created_canonical() {
        return Err(());
    }
    let native = os_api::OS::tuning_machine_identity().map_err(|_| ())?;
    let machine = super::writer::native_machine(native.source(), native.bytes()).map_err(|_| ())?;
    let old = empty_document(machine.clone(), 1);
    let new = empty_document(machine.clone(), 2);
    guarded_bytes(
        store.authority(),
        initial.freshly_created_canonical(),
        &first,
        &machine,
        &old,
    )?;
    guarded_bytes(
        store.authority(),
        initial.freshly_created_canonical(),
        &first,
        &machine,
        &new,
    )?;
    let outcome = protocol(&mut store, &old, &new, case, || {
        let current = os_api::OS::tuning_machine_identity().map_err(|_| ())?;
        if current == native {
            Ok(())
        } else {
            Err(())
        }
    })?;
    let final_state = store.lab_snapshot().map_err(|_| ())?;
    if !matches!(
        TuningStore::open_canonical(MAX_JOURNAL_BYTES),
        Err(TuningNativeError::Busy)
    ) {
        return Err(());
    }
    drop(store);
    reopen_matches(
        TuningStore::open_canonical(MAX_JOURNAL_BYTES),
        &final_state,
        &new,
        case,
    )?;
    Ok(outcome)
}
fn report(result: &str, terminal: bool, verified: bool) -> Result<(), ()> {
    use std::io::Write;
    // Every argument comes from fixed call-site literals/booleans; no identities or error text.
    writeln!(std::io::stdout().lock(),"{{\"schema\":1,\"result\":\"{result}\",\"worker_terminal\":{terminal},\"verified\":{verified}}}").map_err(|_| ())
}
fn host_report(
    result: HostResult,
    terminal: bool,
    verified: bool,
    rollback: bool,
) -> Result<(), ()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    out.write_all(host_report_line(result, terminal, verified, rollback).as_bytes())
        .map_err(|_| ())?;
    out.flush().map_err(|_| ())
}
fn host_run(case: LabCase) -> Result<HostOutcome, ()> {
    // This route never invokes a metadata bootstrap, including its final reopen.
    let mut store = TuningStore::open_canonical_existing(MAX_JOURNAL_BYTES).map_err(|_| ())?;
    let first = store.read();
    let initial = store.lab_snapshot().map_err(|_| ())?;
    let native = os_api::OS::tuning_machine_identity().map_err(|_| ())?;
    let machine = super::writer::native_machine(native.source(), native.bytes()).map_err(|_| ())?;
    let old = empty_document(machine.clone(), 1);
    let new = empty_document(machine.clone(), 2);
    for document in [&old, &new] {
        host_guarded_bytes(
            store.authority(),
            initial.opened_canonical(),
            &first,
            &machine,
            document,
        )?;
    }
    store.lab_arm_rollback().map_err(|_| ())?;
    // No fallible early return after arm. Unknown native completion retains this
    // very worker/store/capability; it cannot be reported as terminal rollback.
    let proof = rollback_protocol(&mut store, &old, &new, case, || {
        let current = os_api::OS::tuning_machine_identity().map_err(|_| ())?;
        if current == native {
            Ok(())
        } else {
            Err(())
        }
    });
    let rollback_proven = !matches!(proof, Err(HostFailure::RollbackUnresolved));
    let held_proof = rollback_proven
        && store.lab_snapshot().is_ok_and(|snapshot| {
            snapshot.opened_canonical()
                && snapshot.root_id() == initial.root_id()
                && snapshot.lease_id() == initial.lease_id()
                && [
                    ArtifactKind::Journal,
                    ArtifactKind::Stage,
                    ArtifactKind::Previous,
                ]
                .into_iter()
                .all(|kind| snapshot.artifact(kind).is_none())
        });
    drop(store);
    if !held_proof {
        return Ok(HostOutcome {
            result: HostResult::RollbackUnresolved,
            verified: false,
            rollback_verified: false,
        });
    }
    let reopened_proof = (|| {
        let mut reopened =
            TuningStore::open_canonical_existing(MAX_JOURNAL_BYTES).map_err(|_| ())?;
        if !matches!(reopened.read(), Ok(None)) {
            return Err(());
        }
        let snapshot = reopened.lab_snapshot().map_err(|_| ())?;
        if !snapshot.opened_canonical()
            || snapshot.root_id() != initial.root_id()
            || snapshot.lease_id() != initial.lease_id()
        {
            return Err(());
        }
        Ok(())
    })();
    if reopened_proof.is_err() {
        return Ok(HostOutcome {
            result: HostResult::RollbackUnresolved,
            verified: false,
            rollback_verified: false,
        });
    }
    Ok(match proof {
        Ok(fault) => HostOutcome {
            result: if fault {
                HostResult::FaultRolledBack
            } else {
                HostResult::VerifiedRolledBack
            },
            verified: true,
            rollback_verified: true,
        },
        Err(_) => HostOutcome {
            result: HostResult::Failed,
            verified: false,
            rollback_verified: true,
        },
    })
}
#[test]
#[ignore = "explicit host lab only; existing empty canonical metadata and verified exact rollback required"]
fn host_canonical_publication_lab() -> Result<(), &'static str> {
    let case = std::env::var("CPU_AFFINITY_PUBLICATION_HOST_LAB_CASE").ok();
    let marker = std::env::var("CPU_AFFINITY_PUBLICATION_HOST_LAB").ok();
    let Some(case) = host_selection(case.as_deref(), marker.as_deref()) else {
        let _ = host_report(HostResult::Refused, true, false, false);
        return Err("explicit host rollback selection required");
    };
    match wait_worker(
        move || host_run(case),
        std::time::Duration::from_secs(10),
        || {
            let _ = host_report(HostResult::Pending, false, false, false);
        },
    ) {
        Ok((Ok(outcome), false)) => {
            host_report(
                outcome.result,
                true,
                outcome.verified,
                outcome.rollback_verified,
            )
            .map_err(|_| "host report unavailable")?;
            if outcome.verified && outcome.rollback_verified {
                Ok(())
            } else {
                Err("host proof failed")
            }
        }
        Ok((outcome, true)) => {
            let rolled_back = outcome.is_ok_and(|value| value.rollback_verified);
            let _ = host_report(HostResult::TimedOut, true, false, rolled_back);
            Err("host worker exceeded deadline")
        }
        Ok((Err(()), false)) => {
            let _ = host_report(HostResult::Refused, true, false, false);
            Err("host proof refused")
        }
        Err(()) => {
            let _ = host_report(HostResult::WorkerFailed, true, false, false);
            Err("host worker failed")
        }
    }
}
#[test]
#[ignore = "explicitly selected fresh disposable Windows VM snapshot only; writes canonical lab metadata"]
fn canonical_publication_lab() -> Result<(), &'static str> {
    let case = std::env::var("CPU_AFFINITY_PUBLICATION_LAB_CASE").ok();
    let snapshot = std::env::var("CPU_AFFINITY_PUBLICATION_LAB_SNAPSHOT").ok();
    let Some(case) = selection(case.as_deref(), snapshot.as_deref()) else {
        let _ = report("selection_refused", true, false);
        return Err("explicit disposable snapshot selection required");
    };
    match wait_worker(
        move || canonical_run(case),
        std::time::Duration::from_secs(10),
        || {
            let _ = report("pending", false, false);
        },
    ) {
        Ok((Ok(fault), false)) => report(
            if fault { "fault_retained" } else { "verified" },
            true,
            true,
        )
        .map_err(|_| "lab report unavailable"),
        Ok((_, true)) => {
            let _ = report("timed_out", true, false);
            Err("lab worker exceeded deadline")
        }
        Ok((Err(()), false)) => {
            let _ = report("refused", true, false);
            Err("lab proof refused")
        }
        Err(()) => {
            let _ = report("worker_failed", true, false);
            Err("lab worker failed")
        }
    }
}
