//! Ordinary group edits, shared below GUI and agent adapters.
use super::RulesContext;
use crate::app::{
    models::{AppStateStorage, CoreGroup},
    shared::ids::GroupId,
};

#[derive(Clone, Debug)]
pub(crate) struct GroupEdit {
    /// None creates a group; Some edits that logical group.
    pub group: Option<GroupId>,
    pub name: String,
    pub cpus: Vec<usize>,
    pub run_all: bool,
    /// Only an explicit agent edit changes visibility; GUI leaves it untouched.
    pub hidden: Option<bool>,
}
impl GroupEdit {
    pub(crate) fn apply(
        self,
        state: &mut AppStateStorage,
        rules: &mut RulesContext,
        total: usize,
    ) -> Result<(), String> {
        let (name, cores) = validate_group_values(&self.name, &self.cpus, total)?;
        if let Some(id) = self.group {
            let group = rules
                .group_index_for_id(&id)
                .and_then(|index| state.groups.get_mut(index))
                .ok_or("Group no longer exists")?;
            group.name = name;
            group.cores = cores;
            group.run_all_button = self.run_all;
            if let Some(hidden) = self.hidden {
                group.is_hidden = hidden;
            }
        } else {
            state.groups.push(CoreGroup {
                name,
                cores,
                programs: Vec::new(),
                is_hidden: false,
                run_all_button: self.run_all,
            });
            rules.append_group();
        }
        Ok(())
    }
}

pub(crate) fn validate_group_values(
    name: &str,
    cpus: &[usize],
    total: usize,
) -> Result<(String, Vec<usize>), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Group name cannot be empty".into());
    }
    if name.len() > 256 || name.contains('\0') {
        return Err("Group name must contain at most 256 bytes and no NUL".into());
    }
    if cpus.is_empty() {
        return Err("At least one core must be selected".into());
    }
    if cpus.len() > 4096 || cpus.iter().any(|cpu| *cpu >= total) {
        return Err("Select existing logical CPUs".into());
    }
    let mut cpus = cpus.to_vec();
    cpus.sort_unstable();
    cpus.dedup();
    Ok((name.into(), cpus))
}
