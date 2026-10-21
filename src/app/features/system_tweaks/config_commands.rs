use super::{HeadlessTuning, SaveError};
use crate::app::{
    features::rules::commands::{self, BaseCommand},
    models::{effective_total_threads, CoreGroup},
};

impl HeadlessTuning {
    pub(crate) fn configure_base(&mut self, command: BaseCommand) -> Result<(), SaveError> {
        match command {
            BaseCommand::RemoveGroup { group } => return self.remove_group(&group),
            BaseCommand::RemoveRule { group, rule } => return self.remove_rule(&group, &rule),
            BaseCommand::MoveRule {
                group,
                rule,
                target,
                index,
            } => return self.move_rule_at(&group, &rule, &target, index),
            BaseCommand::UpdateRule { group, rule, app } => {
                commands::validate_app(&app).map_err(|_| SaveError::Invalid)?;
                return self.edit_rule(&group, &rule, *app);
            }
            _ => {}
        }
        let mut candidate = self.state.clone();
        let mut identities = self.rules.clone();
        match command {
            BaseCommand::CreateGroup {
                name,
                cpus,
                run_all,
            } => {
                if candidate.groups.len() >= 256 {
                    return Err(SaveError::Invalid);
                }
                let (name, cores) =
                    commands::validate_group(&name, &cpus, effective_total_threads())
                        .map_err(|_| SaveError::Invalid)?;
                candidate.groups.push(CoreGroup {
                    name,
                    cores,
                    programs: Vec::new(),
                    is_hidden: false,
                    run_all_button: run_all,
                });
                identities.append_group();
            }
            BaseCommand::UpdateGroup {
                group,
                name,
                cpus,
                run_all,
                hidden,
            } => {
                let index = identities
                    .group_index_for_id(&group)
                    .ok_or(SaveError::Invalid)?;
                let (name, cores) =
                    commands::validate_group(&name, &cpus, effective_total_threads())
                        .map_err(|_| SaveError::Invalid)?;
                let group = &mut candidate.groups[index];
                group.name = name;
                group.cores = cores;
                group.run_all_button = run_all;
                if let Some(hidden) = hidden {
                    group.is_hidden = hidden;
                }
            }
            BaseCommand::MoveGroup { group, index } => {
                let source = identities
                    .group_index_for_id(&group)
                    .ok_or(SaveError::Invalid)?;
                if index >= candidate.groups.len() {
                    return Err(SaveError::Invalid);
                }
                let group = candidate.groups.remove(source);
                candidate.groups.insert(index, group);
                identities.move_group_to_index(source, index);
            }
            BaseCommand::CreateRule { group, app } => {
                commands::validate_app(&app).map_err(|_| SaveError::Invalid)?;
                let index = identities
                    .group_index_for_id(&group)
                    .ok_or(SaveError::Invalid)?;
                if candidate.groups[index].programs.len() >= 1024
                    || candidate.groups[index]
                        .programs
                        .iter()
                        .any(|other| other.runtime_key() == app.runtime_key())
                {
                    return Err(SaveError::Invalid);
                }
                candidate.groups[index].programs.push(*app);
                identities.append_rules_to_group(index, 1);
            }
            BaseCommand::Preferences(patch) => patch
                .apply(&mut candidate)
                .map_err(|_| SaveError::Invalid)?,
            _ => unreachable!("Handled structural operations above"),
        }
        self.publish(candidate, identities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        features::system_tweaks::TuningRequest, models::AppStateStorage, runtime::AppState,
    };
    use std::sync::{Arc, RwLock};
    #[test]
    fn configuration_client_reports_durable_completion_and_rejects_stale_revision() {
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
                state.groups.clear()
            }))),
            4,
        );
        let client = app.tuning_client().unwrap();
        let id = client
            .request_at_revision(
                TuningRequest::ConfigureBase(BaseCommand::CreateGroup {
                    name: "Games".into(),
                    cpus: vec![0, 1],
                    run_all: false,
                }),
                Some(0),
            )
            .unwrap();
        app.wait_for_test_save();
        let receipt = client.receipt(id).unwrap();
        assert_eq!(receipt.revision, 1);
        assert_eq!(receipt.result, Ok(()));
        assert_eq!(app.persistent_state.read().unwrap().groups[0].name, "Games");
        let id = client
            .request_at_revision(
                TuningRequest::ConfigureBase(BaseCommand::CreateGroup {
                    name: "Stale".into(),
                    cpus: vec![0],
                    run_all: false,
                }),
                Some(0),
            )
            .unwrap();
        app.wait_for_test_save();
        assert_eq!(client.receipt(id).unwrap().result, Err(SaveError::Invalid));
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 1);
    }
}
