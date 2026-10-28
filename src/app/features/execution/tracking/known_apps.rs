use super::*;
use crate::app::features::rules::known_apps::{manual_names, MemberRole, Scope};

#[allow(clippy::too_many_arguments)]
pub(super) fn update_known_rule<O: RunningAppsOs>(
    apps: &mut RunningApps,
    configured: &ConfiguredProgramSnapshot,
    app: &AppToRun,
    scope: Option<&Scope>,
    snapshot: &ProcessSnapshot,
    names: &HashMap<String, Vec<u32>>,
    os: &O,
    outcome: &mut RunningAppsIterationOutcome,
) {
    let key = &configured.key;
    let mut members = collect_named_process_pids(&manual_names(app), names, os);
    let mut primaries = HashMap::new();
    let mut scoped = Vec::new();
    if let Some(scope) = scope {
        let mut wanted = configured.additional_processes_normalized.clone();
        wanted.extend(scope.primary_names());
        wanted.sort();
        wanted.dedup();
        for candidate in collect_named_process_pids(&wanted, names, os) {
            let Ok((image, token)) = os.get_process_image_path_and_instance_token(candidate.pid)
            else {
                continue;
            };
            if token != candidate.token {
                continue;
            }
            let Some(role) = os.known_instance_role(candidate.pid, token, scope, &image) else {
                continue;
            };
            if role == MemberRole::Main {
                primaries.insert(candidate.pid, token);
            }
            if scope.enabled(&image) {
                scoped.push((candidate, role));
            }
        }
        // A detached renderer cannot keep the client running or suppress a main launch.
        for (candidate, role) in scoped {
            if role == MemberRole::Main || !primaries.is_empty() {
                push_unique_pid(&mut members, candidate);
            }
        }
        if configured.manage_descendants {
            let mut descendants: Vec<_> = primaries
                .iter()
                .map(|(&pid, &token)| BoundProcessInstance { pid, token })
                .collect();
            extend_with_descendants(snapshot, &mut descendants, os);
            for member in descendants {
                push_unique_pid(&mut members, member);
            }
        }
    }
    members.sort_unstable_by_key(|member| (member.pid, member.token));
    if members.is_empty() {
        if apps.apps.contains_key(key) {
            apps.remove_app(key);
            outcome.changed = true;
        }
        return;
    }
    if !apps.apps.contains_key(key) {
        apps.add_app(
            key,
            members[0].pid,
            configured.group_id.clone(),
            configured.rule_id.clone(),
        );
        outcome
            .notifications
            .push(format!("App detected: {}", configured.display_name));
        outcome.changed = true;
    }
    let tracked = apps.apps.get_mut(key).expect("entry just admitted");
    let old = (
        tracked.pids.clone(),
        tracked.pid_instance_tokens.clone(),
        tracked.primary_instances.clone(),
    );
    tracked.group_id = configured.group_id.clone();
    tracked.rule_id = configured.rule_id.clone();
    replace_with_current_process_instances(tracked, members, snapshot, os);
    primaries.retain(|pid, token| {
        snapshot.names.contains_key(pid) && os.get_process_instance_token(*pid).ok() == Some(*token)
    });
    tracked.primary_instances = primaries;
    outcome.changed |= old
        != (
            tracked.pids.clone(),
            tracked.pid_instance_tokens.clone(),
            tracked.primary_instances.clone(),
        );
}
