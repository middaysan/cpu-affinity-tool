//! Per-rule drafts share the existing session configuration and durable writer.
use super::{feature, Config, GameRule};
use crate::app::features::system_tweaks::{
    config::{ModulePreference, Profile, TuningConfig},
    platform::contract::{ActivationMode, ProfileId},
    service::SaveError,
    tweaks::background_workload::{configured, ID},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RulePolicy {
    pub enabled: bool,
    pub config: Config,
}

/// Only the internal per-rule shape is disposable; arbitrary/legacy profiles
/// and any profile still referenced by recovery retain their exact payload.
pub(crate) fn prune_deleted_rule_profiles(
    saved: &mut TuningConfig,
    live_rules: &std::collections::BTreeSet<String>,
    retained: impl Fn(ProfileId) -> bool,
) -> bool {
    let before = saved.profiles.len();
    saved.profiles.retain(|profile| {
        if retained(profile.id)
            || profile.modules.len() != 1
            || saved
                .associations
                .iter()
                .any(|association| association.profile == profile.id)
            || saved
                .manual_targets
                .iter()
                .any(|target| target.profile == profile.id)
        {
            return true;
        }
        let preference = &profile.modules[0];
        if preference.feature.as_str() != ID || preference.mode != ActivationMode::Session {
            return true;
        }
        let Ok(config) = configured(&preference.config) else {
            return true;
        };
        let [game] = config.games.as_slice() else {
            return true;
        };
        profile.name != format!("Background workload [rule {}]", game.rule)
            || live_rules.contains(&game.rule)
    });
    before != saved.profiles.len()
}

pub(crate) fn rule_policy(saved: &TuningConfig, rule: &str) -> Result<RulePolicy, SaveError> {
    let mut result = None;
    for profile in &saved.profiles {
        for preference in &profile.modules {
            if preference.feature.as_str() != ID {
                continue;
            }
            let mut config = configured(&preference.config).map_err(|_| SaveError::Invalid)?;
            if let Some(game) = config.games.iter().find(|game| game.rule == rule).cloned() {
                if result.is_some() || preference.mode != ActivationMode::Session {
                    return Err(SaveError::Invalid);
                }
                config.games = vec![game];
                result = Some(RulePolicy {
                    enabled: preference.enabled,
                    config,
                });
            }
        }
    }
    Ok(result.unwrap_or_default())
}

pub(crate) fn put_rule_policy(
    saved: &mut TuningConfig,
    game: GameRule,
    draft: &RulePolicy,
) -> Result<(), SaveError> {
    let previous = rule_policy(saved, &game.rule)?;
    let mut desired = draft.clone();
    desired.config = desired.config.without_cpu_placement();
    // Retain original source metadata when the logical rule moves groups.
    desired.config.games = vec![previous.config.games.first().cloned().unwrap_or(game)];
    if !desired.config.valid() {
        return Err(SaveError::Invalid);
    }
    let matching = saved.profiles.iter().enumerate().find_map(|(pi, profile)| {
        profile
            .modules
            .iter()
            .enumerate()
            .find_map(|(mi, preference)| {
                (preference.feature.as_str() == ID)
                    .then(|| configured(&preference.config).ok())
                    .flatten()
                    .filter(|config| {
                        config
                            .games
                            .iter()
                            .any(|item| item.rule == desired.config.games[0].rule)
                    })
                    .map(|config| (pi, mi, config))
            })
    });
    if matching.is_none() && !desired.enabled && draft.config == Config::default() {
        return Ok(());
    }
    let preference = ModulePreference {
        feature: feature(),
        enabled: desired.enabled,
        mode: ActivationMode::Session,
        config: desired.config.envelope(),
    };
    if let Some((pi, mi, mut config)) = matching {
        if config.games.len() == 1 {
            saved.profiles[pi].modules[mi] = preference;
            return Ok(());
        }
        // Split an old shared selection only in the candidate; other games keep
        // their exact policy, profile identity and activation state.
        config
            .games
            .retain(|item| item.rule != desired.config.games[0].rule);
        saved.profiles[pi].modules[mi].config = config.envelope();
    }
    if saved.profiles.len() >= 64 {
        return Err(SaveError::Invalid);
    }
    let id = ProfileId::new(saved.next_profile).map_err(|_| SaveError::Invalid)?;
    saved.next_profile = saved
        .next_profile
        .checked_add(1)
        .ok_or(SaveError::Invalid)?;
    saved.profiles.push(Profile {
        version: 1,
        id,
        name: format!(
            "Background workload [rule {}]",
            desired.config.games[0].rule
        ),
        modules: vec![preference],
    });
    Ok(())
}
