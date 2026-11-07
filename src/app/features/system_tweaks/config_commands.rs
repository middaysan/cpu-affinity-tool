use super::{HeadlessTuning, SaveError};
use crate::app::features::rules::commands::{self, BaseCommand};

impl HeadlessTuning {
    pub(crate) fn configure_base(&mut self, command: BaseCommand) -> Result<(), SaveError> {
        match command {
            BaseCommand::Group(edit) => {
                // Keep the existing agent admission limit separate from GUI edits.
                if edit.group.is_none() && self.state.groups.len() >= 256 {
                    return Err(SaveError::Invalid);
                }
                return self.edit_group(edit);
            }
            BaseCommand::Preferences(patch) => return self.configure_preferences(patch),
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
    fn agent_group_limit_does_not_restrict_gui_group_creation() {
        use crate::app::{features::rules::GroupEdit, models::CoreGroup};
        let saved = AppStateStorage::test_fixture(|state| {
            state.groups = (0..256)
                .map(|i| CoreGroup {
                    name: format!("Group {i}"),
                    cores: vec![0],
                    programs: vec![],
                    is_hidden: false,
                    run_all_button: false,
                })
                .collect();
            state.rule_identities = None;
        });
        let mut app = AppState::new_for_test(Arc::new(RwLock::new(saved)), 2);
        let before = app.rules.to_persisted_identities();
        let client = app.tuning_client().unwrap();
        let id = client
            .request_at_revision(
                TuningRequest::ConfigureBase(BaseCommand::Group(GroupEdit {
                    group: None,
                    name: "Agent".into(),
                    cpus: vec![0],
                    run_all: false,
                    hidden: None,
                })),
                Some(0),
            )
            .unwrap();
        app.wait_for_test_save();
        assert_eq!(client.receipt(id).unwrap().result, Err(SaveError::Invalid));
        assert_eq!(app.rules.to_persisted_identities(), before);
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 256);
        app.start_creating_group();
        app.ui.group_form.group_name = "GUI".into();
        app.ui.group_form.core_selection[0] = true;
        app.commit_group_form_session();
        app.wait_for_test_save();
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 257);
        assert_eq!(app.persistent_state.read().unwrap().groups[256].name, "GUI");
        assert_eq!(
            &app.rules.to_persisted_identities().groups[..256],
            before.groups.as_slice()
        );
    }

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
                TuningRequest::ConfigureBase(BaseCommand::Group(
                    crate::app::features::rules::GroupEdit {
                        group: None,
                        name: "Games".into(),
                        cpus: vec![0, 1],
                        run_all: false,
                        hidden: None,
                    },
                )),
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
                TuningRequest::ConfigureBase(BaseCommand::Group(
                    crate::app::features::rules::GroupEdit {
                        group: None,
                        name: "Stale".into(),
                        cpus: vec![0],
                        run_all: false,
                        hidden: None,
                    },
                )),
                Some(0),
            )
            .unwrap();
        app.wait_for_test_save();
        assert_eq!(client.receipt(id).unwrap().result, Err(SaveError::Invalid));
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 1);
    }
}
