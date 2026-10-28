//! Bounded background-role and placement decisions. Native authority lives in Windows.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CATALOG_VERSION: u32 = 1;
pub const MAX_TARGETS: usize = 128;
pub const MAX_CPU_SETS: usize = 16384;
pub mod topology;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementWitness {
    pub topology: Vec<PlacementCpu>,
    pub baseline: Vec<u32>,
    pub desired: Vec<u32>,
}
impl PlacementWitness {
    pub fn valid(&self) -> bool {
        self.topology.len() <= 512
            && self.baseline.len() <= MAX_CPU_SETS
            && self.baseline.windows(2).all(|pair| pair[0] < pair[1])
            && self.desired.windows(2).all(|pair| pair[0] < pair[1])
            && self
                .baseline
                .iter()
                .all(|id| self.topology.iter().any(|cpu| cpu.id == *id))
            && validate_pool(&self.desired, &self.topology).is_ok()
            && preserves_restriction(&self.desired, &self.baseline).is_ok()
    }
    pub fn permits(&self, expected: &[u32], desired: &[u32]) -> bool {
        self.valid()
            && ((expected == self.baseline && desired == self.desired)
                || (expected == self.desired && desired == self.baseline))
    }
}
pub fn validate_placement(
    process: &crate::gaming::ProcessIdentity,
    witness: &PlacementWitness,
) -> Result<(), crate::gaming::GamingError> {
    #[cfg(windows)]
    {
        crate::windows::background::validate_placement(process, witness)
    }
    #[cfg(not(windows))]
    {
        let _ = (process, witness);
        Err(crate::gaming::GamingError::Unsupported(
            "Background CPU placement requires Windows",
        ))
    }
}
pub fn placement_topology(
    process: Option<&crate::gaming::ProcessIdentity>,
) -> Result<Vec<PlacementCpu>, crate::gaming::GamingError> {
    #[cfg(windows)]
    {
        crate::windows::background::placement_topology(process)
    }
    #[cfg(not(windows))]
    {
        let _ = process;
        Err(crate::gaming::GamingError::Unsupported(
            "Background CPU placement requires Windows",
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub volume: u32,
    pub index: u64,
    pub size: u64,
    pub modified: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RoleEvidence {
    Service {
        name: String,
        service_type: u32,
        arguments: String,
        dll_path: Option<String>,
        dll_identity: Option<FileIdentity>,
    },
    ScheduledTask {
        task_path: String,
        action_path: String,
        arguments: String,
        component_path: Option<String>,
        component_identity: Option<FileIdentity>,
    },
    Framework {
        installed_release: u32,
        root: String,
        service_name: Option<String>,
    },
}

/// Retained comparison evidence, never an authorization token. Every write must
/// independently regenerate and compare this witness on the live native target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetProof {
    pub catalog_version: u32,
    pub role: Role,
    pub pid: u32,
    pub creation_token: u64,
    pub image_path: String,
    pub sid: String,
    pub session: u32,
    pub logon: u64,
    pub os_build: u32,
    pub image: FileIdentity,
    pub context: RoleEvidence,
}
impl TargetProof {
    pub fn valid(&self) -> bool {
        let text = |value: &str| !value.is_empty() && value.len() <= 32767 && !value.contains('\0');
        self.catalog_version == CATALOG_VERSION
            && self.pid > 4
            && self.creation_token != 0
            && text(&self.image_path)
            && crate::gaming::valid_account_sid(&self.sid)
            && self.logon != 0
            && self.os_build >= 22000
            && self.image.index != 0
            && self.image.size != 0
            && match &self.context {
                RoleEvidence::Service {
                    name,
                    service_type,
                    arguments,
                    dll_path,
                    dll_identity,
                } => {
                    self.role
                        .service()
                        .is_some_and(|expected| expected.eq_ignore_ascii_case(name))
                        && matches!(*service_type, 0x10 | 0x20)
                        && arguments.len() <= 128
                        && !arguments.contains('\0')
                        && self.sid == "S-1-5-18"
                        && self.session == 0
                        && match self.role {
                            Role::DiagnosticsTelemetry => {
                                dll_path.as_deref().is_some_and(text)
                                    && dll_identity
                                        .is_some_and(|value| value.index != 0 && value.size != 0)
                            }
                            _ => dll_path.is_none() && dll_identity.is_none(),
                        }
                }
                RoleEvidence::ScheduledTask {
                    task_path,
                    action_path,
                    arguments,
                    component_path,
                    component_identity,
                } => {
                    matches!(self.role, Role::Appraiser | Role::DeviceCensus)
                        && self.sid == "S-1-5-18"
                        && self.session == 0
                        && text(task_path)
                        && action_path == &self.image_path
                        && arguments.len() <= 256
                        && !arguments.contains('\0')
                        && component_path.as_deref().is_none_or(text)
                        && component_path.is_some() == component_identity.is_some()
                }
                RoleEvidence::Framework {
                    installed_release,
                    root,
                    service_name,
                } => {
                    self.role == Role::FrameworkOptimizer
                        && *installed_release != 0
                        && text(root)
                        && service_name
                            .as_ref()
                            .is_none_or(|value| text(value) && value.len() <= 256)
                }
            }
    }
    pub fn matches(&self, process: &crate::gaming::ProcessIdentity) -> bool {
        self.valid()
            && self.pid == process.pid
            && self.creation_token == process.creation_token
            && self.image_path == process.image_path
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub role: Role,
    pub pid: u32,
    pub process: Option<crate::gaming::ProcessIdentity>,
    pub session: Option<u32>,
    pub verified: bool,
    pub proof: Option<TargetProof>,
    pub reason: String,
}

#[derive(Default)]
pub struct Resolver {
    #[cfg(windows)]
    native: crate::windows::background::Resolver,
}

impl Resolver {
    pub fn write_cpu_sets_bound(
        &mut self,
        process: &crate::gaming::ProcessIdentity,
        expected: &[u32],
        desired: &[u32],
        proof: &TargetProof,
        placement: &PlacementWitness,
    ) -> Result<(), crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native
                .write_cpu_sets_bound(process, expected, desired, proof, placement)
        }
        #[cfg(not(windows))]
        {
            let _ = (process, expected, desired, proof, placement);
            Err(crate::gaming::GamingError::Unsupported(
                "Background CPU placement requires Windows",
            ))
        }
    }
    /// Reuse the existing observation's bounded process enumeration.
    pub fn inspect_named(
        &mut self,
        names: &BTreeMap<u32, String>,
    ) -> Result<Vec<Candidate>, crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native.inspect_named(names)
        }
        #[cfg(not(windows))]
        {
            let _ = names;
            Err(crate::gaming::GamingError::Unsupported(
                "Background control requires Windows",
            ))
        }
    }
    pub fn capture_proof(
        &mut self,
        process: &crate::gaming::ProcessIdentity,
    ) -> Result<TargetProof, crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native.capture_proof(process)
        }
        #[cfg(not(windows))]
        {
            let _ = process;
            Err(crate::gaming::GamingError::Unsupported(
                "Background control requires Windows",
            ))
        }
    }
    pub fn write_priority_bound(
        &mut self,
        process: &crate::gaming::ProcessIdentity,
        expected: u32,
        desired: u32,
        proof: &TargetProof,
    ) -> Result<(), crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native
                .write_priority_bound(process, expected, desired, proof)
        }
        #[cfg(not(windows))]
        {
            let _ = (process, expected, desired, proof);
            Err(crate::gaming::GamingError::Unsupported(
                "Background control requires Windows",
            ))
        }
    }
    pub fn write_qos_bound(
        &mut self,
        process: &crate::gaming::ProcessIdentity,
        expected: crate::gaming::OwnedPolicy,
        desired: crate::gaming::OwnedPolicy,
        proof: &TargetProof,
    ) -> Result<(), crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native
                .write_qos_bound(process, expected, desired, proof)
        }
        #[cfg(not(windows))]
        {
            let _ = (process, expected, desired, proof);
            Err(crate::gaming::GamingError::Unsupported(
                "Background control requires Windows",
            ))
        }
    }
    pub fn qualify_target(
        &mut self,
        process: &crate::gaming::ProcessIdentity,
    ) -> Result<Role, crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native.qualify_target(process)
        }
        #[cfg(not(windows))]
        {
            let _ = process;
            Err(crate::gaming::GamingError::Unsupported(
                "Background control requires Windows",
            ))
        }
    }
    /// Explicit/active-session only. Construction performs no native work.
    pub fn inspect(&mut self) -> Result<Vec<Candidate>, crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native.inspect()
        }
        #[cfg(not(windows))]
        {
            Err(crate::gaming::GamingError::Unsupported(
                "Background control requires Windows",
            ))
        }
    }

    pub fn write_priority(
        &mut self,
        process: &crate::gaming::ProcessIdentity,
        expected: u32,
        desired: u32,
    ) -> Result<(), crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native.write_priority(process, expected, desired)
        }
        #[cfg(not(windows))]
        {
            let _ = (process, expected, desired);
            Err(crate::gaming::GamingError::Unsupported(
                "Background control requires Windows",
            ))
        }
    }

    pub fn write_qos(
        &mut self,
        process: &crate::gaming::ProcessIdentity,
        expected: crate::gaming::OwnedPolicy,
        desired: crate::gaming::OwnedPolicy,
    ) -> Result<(), crate::gaming::GamingError> {
        #[cfg(windows)]
        {
            self.native.write_qos(process, expected, desired)
        }
        #[cfg(not(windows))]
        {
            let _ = (process, expected, desired);
            Err(crate::gaming::GamingError::Unsupported(
                "Background control requires Windows",
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Appraiser,
    DeviceCensus,
    FrameworkOptimizer,
    DiagnosticsTelemetry,
    SearchIndexer,
    SearchProtocol,
    SearchFilter,
}

impl Role {
    /// Independent service/function qualification is deliberately separate from
    /// signed-image and live role verification. No production role has completed it yet.
    pub fn mutation_qualified(self) -> bool {
        false
    }
    pub fn qualification_note(self) -> &'static str {
        "System role not qualified for changes; controlled Windows VM validation is pending"
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Appraiser => "Windows compatibility assessment",
            Self::DeviceCensus => "Windows device census",
            Self::FrameworkOptimizer => ".NET Framework optimization",
            Self::DiagnosticsTelemetry => "Windows diagnostic telemetry",
            Self::SearchIndexer => "Windows Search indexing",
            Self::SearchProtocol => "Windows Search protocol host",
            Self::SearchFilter => "Windows Search filter host",
        }
    }
    pub fn base_candidate(self) -> bool {
        matches!(
            self,
            Self::Appraiser
                | Self::DeviceCensus
                | Self::FrameworkOptimizer
                | Self::DiagnosticsTelemetry
        )
    }
    pub fn service(self) -> Option<&'static str> {
        match self {
            Self::DiagnosticsTelemetry => Some("DiagTrack"),
            Self::SearchIndexer => Some("WSearch"),
            _ => None,
        }
    }
    pub fn permits_priority(self) -> bool {
        self != Self::FrameworkOptimizer
    }
}

/// A match starts qualification. It never authorizes a process write.
pub fn candidate_role(name: &str) -> Option<Role> {
    [
        ("CompatTelRunner.exe", Role::Appraiser),
        ("DeviceCensus.exe", Role::DeviceCensus),
        ("mscorsvw.exe", Role::FrameworkOptimizer),
        ("SearchIndexer.exe", Role::SearchIndexer),
        ("SearchProtocolHost.exe", Role::SearchProtocol),
        ("SearchFilterHost.exe", Role::SearchFilter),
    ]
    .into_iter()
    .find_map(|(expected, role)| name.eq_ignore_ascii_case(expected).then_some(role))
}

pub fn service_role(name: &str) -> Option<Role> {
    [Role::DiagnosticsTelemetry, Role::SearchIndexer]
        .into_iter()
        .find(|role| {
            role.service()
                .is_some_and(|expected| expected.eq_ignore_ascii_case(name))
        })
}

pub fn excluded_image(name: &str) -> bool {
    const EXCLUDED: &[&str] = &[
        "system",
        "idle",
        "registry",
        "smss.exe",
        "csrss.exe",
        "wininit.exe",
        "winlogon.exe",
        "services.exe",
        "lsass.exe",
        "dwm.exe",
        "explorer.exe",
        "shellexperiencehost.exe",
        "startmenuexperiencehost.exe",
        "searchhost.exe",
        "searchapp.exe",
        "audiodg.exe",
        "msmpeng.exe",
        "nissrv.exe",
        "sense.exe",
        "securityhealthservice.exe",
        "wudfhost.exe",
        "cpu-affinity-tool.exe",
        "obs64.exe",
        "obs32.exe",
        "discord.exe",
    ];
    EXCLUDED
        .iter()
        .any(|expected| name.eq_ignore_ascii_case(expected))
}

pub fn excluded_service(name: &str) -> bool {
    const EXCLUDED: &[&str] = &[
        "Audiosrv",
        "AudioEndpointBuilder",
        "MMCSS",
        "GameInputSvc",
        "GameInputRedistService",
        "RpcSs",
        "RpcEptMapper",
        "DcomLaunch",
        "PlugPlay",
        "Power",
        "EventLog",
        "Schedule",
        "Winmgmt",
        "BFE",
        "MpsSvc",
        "Dhcp",
        "Dnscache",
        "NlaSvc",
        "nsi",
        "WlanSvc",
        "WinDefend",
        "WdNisSvc",
        "Sense",
        "SecurityHealthService",
    ];
    EXCLUDED
        .iter()
        .any(|expected| name.eq_ignore_ascii_case(expected))
        || name.to_ascii_lowercase().starts_with("gameinput")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceMember {
    pub name: String,
    pub pid: u32,
    pub running: bool,
}

pub fn single_service_role(role: Role, pid: u32, members: &[ServiceMember]) -> bool {
    pid != 0
        && members.len() == 1
        && members[0].pid == pid
        && members[0].running
        && !excluded_service(&members[0].name)
        && role
            .service()
            .is_some_and(|name| name.eq_ignore_ascii_case(&members[0].name))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriorityDecision {
    LowerToBelowNormal,
    AlreadySuitable,
    Unexpected,
}

pub fn priority_change(raw: u32) -> PriorityDecision {
    match raw {
        0x20 => PriorityDecision::LowerToBelowNormal,
        0x40 | 0x4000 => PriorityDecision::AlreadySuitable,
        _ => PriorityDecision::Unexpected,
    }
}

/// Joined CPU Set and logical-processor topology; group is part of every relationship.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementCpu {
    pub id: u32,
    pub group: u16,
    pub logical: u8,
    pub physical_core: u32,
    pub efficiency_class: u8,
    pub numa: u32,
    pub cache: u32,
    pub allowed: bool,
    pub reserved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementSkip {
    Empty,
    UnknownSet,
    Unavailable,
    SplitPhysicalCore,
    Homogeneous,
    InvalidTopology,
    ExistingRestriction,
}

fn checked_topology(topology: &[PlacementCpu]) -> Result<(), PlacementSkip> {
    if topology.is_empty() || topology.len() > MAX_CPU_SETS {
        return Err(PlacementSkip::InvalidTopology);
    }
    let mut ids = BTreeSet::new();
    let mut logicals = BTreeSet::new();
    let mut classes = BTreeMap::new();
    for cpu in topology {
        if cpu.logical >= 64 || !ids.insert(cpu.id) || !logicals.insert((cpu.group, cpu.logical)) {
            return Err(PlacementSkip::InvalidTopology);
        }
        if classes
            .insert((cpu.group, cpu.physical_core), cpu.efficiency_class)
            .is_some_and(|previous| previous != cpu.efficiency_class)
        {
            return Err(PlacementSkip::InvalidTopology);
        }
    }
    Ok(())
}

pub fn validate_pool(ids: &[u32], topology: &[PlacementCpu]) -> Result<(), PlacementSkip> {
    if ids.is_empty() {
        return Err(PlacementSkip::Empty);
    }
    checked_topology(topology)?;
    let selected: BTreeSet<_> = ids.iter().copied().collect();
    if selected.len() != ids.len() || ids.len() > MAX_CPU_SETS {
        return Err(PlacementSkip::UnknownSet);
    }
    let by_id: BTreeMap<_, _> = topology.iter().map(|cpu| (cpu.id, cpu)).collect();
    let mut cores = BTreeSet::new();
    for id in ids {
        let cpu = by_id.get(id).ok_or(PlacementSkip::UnknownSet)?;
        if !cpu.allowed || cpu.reserved {
            return Err(PlacementSkip::Unavailable);
        }
        cores.insert((cpu.group, cpu.physical_core));
    }
    if topology
        .iter()
        .any(|cpu| cores.contains(&(cpu.group, cpu.physical_core)) && !selected.contains(&cpu.id))
    {
        return Err(PlacementSkip::SplitPhysicalCore);
    }
    Ok(())
}

pub fn efficient_pool(topology: &[PlacementCpu]) -> Result<Vec<u32>, PlacementSkip> {
    checked_topology(topology)?;
    let classes: BTreeSet<_> = topology.iter().map(|cpu| cpu.efficiency_class).collect();
    if classes.len() < 2 {
        return Err(PlacementSkip::Homogeneous);
    }
    let minimum = *classes.first().unwrap();
    let ids: Vec<_> = topology
        .iter()
        .filter(|cpu| cpu.efficiency_class == minimum)
        .map(|cpu| cpu.id)
        .collect();
    validate_pool(&ids, topology)?;
    let mut ids = ids;
    ids.sort_unstable();
    Ok(ids)
}

pub fn preserves_restriction(desired: &[u32], existing: &[u32]) -> Result<(), PlacementSkip> {
    if !existing.is_empty() && desired.iter().any(|id| !existing.contains(id)) {
        return Err(PlacementSkip::ExistingRestriction);
    }
    Ok(())
}

/// Display/cache fingerprint only. Writes must compare complete fresh topology as well.
pub fn topology_fingerprint(topology: &[PlacementCpu]) -> u64 {
    let mut sorted: Vec<_> = topology.iter().collect();
    sorted.sort_unstable_by_key(|cpu| cpu.id);
    let mut hash = 0xcbf29ce484222325u64;
    for cpu in sorted {
        for bytes in [
            cpu.id.to_le_bytes().as_slice(),
            cpu.group.to_le_bytes().as_slice(),
            &[cpu.logical],
            cpu.physical_core.to_le_bytes().as_slice(),
            &[cpu.efficiency_class],
            cpu.numa.to_le_bytes().as_slice(),
            cpu.cache.to_le_bytes().as_slice(),
            &[u8::from(cpu.allowed), u8::from(cpu.reserved)],
        ] {
            for byte in bytes {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
        }
    }
    hash
}

#[cfg(test)]
mod tests;
