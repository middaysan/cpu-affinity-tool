//! Readonly lab evidence from the actual retained store. No authority constructors.
use super::super::store_io::native::ObjectId;
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningLabCreation {
    Created,
    Opened,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningLabArtifactKind {
    Journal,
    Stage,
    Previous,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TuningLabObjectId(ObjectId);
impl std::fmt::Debug for TuningLabObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningLabObjectId(redacted)")
    }
}
pub struct TuningLabArtifact {
    id: TuningLabObjectId,
    bytes: Vec<u8>,
}
impl TuningLabArtifact {
    pub fn id(&self) -> TuningLabObjectId {
        self.id
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl std::fmt::Debug for TuningLabArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningLabArtifact(redacted)")
    }
}
pub struct TuningLabSnapshot {
    authority: TuningStoreAuthority,
    managed: Vec<TuningLabCreation>,
    lease: TuningLabCreation,
    root_id: TuningLabObjectId,
    lease_id: TuningLabObjectId,
    artifacts: Vec<(TuningLabArtifactKind, TuningLabArtifact)>,
    last_stage: Option<TuningLabObjectId>,
}
impl TuningLabSnapshot {
    pub fn authority(&self) -> TuningStoreAuthority {
        self.authority
    }
    pub fn managed_creation(&self) -> &[TuningLabCreation] {
        &self.managed
    }
    pub fn lease_creation(&self) -> TuningLabCreation {
        self.lease
    }
    pub fn root_id(&self) -> TuningLabObjectId {
        self.root_id
    }
    pub fn lease_id(&self) -> TuningLabObjectId {
        self.lease_id
    }
    pub fn last_synced_stage_id(&self) -> Option<TuningLabObjectId> {
        self.last_stage
    }
    pub fn freshly_created_canonical(&self) -> bool {
        self.authority == TuningStoreAuthority::Canonical
            && self.managed == [TuningLabCreation::Created; 2]
            && self.lease == TuningLabCreation::Created
    }
    pub fn artifact(&self, kind: TuningLabArtifactKind) -> Option<&TuningLabArtifact> {
        self.artifacts
            .iter()
            .find(|(key, _)| *key == kind)
            .map(|(_, value)| value)
    }
}
impl std::fmt::Debug for TuningLabSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningLabSnapshot(redacted)")
    }
}
impl TuningStore {
    /// Readonly fixed-name evidence, also available after a poisoned publication.
    /// This never clears poison, repairs artifacts or supplies write admission.
    pub fn lab_snapshot(&self) -> Result<TuningLabSnapshot, TuningNativeError> {
        self.validate_objects()?;
        let fixed = [
            (TuningLabArtifactKind::Journal, JOURNAL_NAME),
            (TuningLabArtifactKind::Stage, STAGING_NAME),
            (TuningLabArtifactKind::Previous, BACKUP_NAME),
        ];
        let entries = self.root().entries()?;
        for entry in &entries {
            if entry.name != units(LEASE_NAME)
                && !fixed.iter().any(|(_, name)| entry.name == units(name))
            {
                return Err(artifact_error());
            }
        }
        let mut retained = Vec::new();
        let mut artifacts = Vec::new();
        for (kind, name) in fixed {
            let found: Vec<_> = entries
                .iter()
                .filter(|entry| entry.name == units(name))
                .collect();
            if found.len() > 1 {
                return Err(artifact_error());
            }
            if let Some(entry) = found.first() {
                let (mut node, _) = self.root().open(
                    &units(name),
                    Kind::File,
                    OpenMode::Existing,
                    Sharing::Renameable,
                    true,
                    Some(true),
                )?;
                if !self.root().entry_matches(entry, &node)? {
                    return Err(artifact_error());
                }
                let bytes = node.read(self.max_bytes)?;
                self.check_name(name, &node, &bytes)?;
                artifacts.push((
                    kind,
                    TuningLabArtifact {
                        id: TuningLabObjectId(node.id()),
                        bytes,
                    },
                ));
                retained.push((name, node));
            }
        }
        let fresh = self.root().entries()?;
        if fresh.len() != entries.len() {
            return Err(artifact_error());
        }
        for entry in &fresh {
            if entry.name == units(LEASE_NAME) {
                if !self.root().entry_matches(entry, &self.lease)? {
                    return Err(artifact_error());
                }
            } else {
                let Some((_, node)) = retained.iter().find(|(name, _)| entry.name == units(name))
                else {
                    return Err(artifact_error());
                };
                if !self.root().entry_matches(entry, node)? {
                    return Err(artifact_error());
                }
            }
        }
        for ((name, node), (_, artifact)) in retained.iter().zip(&artifacts) {
            self.check_name(name, node, &artifact.bytes)?;
        }
        self.validate_objects()?;
        fn creation(value: Disposition) -> TuningLabCreation {
            match value {
                Disposition::Created => TuningLabCreation::Created,
                Disposition::Opened => TuningLabCreation::Opened,
            }
        }
        let managed = self
            .directories
            .iter()
            .filter(|entry| entry.scope == Some(true))
            .map(|entry| entry.creation.map(creation).ok_or(artifact_error()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TuningLabSnapshot {
            authority: self.authority(),
            managed,
            lease: creation(self.lease_creation),
            root_id: TuningLabObjectId(self.root().id()),
            lease_id: TuningLabObjectId(self.lease.id()),
            artifacts,
            last_stage: self.last_synced_stage.map(TuningLabObjectId),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::super::super::test_support::TuningTemporaryDirectory;
    use super::*;
    use crate::tuning::MAX_TUNING_JOURNAL_BYTES;
    #[test]
    fn lab_evidence_records_actual_created_opened_lease_without_promoting_fixture() {
        let root = TuningTemporaryDirectory::new().unwrap();
        let store =
            TuningStore::open_test_directory(root.path(), MAX_TUNING_JOURNAL_BYTES).unwrap();
        let first = store.lab_snapshot().unwrap();
        assert_eq!(first.authority(), TuningStoreAuthority::IsolatedTest);
        assert_eq!(first.managed_creation(), [TuningLabCreation::Opened]);
        assert_eq!(first.lease_creation(), TuningLabCreation::Created);
        assert!(!first.freshly_created_canonical());
        assert!(first.artifact(TuningLabArtifactKind::Journal).is_none());
        drop(store);
        let reopened =
            TuningStore::open_test_directory(root.path(), MAX_TUNING_JOURNAL_BYTES).unwrap();
        let second = reopened.lab_snapshot().unwrap();
        assert_eq!(second.lease_creation(), TuningLabCreation::Opened);
        assert!(
            first.root_id() == second.root_id() && first.lease_id() == second.lease_id(),
            "exact retained identities"
        );
        drop(reopened);
        root.cleanup().unwrap();
    }
    #[test]
    fn lab_evidence_retains_full_bytes_ids_and_all_five_fault_artifacts() {
        for point in [
            TuningPublishPoint::AfterStageSync,
            TuningPublishPoint::AfterOldMove,
            TuningPublishPoint::AfterPublish,
            TuningPublishPoint::BeforeBackupCleanup,
            TuningPublishPoint::AfterBackupCleanup,
        ] {
            let root = TuningTemporaryDirectory::new().unwrap();
            let mut store =
                TuningStore::open_test_directory(root.path(), MAX_TUNING_JOURNAL_BYTES).unwrap();
            store.publish(b"old actual fixture").unwrap();
            let original = store.lab_snapshot().unwrap();
            assert!(
                original.last_synced_stage_id()
                    == Some(
                        original
                            .artifact(TuningLabArtifactKind::Journal)
                            .unwrap()
                            .id()
                    ),
                "actual synced-stage receipt follows journal"
            );
            store.fail_publication_at(point);
            assert!(store.publish(b"new actual fixture").is_err());
            let failed = store.lab_snapshot().unwrap();
            let original_id = original
                .artifact(TuningLabArtifactKind::Journal)
                .unwrap()
                .id();
            let old_kind = if point == TuningPublishPoint::AfterStageSync {
                TuningLabArtifactKind::Journal
            } else {
                TuningLabArtifactKind::Previous
            };
            if point != TuningPublishPoint::AfterBackupCleanup {
                let old = failed.artifact(old_kind).unwrap();
                assert!(old.id() == original_id, "exact old artifact identity");
                assert_eq!(old.bytes(), b"old actual fixture");
            }
            let new_kind = if matches!(
                point,
                TuningPublishPoint::AfterStageSync | TuningPublishPoint::AfterOldMove
            ) {
                TuningLabArtifactKind::Stage
            } else {
                TuningLabArtifactKind::Journal
            };
            let new = failed.artifact(new_kind).unwrap();
            assert!(
                failed.last_synced_stage_id() == Some(new.id()),
                "exact stage receipt survives selected fault boundary"
            );
            assert!(new.id() != original_id, "distinct staged identity");
            assert_eq!(new.bytes(), b"new actual fixture");
            assert_eq!(store.read().unwrap_err(), TuningNativeError::Poisoned);
            assert_eq!(
                store.publish(b"never retry").unwrap_err(),
                TuningNativeError::Poisoned
            );
            assert!(matches!(
                TuningStore::open_test_directory(root.path(), MAX_TUNING_JOURNAL_BYTES),
                Err(TuningNativeError::Busy)
            ));
            drop(store);
            root.cleanup().unwrap();
        }
    }
    #[test]
    fn lab_evidence_unknown_entry_is_preserved_and_refuses_snapshot() {
        let root = TuningTemporaryDirectory::new().unwrap();
        let store =
            TuningStore::open_test_directory(root.path(), MAX_TUNING_JOURNAL_BYTES).unwrap();
        let foreign = root.path().join("fixture-unknown");
        std::fs::write(&foreign, b"retain unknown").unwrap();
        assert!(matches!(
            store.lab_snapshot(),
            Err(TuningNativeError::UnsafeStore(_))
        ));
        assert_eq!(std::fs::read(&foreign).unwrap(), b"retain unknown");
        drop(store);
        root.cleanup().unwrap();
    }
}
