//! Persisted ordering and explicit executable-scoped choices, never process handles.
use crate::app::{features::rules::RulesContext, models::AppStateStorage, shared::ids::RuleId};
use serde::{Deserialize, Serialize};

const MAX_RULES: usize = 4096;
const MAX_CHOICES: usize = 128;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Choice {
    pub image: String,
    pub owner: RuleId,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    version: u32,
    generation: u64,
    pub order: Vec<RuleId>,
    pub legacy: Vec<RuleId>,
    pub choices: Vec<Choice>,
}

pub(crate) fn image_key(image: &str) -> String {
    if !cfg!(target_os = "windows") {
        return image.into();
    }
    let image = image
        .strip_prefix(r"\\?\UNC\")
        .map(|value| format!(r"\\{value}"))
        .unwrap_or_else(|| image.strip_prefix(r"\\?\").unwrap_or(image).into());
    image.replace('/', r"\").to_ascii_lowercase()
}

fn rule_ids(state: &AppStateStorage) -> Vec<RuleId> {
    RulesContext::from_storage(state)
        .to_persisted_identities()
        .groups
        .into_iter()
        .flat_map(|group| group.rule_ids)
        .collect()
}

impl Policy {
    pub fn read(state: &AppStateStorage) -> Result<Self, String> {
        let Some(raw) = state.process_control_raw() else {
            let ids = rule_ids(state);
            if ids.len() > MAX_RULES {
                return Err("Too many rules for process control".into());
            }
            return Ok(Self {
                version: 2,
                generation: 0,
                order: ids.clone(),
                legacy: ids,
                choices: vec![],
            });
        };
        if raw.get().len() > 3 * 1024 * 1024 {
            return Err("Process control settings exceed the supported size".into());
        }
        let policy: Self = serde_json::from_str(raw.get())
            .map_err(|_| "Process control settings are unsupported")?;
        let valid_id = |id: &RuleId| !id.0.is_empty() && id.0.len() <= 128;
        let mut ids = std::collections::HashSet::new();
        let mut images = std::collections::HashSet::new();
        let mut preferences = std::collections::HashSet::new();
        if !matches!(policy.version, 1 | 2)
            || policy.order.len() > MAX_RULES
            || policy.legacy.len() > MAX_RULES
            || policy.choices.len() > MAX_CHOICES
            || policy
                .order
                .iter()
                .any(|id| !valid_id(id) || !ids.insert(id.clone()))
            || policy.legacy.iter().any(|id| !ids.contains(id))
            || policy.choices.iter().any(|choice| {
                !valid_id(&choice.owner)
                    || choice.image.is_empty()
                    || choice.image.len() > 16000
                    || choice.image.contains('\0')
                    || choice.image != image_key(&choice.image)
                    || (policy.version == 1 && !images.insert(choice.image.clone()))
                    || !preferences.insert((choice.image.clone(), choice.owner.clone()))
            })
        {
            return Err("Process control settings are unsupported".into());
        }
        Ok(policy)
    }

    pub fn prepare_save(
        before: &AppStateStorage,
        next: &mut AppStateStorage,
    ) -> Result<(), String> {
        let mut policy = if next.process_control_raw().map(|raw| raw.get())
            != before.process_control_raw().map(|raw| raw.get())
        {
            Self::read(next)?
        } else {
            Self::read(before)?
        };
        // Only the application writer may change ownership metadata.
        let next_ids = rule_ids(next);
        if next_ids.len() > MAX_RULES {
            return Err("Too many rules for process control".into());
        }
        policy.order.retain(|id| next_ids.contains(id));
        policy.legacy.retain(|id| next_ids.contains(id));
        policy
            .choices
            .retain(|choice| next_ids.contains(&choice.owner));
        for id in next_ids {
            if !policy.order.contains(&id) {
                policy.order.push(id);
            }
        }
        policy.version = 2;
        policy.generation = policy
            .generation
            .checked_add(1)
            .ok_or("Process control revision exhausted")?;
        policy.store(next)
    }

    pub fn choose(state: &mut AppStateStorage, choice: Choice) -> Result<(), String> {
        let mut policy = Self::read(state)?;
        if !policy.order.contains(&choice.owner)
            || choice.image.is_empty()
            || choice.image.len() > 16000
            || choice.image.contains('\0')
        {
            return Err("Invalid process control choice".into());
        }
        let choice = Choice {
            image: image_key(&choice.image),
            ..choice
        };
        // Most-recent eligible preference wins. Retain older owners for disjoint
        // overlap sets of the same executable; never make one pair erase another.
        if let Some(index) = policy.choices.iter().position(|current| current == &choice) {
            policy.choices.remove(index);
        } else if policy.choices.len() >= MAX_CHOICES {
            return Err("Too many saved process control choices".into());
        }
        policy.choices.insert(0, choice);
        policy.version = 2;
        policy.store(state)
    }

    fn store(&self, state: &mut AppStateStorage) -> Result<(), String> {
        state.set_process_control_raw(
            serde_json::to_string(self).map_err(|_| "Could not encode process control")?,
        );
        Ok(())
    }
    pub fn rank(&self, rule: &RuleId) -> usize {
        self.order
            .iter()
            .position(|id| id == rule)
            .unwrap_or(usize::MAX)
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn s110_policy_preserves_legacy_lists_and_appends_new_rules() {
        let before = AppStateStorage::test_fixture(|_| {});
        let mut next = before.clone();
        let old = serde_json::to_value(&before.groups).unwrap();
        Policy::prepare_save(&before, &mut next).unwrap();
        assert_eq!(serde_json::to_value(&next.groups).unwrap(), old);
        let policy = Policy::read(&next).unwrap();
        assert_eq!(policy.legacy, rule_ids(&before));
        assert_eq!(policy.order, rule_ids(&before));
        let mut v1 = next.clone();
        let mut raw = serde_json::to_value(&policy).unwrap();
        raw["version"] = serde_json::json!(1);
        v1.set_process_control_raw(raw.to_string());
        assert!(Policy::read(&v1).is_ok());

        #[cfg(target_os = "windows")]
        assert_eq!(
            image_key(r"\\?\C:\App\Helper.exe"),
            image_key("c:/app/helper.exe")
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            image_key(r"\\?\UNC\server\share\Helper.exe"),
            image_key(r"\\server\share\Helper.exe")
        );
    }
}
