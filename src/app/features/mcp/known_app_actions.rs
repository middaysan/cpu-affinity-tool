//! Saved composite applications use the same bounded intent as GUI launches.
use super::{
    operations::{ApiError, ApiResult},
    policy::AgentAccess,
    server::LiveAccess,
    services::Services,
};
use crate::app::{
    features::{
        execution::launch::{begin_launch, cancel_launch, identity_for_app, SteamLaunchBegin},
        rules::known_apps::{self, MemberRole, Scope},
    },
    models::AppToRun,
    shared::ids::{GroupId, RuleId},
};
use os_api::{
    agent::{ExactProcess, LaunchSpec},
    OS,
};
use serde_json::json;

pub(super) fn saved_scope(
    app: &AppToRun,
    capture: bool,
    elevated: bool,
) -> Result<Option<Scope>, ApiError> {
    if !app.is_path_target() || app.builtin_process_binding.is_none() {
        return Ok(None);
    }
    let scope = known_apps::project(app)
        .map_err(|error| ApiError::new("unsupported_app_definition", error))?;
    if capture || elevated {
        return Err(ApiError::new("unsupported","Built-in application activation uses ordinary rights and no inherited output capture. Use explicit launch_program for independent managed programs."));
    }
    Ok(Some(scope))
}

pub(super) struct SavedRequest<'a> {
    pub app: &'a AppToRun,
    pub group: &'a GroupId,
    pub rule: &'a RuleId,
    pub restart: bool,
    pub agent: &'a AgentAccess,
    pub live: &'a LiveAccess,
    pub generation: u64,
}

pub(super) fn launch(services: &Services, request: SavedRequest<'_>, scope: Scope) -> ApiResult {
    let SavedRequest {
        app,
        group,
        rule,
        restart,
        agent,
        live,
        generation,
    } = request;
    let path = app
        .bin_path()
        .ok_or_else(|| ApiError::invalid("Known application requires an executable target"))?;
    if !scope.qualify_launch(app) {
        return Err(ApiError::new(
            "unsupported_app_definition",
            "The application installation could not be verified",
        ));
    }
    let app_key = services.control.key(rule).unwrap_or_else(|| app.get_key());
    let mut existing = Vec::new();
    {
        let running = services.running.blocking_read();
        if let Some(tracked) = services
            .control
            .member(&running, &app_key)
            .map(|(_, app)| app)
        {
            for (&pid, &token) in &tracked.primary_instances {
                let process = ExactProcess::open(pid, token, false, false, true)
                    .map_err(|error| ApiError::new("native", error))?;
                if process
                    .state()
                    .map_err(|error| ApiError::new("native", error))?
                    .exited
                {
                    continue;
                }
                let image = OS::get_process_image_path(pid)
                    .map_err(|error| ApiError::new("native", error))?;
                if let Some((root, image)) =
                    os_api::app_identity::canonical_scope(&scope.root, &image)
                {
                    if scope.witness_role_under_root(&root, &image) == Some(MemberRole::Main) {
                        existing.push((pid, token));
                    }
                }
            }
        }
    }
    if !existing.is_empty() {
        if restart {
            return Err(ApiError::new(
                "still_running",
                "Close the application's exact main instances before restart",
            ));
        }
        return Ok(
            json!({"status":"already_tracked","instances":existing.into_iter().map(|(pid,token)|json!({"pid":pid,"creation_token":token.to_string()})).collect::<Vec<_>>()}),
        );
    }
    if !live.current(agent, generation) {
        return Err(ApiError::new(
            "revoked",
            "Access revoked before application activation",
        ));
    }
    let identity = identity_for_app(app).ok_or_else(|| {
        ApiError::new(
            "unsupported_app_definition",
            "Application launch identity is unavailable",
        )
    })?;
    let dispatch = if app.additional_processes.is_empty() {
        None
    } else {
        match begin_launch(rule, identity.clone()) {
            SteamLaunchBegin::AlreadyPending => return Ok(ack(group, rule)),
            SteamLaunchBegin::Busy => {
                return Err(ApiError::new(
                    "busy",
                    "Too many pending application launches",
                ))
            }
            SteamLaunchBegin::Started(ticket) => Some(ticket),
        }
    };
    let result = (|| {
        if !live.current(agent, generation) {
            return Err("Access revoked before application activation".into());
        }
        if scope.is_launcher() {
            return known_apps::launch_ordinary(app);
        }
        let child = services.control.launch(rule, &app_key, path, |owner| {
            let cpus = owner.fields.affinity.then(|| {
                (0..usize::BITS as usize)
                    .filter(|cpu| owner.mask & (1usize << cpu) != 0)
                    .collect()
            });
            let mut child = os_api::agent::launch_executable(&LaunchSpec {
                executable: path.into(),
                args: app.args.clone(),
                cwd: path.parent().map(Into::into),
                cpus,
                priority: super::actions::launch_priority(
                    owner.app.priority,
                    owner.fields.priority,
                )
                .map_err(|error| error.message)?,
                elevated: false,
                capture: false,
                environment: Default::default(),
            })?;
            services
                .scheduling
                .started(child.pid, child.creation_token, owner.fields)
                .map_err(|error| format!("Scheduling handoff refused: {error:?}"))?;
            if !live.current(agent, generation) {
                return Err("Access revoked before application resume".into());
            }
            child.resume()?;
            Ok(child)
        })?;
        if scope.enabled(path) {
            let mut running = services.running.blocking_write();
            running.mutate_membership(|running| {
                let key = app_key.clone();
                if !running.apps.contains_key(&key) {
                    running.add_app(&key, child.pid, group.clone(), rule.clone());
                }
                if let Some(tracked) = running.apps.get_mut(&key) {
                    if !tracked.pids.contains(&child.pid) {
                        tracked.pids.push(child.pid);
                    }
                    tracked
                        .pid_instance_tokens
                        .insert(child.pid, child.creation_token);
                    tracked
                        .primary_instances
                        .insert(child.pid, child.creation_token);
                }
            });
        }
        Ok(())
    })();
    if let Err(error) = result {
        if let Some(ticket) = dispatch {
            cancel_launch(rule, identity, ticket);
        }
        return Err(ApiError::new("native", error));
    }
    services.wake.request_repaint();
    Ok(ack(group, rule))
}

fn ack(group: &GroupId, rule: &RuleId) -> serde_json::Value {
    json!({"status":"activation_requested","group_id":group,"rule_id":rule,"pid":null,"creation_token":null,"capture_supported":false,"elevated":false,"settings":"Pending application discovery and native readback","note":"Activation acknowledgement does not prove startup. Read tracked CPU/RAM and exact process identities; the launcher is not a managed application process."})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::features::rules::known_apps::Entry;
    #[test]
    fn s107_saved_agent_launch_uses_composite_activation_and_rejects_unknown_metadata() {
        let (state, _) = crate::app::features::execution::known_tracking_fixture();
        let mut app = state.groups[0].programs[0].clone();
        assert_eq!(
            saved_scope(&app, false, false).unwrap().unwrap().entry,
            Entry::Steam
        );
        assert!(saved_scope(&app, true, false).is_err());
        assert!(saved_scope(&app, false, true).is_err());
        app.builtin_process_binding.as_mut().unwrap()["definition_version"] = 99.into();
        assert!(saved_scope(&app, false, false).is_err());
        app.builtin_process_binding = None;
        assert!(
            saved_scope(&app, true, true).unwrap().is_none(),
            "old explicit rules keep their launch contract"
        );
    }
}
