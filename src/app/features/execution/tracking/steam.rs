use super::*;

/// Retain observed plausible main lifetimes even when ambiguity withdraws publication.
/// A temporarily hidden window or a changed runtime key cannot erase ownership evidence.
pub(super) fn prepare_steam_observations<O: RunningAppsOs>(
    apps: &mut RunningApps,
    configured: &[ConfiguredProgramSnapshot],
    observations: &mut SteamObservations,
    snapshot: &ProcessSnapshot,
    os: &O,
) {
    let rules: HashSet<_> = configured
        .iter()
        .filter_map(|c| {
            if let ConfiguredProgramMatcher::Steam { app_id, .. } = c.matcher {
                Some((c.rule_id.clone(), app_id))
            } else {
                None
            }
        })
        .collect();
    apps.steam_helpers.retain(|id, helpers| {
        helpers.retain(|&pid, token| {
            snapshot.names.contains_key(&pid)
                && os
                    .get_process_instance_token(pid)
                    .map_or(true, |current| current == *token)
        });
        rules.contains(id)
    });
    apps.steam_evidence.retain(|id, _| rules.contains(id));
    for c in configured {
        let ConfiguredProgramMatcher::Steam { app_id, executable } = &c.matcher else {
            continue;
        };
        let id = (c.rule_id.clone(), *app_id);
        let helpers = apps.steam_helpers.get(&id);
        let evidence = apps.steam_evidence.entry(id).or_default();
        let plausible = |p: &os_api::steam::GameProcess| {
            os_api::steam::automatic_image(&p.path)
                && !c
                    .additional_processes_normalized
                    .contains(&normalize_process_name(
                        &p.path.file_name().unwrap_or_default().to_string_lossy(),
                    ))
                && helpers.is_none_or(|h| h.get(&p.pid) != Some(&p.creation_token))
        };
        evidence.retain(|pid, p| {
            snapshot.names.contains_key(pid)
                && plausible(p)
                && os
                    .get_process_instance_token(*pid)
                    .map_or(true, |t| t == p.creation_token)
        });
        let Some((_, fresh)) = observations.get_mut(&c.key) else {
            continue;
        };
        if executable.is_none() {
            for p in fresh
                .iter()
                .filter(|p| plausible(p) && snapshot.names.contains_key(&p.pid))
            {
                if evidence.len() < 4096 || evidence.contains_key(&p.pid) {
                    evidence.insert(p.pid, p.clone());
                }
            }
            for p in evidence.values() {
                if !fresh
                    .iter()
                    .any(|f| f.pid == p.pid && f.creation_token == p.creation_token)
                {
                    fresh.push(p.clone());
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)] // One discovery observation, no independent sampler/state owner.
pub(super) fn update_steam_rule<O: RunningAppsOs>(
    apps: &mut RunningApps,
    configured: &ConfiguredProgramSnapshot,
    app_id: u32,
    executable: Option<&Path>,
    metadata_root: Option<PathBuf>,
    candidates: Vec<os_api::steam::GameProcess>,
    claims: &HashMap<(u32, u64), HashSet<u32>>,
    snapshot: &ProcessSnapshot,
    names: &HashMap<String, Vec<u32>>,
    os: &O,
    outcome: &mut RunningAppsIterationOutcome,
) {
    let key = &configured.key;
    // An explicitly selected image outside a Steam stub/junction uses its own directory.
    let root = executable
        .and_then(|e| {
            metadata_root
                .as_ref()
                .filter(|r| os_api::steam::path_within(e, r))
                .cloned()
                .or_else(|| e.parent().map(Path::to_path_buf))
        })
        .or(metadata_root)
        .filter(|root| root.parent().is_some());
    let helpers = apps
        .steam_helpers
        .entry((configured.rule_id.clone(), app_id))
        .or_default();
    let raw_present = !candidates.is_empty();
    // Unknown access is not proof that an established competing lifetime ended.
    let potential_paths: HashSet<_> = candidates
        .iter()
        .filter(|p| {
            snapshot.names.contains_key(&p.pid)
                && os
                    .get_process_instance_token(p.pid)
                    .map_or(true, |t| t == p.creation_token)
                && helpers.get(&p.pid) != Some(&p.creation_token)
                && os_api::steam::automatic_image(&p.path)
                && !configured
                    .additional_processes_normalized
                    .contains(&normalize_process_name(
                        &p.path.file_name().unwrap_or_default().to_string_lossy(),
                    ))
        })
        .map(|p| normalize_path_for_prefix(&p.path))
        .collect();
    let mut mains: Vec<_> = candidates
        .into_iter()
        .filter(|p| {
            snapshot.names.contains_key(&p.pid)
                && os.get_process_instance_token(p.pid).ok() == Some(p.creation_token)
                && os.steam_instance_allowed(p.pid, p.creation_token)
                && if let Some(e) = executable {
                    path_eq_case_insensitive(&p.path, e)
                } else {
                    helpers.get(&p.pid) != Some(&p.creation_token)
                        && !configured.additional_processes_normalized.contains(
                            &normalize_process_name(
                                &p.path.file_name().unwrap_or_default().to_string_lossy(),
                            ),
                        )
                        && os_api::steam::automatic_image(&p.path)
                        && claims
                            .get(&(p.pid, p.creation_token))
                            .is_some_and(|ids| ids.len() == 1)
                }
        })
        .collect();
    let mut conflict = executable.is_none()
        && (helpers.len() >= 4096
            || potential_paths.len() > 1
            || claims.iter().any(|(id, ids)| {
                ids.contains(&app_id) && ids.len() > 1 && snapshot.names.contains_key(&id.0)
            }));
    if conflict {
        mains.clear();
    }
    // No fresh window is normal for a minimized or temporarily hidden main. A fresh
    // ambiguity is different: it withdraws automatic authority rather than freezing a guess.
    if !conflict {
        if let Some(previous) = apps.apps.get(key) {
            for (&pid, &token) in &previous.primary_instances {
                if let Ok((path, current)) = os.get_process_image_path_and_instance_token(pid) {
                    let belongs = executable.map_or_else(
                        || {
                            root.as_ref()
                                .is_some_and(|r| os_api::steam::path_within(&path, r))
                        },
                        |e| path_eq_case_insensitive(&path, e),
                    );
                    if token == current
                        && snapshot.names.contains_key(&pid)
                        && belongs
                        && os_api::steam::eligible_image(&path)
                        && os.steam_instance_allowed(pid, token)
                        && !mains.iter().any(|p| p.pid == pid)
                    {
                        mains.push(os_api::steam::GameProcess {
                            pid,
                            creation_token: token,
                            path,
                            has_window: false,
                        });
                    }
                }
            }
        }
    }
    if executable.is_none()
        && mains
            .iter()
            .map(|p| normalize_path_for_prefix(&p.path))
            .collect::<HashSet<_>>()
            .len()
            > 1
    {
        mains.clear();
        conflict = true;
    }
    let primary: HashMap<_, _> = mains.iter().map(|p| (p.pid, p.creation_token)).collect();
    let mut members: Vec<_> = mains
        .iter()
        .map(|p| BoundProcessInstance {
            pid: p.pid,
            token: p.creation_token,
        })
        .collect();
    if !members.is_empty() {
        let mut extras =
            collect_named_process_pids(&configured.additional_processes_normalized, names, os);
        if configured.manage_descendants {
            let mut children = members.clone();
            extend_with_descendants(snapshot, &mut children, os);
            extras.extend(children);
        }
        for member in extras {
            if primary.contains_key(&member.pid) {
                continue;
            }
            if let Ok((path, token)) = os.get_process_image_path_and_instance_token(member.pid) {
                if token == member.token
                    && os.steam_instance_allowed(member.pid, token)
                    && root
                        .as_ref()
                        .is_some_and(|r| os_api::steam::path_within(&path, r))
                    && os_api::steam::eligible_image(&path)
                {
                    // Never discard a live helper's role under capacity pressure.
                    if helpers.len() < 4096 || helpers.contains_key(&member.pid) {
                        helpers.insert(member.pid, token);
                        push_unique_pid(&mut members, member);
                    }
                }
            }
        }
    }
    let unresolved = executable.is_none() && primary.is_empty() && (raw_present || conflict);
    let previous_unresolved = apps.steam_unresolved.contains(&configured.rule_id);
    if unresolved {
        apps.steam_unresolved.insert(configured.rule_id.clone());
    } else {
        apps.steam_unresolved.remove(&configured.rule_id);
    }
    outcome.changed |= unresolved != previous_unresolved;
    if primary.is_empty() {
        if apps.apps.remove(key).is_some() {
            outcome.changed = true;
            outcome.notifications.push(format!(
                "Game {}: {}",
                if unresolved {
                    "needs process selection"
                } else {
                    "stopped"
                },
                configured.display_name
            ));
        }
        return;
    }
    let old = apps.apps.get(key).map(|a| a.pid_instance_tokens.clone());
    let was_tracked = old.is_some();
    if !was_tracked {
        apps.add_app(
            key,
            members[0].pid,
            configured.group_id.clone(),
            configured.rule_id.clone(),
        );
    }
    let app = apps.apps.get_mut(key).expect("inserted game");
    if !was_tracked {
        app.settings_matched = false;
    }
    app.group_id = configured.group_id.clone();
    app.rule_id = configured.rule_id.clone();
    replace_with_current_process_instances(app, members, snapshot, os);
    app.primary_instances = primary
        .into_iter()
        .filter(|(pid, token)| app.pid_instance_tokens.get(pid) == Some(token))
        .collect();
    // Lifetime can expire during observation. Publish neither empty nor helper-only games.
    if app.primary_instances.is_empty() {
        apps.apps.remove(key);
        outcome.changed |= was_tracked;
        return;
    }
    outcome.changed |= old.as_ref() != Some(&app.pid_instance_tokens);
    if !was_tracked {
        outcome
            .notifications
            .push(format!("Game discovered: {}", configured.display_name));
    }
}
