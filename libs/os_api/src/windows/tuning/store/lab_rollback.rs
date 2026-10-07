//! One invocation's nonshipping rollback. No caller-created deletion receipts.
use super::super::store_io::native::ObjectId;
use super::*;
struct Receipt {
    node: Node,
    name: &'static str,
    bytes: Vec<u8>,
}
pub(super) struct LabRollback {
    root: ObjectId,
    lease: ObjectId,
    receipts: Vec<Receipt>,
    publications: usize,
    spent: bool,
    #[cfg(test)]
    fail_before_write: bool,
    #[cfg(test)]
    fail_cleanup: bool,
}
fn refusal() -> TuningNativeError {
    TuningNativeError::UnsafeStore("lab rollback ownership unproven")
}
impl TuningStore {
    /// Arm only on opened metadata and an empty actual namespace.
    pub fn lab_arm_rollback(&mut self) -> Result<(), TuningNativeError> {
        if self.lab_rollback.is_some() || self.poisoned {
            return Err(refusal());
        }
        self.validate_objects()?;
        let managed: Vec<_> = self
            .directories
            .iter()
            .filter(|entry| entry.scope == Some(true))
            .collect();
        let expected = if self.authority() == TuningStoreAuthority::Canonical {
            2
        } else {
            1
        };
        if managed.len() != expected
            || managed
                .iter()
                .any(|entry| entry.creation != Some(Disposition::Opened))
            || self.lease_creation != Disposition::Opened
            || self.clean_journal(true)?.is_some()
        {
            return Err(refusal());
        }
        self.lab_rollback = Some(LabRollback {
            root: self.root().id(),
            lease: self.lease.id(),
            // Reserve the complete fixed budget before any native CreateNew.
            receipts: Vec::with_capacity(2),
            publications: 0,
            spent: false,
            #[cfg(test)]
            fail_before_write: false,
            #[cfg(test)]
            fail_cleanup: false,
        });
        Ok(())
    }
    /// Consume the invocation's retained-object cleanup capability exactly once.
    pub fn lab_rollback(&mut self) -> Result<(), TuningNativeError> {
        let Some(cap) = self.lab_rollback.as_mut() else {
            return Err(refusal());
        };
        if cap.spent {
            return Err(refusal());
        }
        cap.spent = true;
        let result = self.lab_rollback_inner();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn lab_rollback_inner(&mut self) -> Result<(), TuningNativeError> {
        // All objects are authenticated before the first destructive call.
        self.lab_validate_owned_set()?;
        #[cfg(test)]
        if self
            .lab_rollback
            .as_ref()
            .is_some_and(|cap| cap.fail_cleanup)
        {
            return Err(TuningNativeError::Io {
                operation: "injected lab cleanup failure",
                code: 0,
            });
        }
        while !self.lab_rollback.as_ref().unwrap().receipts.is_empty() {
            self.lab_validate_owned_set()?;
            let node = self
                .lab_rollback
                .as_ref()
                .unwrap()
                .receipts
                .last()
                .unwrap()
                .node
                .clone_for_lab()?;
            // Consume the retained native object, never a path-selected target.
            // A terminal cleanup failure keeps the original handle in the cap.
            node.delete_owned()?;
            self.lab_rollback.as_mut().unwrap().receipts.pop();
        }
        self.lab_validate_owned_set()?;
        if self.clean_journal(true)?.is_some() {
            return Err(refusal());
        }
        Ok(())
    }
    fn lab_validate_owned_set(&self) -> Result<(), TuningNativeError> {
        let cap = self.lab_rollback.as_ref().ok_or_else(refusal)?;
        self.validate_objects()?;
        if self.root().id() != cap.root || self.lease.id() != cap.lease {
            return Err(refusal());
        }
        let entries = self.root().entries()?;
        if entries.len() != cap.receipts.len() + 1 {
            return Err(refusal());
        }
        let mut lease_seen = false;
        for entry in &entries {
            if entry.name == units(LEASE_NAME) {
                if lease_seen || !self.root().entry_matches(entry, &self.lease)? {
                    return Err(refusal());
                }
                lease_seen = true;
            } else {
                let matching: Vec<_> = cap
                    .receipts
                    .iter()
                    .filter(|receipt| entry.name == units(receipt.name))
                    .collect();
                if matching.len() != 1 || !self.root().entry_matches(entry, &matching[0].node)? {
                    return Err(refusal());
                }
            }
        }
        if !lease_seen {
            return Err(refusal());
        }
        for receipt in &cap.receipts {
            self.check_name(receipt.name, &receipt.node, &receipt.bytes)?;
        }
        self.validate_objects()
    }
    pub(super) fn lab_admit_publication(&self) -> Result<(), TuningNativeError> {
        if self
            .lab_rollback
            .as_ref()
            .is_some_and(|cap| cap.spent || cap.publications >= 2)
        {
            return Err(refusal());
        }
        Ok(())
    }
    pub(super) fn lab_register_stage(&mut self, stage: Node) -> Result<Node, TuningNativeError> {
        let Some(cap) = self.lab_rollback.as_mut() else {
            return Ok(stage);
        };
        if cap.spent || cap.publications >= 2 {
            return Err(refusal());
        }
        // Ownership is registered before duplicate-handle allocation, writes or
        // sync. A terminal duplication failure leaves the original in the cap.
        cap.receipts.push(Receipt {
            node: stage,
            name: STAGING_NAME,
            bytes: Vec::new(),
        });
        cap.publications += 1;
        #[cfg(test)]
        if cap.fail_before_write {
            return Err(TuningNativeError::Io {
                operation: "injected pre-write lab failure",
                code: 0,
            });
        }
        cap.receipts.last().unwrap().node.clone_for_lab()
    }
    pub(super) fn lab_seal_stage(
        &mut self,
        id: ObjectId,
        bytes: &[u8],
    ) -> Result<(), TuningNativeError> {
        if let Some(cap) = self.lab_rollback.as_mut() {
            let receipt = cap
                .receipts
                .iter_mut()
                .find(|receipt| receipt.node.id() == id && receipt.name == STAGING_NAME)
                .ok_or_else(refusal)?;
            receipt.node.verify()?;
            receipt.bytes = bytes.to_vec();
        }
        Ok(())
    }
    pub(super) fn lab_moved(
        &mut self,
        id: ObjectId,
        from: &'static str,
        to: &'static str,
    ) -> Result<(), TuningNativeError> {
        if let Some(cap) = self.lab_rollback.as_mut() {
            let receipt = cap
                .receipts
                .iter_mut()
                .find(|receipt| receipt.node.id() == id && receipt.name == from)
                .ok_or_else(refusal)?;
            receipt.name = to;
        }
        Ok(())
    }
    pub(super) fn lab_deleted(&mut self, id: ObjectId) -> Result<(), TuningNativeError> {
        if let Some(cap) = self.lab_rollback.as_mut() {
            let index = cap
                .receipts
                .iter()
                .position(|receipt| receipt.node.id() == id && receipt.name == BACKUP_NAME)
                .ok_or_else(refusal)?;
            // Closing the cap's duplicate completes exact old delete-pending
            // before the publisher's final namespace proof.
            cap.receipts.remove(index);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::super::super::test_support::TuningTemporaryDirectory;
    use super::*;
    use std::fs;
    fn opened(root: &TuningTemporaryDirectory) -> TuningStore {
        drop(TuningStore::open_test_directory(root.path(), 4096).unwrap());
        TuningStore::open_test_directory(root.path(), 4096).unwrap()
    }
    #[test]
    fn rollback_success_and_five_faults_preserve_original_metadata_and_absence() {
        for point in [
            None,
            Some(TuningPublishPoint::AfterStageSync),
            Some(TuningPublishPoint::AfterOldMove),
            Some(TuningPublishPoint::AfterPublish),
            Some(TuningPublishPoint::BeforeBackupCleanup),
            Some(TuningPublishPoint::AfterBackupCleanup),
        ] {
            let root = TuningTemporaryDirectory::new().unwrap();
            let mut store = opened(&root);
            let original = store.lab_snapshot().unwrap();
            store.lab_arm_rollback().unwrap();
            store.publish(b"first bounded lab record").unwrap();
            if let Some(point) = point {
                store.fail_publication_at(point);
            }
            assert_eq!(
                store.publish(b"second bounded lab record").is_err(),
                point.is_some()
            );
            store.lab_rollback().expect("exact owned rollback");
            let final_state = store.lab_snapshot().unwrap();
            assert!(
                original.root_id() == final_state.root_id()
                    && original.lease_id() == final_state.lease_id(),
                "original metadata retained"
            );
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
            assert!(store.lab_rollback().is_err(), "single use");
            if point.is_some() {
                assert!(matches!(store.read(), Err(TuningNativeError::Poisoned)));
            }
            drop(store);
            let mut reopened = TuningStore::open_test_directory(root.path(), 4096).unwrap();
            assert!(matches!(reopened.read(), Ok(None)));
            let snapshot = reopened.lab_snapshot().unwrap();
            assert!(
                original.root_id() == snapshot.root_id()
                    && original.lease_id() == snapshot.lease_id(),
                "ordinary reopened original metadata"
            );
            drop(reopened);
            root.cleanup().unwrap();
        }
    }
    #[test]
    fn rollback_arm_refuses_created_lease_present_and_unknown_without_mutation() {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut first = TuningStore::open_test_directory(root.path(), 4096).unwrap();
        let created_refusal = first.lab_arm_rollback().is_err();
        drop(first);
        let mut store = opened(&root);
        store.publish(b"preexisting record").unwrap();
        let baseline = store.lab_snapshot().unwrap();
        let present_refusal = store.lab_arm_rollback().is_err();
        let retained = store.lab_snapshot().unwrap();
        assert!(
            baseline
                .artifact(TuningLabArtifactKind::Journal)
                .unwrap()
                .id()
                == retained
                    .artifact(TuningLabArtifactKind::Journal)
                    .unwrap()
                    .id(),
            "present object retained"
        );
        assert_eq!(
            retained
                .artifact(TuningLabArtifactKind::Journal)
                .unwrap()
                .bytes(),
            b"preexisting record"
        );
        drop(store);
        assert!(
            created_refusal && present_refusal,
            "no arm from created metadata or preexisting journal"
        );
        root.cleanup().unwrap();
    }
    #[test]
    fn rollback_full_set_rejects_unknown_entry_before_deleting_owned_journal() {
        let root = TuningTemporaryDirectory::new().unwrap();
        let mut store = opened(&root);
        store.lab_arm_rollback().unwrap();
        store.publish(b"owned record").unwrap();
        let before = fs::read(root.path().join(JOURNAL_NAME)).unwrap();
        fs::write(root.path().join("fixture-foreign"), b"foreign sentinel").unwrap();
        assert!(store.lab_rollback().is_err());
        assert_eq!(fs::read(root.path().join(JOURNAL_NAME)).unwrap(), before);
        assert_eq!(
            fs::read(root.path().join("fixture-foreign")).unwrap(),
            b"foreign sentinel"
        );
        assert!(
            matches!(store.read(), Err(TuningNativeError::Poisoned)),
            "refusal remains poisoned"
        );
        drop(store);
        fs::remove_file(root.path().join("fixture-foreign")).unwrap();
        root.cleanup().unwrap();
    }
    #[test]
    fn existing_only_fixture_selection_never_creates_missing_lease() {
        let root = TuningTemporaryDirectory::new().unwrap();
        let refused = TuningStore::open_test_existing(root.path(), 4096).is_err();
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            0,
            "no bootstrap side effect"
        );
        assert!(refused);
        let (base, policy) = test_selection(root.path(), 4096).unwrap();
        let child = root.path().join("fixture-missing-managed");
        assert!(TuningStore::open_selected(child.clone(), base, policy, 4096, true).is_err());
        assert!(!child.exists());
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            0,
            "missing managed directory remains absent"
        );
        root.cleanup().unwrap();
    }
    #[test]
    fn rollback_identity_and_full_bytes_mismatch_never_delete_any_artifact() {
        for replace in [false, true] {
            let root = TuningTemporaryDirectory::new().unwrap();
            let outside = TuningTemporaryDirectory::new().unwrap();
            let mut store = opened(&root);
            store.lab_arm_rollback().unwrap();
            store.publish(b"owned exact bytes").unwrap();
            store.fail_publication_at(TuningPublishPoint::AfterOldMove);
            assert!(store.publish(b"second owned bytes").is_err());
            let original = store.lab_snapshot().unwrap();
            let before = original
                .artifact(TuningLabArtifactKind::Previous)
                .unwrap()
                .id();
            let stage = original
                .artifact(TuningLabArtifactKind::Stage)
                .unwrap()
                .id();
            if replace {
                fs::rename(
                    root.path().join(BACKUP_NAME),
                    outside.path().join("fixture-held-original"),
                )
                .unwrap();
                let (mut foreign, created) = store
                    .root()
                    .open(
                        &units(BACKUP_NAME),
                        Kind::File,
                        OpenMode::CreateNew,
                        Sharing::Renameable,
                        false,
                        Some(true),
                    )
                    .unwrap();
                assert_eq!(created, Disposition::Created);
                foreign.write_synced(b"owned exact bytes", 4096).unwrap();
                drop(foreign);
            } else {
                fs::write(root.path().join(BACKUP_NAME), b"mutated exact bytes").unwrap();
            }
            let evidence = store.lab_snapshot().unwrap();
            let foreign = evidence.artifact(TuningLabArtifactKind::Previous).unwrap();
            assert_eq!(
                foreign.id() == before,
                !replace,
                "actual native replacement identity"
            );
            let result = store.lab_rollback();
            let retained = store.lab_snapshot().unwrap();
            let artifact = retained.artifact(TuningLabArtifactKind::Previous).unwrap();
            assert!(
                artifact.id() == foreign.id(),
                "mismatched object never deleted"
            );
            assert_eq!(artifact.bytes(), foreign.bytes());
            assert!(
                retained
                    .artifact(TuningLabArtifactKind::Stage)
                    .unwrap()
                    .id()
                    == stage,
                "whole-set proof precedes even the valid stage deletion"
            );
            assert_eq!(
                retained
                    .artifact(TuningLabArtifactKind::Stage)
                    .unwrap()
                    .bytes(),
                b"second owned bytes"
            );
            if replace {
                assert_eq!(
                    fs::read(outside.path().join("fixture-held-original")).unwrap(),
                    b"owned exact bytes"
                );
            }
            assert!(result.is_err());
            assert!(matches!(store.read(), Err(TuningNativeError::Poisoned)));
            drop(store);
            root.cleanup().unwrap();
            outside.cleanup().unwrap();
        }
    }
    #[test]
    fn rollback_owns_created_stage_before_first_write_and_retains_failed_cleanup() {
        for cleanup_failure in [false, true] {
            let root = TuningTemporaryDirectory::new().unwrap();
            let mut store = opened(&root);
            store.lab_arm_rollback().unwrap();
            store.lab_rollback.as_mut().unwrap().fail_before_write = true;
            assert!(matches!(
                store.publish(b"unwritten payload"),
                Err(TuningNativeError::Io {
                    operation: "injected pre-write lab failure",
                    code: 0
                })
            ));
            let before = store.lab_snapshot().unwrap();
            assert!(
                before
                    .artifact(TuningLabArtifactKind::Stage)
                    .unwrap()
                    .bytes()
                    .is_empty()
            );
            store.lab_rollback.as_mut().unwrap().fail_cleanup = cleanup_failure;
            let result = store.lab_rollback();
            let after = store.lab_snapshot().unwrap();
            if cleanup_failure {
                assert!(matches!(
                    result,
                    Err(TuningNativeError::Io {
                        operation: "injected lab cleanup failure",
                        code: 0
                    })
                ));
                assert!(
                    before.artifact(TuningLabArtifactKind::Stage).unwrap().id()
                        == after.artifact(TuningLabArtifactKind::Stage).unwrap().id(),
                    "failed cleanup keeps created handle/object"
                );
                assert!(store.lab_rollback().is_err(), "failed cap cannot retry");
            } else {
                result.unwrap();
                assert!(after.artifact(TuningLabArtifactKind::Stage).is_none());
            }
            assert!(matches!(store.read(), Err(TuningNativeError::Poisoned)));
            drop(store);
            root.cleanup().unwrap();
        }
    }
    #[test]
    fn rollback_refuses_unregistered_created_stage_and_third_publication() {
        for foreign in [false, true] {
            let root = TuningTemporaryDirectory::new().unwrap();
            let mut store = opened(&root);
            store.lab_arm_rollback().unwrap();
            if foreign {
                fs::write(
                    root.path().join(STAGING_NAME),
                    b"unregistered native object",
                )
                .unwrap();
                assert!(store.lab_rollback().is_err());
                assert_eq!(
                    fs::read(root.path().join(STAGING_NAME)).unwrap(),
                    b"unregistered native object"
                );
            } else {
                store.publish(b"first").unwrap();
                store.publish(b"second").unwrap();
                let before = store.lab_snapshot().unwrap();
                assert!(store.publish(b"third forbidden").is_err());
                let after = store.lab_snapshot().unwrap();
                assert!(
                    before
                        .artifact(TuningLabArtifactKind::Journal)
                        .unwrap()
                        .id()
                        == after.artifact(TuningLabArtifactKind::Journal).unwrap().id(),
                    "bounded publication admission precedes native create"
                );
                assert_eq!(
                    after
                        .artifact(TuningLabArtifactKind::Journal)
                        .unwrap()
                        .bytes(),
                    b"second"
                );
                store.lab_rollback().unwrap();
                assert!(store.publish(b"after consumed rollback").is_err());
            }
            drop(store);
            root.cleanup().unwrap();
        }
    }
}
