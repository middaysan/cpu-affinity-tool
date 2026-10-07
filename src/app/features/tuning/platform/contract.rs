//! Runtime values are typed; constructors validate shape, not OS authenticity.
//! Native adapters must supply observed identities and revalidate them at each write.

use serde::{Deserialize, Serialize};
use std::num::{NonZeroU32, NonZeroU64};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidIdentity;

impl std::fmt::Display for InvalidIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid identity")
    }
}
impl std::error::Error for InvalidIdentity {}

fn stable_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

macro_rules! text_identity {
    ($name:ident, $validate:expr) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdentity> {
                let value = value.into();
                if ($validate)(&value) {
                    Ok(Self(value))
                } else {
                    Err(InvalidIdentity)
                }
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = InvalidIdentity;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

text_identity!(FeatureId, stable_name);
// Names identify OS settings independently of the feature implementing them.
text_identity!(ResourceId, stable_name);

fn canonical_decimal(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|c| c.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}
fn valid_sid(value: &str) -> bool {
    let parts: Vec<_> = value.split('-').collect();
    (4..=18).contains(&parts.len())
        && parts[0] == "S"
        && parts[1] == "1"
        && canonical_decimal(parts[2])
        && parts[2]
            .parse::<u64>()
            .is_ok_and(|authority| authority < (1_u64 << 48))
        && parts[3..]
            .iter()
            .all(|part| canonical_decimal(part) && part.parse::<u32>().is_ok())
}
text_identity!(UserSid, valid_sid);

fn valid_image_path(value: &str) -> bool {
    if value.len() > 32767 || value.contains(['\0', '/']) || value.ends_with('\\') {
        return false;
    }
    let bytes = value.as_bytes();
    let drive_absolute =
        bytes.len() >= 4 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    let unc_absolute = value.starts_with("\\\\") && value[2..].split('\\').count() >= 3;
    let tail = if drive_absolute {
        &value[3..]
    } else if unc_absolute {
        &value[2..]
    } else {
        return false;
    };
    tail.split('\\')
        .all(|part| !part.is_empty() && part != "." && part != "..")
}
// This is a normalized absolute Windows image-path shape, not a path lookup.
text_identity!(ImagePath, valid_image_path);
text_identity!(DeviceIdentity, |value: &str| !value.is_empty()
    && value.len() <= 4096
    && !value.contains('\0')
    && value.trim() == value);

macro_rules! numeric_identity {
    ($name:ident, $raw:ty, $nonzero:ty) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name($nonzero);
        impl $name {
            pub fn new(value: $raw) -> Result<Self, InvalidIdentity> {
                <$nonzero>::new(value).map(Self).ok_or(InvalidIdentity)
            }
            pub fn get(self) -> $raw {
                self.0.get()
            }
        }
    };
}
numeric_identity!(ProfileId, u64, NonZeroU64);
numeric_identity!(OwnerId, u64, NonZeroU64);
numeric_identity!(OperationId, u64, NonZeroU64);
numeric_identity!(LogonId, u64, NonZeroU64);
numeric_identity!(SchemaVersion, u32, NonZeroU32);
numeric_identity!(ConfigGeneration, u64, NonZeroU64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
enum ContinuityProducer {
    #[serde(rename = "windows_hklm_software_volatile_marker")]
    ProtectedHive,
    #[cfg(test)]
    #[serde(rename = "test_continuity_identity")]
    Test,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
enum ContinuityDomain {
    #[serde(rename = "cpu_affinity_tool_tuning")]
    Tuning,
    #[cfg(test)]
    #[serde(rename = "private_fixture")]
    Test,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
enum ContinuityView {
    #[serde(rename = "native64")]
    Native64,
    #[cfg(test)]
    #[serde(rename = "fixture")]
    Test,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootIdWire {
    producer: ContinuityProducer,
    version: SchemaVersion,
    domain: ContinuityDomain,
    view: ContinuityView,
    nonce: [u8; 32],
}

/// Protected-hive continuity data, not an OS-provided boot UUID or authority.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "BootIdWire", into = "BootIdWire")]
pub struct BootId(BootIdWire);

impl BootId {
    /// Shape only: a protected native producer must authenticate continuity.
    pub fn observed_nonce(nonce: [u8; 32]) -> Result<Self, InvalidIdentity> {
        BootIdWire {
            producer: ContinuityProducer::ProtectedHive,
            version: SchemaVersion::new(1)?,
            domain: ContinuityDomain::Tuning,
            view: ContinuityView::Native64,
            nonce,
        }
        .try_into()
    }
    #[cfg(test)]
    pub fn new(value: [u8; 16]) -> Result<Self, InvalidIdentity> {
        // Synthetic test-domain values never originate from a native marker.
        let mut nonce = [0; 32];
        nonce[..16].copy_from_slice(&value);
        nonce[16..].copy_from_slice(&value);
        BootIdWire {
            producer: ContinuityProducer::Test,
            version: SchemaVersion::new(1)?,
            domain: ContinuityDomain::Test,
            view: ContinuityView::Test,
            nonce,
        }
        .try_into()
    }
    pub fn bytes(self) -> [u8; 32] {
        self.0.nonce
    }
}
impl TryFrom<BootIdWire> for BootId {
    type Error = InvalidIdentity;
    fn try_from(wire: BootIdWire) -> Result<Self, Self::Error> {
        let valid_domain = matches!(
            (wire.producer, wire.domain, wire.view),
            (
                ContinuityProducer::ProtectedHive,
                ContinuityDomain::Tuning,
                ContinuityView::Native64
            )
        );
        #[cfg(test)]
        let valid_domain = valid_domain
            || matches!(
                (wire.producer, wire.domain, wire.view),
                (
                    ContinuityProducer::Test,
                    ContinuityDomain::Test,
                    ContinuityView::Test
                )
            );
        if !valid_domain || wire.version.get() != 1 || wire.nonce == [0; 32] {
            return Err(InvalidIdentity);
        }
        Ok(Self(wire))
    }
}
impl From<BootId> for BootIdWire {
    fn from(value: BootId) -> Self {
        value.0
    }
}
impl std::fmt::Debug for BootId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootId")
            .field("producer", &self.0.producer)
            .field("view", &self.0.view)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod boot_continuity_tests {
    use super::*;

    #[test]
    fn boot_continuity_preserves_full_nonce_domain_and_wire() {
        let first = [9; 32];
        let mut second = first;
        second[31] = 10;
        let identity = BootId::observed_nonce(first).unwrap();
        assert_ne!(identity, BootId::observed_nonce(second).unwrap());
        let encoded = serde_json::to_value(identity).unwrap();
        assert_eq!(encoded["producer"], "windows_hklm_software_volatile_marker");
        assert_eq!(encoded["domain"], "cpu_affinity_tool_tuning");
        assert_eq!(encoded["view"], "native64");
        assert_eq!(encoded["version"], 1);
        assert_eq!(encoded["nonce"], serde_json::json!(first));
        assert_eq!(serde_json::from_value::<BootId>(encoded).unwrap(), identity);
        let fixture = BootId::new([9; 16]).unwrap();
        assert_ne!(identity, fixture);
        assert_eq!(
            serde_json::to_value(fixture).unwrap()["producer"],
            "test_continuity_identity"
        );
    }

    #[test]
    fn boot_continuity_rejects_legacy_arrays_and_unknown_wire() {
        let legacy = [2; 16];
        let nonce = [2; 32];
        let zero = [0; 32];
        assert!(serde_json::from_value::<BootId>(serde_json::json!(legacy)).is_err());
        assert!(BootId::observed_nonce([0; 32]).is_err());
        let wire = serde_json::json!({
            "producer": "windows_hklm_software_volatile_marker",
            "domain": "cpu_affinity_tool_tuning",
            "view": "native64",
            "version": 1,
            "nonce": nonce,
        });
        for (field, wrong) in [
            ("producer", serde_json::json!("opaque_boot_uuid")),
            ("domain", serde_json::json!("private_fixture")),
            ("view", serde_json::json!("native32")),
            ("version", serde_json::json!(2)),
            ("nonce", serde_json::json!(legacy)),
            ("nonce", serde_json::json!(zero)),
        ] {
            let mut invalid = wire.clone();
            invalid[field] = wrong;
            assert!(
                serde_json::from_value::<BootId>(invalid).is_err(),
                "{field}"
            );
        }
        let mut extra = wire;
        extra["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<BootId>(extra).is_err());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MachineIdSource {
    Tpm,
    Uefi,
    #[cfg(test)]
    Test,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "MachineIdWire", into = "MachineIdWire")]
pub struct MachineId(std::sync::Arc<MachineIdWire>);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MachineIdWire {
    producer: ResourceId,
    version: SchemaVersion,
    source: MachineIdSource,
    bytes: Vec<u8>,
}
impl MachineId {
    /// Shape only. Native producer must supply and revalidate all observed bytes.
    pub fn observed(source: MachineIdSource, bytes: Vec<u8>) -> Result<Self, InvalidIdentity> {
        MachineIdWire {
            producer: ResourceId::new("windows_publisher_system_id_unpackaged_win32")?,
            version: SchemaVersion::new(1)?,
            source,
            bytes,
        }
        .try_into()
    }
    #[cfg(test)]
    pub fn new(bytes: [u8; 16]) -> Result<Self, InvalidIdentity> {
        MachineIdWire {
            producer: ResourceId::new("test_machine_identity")?,
            version: SchemaVersion::new(1)?,
            source: MachineIdSource::Test,
            bytes: bytes.to_vec(),
        }
        .try_into()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0.bytes
    }
    pub fn source(&self) -> MachineIdSource {
        self.0.source
    }
    pub fn producer(&self) -> &ResourceId {
        &self.0.producer
    }
}
impl TryFrom<MachineIdWire> for MachineId {
    type Error = InvalidIdentity;
    fn try_from(wire: MachineIdWire) -> Result<Self, Self::Error> {
        let valid_domain = wire.producer.as_str() == "windows_publisher_system_id_unpackaged_win32"
            && matches!(wire.source, MachineIdSource::Tpm | MachineIdSource::Uefi);
        #[cfg(test)]
        let valid_domain = valid_domain
            || (wire.producer.as_str() == "test_machine_identity"
                && wire.source == MachineIdSource::Test);
        if !valid_domain
            || wire.version.get() != 1
            || wire.bytes.is_empty()
            || wire.bytes.len() > 4096
            || wire.bytes.iter().all(|byte| *byte == 0)
        {
            return Err(InvalidIdentity);
        }
        Ok(Self(std::sync::Arc::new(wire)))
    }
}
impl From<MachineId> for MachineIdWire {
    fn from(value: MachineId) -> Self {
        std::sync::Arc::unwrap_or_clone(value.0)
    }
}

#[cfg(test)]
mod machine_binding_tests {
    use super::*;
    #[test]
    fn machine_binding_preserves_native_source_domain_and_entire_variable_buffer() {
        let wire = serde_json::json!({"producer":"windows_publisher_system_id_unpackaged_win32", "version":1, "source":"Tpm", "bytes":vec![17u8; 27]});
        let parsed = serde_json::from_value::<MachineId>(wire.clone());
        assert!(
            parsed.is_ok(),
            "native machine buffers have no documented fixed16 length"
        );
        assert_eq!(serde_json::to_value(parsed.unwrap()).unwrap(), wire);
    }
    #[test]
    fn machine_binding_rejects_legacy_array_unknown_source_domain_and_invalid_buffers() {
        assert!(serde_json::from_value::<MachineId>(serde_json::json!(vec![1u8; 16])).is_err());
        let valid = serde_json::json!({"producer":"windows_publisher_system_id_unpackaged_win32", "version":1, "source":"Tpm", "bytes":vec![17u8; 27]});
        for (field, value) in [
            ("producer", serde_json::json!("other_producer")),
            ("version", serde_json::json!(2)),
            ("source", serde_json::json!("Registry")),
            ("bytes", serde_json::json!([])),
            ("bytes", serde_json::json!([0])),
            ("bytes", serde_json::json!(vec![1u8; 4097])),
        ] {
            let mut wire = valid.clone();
            wire[field] = value;
            assert!(serde_json::from_value::<MachineId>(wire).is_err());
        }
        let mut uefi = valid.clone();
        uefi["source"] = serde_json::json!("Uefi");
        assert_ne!(
            serde_json::from_value::<MachineId>(valid).unwrap(),
            serde_json::from_value::<MachineId>(uefi).unwrap()
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineIdentity {
    pub id: MachineId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserIdentity {
    pub machine: MachineIdentity,
    pub sid: UserSid,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionIdentity {
    pub user: UserIdentity,
    pub session_id: u32,
    pub logon: LogonId,
    pub boot: BootId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub session: SessionIdentity,
    pid: NonZeroU32,
    creation_token: NonZeroU64,
    pub image_path: ImagePath,
}
impl ProcessIdentity {
    pub fn new(
        session: SessionIdentity,
        pid: u32,
        creation_token: u64,
        image_path: ImagePath,
    ) -> Result<Self, InvalidIdentity> {
        Ok(Self {
            session,
            pid: NonZeroU32::new(pid).ok_or(InvalidIdentity)?,
            creation_token: NonZeroU64::new(creation_token).ok_or(InvalidIdentity)?,
            image_path,
        })
    }
    pub fn pid(&self) -> u32 {
        self.pid.get()
    }
    pub fn creation_token(&self) -> u64 {
        self.creation_token.get()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TargetScope {
    Machine,
    User,
    Session,
    Process,
    PnpDevice,
    Gpu,
    Display,
}
impl TargetScope {
    pub fn can_project_to(self, resource: Self) -> bool {
        self == resource
            || match self {
                Self::Process => matches!(resource, Self::Session | Self::User | Self::Machine),
                Self::Session => matches!(resource, Self::User | Self::Machine),
                Self::User | Self::PnpDevice | Self::Gpu | Self::Display => {
                    resource == Self::Machine
                }
                Self::Machine => false,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TargetIdentity {
    Machine(MachineIdentity),
    User(UserIdentity),
    Session(SessionIdentity),
    Process(ProcessIdentity),
    PnpDevice {
        machine: MachineIdentity,
        device: DeviceIdentity,
    },
    Gpu {
        machine: MachineIdentity,
        adapter: DeviceIdentity,
    },
    Display {
        machine: MachineIdentity,
        display: DeviceIdentity,
    },
}
impl TargetIdentity {
    pub fn scope(&self) -> TargetScope {
        match self {
            Self::Machine(_) => TargetScope::Machine,
            Self::User(_) => TargetScope::User,
            Self::Session(_) => TargetScope::Session,
            Self::Process(_) => TargetScope::Process,
            Self::PnpDevice { .. } => TargetScope::PnpDevice,
            Self::Gpu { .. } => TargetScope::Gpu,
            Self::Display { .. } => TargetScope::Display,
        }
    }
    /// Exact identity projection, permitting declared settings at an ancestor
    /// scope without accepting another machine/user/session's resource key.
    pub fn resource_target(&self, scope: TargetScope) -> Option<Self> {
        if self.scope() == scope {
            return Some(self.clone());
        }
        match self {
            Self::Process(process) => Self::Session(process.session.clone()).resource_target(scope),
            Self::Session(session) => Self::User(session.user.clone()).resource_target(scope),
            Self::User(user) if scope == TargetScope::Machine => {
                Some(Self::Machine(user.machine.clone()))
            }
            Self::PnpDevice { machine, .. }
            | Self::Gpu { machine, .. }
            | Self::Display { machine, .. }
                if scope == TargetScope::Machine =>
            {
                Some(Self::Machine(machine.clone()))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredIntent {
    pub profile: ProfileId,
    pub generation: ConfigGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleActivationWire {
    group: crate::app::shared::ids::GroupId,
    rule: crate::app::shared::ids::RuleId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RuleActivationWire", into = "RuleActivationWire")]
pub struct RuleActivation(RuleActivationWire);
impl RuleActivation {
    pub fn new(
        group: crate::app::shared::ids::GroupId,
        rule: crate::app::shared::ids::RuleId,
    ) -> Result<Self, InvalidIdentity> {
        RuleActivationWire { group, rule }.try_into()
    }
    pub fn group(&self) -> &crate::app::shared::ids::GroupId {
        &self.0.group
    }
    pub fn rule(&self) -> &crate::app::shared::ids::RuleId {
        &self.0.rule
    }
}
impl TryFrom<RuleActivationWire> for RuleActivation {
    type Error = InvalidIdentity;
    fn try_from(wire: RuleActivationWire) -> Result<Self, Self::Error> {
        let valid =
            |id: &str| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control);
        if valid(&wire.group.0) && valid(&wire.rule.0) {
            Ok(Self(wire))
        } else {
            Err(InvalidIdentity)
        }
    }
}
impl From<RuleActivation> for RuleActivationWire {
    fn from(value: RuleActivation) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationEligibility {
    Running,
    Foreground,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GameActivationWire {
    configured: ConfiguredIntent,
    rule: RuleActivation,
    eligibility: ActivationEligibility,
    session: SessionIdentity,
    processes: Vec<ProcessIdentity>,
}
/// Durable shape only. Native authenticity and complete eligibility must come
/// from the execution/foreground producer; deserialization grants no authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "GameActivationWire", into = "GameActivationWire")]
pub struct GameActivation(std::sync::Arc<GameActivationWire>);
impl GameActivation {
    pub fn new(
        configured: ConfiguredIntent,
        rule: RuleActivation,
        eligibility: ActivationEligibility,
        session: SessionIdentity,
        processes: Vec<ProcessIdentity>,
    ) -> Result<Self, InvalidIdentity> {
        GameActivationWire {
            configured,
            rule,
            eligibility,
            session,
            processes,
        }
        .try_into()
    }
    pub fn configured(&self) -> ConfiguredIntent {
        self.0.configured
    }
    pub fn rule(&self) -> &RuleActivation {
        &self.0.rule
    }
    pub fn eligibility(&self) -> ActivationEligibility {
        self.0.eligibility
    }
    pub fn session(&self) -> &SessionIdentity {
        &self.0.session
    }
    pub fn processes(&self) -> &[ProcessIdentity] {
        &self.0.processes
    }
}
impl TryFrom<GameActivationWire> for GameActivation {
    type Error = InvalidIdentity;
    fn try_from(mut wire: GameActivationWire) -> Result<Self, Self::Error> {
        if wire.processes.is_empty()
            || wire.processes.len() > 64
            || wire
                .processes
                .iter()
                .any(|process| process.session != wire.session)
        {
            return Err(InvalidIdentity);
        }
        wire.processes.sort();
        let mut pids = std::collections::BTreeSet::new();
        if wire
            .processes
            .iter()
            .any(|process| !pids.insert(process.pid()))
        {
            return Err(InvalidIdentity);
        }
        struct Budget(usize);
        impl std::io::Write for Budget {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self
                    .0
                    .checked_add(bytes.len())
                    .filter(|count| *count <= 64 * 1024)
                    .ok_or_else(|| std::io::Error::other("activation binding capacity"))?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(&mut Budget(0), &wire).map_err(|_| InvalidIdentity)?;
        Ok(Self(std::sync::Arc::new(wire)))
    }
}
impl From<GameActivation> for GameActivationWire {
    fn from(value: GameActivation) -> Self {
        std::sync::Arc::unwrap_or_clone(value.0)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ActivationBinding {
    #[default]
    Unknown,
    Manual(ConfiguredIntent),
    Game(GameActivation),
}
impl ActivationBinding {
    pub fn configured(&self) -> Option<ConfiguredIntent> {
        match self {
            Self::Unknown => None,
            Self::Manual(intent) => Some(*intent),
            Self::Game(game) => Some(game.configured()),
        }
    }
    pub(super) fn permits(&self, mode: ActivationMode, target: &TargetIdentity) -> bool {
        match self {
            Self::Unknown => true,
            Self::Manual(_) => mode == ActivationMode::Manual,
            Self::Game(game) if mode == ActivationMode::Session => match target {
                TargetIdentity::Machine(machine)
                | TargetIdentity::PnpDevice { machine, .. }
                | TargetIdentity::Gpu { machine, .. }
                | TargetIdentity::Display { machine, .. } => {
                    game.session().user.machine == *machine
                }
                TargetIdentity::User(user) => game.session().user == *user,
                TargetIdentity::Session(session) => game.session() == session,
                TargetIdentity::Process(process) => game.processes().contains(process),
            },
            _ => false,
        }
    }
    pub(super) fn succeeds(&self, previous: &Self) -> bool {
        let same_actor = match (self, previous) {
            (Self::Unknown, Self::Unknown) => true,
            (Self::Manual(new), Self::Manual(old)) => new.profile == old.profile,
            (Self::Game(new), Self::Game(old)) => {
                new.configured().profile == old.configured().profile
                    && new.rule() == old.rule()
                    && new.eligibility() == old.eligibility()
                    && new.session() == old.session()
                    && new.processes() == old.processes()
            }
            _ => false,
        };
        same_actor
            && match (self.configured(), previous.configured()) {
                (Some(new), Some(old)) => new.generation >= old.generation,
                (None, None) => true,
                _ => false,
            }
    }
}

#[cfg(test)]
mod activation_binding_tests {
    use super::*;
    use crate::app::shared::ids::{GroupId, RuleId};
    fn configured() -> ConfiguredIntent {
        ConfiguredIntent {
            profile: ProfileId::new(1).unwrap(),
            generation: ConfigGeneration::new(1).unwrap(),
        }
    }
    fn session() -> SessionIdentity {
        SessionIdentity {
            user: UserIdentity {
                machine: MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                },
                sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(17).unwrap(),
            boot: BootId::new([2; 16]).unwrap(),
        }
    }
    fn process(pid: u32) -> ProcessIdentity {
        ProcessIdentity::new(
            session(),
            pid,
            123,
            ImagePath::new("C:\\Games\\game.exe").unwrap(),
        )
        .unwrap()
    }
    fn game(processes: Vec<ProcessIdentity>) -> Result<GameActivation, InvalidIdentity> {
        GameActivation::new(
            configured(),
            RuleActivation::new(GroupId("group-0".into()), RuleId("rule-0".into()))?,
            ActivationEligibility::Running,
            session(),
            processes,
        )
    }
    #[test]
    fn explicit_manual_and_exact_game_binding_round_trip() {
        let manual = ActivationBinding::Manual(configured());
        assert_eq!(
            serde_json::from_value::<ActivationBinding>(serde_json::to_value(&manual).unwrap())
                .unwrap(),
            manual
        );
        let original = ActivationBinding::Game(
            game(vec![process(43), process(42)]).expect("valid complete process set"),
        );
        assert_eq!(
            serde_json::from_value::<ActivationBinding>(serde_json::to_value(&original).unwrap())
                .unwrap(),
            original
        );
        let ActivationBinding::Game(game) = original else {
            unreachable!()
        };
        assert_eq!(game.processes()[0].pid(), 42);
    }
    #[test]
    fn game_binding_rejects_empty_duplicate_mixed_session_and_oversized_sets() {
        let valid = game(vec![process(42)]).expect("one eligible verified process");
        let mut wire = serde_json::to_value(&valid).unwrap();
        for processes in [
            vec![],
            vec![process(42), process(42)],
            (1..=65).map(process).collect(),
        ] {
            wire["processes"] = serde_json::to_value(processes).unwrap();
            assert!(serde_json::from_value::<GameActivation>(wire.clone()).is_err());
        }
        let mut other = process(43);
        other.session.logon = LogonId::new(18).unwrap();
        assert!(game(vec![process(42), other]).is_err());
        let alias = ProcessIdentity::new(
            session(),
            42,
            123,
            ImagePath::new("C:\\Games\\other.exe").unwrap(),
        )
        .unwrap();
        assert!(game(vec![process(42), alias]).is_err());
    }
    #[test]
    fn shared_rule_ids_keep_existing_identity_and_validate_bounds_on_decode() {
        let rule =
            RuleActivation::new(GroupId("legitimate-id:old".into()), RuleId("rule-0".into()))
                .expect("opaque logical IDs remain readable");
        let mut wire = serde_json::to_value(rule).unwrap();
        for invalid in [
            "".to_owned(),
            "x".repeat(129),
            "rule\0path".into(),
            "rule\n".into(),
        ] {
            wire["rule"] = serde_json::json!(invalid);
            assert!(serde_json::from_value::<RuleActivation>(wire.clone()).is_err());
        }
        assert!(ConfigGeneration::new(0).is_err());
    }
    #[test]
    fn activation_binding_budget_and_reused_pid_aliases_are_rejected() {
        let reused = ProcessIdentity::new(
            session(),
            42,
            124,
            ImagePath::new("C:\\Games\\game.exe").unwrap(),
        )
        .unwrap();
        assert!(game(vec![process(42), reused]).is_err());
        let long = ImagePath::new(format!("C:\\Games\\{}.exe", "x".repeat(32000))).unwrap();
        let processes = (1..=3)
            .map(|pid| ProcessIdentity::new(session(), pid, 123, long.clone()).unwrap())
            .collect();
        assert!(
            game(processes).is_err(),
            "a bounded queued binding cannot retain megabytes of repeated provenance"
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceKey {
    pub target: TargetIdentity,
    pub field: ResourceId,
}

/// Exact module-produced canonical desired bytes. The ownership core compares
/// these tokens without interpreting them. Different semantics/versions conflict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DesiredStateWire", into = "DesiredStateWire")]
pub struct DesiredStateToken {
    pub semantics: ResourceId,
    pub version: SchemaVersion,
    bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesiredStateWire {
    semantics: ResourceId,
    version: SchemaVersion,
    bytes: Vec<u8>,
}
impl TryFrom<DesiredStateWire> for DesiredStateToken {
    type Error = InvalidIdentity;
    fn try_from(value: DesiredStateWire) -> Result<Self, Self::Error> {
        Self::new(value.semantics, value.version, value.bytes)
    }
}
impl From<DesiredStateToken> for DesiredStateWire {
    fn from(value: DesiredStateToken) -> Self {
        Self {
            semantics: value.semantics,
            version: value.version,
            bytes: value.bytes,
        }
    }
}
impl DesiredStateToken {
    pub fn new(
        semantics: ResourceId,
        version: SchemaVersion,
        bytes: Vec<u8>,
    ) -> Result<Self, InvalidIdentity> {
        if bytes.is_empty() || bytes.len() > 4096 {
            return Err(InvalidIdentity);
        }
        Ok(Self {
            semantics,
            version,
            bytes,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceClaim {
    pub key: ResourceKey,
    pub desired: DesiredStateToken,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceObservation {
    pub key: ResourceKey,
    pub value: DesiredStateToken,
}

/// Orchestration-only exact evidence. Modules and the shell cannot construct it.
pub(super) struct ExpectedCurrentProof {
    states: Vec<ResourceObservation>,
}
impl ExpectedCurrentProof {
    pub(super) fn new(states: Vec<ResourceObservation>) -> Self {
        Self { states }
    }
    pub(super) fn states(&self) -> &[ResourceObservation] {
        &self.states
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActivationMode {
    Manual,
    Session,
    ForegroundSession,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementChoice {
    #[default]
    Unmanaged,
    Managed(ActivationMode),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleAccess {
    ReadOnly,
    Managed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Immediate,
    GameRestartRequired,
    RestartRequired,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capability {
    Available,
    Unsupported(String),
    Denied(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidConfig,
    Unsupported,
    Denied,
    Read,
    Write,
    Verification,
    ExternalChange,
    InvalidPlan,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeError {
    pub domain: ResourceId,
    pub code: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleError {
    pub kind: ErrorKind,
    pub operation: &'static str,
    pub native: Option<NativeError>,
    detail: Option<String>,
}
impl ModuleError {
    pub fn new(kind: ErrorKind, operation: &'static str) -> Self {
        Self {
            kind,
            operation,
            native: None,
            detail: None,
        }
    }
    pub fn with_native(mut self, native: NativeError) -> Self {
        self.native = Some(native);
        self
    }
    pub fn with_detail(mut self, detail: impl Into<String>) -> Result<Self, InvalidIdentity> {
        let detail = detail.into();
        if detail.len() > 1024 || detail.contains('\0') {
            return Err(InvalidIdentity);
        }
        self.detail = Some(detail);
        Ok(self)
    }
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceDeclaration {
    pub field: ResourceId,
    pub scope: TargetScope,
}
#[derive(Clone, Debug)]
pub struct ModuleDescriptor {
    pub id: FeatureId,
    pub config_version: SchemaVersion,
    pub snapshot_version: SchemaVersion,
    pub title: &'static str,
    pub description: &'static str,
    pub access: ModuleAccess,
    pub target_scopes: Vec<TargetScope>,
    pub resources: Vec<ResourceDeclaration>,
    pub activation_modes: Vec<ActivationMode>,
    pub effect: Effect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleStatus {
    Unmanaged,
    Unsupported,
    PermissionDenied,
    Applying,
    VerifiedActive(Effect),
    PartialFailure,
    Restoring,
    ExternalChange,
    RecoveryRequired,
    StoredRestartRequired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateMatch {
    Baseline,
    /// Index into the snapshot's known native states, not an inferred completed step count.
    WrittenStep(usize),
    ExternalChange,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verification {
    Verified(Effect),
    Mismatch,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationSummary {
    pub summary: String,
    pub effect: Effect,
}

/// Typed plan; snapshot retains the first original baseline and known written
/// states. Zero steps is a recoverable no-op, never an invented default.
pub struct Plan<Snapshot, Step> {
    pub target: TargetIdentity,
    pub claims: Vec<ResourceClaim>,
    pub snapshot: Snapshot,
    pub steps: Vec<PlannedStep<Step>>,
    pub effect: Effect,
}

pub struct PlannedStep<Step> {
    /// Every field that this native instruction can write, including rollback
    /// side effects. The runner uses these for prewrite journaling before native
    /// execution; durable journaling arrives in F3.
    pub writes: Vec<ResourceKey>,
    /// Exact states this instruction can establish, projected from its typed
    /// native values. They are staged as possible evidence before execution.
    pub after: Vec<ResourceObservation>,
    pub instruction: Step,
}
