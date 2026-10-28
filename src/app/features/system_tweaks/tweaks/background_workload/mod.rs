//! Background CPU policy, executed only by the existing System Tweaks worker.

use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
};
use os_api::{
    background::{priority_change, PriorityDecision, Role},
    gaming::{self, OwnedPolicy, EXECUTION_SPEED},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

pub(crate) const ID: &str = "background_workload";
const PRIORITY: &str = "process_priority_class";
const QOS: &str = "process_execution_speed";
const SETS: &str = "process_default_cpu_sets";
mod placement;
pub(crate) use placement::{placement_plan, resolve_pool};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GameRule {
    pub group: String,
    pub rule: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CpuMode {
    #[default]
    PriorityEco,
    EfficientCores,
    ManualPool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub catalog_version: u32,
    pub games: Vec<GameRule>,
    pub cpu_mode: CpuMode,
    pub manual_ids: Vec<u32>,
    pub manual_topology: Vec<os_api::background::PlacementCpu>,
    pub excluded_roles: Vec<Role>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            catalog_version: os_api::background::CATALOG_VERSION,
            games: vec![],
            cpu_mode: CpuMode::PriorityEco,
            manual_ids: vec![],
            manual_topology: vec![],
            excluded_roles: vec![],
        }
    }
}
impl Config {
    pub(crate) fn without_cpu_placement(mut self) -> Self {
        self.cpu_mode = CpuMode::PriorityEco;
        self.manual_ids.clear();
        self.manual_topology.clear();
        self
    }
    pub(crate) fn valid(&self) -> bool {
        self.catalog_version == os_api::background::CATALOG_VERSION
            && self.games.len() <= 64
            && self.games.iter().all(|game| {
                [&game.group, &game.rule]
                    .iter()
                    .all(|id| !id.is_empty() && id.len() <= 128 && !id.contains('\0'))
            })
            && self
                .games
                .iter()
                .map(|game| &game.rule)
                .collect::<BTreeSet<_>>()
                .len()
                == self.games.len()
            && self.excluded_roles.len() <= 7
            && self.excluded_roles.iter().collect::<BTreeSet<_>>().len()
                == self.excluded_roles.len()
            && self.manual_ids.len() <= 512
            && self.manual_topology.len() <= 512
            && self.manual_ids.iter().collect::<BTreeSet<_>>().len() == self.manual_ids.len()
            && (self.cpu_mode != CpuMode::ManualPool
                || self.manual_ids.is_empty()
                || os_api::background::validate_pool(&self.manual_ids, &self.manual_topology)
                    .is_ok())
    }
    pub(crate) fn envelope(&self) -> ModuleEnvelope {
        envelope(ID, self)
    }
}
pub(crate) fn configured(value: &ModuleEnvelope) -> Result<Config, ModuleError> {
    let config: Config = decode(ID, value)?;
    if config.valid() {
        Ok(config)
    } else {
        Err(error(ErrorKind::InvalidConfig, "background configuration"))
    }
}

pub(crate) struct TargetPlan {
    pub changes: Vec<FieldChange>,
    pub noops: usize,
    pub skipped: Vec<&'static str>,
    pub blocked: bool,
}
fn include_placement_intent(
    plan: &mut TargetPlan,
    target: &TargetIdentity,
    witness: &os_api::background::PlacementWitness,
) {
    // Equal intent may join another session's original field; an unowned
    // equal baseline remains a no-op in the controller.
    if witness.baseline == witness.desired {
        plan.noops += 1;
    }
    plan.changes
        .push(field(target, SETS, cpu_bytes(&witness.desired)));
}
pub(crate) fn plan_fields(
    target: &TargetIdentity,
    role: Role,
    priority: Option<u32>,
    qos: Option<OwnedPolicy>,
    _config: &Config,
) -> TargetPlan {
    let mut plan = TargetPlan {
        changes: vec![],
        noops: 0,
        skipped: vec![],
        blocked: false,
    };
    if priority.is_some_and(|value| priority_change(value) == PriorityDecision::Unexpected) {
        plan.blocked = true;
        plan.skipped
            .push("Unexpected priority; target left unchanged");
        return plan;
    }
    if role.permits_priority() {
        match priority.map(priority_change) {
            Some(PriorityDecision::LowerToBelowNormal) => {
                plan.changes
                    .push(field(target, PRIORITY, 0x4000u32.to_le_bytes().to_vec()))
            }
            Some(PriorityDecision::AlreadySuitable) => {
                plan.noops += 1;
                // The controller joins an existing equal owner, but skips an
                // unowned suitable baseline without creating an Undo record.
                if priority == Some(0x4000) {
                    plan.changes
                        .push(field(target, PRIORITY, 0x4000u32.to_le_bytes().to_vec()));
                }
            }
            _ => plan.skipped.push("Priority getter unavailable"),
        }
    }
    let desired = OwnedPolicy {
        control: EXECUTION_SPEED,
        state: EXECUTION_SPEED,
    };
    match qos {
        Some(current) if current == desired => {
            plan.noops += 1;
            plan.changes.push(field(target, QOS, policy_bytes(desired)));
        }
        Some(_) => plan.changes.push(field(target, QOS, policy_bytes(desired))),
        None => plan
            .skipped
            .push("EcoQoS getter unavailable; exact restoration is unavailable"),
    }
    plan
}

#[derive(Default)]
pub(crate) struct BackgroundWorkload {
    scheduling: Arc<super::scheduling::SchedulingHandoff>,
    resolver: Mutex<os_api::background::Resolver>,
    prepared_context: Mutex<Option<PreparedContext>>,
}
struct PreparedContext {
    target: TargetIdentity,
    bytes: Vec<u8>,
    notes: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldContext {
    proof: os_api::background::TargetProof,
    placement: Option<os_api::background::PlacementWitness>,
}
fn encode_context(
    context: &mut FieldContext,
    plan: &mut TargetPlan,
    notes: &mut Vec<String>,
) -> Result<Vec<u8>, ModuleError> {
    let encode = |context: &FieldContext| {
        serde_json::to_vec(context)
            .map_err(|_| error(ErrorKind::Verification, "background context serialization"))
    };
    let mut bytes = encode(context)?;
    if bytes.len() > MAX_FIELD_BYTES && context.placement.is_some() {
        context.placement = None;
        plan.changes
            .retain(|change| change.key.field.as_str() != SETS);
        plan.skipped
            .push("CPU placement evidence exceeds the context bound");
        notes.push("CPU placement skipped: evidence exceeds the context bound".into());
        bytes = encode(context)?;
    }
    if bytes.len() > MAX_FIELD_BYTES {
        return Err(error(ErrorKind::Unsupported, "background context bound"));
    }
    Ok(bytes)
}
fn context_proof(key: &ResourceKey, context: Option<&[u8]>) -> Result<FieldContext, ModuleError> {
    let context = context
        .filter(|bytes| bytes.len() <= MAX_FIELD_BYTES)
        .ok_or_else(|| error(ErrorKind::Verification, "background context required"))?;
    let witness: FieldContext = serde_json::from_slice(context)
        .map_err(|_| error(ErrorKind::Verification, "background context format"))?;
    if !witness.proof.matches(&process(&key.target)?)
        || witness
            .placement
            .as_ref()
            .is_some_and(|placement| !placement.valid())
        || (key.field.as_str() == SETS && witness.placement.is_none())
    {
        return Err(error(
            ErrorKind::Verification,
            "background context identity",
        ));
    }
    Ok(witness)
}
impl BackgroundWorkload {
    pub(crate) fn with_scheduling(scheduling: Arc<super::scheduling::SchedulingHandoff>) -> Self {
        Self {
            scheduling,
            ..Self::default()
        }
    }
}
impl ModuleEditor for BackgroundWorkload {
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::WINDOWS11
    }
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        Config::default().envelope()
    }
    fn accepts_config(&self, value: &ModuleEnvelope) -> bool {
        configured(value).is_ok()
    }
    #[cfg(test)]
    fn show(&self, _: &mut eframe::egui::Ui, _: &mut ModuleEnvelope) -> Result<bool, ()> {
        Ok(false)
    }
}
impl Module for BackgroundWorkload {
    fn requires_session_target(&self) -> bool {
        true
    }
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Reduce background impact",
            "Temporarily lower verified background work while selected games run.",
            TargetScope::Process,
            vec![ActivationMode::Session],
        )
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        vec![]
    }
    fn target_has_expired(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        let identity = process(target)?;
        #[cfg(windows)]
        {
            Ok(os_api::agent::process_instance_exited(
                identity.pid,
                identity.creation_token,
            ))
        }
        #[cfg(not(windows))]
        {
            let _ = identity;
            Ok(false)
        }
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        observation((|| {
            let process = process(target)?;
            let priority = gaming::read_priority(&process).map_err(|e| native("read", e))?;
            let qos = gaming::read_process_policy(&process, EXECUTION_SPEED)
                .map_err(|e| native("read", e))?;
            Ok(format!(
                "Priority class {priority:#x}; EcoQoS {}",
                if qos.control == 0 {
                    "Windows default"
                } else if qos.state != 0 {
                    "on"
                } else {
                    "off"
                }
            ))
        })())
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        value: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        *self
            .prepared_context
            .lock()
            .map_err(|_| error(ErrorKind::Verification, "background context"))? = None;
        let config = configured(value)?;
        let process = process(target)?;
        let proof = self
            .resolver
            .lock()
            .map_err(|_| error(ErrorKind::Verification, "background resolver"))?
            .capture_proof(&process)
            .map_err(|e| native("qualify background", e))?;
        let role = proof.role;
        if !role.mutation_qualified() {
            return Err(error(ErrorKind::Unsupported, role.qualification_note()));
        }
        if !role.base_candidate() || config.excluded_roles.contains(&role) {
            return Err(error(ErrorKind::Unsupported, "background role excluded"));
        }
        let mut plan = plan_fields(
            target,
            role,
            gaming::read_priority(&process).ok(),
            gaming::read_process_policy(&process, EXECUTION_SPEED).ok(),
            &config,
        );
        if plan.blocked {
            return Err(error(
                ErrorKind::Unsupported,
                "Unexpected priority; background target left unchanged",
            ));
        }
        let mut notes: Vec<String> = plan.skipped.iter().map(|note| (*note).into()).collect();
        let placement = if config.cpu_mode != CpuMode::PriorityEco {
            let prepared = (|| {
                let topology = os_api::background::placement_topology(Some(&process))
                    .map_err(|e| native("CPU placement topology", e))?;
                let mut baseline = gaming::read_cpu_sets(&process)
                    .map_err(|e| native("CPU placement baseline", e))?;
                baseline.sort_unstable();
                let witness = placement_plan(&config, &topology, &baseline)
                    .map_err(|_| {
                        error(
                            ErrorKind::Unsupported,
                            "CPU placement conflicts with session pool or existing restrictions",
                        )
                    })?
                    .ok_or_else(|| error(ErrorKind::Unsupported, "CPU placement pool missing"))?;
                os_api::background::validate_placement(&process, &witness)
                    .map_err(|e| native("CPU placement admission", e))?;
                Ok::<_, ModuleError>(witness)
            })();
            match prepared {
                Ok(witness) => {
                    include_placement_intent(&mut plan, target, &witness);
                    Some(witness)
                }
                Err(error) => {
                    notes.push(format!(
                        "CPU placement skipped: {}",
                        error.detail().unwrap_or(error.operation)
                    ));
                    plan.skipped
                        .push("CPU placement unavailable; priority and EcoQoS remain independent");
                    None
                }
            }
        } else {
            None
        };
        let context = encode_context(
            &mut FieldContext { proof, placement },
            &mut plan,
            &mut notes,
        )?;
        *self
            .prepared_context
            .lock()
            .map_err(|_| error(ErrorKind::Verification, "background context"))? =
            Some(PreparedContext {
                target: target.clone(),
                bytes: context,
                notes,
            });
        if plan.changes.is_empty() && !plan.skipped.is_empty() && plan.noops == 0 {
            return Err(error(ErrorKind::Unsupported, "background field preflight")
                .with_detail(plan.skipped.join("; "))
                .unwrap());
        }
        Ok(plan.changes)
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        let process = process(&key.target)?;
        match key.field.as_str() {
            PRIORITY => gaming::read_priority(&process)
                .map(|v| v.to_le_bytes().to_vec())
                .map_err(|e| native("read", e)),
            QOS => gaming::read_process_policy(&process, EXECUTION_SPEED)
                .map(policy_bytes)
                .map_err(|e| native("read", e)),
            SETS => gaming::read_cpu_sets(&process)
                .map(|mut ids| {
                    ids.sort_unstable();
                    cpu_bytes(&ids)
                })
                .map_err(|e| native("read CPU Sets", e)),
            _ => Err(error(ErrorKind::Unsupported, "background field")),
        }
    }
    fn field_context(&self, key: &ResourceKey) -> Result<Option<Vec<u8>>, ModuleError> {
        self.prepared_context
            .lock()
            .map_err(|_| error(ErrorKind::Verification, "background context"))?
            .as_ref()
            .filter(|prepared| prepared.target == key.target)
            .map(|prepared| Some(prepared.bytes.clone()))
            .ok_or_else(|| error(ErrorKind::Verification, "background context not prepared"))
    }
    fn preflight_notes(&self, target: &TargetIdentity) -> Vec<String> {
        self.prepared_context
            .lock()
            .ok()
            .and_then(|prepared| {
                prepared
                    .as_ref()
                    .filter(|prepared| &prepared.target == target)
                    .map(|prepared| prepared.notes.clone())
            })
            .unwrap_or_default()
    }
    fn read_with_context(
        &self,
        key: &ResourceKey,
        context: Option<&[u8]>,
    ) -> Result<Vec<u8>, ModuleError> {
        context_proof(key, context)?;
        self.read(key)
    }
    fn write(&self, _: &ResourceKey, _: &[u8], _: &[u8]) -> Result<(), ModuleError> {
        Err(error(
            ErrorKind::Verification,
            "background context required",
        ))
    }
    fn write_with_context(
        &self,
        key: &ResourceKey,
        expected: &[u8],
        desired: &[u8],
        context: Option<&[u8]>,
    ) -> Result<(), ModuleError> {
        let proof = context_proof(key, context)?;
        let process = process(&key.target)?;
        match key.field.as_str() {
            PRIORITY => {
                self.scheduling
                    .reserve_existing_priority(process.pid, process.creation_token)?;
                self.resolver
                    .lock()
                    .map_err(|_| error(ErrorKind::Verification, "background resolver"))?
                    .write_priority_bound(
                        &process,
                        u32_value(expected)?,
                        u32_value(desired)?,
                        &proof.proof,
                    )
                    .map_err(|e| native("write", e))
            }
            QOS => self
                .resolver
                .lock()
                .map_err(|_| error(ErrorKind::Verification, "background resolver"))?
                .write_qos_bound(
                    &process,
                    policy(expected, EXECUTION_SPEED)?,
                    policy(desired, EXECUTION_SPEED)?,
                    &proof.proof,
                )
                .map_err(|e| native("write", e)),
            SETS => {
                let placement = proof.placement.as_ref().ok_or_else(|| {
                    error(ErrorKind::Verification, "CPU placement context required")
                })?;
                self.resolver
                    .lock()
                    .map_err(|_| error(ErrorKind::Verification, "background resolver"))?
                    .write_cpu_sets_bound(
                        &process,
                        &cpu_ids(expected)?,
                        &cpu_ids(desired)?,
                        &proof.proof,
                        placement,
                    )
                    .map_err(|e| native("write CPU Sets", e))
            }
            _ => Err(error(ErrorKind::Unsupported, "background field")),
        }
    }
    fn field_retired(&self, key: &ResourceKey) {
        if key.field.as_str() == PRIORITY {
            if let Ok(process) = process(&key.target) {
                self.scheduling
                    .retire_existing_priority(process.pid, process.creation_token);
            }
        }
    }
}

#[cfg(test)]
mod tests;
