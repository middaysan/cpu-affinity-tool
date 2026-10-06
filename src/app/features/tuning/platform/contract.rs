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

macro_rules! incarnation_identity {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(try_from = "[u8; 16]", into = "[u8; 16]")]
        pub struct $name([u8; 16]);
        impl $name {
            pub fn new(value: [u8; 16]) -> Result<Self, InvalidIdentity> {
                if value == [0; 16] {
                    Err(InvalidIdentity)
                } else {
                    Ok(Self(value))
                }
            }
            pub fn bytes(self) -> [u8; 16] {
                self.0
            }
        }
        impl TryFrom<[u8; 16]> for $name {
            type Error = InvalidIdentity;
            fn try_from(value: [u8; 16]) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
        impl From<$name> for [u8; 16] {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
incarnation_identity!(MachineId);
incarnation_identity!(BootId);

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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceKey {
    pub target: TargetIdentity,
    pub field: ResourceId,
}

/// Exact module-produced canonical desired bytes. The ownership core compares
/// these tokens without interpreting them. Different semantics/versions conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesiredStateToken {
    pub semantics: ResourceId,
    pub version: SchemaVersion,
    bytes: Vec<u8>,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
