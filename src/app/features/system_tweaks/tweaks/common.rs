use crate::app::features::system_tweaks::platform::{
    codec::{EnvelopeKind, ModuleEnvelope},
    contract::*,
    registry::{FieldChange, ModuleObservation},
};
use os_api::gaming::{self, GamingError, GuidValue, MouseParameters, OwnedPolicy};
use serde::{de::DeserializeOwned, Serialize};

pub(super) fn feature(id: &str) -> FeatureId {
    FeatureId::new(id).expect("static feature ID")
}
pub(super) fn resource(id: &str) -> ResourceId {
    ResourceId::new(id).expect("static resource ID")
}
pub(super) fn envelope<T: Serialize>(id: &str, config: &T) -> ModuleEnvelope {
    ModuleEnvelope {
        feature: feature(id),
        kind: EnvelopeKind::Config,
        version: SchemaVersion::new(1).unwrap(),
        target: None,
        payload: serde_json::to_value(config).expect("typed configuration"),
    }
}
pub(super) fn decode<T: DeserializeOwned>(
    id: &str,
    config: &ModuleEnvelope,
) -> Result<T, ModuleError> {
    if config.feature != feature(id)
        || config.kind != EnvelopeKind::Config
        || config.version.get() != 1
        || config.target.is_some()
        || config.to_bytes().is_err()
    {
        return Err(error(ErrorKind::InvalidConfig, "configuration"));
    }
    serde_json::from_value(config.payload.clone())
        .map_err(|_| error(ErrorKind::InvalidConfig, "configuration"))
}
pub(super) fn descriptor(
    id: &str,
    title: &'static str,
    description: &'static str,
    scope: TargetScope,
    modes: Vec<ActivationMode>,
) -> ModuleDescriptor {
    ModuleDescriptor {
        id: feature(id),
        config_version: SchemaVersion::new(1).unwrap(),
        title,
        description,
        access: ModuleAccess::Managed,
        target_scopes: vec![scope],
        activation_modes: modes,
        effect: Effect::Immediate,
    }
}
pub(super) fn all_modes() -> Vec<ActivationMode> {
    vec![
        ActivationMode::Manual,
        ActivationMode::Session,
        ActivationMode::ForegroundSession,
    ]
}
pub(super) fn automatic_modes() -> Vec<ActivationMode> {
    vec![ActivationMode::Session, ActivationMode::ForegroundSession]
}
pub(super) fn user_targets() -> Vec<TargetIdentity> {
    gaming::interactive_user()
        .map(|user| {
            vec![TargetIdentity::User {
                sid: user.sid,
                logon: user.logon,
            }]
        })
        .unwrap_or_default()
}
pub(super) fn user(target: &TargetIdentity) -> Result<gaming::UserIdentity, ModuleError> {
    match target {
        TargetIdentity::User { sid, logon } if target.valid() => Ok(gaming::UserIdentity {
            sid: sid.clone(),
            logon: *logon,
        }),
        _ => Err(error(ErrorKind::Unsupported, "user target")),
    }
}
pub(crate) fn canonical_account_target(
    target: &TargetIdentity,
) -> Result<TargetIdentity, ModuleError> {
    let sid = match target {
        TargetIdentity::Account { sid } | TargetIdentity::User { sid, .. } if target.valid() => sid,
        _ => return Err(error(ErrorKind::Unsupported, "account target")),
    };
    let account = TargetIdentity::Account { sid: sid.clone() };
    if account.valid() {
        Ok(account)
    } else {
        Err(error(ErrorKind::Unsupported, "account target"))
    }
}
pub(super) fn account_targets() -> Vec<TargetIdentity> {
    user_targets()
        .iter()
        .filter_map(|target| canonical_account_target(target).ok())
        .collect()
}
pub(super) fn account_user(target: &TargetIdentity) -> Result<gaming::UserIdentity, ModuleError> {
    let TargetIdentity::Account { sid } = canonical_account_target(target)? else {
        return Err(error(ErrorKind::Unsupported, "account target"));
    };
    Ok(gaming::UserIdentity { sid, logon: 0 })
}
pub(super) fn observed_account_user(
    target: &TargetIdentity,
) -> Result<gaming::UserIdentity, ModuleError> {
    if matches!(target, TargetIdentity::Machine) {
        gaming::interactive_user().map_err(|e| native("read", e))
    } else {
        account_user(target)
    }
}
pub(super) fn process(target: &TargetIdentity) -> Result<gaming::ProcessIdentity, ModuleError> {
    match target {
        TargetIdentity::Process {
            pid,
            creation_token,
            image_path,
        } if target.valid() => Ok(gaming::ProcessIdentity {
            pid: *pid,
            creation_token: *creation_token,
            image_path: image_path.clone(),
        }),
        _ => Err(error(ErrorKind::Unsupported, "process target")),
    }
}
pub(super) fn check_field(key: &ResourceKey, field: &str) -> Result<(), ModuleError> {
    if key.field == resource(field) {
        Ok(())
    } else {
        Err(error(ErrorKind::Unsupported, "resource field"))
    }
}
pub(super) fn field(target: &TargetIdentity, name: &str, desired: Vec<u8>) -> FieldChange {
    FieldChange {
        key: ResourceKey {
            target: target.clone(),
            field: resource(name),
        },
        desired,
    }
}
pub(super) fn error(kind: ErrorKind, operation: &'static str) -> ModuleError {
    ModuleError::new(kind, operation)
}
pub(super) fn native(operation: &'static str, failure: GamingError) -> ModuleError {
    let (kind, code) = match &failure {
        GamingError::Unsupported(_) => (ErrorKind::Unsupported, None),
        GamingError::Denied { code, .. } => (ErrorKind::Denied, Some(*code)),
        GamingError::Native { code, .. } => (
            if operation == "read" {
                ErrorKind::Read
            } else {
                ErrorKind::Write
            },
            Some(*code),
        ),
        GamingError::Changed => (ErrorKind::ExternalChange, None),
        GamingError::InvalidValue => (ErrorKind::InvalidConfig, None),
    };
    let mut result = error(kind, operation);
    if let Some(code) = code {
        result = result.with_native(NativeError {
            domain: resource("win32"),
            code: code as i64,
        });
    }
    result
        .with_detail(failure.to_string())
        .expect("bounded native detail")
}
pub(super) fn observation(result: Result<String, ModuleError>) -> ModuleObservation {
    let (capability, summary) = match result {
        Ok(summary) => (Capability::Available, summary),
        Err(error) => {
            let detail = error.detail().unwrap_or(error.operation).to_string();
            let capability = if error.kind == ErrorKind::Denied {
                Capability::Denied(detail.clone())
            } else {
                Capability::Unsupported(detail.clone())
            };
            (capability, detail)
        }
    };
    ModuleObservation {
        capability,
        summary: ObservationSummary {
            summary,
            effect: Effect::Immediate,
        },
        effect: Effect::Immediate,
    }
}
pub(super) fn observed_user(target: &TargetIdentity) -> Result<gaming::UserIdentity, ModuleError> {
    let actual = gaming::interactive_user().map_err(|e| native("interactive user", e))?;
    if matches!(target, TargetIdentity::Machine) {
        return Ok(actual);
    }
    let expected = user(target)?;
    if actual == expected {
        Ok(expected)
    } else {
        Err(native("interactive user", GamingError::Changed))
    }
}
pub(super) fn u32_value(bytes: &[u8]) -> Result<u32, ModuleError> {
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| {
        error(ErrorKind::InvalidConfig, "field token")
    })?))
}
pub(super) fn guid(bytes: &[u8]) -> Result<GuidValue, ModuleError> {
    Ok(GuidValue(bytes.try_into().map_err(|_| {
        error(ErrorKind::InvalidConfig, "GUID token")
    })?))
}
pub(super) fn mouse_bytes(value: MouseParameters) -> Vec<u8> {
    value.0.into_iter().flat_map(i32::to_le_bytes).collect()
}
pub(super) fn mouse(bytes: &[u8]) -> Result<MouseParameters, ModuleError> {
    if bytes.len() != 12 {
        return Err(error(ErrorKind::InvalidConfig, "mouse token"));
    }
    Ok(MouseParameters(std::array::from_fn(|i| {
        i32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
    })))
}
pub(super) fn policy_bytes(value: OwnedPolicy) -> Vec<u8> {
    [value.control, value.state]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}
pub(super) fn policy(bytes: &[u8], bit: u32) -> Result<OwnedPolicy, ModuleError> {
    if bytes.len() != 8 {
        return Err(error(ErrorKind::InvalidConfig, "policy token"));
    }
    let value = OwnedPolicy {
        control: u32_value(&bytes[..4])?,
        state: u32_value(&bytes[4..])?,
    };
    if (value.control | value.state) & !bit != 0 {
        return Err(error(ErrorKind::InvalidConfig, "policy token"));
    }
    Ok(value)
}
pub(super) fn cpu_ids(bytes: &[u8]) -> Result<Vec<u32>, ModuleError> {
    if !bytes.len().is_multiple_of(4) || bytes.len() > 65536 {
        return Err(error(ErrorKind::InvalidConfig, "CPU Set token"));
    }
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32_value(chunk))
        .collect()
}
pub(super) fn cpu_bytes(ids: &[u32]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.to_le_bytes()).collect()
}
