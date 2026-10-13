//! Shared IDs and field tokens. Native handles and configuration decoding belong to modules.
use serde::{Deserialize, Serialize};
use std::num::{NonZeroU32, NonZeroU64};

pub(crate) const MAX_FIELD_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidIdentity;
impl std::fmt::Display for InvalidIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid identity")
    }
}
impl std::error::Error for InvalidIdentity {}
macro_rules! text_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdentity> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > 64
                    || !value.as_bytes()[0].is_ascii_lowercase()
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                {
                    return Err(InvalidIdentity);
                }
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = InvalidIdentity;
            fn try_from(v: String) -> Result<Self, Self::Error> {
                Self::new(v)
            }
        }
        impl From<$name> for String {
            fn from(v: $name) -> Self {
                v.0
            }
        }
    };
}
text_id!(FeatureId);
text_id!(ResourceId);
macro_rules! numeric_id {
    ($name:ident,$raw:ty,$nz:ty) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name($nz);
        impl $name {
            pub fn new(v: $raw) -> Result<Self, InvalidIdentity> {
                <$nz>::new(v).map(Self).ok_or(InvalidIdentity)
            }
            pub fn get(self) -> $raw {
                self.0.get()
            }
        }
    };
}
numeric_id!(ProfileId, u64, NonZeroU64);
numeric_id!(SchemaVersion, u32, NonZeroU32);
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
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TargetIdentity {
    Machine,
    User {
        sid: String,
        logon: u64,
    },
    Process {
        pid: u32,
        creation_token: u64,
        image_path: String,
    },
    Device {
        identity: String,
    },
}
impl TargetIdentity {
    pub fn scope(&self) -> TargetScope {
        match self {
            Self::Machine => TargetScope::Machine,
            Self::User { .. } => TargetScope::User,
            Self::Process { .. } => TargetScope::Process,
            Self::Device { .. } => TargetScope::PnpDevice,
        }
    }
    pub fn valid(&self) -> bool {
        match self {
            Self::Machine => true,
            Self::User { sid, logon } => {
                sid.starts_with("S-1-") && sid.len() <= 184 && !sid.contains('\0') && *logon != 0
            }
            Self::Process {
                pid,
                creation_token,
                image_path,
            } => {
                *pid != 0
                    && *creation_token != 0
                    && !image_path.is_empty()
                    && image_path.len() <= 32767
                    && !image_path.contains('\0')
            }
            Self::Device { identity } => {
                !identity.is_empty() && identity.len() <= 4096 && !identity.contains('\0')
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResourceKey {
    pub target: TargetIdentity,
    pub field: ResourceId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActivationMode {
    Manual,
    Session,
    ForegroundSession,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationEligibility {
    Running,
    Foreground,
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
    Conflict,
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
    pub fn with_native(mut self, error: NativeError) -> Self {
        self.native = Some(error);
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
#[derive(Clone, Debug)]
pub struct ModuleDescriptor {
    pub id: FeatureId,
    pub config_version: SchemaVersion,
    pub title: &'static str,
    pub description: &'static str,
    pub access: ModuleAccess,
    pub target_scopes: Vec<TargetScope>,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationSummary {
    pub summary: String,
    pub effect: Effect,
}
