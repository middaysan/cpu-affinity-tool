//! Qualified state source; native receipts never come from caller metadata.
use super::{
    identity::ConfigPrincipal,
    native_error,
    security::{Descriptor, config_security},
    store_io::{
        Kind, OpenMode, Sharing, component,
        native::{Directory, FullObjectId, Node},
    },
};
use crate::tuning::TuningNativeError;
use std::{os::windows::ffi::OsStrExt, path::Path, rc::Rc, sync::Arc};
use windows::{
    Win32::{
        Foundation::HANDLE,
        Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
        Storage::FileSystem::{
            MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACE_FILE_FLAGS, ReplaceFileW,
        },
    },
    core::PCWSTR,
};
type Result<T> = std::result::Result<T, TuningNativeError>;
const LIMIT: usize = 16 * 1024 * 1024;
const LEAF: &str = "state.json";

pub(super) struct ConfigSourcePolicy {
    principal: Rc<ConfigPrincipal>,
}
impl ConfigSourcePolicy {
    pub(super) fn validate(&self, handle: HANDLE, managed: bool) -> Result<()> {
        self.principal.verify()?;
        let (owner, entries) = config_security(handle)?;
        config_acl_admission(&owner, self.principal.sid(), managed, &entries)
    }
    #[cfg(test)]
    pub(super) fn characterization_descriptor(&self) -> Result<Descriptor> {
        Descriptor::for_config_characterization(&self.principal)
    }
}

/// Exact bytes and actual native provenance. Deserialization and fixture metadata
/// cannot construct this type. File identity changes at atomic replacement.
/// ```compile_fail
/// let forged = os_api::tuning::TuningConfigReceipt { bytes: vec![].into() };
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct TuningConfigReceipt {
    bytes: Arc<[u8]>,
    principal: String,
    volume_guid: Vec<u16>,
    directory: FullObjectId,
    file: FullObjectId,
}
impl TuningConfigReceipt {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn principal_sid(&self) -> &str {
        &self.principal
    }
    pub fn volume_guid(&self) -> &[u16] {
        &self.volume_guid
    }
    pub fn volume_serial(&self) -> u64 {
        self.directory.volume
    }
    pub fn directory_id(&self) -> &[u8; 16] {
        &self.directory.id
    }
    pub fn file_id(&self) -> &[u8; 16] {
        &self.file.id
    }
    pub fn leaf(&self) -> &'static str {
        LEAF
    }
}
impl std::fmt::Debug for TuningConfigReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningConfigReceipt(native provenance redacted)")
    }
}

/// Caller/worker-local retained configuration source. It is never stored in
/// shared AppState; a serial worker constructs its own source from sealed input.
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<os_api::tuning::TuningConfigSource>();
/// ```
pub struct TuningConfigSource {
    chain: Vec<Directory>,
    principal: Rc<ConfigPrincipal>,
    volume_guid: Vec<u16>,
    directory_id: FullObjectId,
    leaf: Option<Node>,
    faulted: bool,
    #[cfg(test)]
    characterization_terminal: bool,
    #[cfg(test)]
    characterization_created: std::cell::RefCell<Vec<tests::CreatedFileEvidence>>,
}
impl std::fmt::Debug for TuningConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningConfigSource(worker-local provenance redacted)")
    }
}
struct OwnedStage {
    node: Node,
    name: Vec<u16>,
    id: FullObjectId,
    bytes: Arc<[u8]>,
}
fn relevant_publication_evidence(name: &[u16]) -> bool {
    let prefix = ".state.json.tuning-";
    name.len() >= prefix.len()
        && prefix.encode_utf16().zip(name).all(|(want, got)| {
            let actual = if *got <= 127 {
                (*got as u8).to_ascii_lowercase() as u16
            } else {
                *got
            };
            actual == want
        })
}
fn old_leaf_admission(expected: Result<FullObjectId>, current: FullObjectId) -> Result<()> {
    if expected? != current {
        Err(TuningNativeError::UnsafeStore("config old leaf changed"))
    } else {
        Ok(())
    }
}

fn selected_components(path: &Path) -> Result<(u8, Vec<Vec<u16>>)> {
    let units: Vec<_> = path.as_os_str().encode_wide().collect();
    if units.len() < 4
        || units.len() > 32760
        || units[0] > 127
        || !(units[0] as u8).is_ascii_alphabetic()
        || units[1..3] != [58, 92]
    {
        return Err(TuningNativeError::Unsupported(
            "config local absolute drive path",
        ));
    }
    let parts: Vec<Vec<u16>> = units[3..]
        .split(|u| *u == 92)
        .map(<[u16]>::to_vec)
        .collect();
    if parts.len() < 2
        || parts.len() > 64
        || parts
            .last()
            .is_none_or(|p| p != &LEAF.encode_utf16().collect::<Vec<_>>())
    {
        return Err(TuningNativeError::Unsupported(
            "config fixed leaf/bounded chain",
        ));
    }
    for part in &parts {
        component(part)?;
    }
    Ok((units[0] as u8, parts[..parts.len() - 1].to_vec()))
}
impl TuningConfigSource {
    /// Only the independently selected active state path may be supplied here.
    /// Unsupported/unsafe paths give no receipt; this API has no pathname fallback.
    pub fn open(selected_state: &Path) -> Result<Self> {
        let (drive, parts) = selected_components(selected_state)?;
        let principal = Rc::new(ConfigPrincipal::observe()?);
        let descriptor = Rc::new(Descriptor::for_config(&principal)?);
        let policy = Rc::new(ConfigSourcePolicy {
            principal: Rc::clone(&principal),
        });
        let root = Directory::config_bootstrap(drive, descriptor, policy)?;
        let volume_guid = root.guid_path()?;
        let mut chain = vec![root];
        for (index, name) in parts.iter().enumerate() {
            let managed = index + 1 == parts.len();
            let (node, _) = chain.last().expect("retained root").open(
                name,
                Kind::Directory,
                OpenMode::Existing,
                Sharing::Retained,
                true,
                Some(managed),
            )?;
            chain.push(node.into_directory()?);
        }
        let directory_id = chain.last().expect("retained directory").full_id()?;
        let mut source = Self {
            chain,
            principal,
            volume_guid,
            directory_id,
            leaf: None,
            faulted: false,
            #[cfg(test)]
            characterization_terminal: true,
            #[cfg(test)]
            characterization_created: std::cell::RefCell::new(Vec::new()),
        };
        source.verify()?;
        source.publication_evidence(None)?;
        // A fresh source may be absent; absence is rechecked before publication.
        match source
            .directory()
            .config_leaf(&LEAF.encode_utf16().collect::<Vec<_>>(), false)
        {
            Ok(node) => source.leaf = Some(node),
            Err(TuningNativeError::Io { code: 2, .. }) => {}
            Err(error) => return Err(error),
        }
        Ok(source)
    }
    fn directory(&self) -> &Directory {
        self.chain.last().expect("retained directory")
    }
    #[cfg(test)]
    fn record_characterization_creation(&self, node: &mut Node, name: &[u16]) -> Result<()> {
        let id = node.full_id()?;
        let bytes = Arc::from(node.read(LIMIT)?);
        let mut created = self.characterization_created.borrow_mut();
        if created.len() == 2 {
            return Err(TuningNativeError::Poisoned);
        }
        created.push(tests::CreatedFileEvidence {
            id,
            bytes,
            names: vec![name.to_vec(), LEAF.encode_utf16().collect()],
        });
        Ok(())
    }
    #[cfg(test)]
    fn record_characterization_bytes(&self, node: &mut Node) -> Result<()> {
        let id = node.full_id()?;
        let bytes = Arc::from(node.read(LIMIT)?);
        let mut created = self.characterization_created.borrow_mut();
        let Some(own) = created.iter_mut().find(|own| own.id == id) else {
            return Err(TuningNativeError::Poisoned);
        };
        own.bytes = bytes;
        Ok(())
    }
    /// Poisoned signals blocked publication evidence, including a missing final
    /// leaf after partial ReplaceFile completion. The later state bridge must
    /// preserve this result before ordinary backup/default recovery writes.
    fn publication_evidence(&self, mut owned: Option<&mut OwnedStage>) -> Result<()> {
        let mut owned_seen = owned.is_none();
        for entry in self.directory().entries()? {
            if !relevant_publication_evidence(&entry.name) {
                continue;
            }
            let Some(stage) = owned.as_deref_mut() else {
                return Err(TuningNativeError::Poisoned);
            };
            if entry.name != stage.name
                || !self.directory().entry_matches(&entry, &stage.node)?
                || stage.node.full_id()? != stage.id
                || stage.node.read(LIMIT)?.as_slice() != stage.bytes.as_ref()
            {
                return Err(TuningNativeError::Poisoned);
            }
            let (mut named, _) = self.directory().open(
                &stage.name,
                Kind::File,
                OpenMode::Existing,
                Sharing::Renameable,
                true,
                Some(true),
            )?;
            if named.full_id()? != stage.id || named.read(LIMIT)?.as_slice() != stage.bytes.as_ref()
            {
                return Err(TuningNativeError::Poisoned);
            }
            owned_seen = true;
        }
        if !owned_seen {
            return Err(TuningNativeError::Poisoned);
        }
        Ok(())
    }
    fn verify(&self) -> Result<()> {
        if self.faulted {
            return Err(TuningNativeError::Poisoned);
        }
        self.principal.verify()?;
        for node in &self.chain {
            node.verify()?;
        }
        if self.chain[0].guid_path()? != self.volume_guid
            || self.directory().full_id()? != self.directory_id
        {
            return Err(TuningNativeError::UnsafeStore(
                "config retained origin changed",
            ));
        }
        Ok(())
    }
    fn receipt(&self, file: FullObjectId, bytes: Arc<[u8]>) -> TuningConfigReceipt {
        TuningConfigReceipt {
            bytes,
            principal: self.principal.sid().to_owned(),
            volume_guid: self.volume_guid.clone(),
            directory: self.directory_id,
            file,
        }
    }
    pub fn read(&mut self) -> Result<Option<TuningConfigReceipt>> {
        self.verify()?;
        self.publication_evidence(None)?;
        let name: Vec<_> = LEAF.encode_utf16().collect();
        let mut fresh = match self.directory().config_leaf(&name, false) {
            Ok(node) => node,
            Err(TuningNativeError::Io { code: 2, .. }) if self.leaf.is_none() => return Ok(None),
            Err(error) => return Err(error),
        };
        let id = fresh.full_id()?;
        if let Some(old) = &self.leaf {
            old_leaf_admission(old.full_id(), id)?;
        }
        let bytes: Arc<[u8]> = fresh.read(LIMIT)?.into();
        self.verify()?;
        self.leaf = Some(fresh);
        Ok(Some(self.receipt(id, bytes)))
    }
    fn path(&self, name: &[u16]) -> Result<Vec<u16>> {
        component(name)?;
        let mut path = self.directory().guid_path()?;
        if path.last() != Some(&92) {
            path.push(92);
        }
        path.extend_from_slice(name);
        if path.len() > 32760 {
            return Err(TuningNativeError::Unsupported(
                "config publication path bounds",
            ));
        }
        path.push(0);
        Ok(path)
    }
    fn prepare(&self, bytes: &[u8]) -> Result<OwnedStage> {
        self.verify()?;
        if bytes.len() > LIMIT {
            return Err(TuningNativeError::Capacity {
                size: bytes.len() as u64,
                limit: LIMIT,
            });
        }
        self.publication_evidence(None)?;
        let mut nonce = [0; 16];
        unsafe { BCryptGenRandom(None, &mut nonce, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
            .ok()
            .map_err(|e| native_error("config stage nonce", e))?;
        let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let name: Vec<_> = format!(".state.json.tuning-{suffix}")
            .encode_utf16()
            .collect();
        let (mut node, _) = self.directory().open(
            &name,
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Renameable,
            false,
            Some(true),
        )?;
        #[cfg(test)]
        self.record_characterization_creation(&mut node, &name)?;
        node.write_synced(bytes, LIMIT)?;
        if node.read(LIMIT)? != bytes {
            return Err(TuningNativeError::UnsafeStore("config staged bytes"));
        }
        #[cfg(test)]
        self.record_characterization_bytes(&mut node)?;
        let id = node.full_id()?;
        if id.volume != self.directory_id.volume {
            return Err(TuningNativeError::UnsafeStore("config staged volume"));
        }
        Ok(OwnedStage {
            node,
            name,
            id,
            bytes: Arc::from(bytes),
        })
    }
    /// A failed/uncertain publication poisons this session. There is no retry or
    /// ordinary pathname fallback, and ambiguous owned evidence is left intact.
    pub fn publish(
        &mut self,
        expected: Option<&TuningConfigReceipt>,
        bytes: &[u8],
    ) -> Result<TuningConfigReceipt> {
        self.verify()?;
        if bytes.len() > LIMIT {
            return Err(TuningNativeError::Capacity {
                size: bytes.len() as u64,
                limit: LIMIT,
            });
        }
        let current = self.read()?;
        if current.as_ref() != expected {
            return Err(TuningNativeError::UnsafeStore(
                "config expected bytes/provenance",
            ));
        }
        // Release READ-only source guard only at this caller's explicit save.
        self.leaf = None;
        let name: Vec<_> = LEAF.encode_utf16().collect();
        let old = match expected {
            Some(receipt) => {
                let mut old = self.directory().config_leaf(&name, true)?;
                if old.full_id()? != receipt.file || old.read(LIMIT)? != receipt.bytes() {
                    return Err(TuningNativeError::UnsafeStore("config old source changed"));
                }
                Some(old)
            }
            None => {
                match self.directory().config_leaf(&name, true) {
                    Err(TuningNativeError::Io { code: 2, .. }) => {}
                    _ => return Err(TuningNativeError::UnsafeStore("config absence changed")),
                }
                None
            }
        };
        let stage = self.prepare(bytes)?;
        self.finish_publication(old, expected, stage)
    }
    fn finish_publication(
        &mut self,
        mut old: Option<Node>,
        expected: Option<&TuningConfigReceipt>,
        mut stage: OwnedStage,
    ) -> Result<TuningConfigReceipt> {
        self.verify()?;
        self.publication_evidence(Some(&mut stage))?;
        let name: Vec<_> = LEAF.encode_utf16().collect();
        let old_dacl = if let Some(node) = &old {
            Some(node.config_dacl()?)
        } else {
            None
        };
        // Fresh name-to-old proof immediately before releasing the owned stage.
        if let Some(receipt) = expected {
            let mut current = self.directory().config_leaf(&name, true)?;
            if current.full_id()? != receipt.file || current.read(LIMIT)? != receipt.bytes() {
                return Err(TuningNativeError::UnsafeStore("config old name changed"));
            }
        } else {
            match self.directory().config_leaf(&name, true) {
                Err(TuningNativeError::Io { code: 2, .. }) => {}
                _ => {
                    return Err(TuningNativeError::UnsafeStore(
                        "config first absence changed",
                    ));
                }
            }
        }
        let destination = self.path(&name)?;
        let staged = self.path(&stage.name)?;
        let OwnedStage {
            node,
            name: _,
            id,
            bytes,
        } = stage;
        drop(node);
        self.faulted = true;
        #[cfg(test)]
        {
            self.characterization_terminal = false;
            eprintln!(
                "P1b publication reached=dispatch replacement={}",
                old.is_some()
            );
        }
        if old.is_some() {
            replace_primitive(&destination, &staged)?;
        } else {
            unsafe {
                MoveFileExW(
                    PCWSTR(staged.as_ptr()),
                    PCWSTR(destination.as_ptr()),
                    MOVEFILE_WRITE_THROUGH,
                )
            }
            .map_err(|e| native_error("config first atomic publication", e))?;
        }
        #[cfg(test)]
        {
            self.characterization_terminal = true;
            eprintln!(
                "P1b publication reached=api-success replacement={}",
                old.is_some()
            );
        }
        // This check intentionally avoids verify(), whose sticky fault closes
        // other operations until every actual postcondition has been established.
        self.principal.verify()?;
        for directory in &self.chain {
            directory.verify()?;
        }
        self.publication_evidence(None)?;
        #[cfg(test)]
        eprintln!("P1b publication reached=post-evidence-checked");
        let mut final_node = self.directory().config_leaf(&name, false)?;
        let final_object = final_node.full_id()? == id;
        let exact_bytes = final_node.read(LIMIT)?.as_slice() == bytes.as_ref();
        let final_security = old_dacl.as_ref().is_none_or(|dacl| {
            final_node
                .config_dacl()
                .is_ok_and(|actual| regular_file_dacl_preserved(dacl, &actual))
        });
        #[cfg(test)]
        if !final_security {
            fn describe(label: &'static str, entries: &[super::security::Ace], principal: &str) {
                eprintln!("P1b DACL {label} count={}", entries.len());
                for entry in entries.iter().take(16) {
                    let class = if entry.sid == principal {
                        "ActualPrincipal"
                    } else if entry.sid == "S-1-5-18" {
                        "System"
                    } else if entry.sid == "S-1-5-32-544" {
                        "Admin"
                    } else {
                        "Other"
                    };
                    eprintln!(
                        "P1b DACL {label} class={class} kind={} flags={:#x} mask={:#x}",
                        entry.kind, entry.flags, entry.mask
                    );
                }
            }
            if let Some(entries) = &old_dacl {
                describe("old", entries, self.principal.sid());
            }
            if let Ok(entries) = final_node.config_dacl() {
                describe("final", &entries, self.principal.sid());
            } else {
                eprintln!("P1b DACL final_available=false");
            }
        }
        let old_object = match (&mut old, expected) {
            (Some(node), Some(receipt)) => {
                node.config_retired_bytes(receipt.file, LIMIT)?.as_slice() == receipt.bytes()
            }
            (None, None) => true,
            _ => false,
        };
        let same_origin = self.chain[0].guid_path()? == self.volume_guid
            && self.directory().full_id()? == self.directory_id;
        #[cfg(test)]
        eprintln!(
            "P1b publication reached=final-evidence old_object={old_object} final_object={final_object} exact_bytes={exact_bytes} final_security={final_security} same_origin={same_origin}"
        );
        publication_admission(PublicationEvidence {
            api_success: true,
            old_name: true,
            old_object,
            final_object,
            exact_bytes,
            final_security,
            same_origin,
        })?;
        self.leaf = Some(final_node);
        self.faulted = false;
        Ok(self.receipt(id, bytes))
    }
}
fn replace_primitive(destination: &[u16], staged: &[u16]) -> Result<()> {
    unsafe {
        ReplaceFileW(
            PCWSTR(destination.as_ptr()),
            PCWSTR(staged.as_ptr()),
            None,
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    }
    .map_err(|e| native_error("config atomic replacement", e))
}

fn regular_file_dacl_preserved(
    old: &[super::security::Ace],
    actual: &[super::security::Ace],
) -> bool {
    // Both callers remeasure verified regular, nonreparse files. ReplaceFileW
    // may clear child-only OI/CI flags there; every other ordered ACE bit stays
    // exact, including INHERIT_ONLY, NO_PROPAGATE and INHERITED.
    old.len() == actual.len()
        && old.iter().zip(actual).all(|(before, after)| {
            before.sid == after.sid
                && before.kind == after.kind
                && before.mask == after.mask
                && (after.flags == before.flags || after.flags == before.flags & !0x3)
        })
}

fn config_acl_admission(
    owner: &str,
    principal: &str,
    managed: bool,
    entries: &[super::security::Ace],
) -> Result<()> {
    // Exact known Windows Modules Installer identity may own/mutate ancestors.
    // A service-SID prefix or journal-provided name grants no managed authority.
    let trusted = |sid: &str| {
        sid == principal
            || sid == "S-1-5-18"
            || sid == "S-1-5-32-544"
            || (!managed && sid == super::security::TRUSTED_INSTALLER)
    };
    if !trusted(owner) || entries.is_empty() {
        return Err(TuningNativeError::UnsafeStore("config owner/DACL"));
    }
    let mut full = false;
    for entry in entries {
        if !matches!(entry.kind, 0 | 1) || entry.flags & !0x1f != 0 {
            return Err(TuningNativeError::Unsupported("config complex ACE"));
        }
        if entry.flags & 8 != 0 || entry.kind == 1 {
            continue;
        }
        if !trusted(&entry.sid) {
            // Only a plain effective unrelated-child grant on an already
            // retained ancestor is exempt. Descendants are checked separately.
            let mutation = 0x000d0156 | 0x10000000 | 0x40000000;
            let permitted_child = !managed && entry.flags & !0x10 == 0;
            let forbidden = mutation & if permitted_child { !4 } else { u32::MAX };
            if entry.mask & forbidden != 0 {
                return Err(TuningNativeError::UnsafeStore(
                    "config untrusted mutation grant",
                ));
            }
        }
        if entry.sid == principal && entry.mask & 0x001f01ff == 0x001f01ff {
            full = true;
        }
    }
    if managed && !full {
        return Err(TuningNativeError::UnsafeStore(
            "config missing principal authority",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct PublicationEvidence {
    api_success: bool,
    old_name: bool,
    old_object: bool,
    final_object: bool,
    exact_bytes: bool,
    final_security: bool,
    same_origin: bool,
}
fn publication_admission(e: PublicationEvidence) -> Result<()> {
    if e.api_success
        && e.old_name
        && e.old_object
        && e.final_object
        && e.exact_bytes
        && e.final_security
        && e.same_origin
    {
        Ok(())
    } else {
        Err(TuningNativeError::Poisoned)
    }
}

#[cfg(test)]
mod tests {
    use super::super::security::Ace;
    use super::*;
    const PRINCIPAL: &str = "S-1-5-21-1-2-3-1001";
    #[derive(Clone)]
    pub(super) struct CreatedFileEvidence {
        pub(super) id: FullObjectId,
        pub(super) bytes: Arc<[u8]>,
        pub(super) names: Vec<Vec<u16>>,
    }
    struct CurrentFileEvidence {
        id: FullObjectId,
        bytes: Vec<u8>,
        name: Vec<u16>,
    }
    fn own_set_admission(
        created: &[CreatedFileEvidence],
        current: &[CurrentFileEvidence],
    ) -> Result<()> {
        if created.len() > 2 || current.len() > created.len() {
            return Err(TuningNativeError::Poisoned);
        }
        for (index, file) in current.iter().enumerate() {
            if current[..index]
                .iter()
                .any(|earlier| earlier.id == file.id || earlier.name == file.name)
                || !created.iter().any(|own| {
                    own.id == file.id
                        && own.bytes.as_ref() == file.bytes
                        && own.names.contains(&file.name)
                })
            {
                return Err(TuningNativeError::Poisoned);
            }
        }
        Ok(())
    }
    fn cleanup_created_files(
        source: &mut TuningConfigSource,
        created: &[CreatedFileEvidence],
        original_root: (u32, u64),
    ) -> Result<()> {
        fn context(source: &TuningConfigSource, original_root: (u32, u64)) -> Result<()> {
            if !source.characterization_terminal {
                return Err(TuningNativeError::Poisoned);
            }
            source.principal.verify()?;
            for directory in &source.chain {
                directory.verify()?;
            }
            let root = source.directory().id();
            if (root.volume, root.index) != original_root
                || source.directory().full_id()? != source.directory_id
                || source.chain[0].guid_path()? != source.volume_guid
            {
                return Err(TuningNativeError::UnsafeStore(
                    "characterization original root changed",
                ));
            }
            Ok(())
        }
        fn inventory(source: &TuningConfigSource) -> Result<Vec<CurrentFileEvidence>> {
            let mut result = Vec::new();
            for entry in source.directory().entries()? {
                let (mut node, _) = source.directory().open(
                    &entry.name,
                    Kind::File,
                    OpenMode::Existing,
                    Sharing::Renameable,
                    true,
                    Some(true),
                )?;
                if !source.directory().entry_matches(&entry, &node)? {
                    return Err(TuningNativeError::Poisoned);
                }
                result.push(CurrentFileEvidence {
                    id: node.full_id()?,
                    bytes: node.read(LIMIT)?,
                    name: entry.name,
                });
            }
            Ok(result)
        }
        context(source, original_root)?;
        let current = inventory(source)?;
        own_set_admission(created, &current)?;
        // The no-delete source guard can be released only after its own ID and
        // bytes have been authenticated against the complete observed set.
        if let Some(leaf) = &mut source.leaf {
            let id = leaf.full_id()?;
            let bytes = leaf.read(LIMIT)?;
            if !current.iter().any(|file| {
                file.id == id
                    && file.bytes == bytes
                    && file.name == LEAF.encode_utf16().collect::<Vec<_>>()
            }) {
                return Err(TuningNativeError::Poisoned);
            }
        }
        source.leaf = None;
        // Open every authenticated object with deletion rights before deleting
        // any. These are fresh relative identity/bytes/security checks, not paths.
        let mut deletions = Vec::new();
        for file in &current {
            context(source, original_root)?;
            let (mut node, _) = source.directory().open(
                &file.name,
                Kind::File,
                OpenMode::Existing,
                Sharing::Renameable,
                false,
                Some(true),
            )?;
            if node.full_id()? != file.id || node.read(LIMIT)? != file.bytes {
                return Err(TuningNativeError::Poisoned);
            }
            deletions.push(node);
        }
        let fresh = inventory(source)?;
        own_set_admission(created, &fresh)?;
        if fresh.len() != current.len()
            || fresh.iter().any(|file| {
                !current.iter().any(|old| {
                    old.id == file.id && old.name == file.name && old.bytes == file.bytes
                })
            })
        {
            return Err(TuningNativeError::Poisoned);
        }
        eprintln!(
            "P1b cleanup reached=complete-owned-set-authenticated count={}",
            deletions.len()
        );
        for (mut node, file) in deletions.into_iter().zip(&current) {
            context(source, original_root)?;
            if node.full_id()? != file.id || node.read(LIMIT)? != file.bytes {
                return Err(TuningNativeError::Poisoned);
            }
            source.characterization_terminal = false;
            node.delete_owned()?;
            if !matches!(
                source.directory().config_leaf(&file.name, true),
                Err(TuningNativeError::Io { code: 2, .. })
            ) {
                return Err(TuningNativeError::Poisoned);
            }
            source.characterization_terminal = true;
        }
        context(source, original_root)?;
        if !source.directory().entries()?.is_empty() {
            return Err(TuningNativeError::Poisoned);
        }
        Ok(())
    }
    #[test]
    fn p1b_error_cleanup_authenticates_complete_owned_set_before_deletion() {
        let created = [CreatedFileEvidence {
            id: FullObjectId {
                volume: 1,
                id: [1; 16],
            },
            bytes: Arc::from(b"created bytes".as_slice()),
            names: vec![
                ".state.json.tuning-owned".encode_utf16().collect(),
                LEAF.encode_utf16().collect(),
            ],
        }];
        let good = || CurrentFileEvidence {
            id: created[0].id,
            bytes: created[0].bytes.to_vec(),
            name: LEAF.encode_utf16().collect(),
        };
        assert!(own_set_admission(&created, &[good()]).is_ok());
        assert!(own_set_admission(&created, &[]).is_ok());
        for field in 0..4 {
            let mut bad = good();
            match field {
                0 => bad.id.id = [2; 16],
                1 => bad.id.volume = 2,
                2 => bad.bytes = b"unverified bytes".to_vec(),
                _ => bad.name = "foreign".encode_utf16().collect(),
            }
            assert!(
                own_set_admission(&created, &[bad]).is_err(),
                "unknown set field {field}"
            );
        }
        let mut two_created = created.to_vec();
        two_created.push(CreatedFileEvidence {
            id: FullObjectId {
                volume: 1,
                id: [2; 16],
            },
            bytes: Arc::from(b"second created".as_slice()),
            names: vec![
                ".state.json.tuning-second".encode_utf16().collect(),
                LEAF.encode_utf16().collect(),
            ],
        });
        assert!(own_set_admission(&two_created, &[good(), good()]).is_err());
        assert!(own_set_admission(&[], &[good()]).is_err());
    }
    fn ace(sid: &str, mask: u32) -> Ace {
        Ace {
            kind: 0,
            flags: 0,
            mask,
            sid: sid.to_owned(),
        }
    }
    fn cleanup_owned_empty_after_error(
        cleanup_once: impl FnOnce() -> Result<()>,
        observe_path: impl FnOnce() -> std::io::Result<()>,
    ) -> bool {
        // The actual callback is existing cleanup_empty(): it checks the
        // original creation ID and never enumerates/removes any children.
        if cleanup_once().is_err() {
            return false;
        }
        matches!(observe_path(), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    }
    #[test]
    fn p1b_exact_trustedinstaller_owner_and_grants_only_qualify_ancestors() {
        let sid = super::super::security::TRUSTED_INSTALLER;
        let entries = [ace(sid, 0x001f01ff), ace(PRINCIPAL, 0x001f01ff)];
        assert!(config_acl_admission(sid, PRINCIPAL, false, &entries).is_ok());
        assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, false, &entries).is_ok());
        assert!(config_acl_admission(sid, PRINCIPAL, true, &entries).is_err());
        assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, true, &entries).is_err());
        let ordinary = [ace(PRINCIPAL, 0x001f01ff)];
        assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, true, &ordinary).is_ok());
        for lookalike in [
            format!("{sid}-1"),
            sid.replacen("S-1-5-80-", "S-1-5-080-", 1),
            "S-1-5-80-1-2-3-4-5".to_owned(),
        ] {
            let entries = [ace(&lookalike, 0x001f01ff), ace(PRINCIPAL, 0x001f01ff)];
            assert!(config_acl_admission(&lookalike, PRINCIPAL, false, &entries).is_err());
            assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, false, &entries).is_err());
        }
    }
    #[test]
    fn p1b_error_cleanup_attempts_original_empty_cleanup_once_and_verifies_absence() {
        use std::cell::Cell;
        let cleanup_calls = Cell::new(0);
        let observation_calls = Cell::new(0);
        let clean = cleanup_owned_empty_after_error(
            || {
                cleanup_calls.set(cleanup_calls.get() + 1);
                Ok(())
            },
            || {
                observation_calls.set(observation_calls.get() + 1);
                Err(std::io::ErrorKind::NotFound.into())
            },
        );
        assert!(clean);
        assert_eq!(cleanup_calls.get(), 1);
        assert_eq!(observation_calls.get(), 1);
        for code in [
            TuningNativeError::UnsafeStore("unknown original root"),
            TuningNativeError::Io {
                operation: "nonempty owned root",
                code: 145,
            },
        ] {
            let calls = Cell::new(0);
            assert!(!cleanup_owned_empty_after_error(
                || {
                    calls.set(calls.get() + 1);
                    Err(code)
                },
                || panic!("unknown/nonempty cleanup cannot claim absence")
            ));
            assert_eq!(calls.get(), 1);
        }
        assert!(!cleanup_owned_empty_after_error(|| Ok(()), || Ok(())));
        assert!(!cleanup_owned_empty_after_error(
            || Ok(()),
            || Err(std::io::ErrorKind::PermissionDenied.into())
        ));
    }
    #[test]
    fn p1b_distinct_policy_accepts_observed_principal_and_readonly_other() {
        let entries = [ace(PRINCIPAL, 0x001f01ff), ace("S-1-1-0", 0x00120089)];
        assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, true, &entries).is_ok());
    }
    #[test]
    fn p1b_ancestor_add_unrelated_subdirectory_is_distinct_from_deletion() {
        let mut entries = vec![ace(PRINCIPAL, 0x001f01ff), ace("S-1-1-0", 4)];
        assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, false, &entries).is_ok());
        for mask in [
            2, 0x10, 0x100, 0x40, 0x10000, 0x40000, 0x80000, 0x10000000, 0x40000000,
        ] {
            entries[1].mask = mask;
            assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, false, &entries).is_err());
        }
        entries[1].mask = 4;
        assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, true, &entries).is_err());
    }
    #[test]
    fn p1b_unknown_acl_and_owner_never_qualify() {
        let mut entries = vec![ace(PRINCIPAL, 0x001f01ff)];
        assert!(config_acl_admission("S-1-1-0", PRINCIPAL, true, &entries).is_err());
        for kind in [2, 5, 9, 17] {
            entries[0].kind = kind;
            assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, true, &entries).is_err());
        }
        assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, true, &[]).is_err());
        entries[0].kind = 0;
        entries[0].flags = 0x20;
        assert!(config_acl_admission(PRINCIPAL, PRINCIPAL, true, &entries).is_err());
    }
    #[test]
    fn p1b_regular_file_dacl_preserves_observed_directional_child_flags() {
        let old: Vec<_> = ["S-1-5-18", "S-1-5-32-544", PRINCIPAL]
            .into_iter()
            .map(|sid| {
                let mut entry = ace(sid, 0x001f01ff);
                entry.flags = 3;
                entry
            })
            .collect();
        let mut actual = old.clone();
        for entry in &mut actual {
            entry.flags = 0;
        }
        assert!(regular_file_dacl_preserved(&old, &actual));
        assert!(regular_file_dacl_preserved(&old, &old));
        assert!(regular_file_dacl_preserved(&actual, &actual));
        assert!(!regular_file_dacl_preserved(&actual, &old));
        for flags in [1, 2, 4, 8, 16, 0x1c, 0x20] {
            let mut before = ace(PRINCIPAL, 0x001f01ff);
            before.flags = flags | 3;
            let mut after = before.clone();
            after.flags = flags & !3;
            assert!(regular_file_dacl_preserved(&[before], &[after]));
        }
    }
    #[test]
    fn p1b_regular_file_dacl_refuses_every_other_acl_change() {
        let mut old = vec![ace("S-1-5-18", 0x001f01ff), ace(PRINCIPAL, 0x001f01ff)];
        for entry in &mut old {
            entry.flags = 3;
        }
        let mut actual = old.clone();
        actual.pop();
        assert!(!regular_file_dacl_preserved(&old, &actual));
        let mut stage = old.clone();
        stage.push(ace("S-1-5-18", 0x00120089));
        assert!(!regular_file_dacl_preserved(&old, &stage));
        actual = old.clone();
        actual.swap(0, 1);
        assert!(!regular_file_dacl_preserved(&old, &actual));
        actual = old.clone();
        actual[0].sid = "S-1-1-0".into();
        assert!(!regular_file_dacl_preserved(&old, &actual));
        actual = old.clone();
        actual[0].mask ^= 1;
        assert!(!regular_file_dacl_preserved(&old, &actual));
        actual = old.clone();
        actual[0].kind = 1;
        assert!(!regular_file_dacl_preserved(&old, &actual));
        for flags in [1, 2, 4, 8, 16, 0x20] {
            actual = old.clone();
            actual[0].flags = flags;
            assert!(!regular_file_dacl_preserved(&old, &actual));
        }
        for flags in [4, 8, 16, 0x20] {
            let mut before = old[0].clone();
            before.flags |= flags;
            let mut after = before.clone();
            after.flags = 0;
            assert!(!regular_file_dacl_preserved(&[before], &[after]));
        }
    }
    #[test]
    fn p1b_api_success_without_every_actual_evidence_does_not_issue_receipt() {
        let good = PublicationEvidence {
            api_success: true,
            old_name: true,
            old_object: true,
            final_object: true,
            exact_bytes: true,
            final_security: true,
            same_origin: true,
        };
        assert!(publication_admission(good).is_ok());
        for n in 0..7 {
            let mut bad = good;
            match n {
                0 => bad.api_success = false,
                1 => bad.old_name = false,
                2 => bad.old_object = false,
                3 => bad.final_object = false,
                4 => bad.exact_bytes = false,
                5 => bad.final_security = false,
                _ => bad.same_origin = false,
            }
            assert!(publication_admission(bad).is_err(), "evidence {n}");
        }
    }
    #[test]
    fn p1b_selected_path_is_bounded_fixed_leaf_and_actual_drive() {
        assert!(selected_components(Path::new(r"C:\Users\user\state.json")).is_ok());
        for path in [
            r"state.json",
            r"C:state.json",
            r"\\server\share\state.json",
            r"C:\Users\user\journal.json",
            r"C:\Users\..\state.json",
            r"C:\Users\user\state.json:stream",
            r"C:\state.json",
        ] {
            assert!(
                selected_components(Path::new(path)).is_err(),
                "unsupported shape"
            );
        }
        let path = format!("C:\\{}state.json", "child\\".repeat(65));
        assert!(selected_components(Path::new(&path)).is_err());
    }
    #[test]
    fn p1b_native_receipt_is_immutable_send_sync_evidence() {
        fn immutable<T: Send + Sync + Clone>() {}
        immutable::<TuningConfigReceipt>();
    }
    #[test]
    fn p1b_unresolved_relevant_stage_blocks_even_positive_leaf_absence() {
        for name in [
            ".state.json.tuning-previous",
            ".state.json.tuning-",
            ".STATE.JSON.TUNING-unknown",
        ] {
            assert!(relevant_publication_evidence(
                &name.encode_utf16().collect::<Vec<_>>()
            ));
        }
        assert!(!relevant_publication_evidence(
            &"unrelated-state".encode_utf16().collect::<Vec<_>>()
        ));
    }
    #[test]
    fn p1b_failed_retained_old_identity_never_adopts_a_fresh_leaf() {
        let id = FullObjectId {
            volume: 1,
            id: [1; 16],
        };
        assert!(old_leaf_admission(Err(TuningNativeError::Poisoned), id).is_err());
        assert!(old_leaf_admission(Ok(id), id).is_ok());
    }

    #[test]
    #[ignore = "Explicit normal-token owned-temp production ReplaceFile characterization"]
    fn p1b_owned_temp_atomic_source_characterization() -> Result<()> {
        if super::super::identity::elevated()? {
            return Err(TuningNativeError::Unsupported(
                "characterization requires normal token",
            ));
        }
        let mut root = super::super::test_support::TuningTemporaryDirectory::new()?;
        let root_path = root.path().to_owned();
        let original_root = root.original_identity();
        let mut retained_source = None;
        let run = (|| -> Result<()> {
            fn require(condition: bool, reason: &'static str) -> Result<()> {
                if condition {
                    Ok(())
                } else {
                    Err(TuningNativeError::UnsafeStore(reason))
                }
            }
            retained_source = Some(TuningConfigSource::open(&root_path.join(LEAF))?);
            let source = retained_source
                .as_mut()
                .ok_or(TuningNativeError::Poisoned)?;
            eprintln!("P1b characterization reached=source-opened");
            source.characterization_terminal = false;
            let initial_stage = source.prepare(b"original owned state")?;
            source.characterization_terminal = true;
            eprintln!("P1b characterization reached=initial-stage-created");
            require(
                matches!(source.read(), Err(TuningNativeError::Poisoned)),
                "characterization positive absence with staged evidence is blocked",
            )?;
            require(
                matches!(
                    TuningConfigSource::open(&root_path.join(LEAF)),
                    Err(TuningNativeError::Poisoned)
                ),
                "characterization reopen preserves staged evidence before absence",
            )?;
            let initial = source.finish_publication(None, None, initial_stage)?;
            eprintln!("P1b characterization reached=initial-publication-checked");
            let origin_id = source.directory_id;
            let origin_guid = source.volume_guid.clone();
            source.leaf = None;
            let leaf: Vec<_> = LEAF.encode_utf16().collect();
            let mut old = source.directory().config_leaf(&leaf, true)?;
            require(
                old.full_id()? == initial.file,
                "characterization old actual ID",
            )?;
            let old_dacl = old.config_dacl()?;
            source.verify()?;
            source.publication_evidence(None)?;
            let (mut stage_node, stage_name) = source
                .chain
                .last_mut()
                .expect("retained directory")
                .create_config_dacl_characterization_stage()?;
            source.record_characterization_creation(&mut stage_node, &stage_name)?;
            source.characterization_terminal = false;
            stage_node.write_synced(b"replacement owned state", LIMIT)?;
            source.record_characterization_bytes(&mut stage_node)?;
            source.characterization_terminal = true;
            let mut stage = OwnedStage {
                id: stage_node.full_id()?,
                node: stage_node,
                name: stage_name,
                bytes: Arc::from(b"replacement owned state".as_slice()),
            };
            let stage_dacl = stage.node.config_dacl()?;
            require(
                !regular_file_dacl_preserved(&old_dacl, &stage_dacl),
                "characterization old/stage DACLs must differ before replacement",
            )?;
            eprintln!("P1b characterization reached=distinct-dacl-stage-created");
            let stage_name = stage.name.clone();
            let stage_id = stage.id;
            require(
                stage_id != initial.file,
                "characterization distinct staged ID",
            )?;
            let destination = source.path(&leaf)?;
            let staged = source.path(&stage_name)?;
            source.characterization_terminal = false;
            require(
                matches!(
                    replace_primitive(&destination, &staged),
                    Err(TuningNativeError::Busy)
                ),
                "characterization retained stage must refuse sharing",
            )?;
            // A deliberate sharing refusal is accepted here only after both
            // exact owned names/handles remain unchanged. No source retry API.
            let mut named_old = source.directory().config_leaf(&leaf, true)?;
            require(
                named_old.full_id()? == initial.file && named_old.read(LIMIT)? == initial.bytes(),
                "characterization sharing refusal retained original name",
            )?;
            drop(named_old);
            require(
                old.read(LIMIT)? == initial.bytes() && old.config_dacl()? == old_dacl,
                "characterization sharing refusal retained old evidence",
            )?;
            require(
                stage.node.full_id()? == stage_id
                    && stage.node.read(LIMIT)? == stage.bytes.as_ref(),
                "characterization sharing refusal retained owned stage",
            )?;
            source.characterization_terminal = true;
            eprintln!("P1b characterization reached=held-stage-refusal-verified");
            let final_receipt = source.finish_publication(Some(old), Some(&initial), stage)?;
            eprintln!("P1b characterization reached=replacement-postconditions-checked");
            require(
                final_receipt.file == stage_id
                    && final_receipt.bytes() == b"replacement owned state",
                "characterization final staged ID/exact bytes",
            )?;
            require(
                source.directory_id == origin_id && source.volume_guid == origin_guid,
                "characterization stable directory origin",
            )?;
            require(
                regular_file_dacl_preserved(
                    &old_dacl,
                    &source
                        .leaf
                        .as_ref()
                        .ok_or(TuningNativeError::Poisoned)?
                        .config_dacl()?,
                ),
                "characterization transferred DACL",
            )?;
            require(
                matches!(
                    source.directory().config_leaf(&stage_name, true),
                    Err(TuningNativeError::Io { code: 2, .. })
                ),
                "characterization stage name absent",
            )?;
            require(
                source.directory().full_id()? == origin_id,
                "characterization owned root unchanged",
            )?;
            Ok(())
        })();
        // Success and failure share one cleanup attempt while actual context and
        // all creation proofs are still live. A failed deletion is never retried.
        let cleanup_verified = cleanup_owned_empty_after_error(
            || {
                if let Some(source) = retained_source.as_mut() {
                    let created = source.characterization_created.borrow().clone();
                    cleanup_created_files(source, &created, original_root)?;
                }
                retained_source = None;
                root.cleanup_empty()
            },
            || std::fs::symlink_metadata(&root_path).map(|_| ()),
        );
        if run.is_err() {
            eprintln!("P1b owned-temp error: cleanup_verified={cleanup_verified}");
        } else {
            eprintln!("P1b owned-temp success: cleanup_verified={cleanup_verified}");
        }
        if !cleanup_verified {
            root.retain();
            if run.is_ok() {
                return Err(TuningNativeError::UnsafeStore(
                    "characterization cleanup unverified",
                ));
            }
        }
        run
    }
}
