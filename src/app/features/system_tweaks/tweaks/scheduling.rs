//! Two-field handoff metadata for the existing process lifecycle, not a second owner loop.
use crate::app::{
    features::system_tweaks::{
        config::TuningLoad,
        platform::contract::{ActivationMode, ErrorKind, ModuleError},
    },
    models::AppStateStorage,
    shared::ids::{GroupId, RuleId},
};
use os_api::gaming::ProcessSettingsFields;
use std::{collections::BTreeMap, sync::Mutex};
const MAX_INSTANCES: usize = 1024;
#[derive(Clone, Copy)]
struct Instance {
    fresh: bool,
    reserved: ProcessSettingsFields,
    attempted: ProcessSettingsFields,
}
impl Instance {
    fn unknown() -> Self {
        Self {
            fresh: false,
            reserved: ProcessSettingsFields {
                affinity: false,
                priority: false,
            },
            attempted: ProcessSettingsFields {
                affinity: false,
                priority: false,
            },
        }
    }
}

#[derive(Default)]
pub(crate) struct SchedulingHandoff {
    instances: Mutex<BTreeMap<(u32, u64), Instance>>,
}
impl SchedulingHandoff {
    pub(crate) fn started(
        &self,
        pid: u32,
        token: u64,
        classic: ProcessSettingsFields,
    ) -> Result<(), ModuleError> {
        if pid == 0 || token == 0 {
            return Err(ModuleError::new(ErrorKind::Unsupported, "launch identity"));
        }
        let mut instances = self
            .instances
            .lock()
            .map_err(|_| ModuleError::new(ErrorKind::Verification, "scheduling metadata"))?;
        // A freshly created native tuple positively distinguishes any former use of this PID.
        instances.retain(|(old_pid, old_token), _| *old_pid != pid || *old_token == token);
        if !instances.contains_key(&(pid, token)) && instances.len() >= MAX_INSTANCES {
            return Err(ModuleError::new(
                ErrorKind::Unsupported,
                "scheduling metadata capacity",
            ));
        }
        let instance = instances.entry((pid, token)).or_insert(Instance {
            fresh: true,
            ..Instance::unknown()
        });
        instance.reserved.affinity |= !classic.affinity;
        instance.reserved.priority |= !classic.priority;
        instance.attempted.affinity |= classic.affinity;
        instance.attempted.priority |= classic.priority;
        Ok(())
    }
    pub(crate) fn select(
        &self,
        pid: u32,
        token: u64,
        requested: ProcessSettingsFields,
        write: bool,
    ) -> Result<ProcessSettingsFields, ModuleError> {
        let mut instances = self
            .instances
            .lock()
            .map_err(|_| ModuleError::new(ErrorKind::Verification, "scheduling metadata"))?;
        if !instances.contains_key(&(pid, token)) && instances.len() >= MAX_INSTANCES {
            // Classic operation may continue, but absent metadata cannot authorize baseline capture.
            if requested == ProcessSettingsFields::default() {
                return Ok(requested);
            }
            return Err(ModuleError::new(
                ErrorKind::Unsupported,
                "scheduling metadata capacity",
            ));
        }
        let instance = instances
            .entry((pid, token))
            .or_insert_with(Instance::unknown);
        instance.reserved.affinity |= !requested.affinity;
        instance.reserved.priority |= !requested.priority;
        let selected = ProcessSettingsFields {
            affinity: requested.affinity && !instance.reserved.affinity,
            priority: requested.priority && !instance.reserved.priority,
        };
        // An attempted write is conservative evidence even when native completion is uncertain.
        if write {
            instance.attempted.affinity |= selected.affinity;
            instance.attempted.priority |= selected.priority;
        }
        Ok(selected)
    }
    pub(crate) fn baseline(&self, pid: u32, token: u64, affinity: bool) -> Result<(), ModuleError> {
        let instances = self
            .instances
            .lock()
            .map_err(|_| ModuleError::new(ErrorKind::Verification, "scheduling metadata"))?;
        if instances.get(&(pid, token)).is_some_and(|instance| {
            instance.fresh
                && !(if affinity {
                    instance.attempted.affinity
                } else {
                    instance.attempted.priority
                })
        }) {
            return Ok(());
        }
        Err(ModuleError::new(ErrorKind::Unsupported, "process baseline").with_detail("Restart this game through this app: its original scheduling setting is unknown or was already managed by a classic rule.").unwrap())
    }
}

/// Only module-owned code knows these concrete configuration IDs and codecs.
pub(crate) fn fields_for_rule(
    state: &AppStateStorage,
    group: &GroupId,
    rule: &RuleId,
) -> ProcessSettingsFields {
    let mut fields = ProcessSettingsFields::default();
    // Retained Windows profiles cannot reserve fields when no module can apply them.
    if !crate::app::features::system_tweaks::MODULES_ENABLED {
        return fields;
    }
    let TuningLoad::Known(config) = state.tuning.decode() else {
        return fields;
    };
    let Some(association) = config
        .associations
        .iter()
        .find(|a| &a.group == group && &a.rule == rule)
    else {
        return fields;
    };
    let Some(profile) = config.profiles.iter().find(|p| p.id == association.profile) else {
        return fields;
    };
    for preference in profile.modules.iter().filter(|p| {
        p.enabled
            && matches!(
                p.mode,
                ActivationMode::Session | ActivationMode::ForegroundSession
            )
    }) {
        if preference.feature.as_str() == "cpu_sets"
            && super::cpu_sets::configured_ids(&preference.config).is_ok()
        {
            fields.affinity = false;
        }
        if preference.feature.as_str() == "process_priority"
            && super::process_priority::configured_priority(&preference.config).is_ok()
        {
            fields.priority = false;
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handoff_reserved_field_stays_reserved_after_disable_and_foreground_loss() {
        let handoff = SchedulingHandoff::default();
        let managed = ProcessSettingsFields {
            affinity: false,
            priority: false,
        };
        handoff.started(91, 9001, managed).unwrap();
        assert!(handoff.baseline(91, 9001, true).is_ok());
        assert!(handoff.baseline(91, 9001, false).is_ok());
        assert_eq!(
            handoff
                .select(91, 9001, ProcessSettingsFields::default(), true)
                .unwrap(),
            managed
        );
    }
    #[test]
    fn handoff_classic_instance_requires_restart_only_for_previously_written_field() {
        let handoff = SchedulingHandoff::default();
        handoff
            .started(
                92,
                9002,
                ProcessSettingsFields {
                    affinity: true,
                    priority: false,
                },
            )
            .unwrap();
        assert!(handoff.baseline(92, 9002, true).is_err());
        assert!(handoff.baseline(92, 9002, false).is_ok());
    }
    #[test]
    fn handoff_unknown_rediscovered_instance_never_claims_original_baseline() {
        let handoff = SchedulingHandoff::default();
        let managed = ProcessSettingsFields {
            affinity: false,
            priority: false,
        };
        assert_eq!(handoff.select(93, 9003, managed, true).unwrap(), managed);
        assert!(handoff.baseline(93, 9003, true).is_err());
        assert!(handoff.baseline(93, 9003, false).is_err());
    }
}
