mod service;

use crate::app::models::{AppStateStorage, AppToRun};
use crate::app::shared::ids::{GroupId, RuleId};
use serde::{Deserialize, Serialize};

pub use service::{
    add_apps_to_group, add_installed_app_to_group, create_group, load_group_for_edit, load_rule,
    move_rule_between_groups_at, remove_group, remove_rule_from_group, set_group_is_hidden,
    swap_groups, update_group_properties, update_rule,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedRuleIdentities {
    pub groups: Vec<PersistedGroupIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedGroupIdentity {
    pub id: GroupId,
    pub rule_ids: Vec<RuleId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuleConfig {
    pub id: RuleId,
    pub app: AppToRun,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupConfig {
    pub id: GroupId,
    pub name: String,
    pub cores: Vec<usize>,
    pub is_hidden: bool,
    pub run_all_enabled: bool,
    pub rules: Vec<RuleConfig>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RulesSnapshot {
    pub groups: Vec<GroupConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct RulesContext {
    next_group_id: usize,
    next_rule_id: usize,
    group_ids: Vec<GroupId>,
    rule_ids: Vec<Vec<RuleId>>,
}

impl RulesContext {
    pub fn from_storage(storage: &AppStateStorage) -> Self {
        let mut context = Self::default();
        if let Some(identities) = storage.rule_identities.as_ref() {
            context.rebuild_from_persisted(storage, identities);
        } else {
            context.rebuild_from_storage(storage);
        }
        context
    }

    pub fn rebuild_from_storage(&mut self, storage: &AppStateStorage) {
        self.group_ids.clear();
        self.rule_ids.clear();

        for group in &storage.groups {
            let group_id = self.allocate_group_id();
            let mut rule_ids = Vec::with_capacity(group.programs.len());
            for _ in &group.programs {
                rule_ids.push(self.allocate_rule_id());
            }
            self.group_ids.push(group_id);
            self.rule_ids.push(rule_ids);
        }
    }

    fn rebuild_from_persisted(
        &mut self,
        storage: &AppStateStorage,
        identities: &PersistedRuleIdentities,
    ) {
        self.group_ids = identities
            .groups
            .iter()
            .map(|group| group.id.clone())
            .collect();
        self.rule_ids = identities
            .groups
            .iter()
            .map(|group| group.rule_ids.clone())
            .collect();
        self.seed_next_ids();
        self.reconcile_with_storage(storage);
    }

    pub fn reconcile_with_storage(&mut self, storage: &AppStateStorage) {
        while self.group_ids.len() > storage.groups.len() {
            self.group_ids.pop();
            self.rule_ids.pop();
        }

        while self.group_ids.len() < storage.groups.len() {
            let group_id = self.allocate_group_id();
            self.group_ids.push(group_id);
            self.rule_ids.push(Vec::new());
        }

        for (group_index, group) in storage.groups.iter().enumerate() {
            let Some(rule_ids) = self.rule_ids.get_mut(group_index) else {
                continue;
            };

            while rule_ids.len() > group.programs.len() {
                rule_ids.pop();
            }
        }

        for (group_index, group) in storage.groups.iter().enumerate() {
            let current_len = self.rule_ids.get(group_index).map(Vec::len).unwrap_or(0);
            if current_len >= group.programs.len() {
                continue;
            }

            let mut new_rule_ids = Vec::with_capacity(group.programs.len() - current_len);
            for _ in current_len..group.programs.len() {
                new_rule_ids.push(self.allocate_rule_id());
            }

            if let Some(rule_ids) = self.rule_ids.get_mut(group_index) {
                rule_ids.extend(new_rule_ids);
            }
        }
    }

    pub fn snapshot(&self, storage: &AppStateStorage) -> RulesSnapshot {
        let groups = storage
            .groups
            .iter()
            .enumerate()
            .map(|(group_index, group)| GroupConfig {
                id: self
                    .group_id_for_index(group_index)
                    .unwrap_or_else(|| GroupId(format!("missing-group-{group_index}"))),
                name: group.name.clone(),
                cores: group.cores.clone(),
                is_hidden: group.is_hidden,
                run_all_enabled: group.run_all_button,
                rules: group
                    .programs
                    .iter()
                    .enumerate()
                    .map(|(rule_index, app)| RuleConfig {
                        id: self
                            .rule_id_for_index(group_index, rule_index)
                            .unwrap_or_else(|| {
                                RuleId(format!("missing-rule-{group_index}-{rule_index}"))
                            }),
                        app: app.clone(),
                    })
                    .collect(),
            })
            .collect();

        RulesSnapshot { groups }
    }

    pub fn group_id_for_index(&self, group_index: usize) -> Option<GroupId> {
        self.group_ids.get(group_index).cloned()
    }

    pub fn group_index_for_id(&self, group_id: &GroupId) -> Option<usize> {
        self.group_ids
            .iter()
            .position(|candidate| candidate == group_id)
    }

    pub fn rule_id_for_index(&self, group_index: usize, rule_index: usize) -> Option<RuleId> {
        self.rule_ids
            .get(group_index)
            .and_then(|rules| rules.get(rule_index))
            .cloned()
    }

    pub fn rule_index_for_id(&self, group_index: usize, rule_id: &RuleId) -> Option<usize> {
        self.rule_ids
            .get(group_index)
            .and_then(|rules| rules.iter().position(|candidate| candidate == rule_id))
    }

    pub fn swap_groups(&mut self, left: usize, right: usize) {
        if left < self.group_ids.len() && right < self.group_ids.len() {
            self.group_ids.swap(left, right);
            self.rule_ids.swap(left, right);
        }
    }

    pub fn append_group(&mut self) -> GroupId {
        let id = self.allocate_group_id();
        self.group_ids.push(id.clone());
        self.rule_ids.push(Vec::new());
        id
    }

    pub fn remove_group(&mut self, group_index: usize) {
        if group_index < self.group_ids.len() {
            self.group_ids.remove(group_index);
            self.rule_ids.remove(group_index);
        }
    }

    pub fn append_rules_to_group(&mut self, group_index: usize, count: usize) {
        if self.rule_ids.get(group_index).is_none() {
            return;
        }

        let mut new_rule_ids = Vec::with_capacity(count);
        for _ in 0..count {
            new_rule_ids.push(self.allocate_rule_id());
        }

        if let Some(rule_ids) = self.rule_ids.get_mut(group_index) {
            rule_ids.extend(new_rule_ids);
        }
    }

    pub fn remove_rule(&mut self, group_index: usize, rule_index: usize) {
        if let Some(rule_ids) = self.rule_ids.get_mut(group_index) {
            if rule_index < rule_ids.len() {
                rule_ids.remove(rule_index);
            }
        }
    }

    pub fn can_move_rule_between_groups_at(
        &self,
        source_group_index: usize,
        source_rule_index: usize,
        target_group_index: usize,
        target_rule_index: usize,
    ) -> bool {
        source_group_index < self.rule_ids.len()
            && target_group_index < self.rule_ids.len()
            && self
                .rule_ids
                .get(source_group_index)
                .is_some_and(|rules| source_rule_index < rules.len())
            && self
                .rule_ids
                .get(target_group_index)
                .is_some_and(|rules| target_rule_index <= rules.len())
    }

    pub fn move_rule_between_groups_at(
        &mut self,
        source_group_index: usize,
        source_rule_index: usize,
        target_group_index: usize,
        target_rule_index: usize,
    ) -> Option<RuleId> {
        if !self.can_move_rule_between_groups_at(
            source_group_index,
            source_rule_index,
            target_group_index,
            target_rule_index,
        ) {
            return None;
        }

        let rule_id = self.rule_ids[source_group_index].remove(source_rule_index);
        let insert_index =
            if source_group_index == target_group_index && target_rule_index > source_rule_index {
                target_rule_index - 1
            } else {
                target_rule_index
            };
        self.rule_ids[target_group_index].insert(insert_index, rule_id.clone());
        Some(rule_id)
    }

    pub fn to_persisted_identities(&self) -> PersistedRuleIdentities {
        PersistedRuleIdentities {
            groups: self
                .group_ids
                .iter()
                .enumerate()
                .map(|(group_index, group_id)| PersistedGroupIdentity {
                    id: group_id.clone(),
                    rule_ids: self.rule_ids.get(group_index).cloned().unwrap_or_default(),
                })
                .collect(),
        }
    }

    fn allocate_group_id(&mut self) -> GroupId {
        let id = GroupId(format!("group-{}", self.next_group_id));
        self.next_group_id += 1;
        id
    }

    fn allocate_rule_id(&mut self) -> RuleId {
        let id = RuleId(format!("rule-{}", self.next_rule_id));
        self.next_rule_id += 1;
        id
    }

    fn seed_next_ids(&mut self) {
        self.next_group_id = self
            .group_ids
            .iter()
            .filter_map(|id| id.0.strip_prefix("group-")?.parse::<usize>().ok())
            .max()
            .map(|value| value + 1)
            .unwrap_or(0);
        self.next_rule_id = self
            .rule_ids
            .iter()
            .flatten()
            .filter_map(|id| id.0.strip_prefix("rule-")?.parse::<usize>().ok())
            .max()
            .map(|value| value + 1)
            .unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::RulesContext;
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema};
    use os_api::PriorityClass;
    use std::path::PathBuf;

    fn sample_storage() -> AppStateStorage {
        AppStateStorage {
            version: 5,
            groups: vec![CoreGroup {
                name: "Games".into(),
                cores: vec![0, 1],
                programs: vec![AppToRun::new_path(
                    PathBuf::from(r"C:\game.lnk"),
                    vec![],
                    PathBuf::from(r"C:\game.exe"),
                    PriorityClass::Normal,
                    false,
                )],
                is_hidden: false,
                run_all_button: true,
            }],
            cpu_schema: CpuSchema {
                model: "Test CPU".into(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
            rule_identities: None,
            loaded_version: 5,
            pending_pre_v6_backup: false,
        }
    }

    #[test]
    fn test_swap_and_append_preserve_existing_ids() {
        let mut storage = sample_storage();
        storage.groups.push(CoreGroup {
            name: "Work".into(),
            cores: vec![2, 3],
            programs: vec![],
            is_hidden: false,
            run_all_button: false,
        });

        let mut context = RulesContext::from_storage(&storage);
        let first = context.group_id_for_index(0).unwrap();
        let second = context.group_id_for_index(1).unwrap();

        context.swap_groups(0, 1);
        assert_eq!(context.group_id_for_index(0), Some(second));
        assert_eq!(context.group_id_for_index(1), Some(first));

        context.append_rules_to_group(0, 2);
        assert!(context.rule_id_for_index(0, 1).is_some());
    }

    #[test]
    fn test_move_rule_between_groups_preserves_rule_id() {
        let mut storage = sample_storage();
        storage.groups.push(CoreGroup {
            name: "Work".into(),
            cores: vec![2, 3],
            programs: vec![],
            is_hidden: false,
            run_all_button: false,
        });

        let mut context = RulesContext::from_storage(&storage);
        let rule_id = context.rule_id_for_index(0, 0).unwrap();

        assert_eq!(
            context.move_rule_between_groups_at(0, 0, 1, 0),
            Some(rule_id.clone())
        );
        assert_eq!(context.rule_id_for_index(1, 0), Some(rule_id));
        assert!(context.rule_id_for_index(0, 0).is_none());
    }

    #[test]
    fn test_move_rule_within_group_preserves_rule_id_order() {
        let mut storage = sample_storage();
        storage.groups[0].programs.push(AppToRun::new_path(
            PathBuf::from(r"C:\helper.lnk"),
            vec![],
            PathBuf::from(r"C:\helper.exe"),
            PriorityClass::Normal,
            false,
        ));

        let mut context = RulesContext::from_storage(&storage);
        let first = context.rule_id_for_index(0, 0).unwrap();
        let second = context.rule_id_for_index(0, 1).unwrap();

        assert_eq!(
            context.move_rule_between_groups_at(0, 0, 0, 2),
            Some(first.clone())
        );
        assert_eq!(context.rule_id_for_index(0, 0), Some(second));
        assert_eq!(context.rule_id_for_index(0, 1), Some(first));
    }
}
