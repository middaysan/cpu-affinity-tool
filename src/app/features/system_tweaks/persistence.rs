//! One bounded recovery document and one retained, per-user writer lease.
use super::platform::{codec::ModuleEnvelope, contract::*, controller::OwnerKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
const LIMIT: usize = 4 * 1024 * 1024;
const MAX_FIELD_OWNERS: usize = 256;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivationFacts {
    pub pid: u32,
    pub token: u64,
    pub image: String,
    pub trigger: ActivationEligibility,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerRecord {
    pub owner: OwnerKey,
    pub config: ModuleEnvelope,
    pub activation: Option<ActivationFacts>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldRecord {
    pub module: FeatureId,
    pub version: SchemaVersion,
    pub key: ResourceKey,
    pub baseline: Vec<u8>,
    pub current: Vec<u8>,
    pub desired: Vec<u8>,
    pub attempted: Option<Vec<u8>>,
    pub owners: Vec<OwnerKey>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryDocument {
    pub version: u32,
    pub source: String,
    pub fields: Vec<FieldRecord>,
    pub owners: Vec<OwnerRecord>,
}
pub(crate) trait RecoveryStore: Send {
    fn load(&mut self) -> Result<Option<RecoveryDocument>, ModuleError>;
    fn publish(&mut self, document: &RecoveryDocument) -> Result<(), ModuleError>;
}
pub(crate) fn failure(operation: &'static str, error: impl std::fmt::Display) -> ModuleError {
    let text = error.to_string();
    let mut end = text.len().min(1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    ModuleError::new(ErrorKind::Verification, operation)
        .with_detail(text[..end].replace('\0', " "))
        .unwrap()
}
impl RecoveryDocument {
    pub(crate) fn validate(&self) -> Result<(), ModuleError> {
        let bad = || {
            failure(
                "recovery document",
                "Unknown, malformed or oversized recovery evidence",
            )
        };
        if self.version != 1
            || self.source.is_empty()
            || self.source.len() > 32767
            || self.source.contains('\0')
            || self.fields.len() > 256
            || self.owners.len() > 256
        {
            return Err(bad());
        }
        let mut owners = BTreeSet::new();
        for record in &self.owners {
            if !owners.insert(record.owner.clone())
                || record.config.feature != *record.owner.feature()
                || record.config.to_bytes().is_err()
            {
                return Err(bad());
            }
            match (&record.owner, &record.activation) {
                (OwnerKey::Manual { .. }, None) => {}
                (
                    OwnerKey::Session {
                        pid,
                        token,
                        group,
                        rule,
                        ..
                    },
                    Some(facts),
                ) if *pid != 0
                    && *token != 0
                    && *pid == facts.pid
                    && *token == facts.token
                    && !facts.image.is_empty()
                    && facts.image.len() <= 32767
                    && !facts.image.contains('\0')
                    && !group.is_empty()
                    && group.len() <= 128
                    && !rule.is_empty()
                    && rule.len() <= 128 => {}
                _ => return Err(bad()),
            }
        }
        let mut keys = BTreeSet::new();
        let mut used = BTreeSet::new();
        for field in &self.fields {
            if !keys.insert(field.key.clone())
                || !field.key.target.valid()
                || field.baseline.len() > MAX_FIELD_BYTES
                || field.current.len() > MAX_FIELD_BYTES
                || field.desired.len() > MAX_FIELD_BYTES
                || field
                    .attempted
                    .as_ref()
                    .is_some_and(|v| v.len() > MAX_FIELD_BYTES)
                || field.owners.is_empty()
                || field.owners.len() > MAX_FIELD_OWNERS
            {
                return Err(bad());
            }
            let mut unique = BTreeSet::new();
            for owner in &field.owners {
                if !owners.contains(owner) || !unique.insert(owner.clone()) {
                    return Err(bad());
                }
                used.insert(owner.clone());
            }
        }
        let raw = serde_json::to_string(self).map_err(|_| bad())?;
        if used != owners || raw.len() > LIMIT || super::config::bounded_tree(&raw).is_err() {
            return Err(bad());
        }
        Ok(())
    }
}
pub(crate) fn source_identifier(path: &Path) -> Result<String, ModuleError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| failure("state source", e))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(failure("state source", "Invalid absolute state path"));
                }
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    let text = normalized
        .to_str()
        .ok_or_else(|| failure("state source", "State path encoding"))?;
    #[cfg(windows)]
    let text = text.replace('\\', "/").to_lowercase();
    #[cfg(not(windows))]
    let text = text.to_owned();
    if !normalized.is_absolute() || text.len() > 32767 || text.contains('\0') {
        return Err(failure("state source", "Invalid absolute state path"));
    }
    Ok(text)
}
pub(crate) fn common_directory() -> Result<PathBuf, ModuleError> {
    #[cfg(windows)]
    let directory = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let directory = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let directory = directory
        .filter(|p| p.is_absolute())
        .ok_or_else(|| failure("recovery location", "User-local data directory unavailable"))?;
    Ok(directory.join("CpuAffinityTool").join("tuning"))
}
pub(crate) struct DiskRecoveryStore {
    directory: PathBuf,
    _lease: File,
    poisoned: bool,
}
impl DiskRecoveryStore {
    pub(crate) fn open(directory: &Path) -> Result<Self, ModuleError> {
        if !directory.is_absolute() {
            return Err(failure(
                "recovery location",
                "Recovery directory must be absolute",
            ));
        }
        std::fs::create_dir_all(directory).map_err(|e| failure("recovery directory", e))?;
        for path in [
            directory.to_path_buf(),
            directory.join("recovery.lock"),
            directory.join("recovery-v1.json"),
        ] {
            match std::fs::symlink_metadata(path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(failure("recovery location", "Recovery symlink refused"))
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(failure("recovery location", e)),
            }
        }
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("recovery.lock"))
            .map_err(|e| failure("recovery lease", e))?;
        lease.try_lock().map_err(|e| failure("recovery lease", e))?;
        match std::fs::symlink_metadata(directory.join("recovery-v1.staging")) {
            Ok(_) => {
                return Err(failure(
                    "recovery staging",
                    "Interrupted recovery publication needs attention",
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(failure("recovery staging", e)),
        }
        Ok(Self {
            directory: directory.into(),
            _lease: lease,
            poisoned: false,
        })
    }
}
impl RecoveryStore for DiskRecoveryStore {
    fn load(&mut self) -> Result<Option<RecoveryDocument>, ModuleError> {
        let file = match File::open(self.directory.join("recovery-v1.json")) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(failure("read recovery", e)),
        };
        let mut bytes = Vec::new();
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| failure("read recovery", e))?;
        if bytes.len() > LIMIT {
            return Err(failure("read recovery", "Recovery file exceeds 4 MiB"));
        }
        super::config::bounded_tree(
            std::str::from_utf8(&bytes).map_err(|e| failure("read recovery", e))?,
        )
        .map_err(|e| failure("read recovery", e))?;
        let document: RecoveryDocument =
            serde_json::from_slice(&bytes).map_err(|e| failure("read recovery", e))?;
        document.validate()?;
        Ok(Some(document))
    }
    fn publish(&mut self, document: &RecoveryDocument) -> Result<(), ModuleError> {
        if self.poisoned {
            return Err(failure(
                "publish recovery",
                "Earlier publication failed; retained evidence needs attention",
            ));
        }
        document.validate()?;
        let result = (|| {
            let bytes = serde_json::to_vec(document).map_err(|e| failure("publish recovery", e))?;
            let staging = self.directory.join("recovery-v1.staging");
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)
                .map_err(|e| failure("stage recovery", e))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| failure("sync recovery", e))?;
            drop(file);
            crate::app::models::AppStateStorage::publish_recovery_file(
                &staging,
                &self.directory.join("recovery-v1.json"),
            )
            .map_err(|e| failure("publish recovery", e))
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}
