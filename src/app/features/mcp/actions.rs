use super::{
    operations::{ApiError, ApiResult},
    policy::AgentAccess,
    server::LiveAccess,
    services::{decode, Services},
};
use crate::app::{
    models::LaunchTarget,
    shared::ids::{GroupId, RuleId},
};
use os_api::{
    agent::{ExactProcess, LaunchSpec},
    PriorityClass, OS,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Args {
    request_key: Option<String>,
    expected_revision: Option<String>,
    group_id: Option<String>,
    rule_id: Option<String>,
    path: Option<PathBuf>,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    cpus: Option<Vec<usize>>,
    priority: Option<PriorityClass>,
    elevated: bool,
    capture: bool,
    environment: BTreeMap<String, String>,
    name: Option<String>,
    pid: Option<u32>,
    creation_token: Option<String>,
    run_id: Option<String>,
    stream: Option<String>,
    after: Option<String>,
    limit: Option<usize>,
    offset: usize,
    force: bool,
}
fn needed<T>(v: Option<T>, label: &str) -> Result<T, ApiError> {
    v.ok_or_else(|| ApiError::invalid(format!("{label} is required")))
}
fn native(error: String) -> ApiError {
    ApiError::new("native", error)
}
fn launch_priority(priority: PriorityClass, classic: bool) -> Result<PriorityClass, ApiError> {
    let priority = if classic {
        priority
    } else {
        PriorityClass::Normal
    };
    if priority == PriorityClass::Realtime {
        return Err(ApiError::invalid("Realtime priority is not supported"));
    }
    Ok(priority)
}
fn rollback_registered_root(
    running: &mut crate::app::models::RunningApps,
    key: &crate::app::models::AppRuntimeKey,
    pid: u32,
    token: u64,
) {
    running.mutate_membership(|running| {
        if let Some(app) = running
            .apps
            .get_mut(key)
            .filter(|app| app.pid_instance_tokens.get(&pid) == Some(&token))
        {
            app.pids.retain(|p| *p != pid);
            app.pid_instance_tokens.remove(&pid);
            if app.pids.is_empty() {
                running.apps.remove(key);
            }
        }
    });
}
impl Services {
    pub(super) async fn remote_read(
        &self,
        name: &str,
        args: Value,
        agent: &AgentAccess,
    ) -> Option<ApiResult> {
        if matches!(name, "list_installed_apps" | "resolve_installed_app") {
            let name = name.to_owned();
            return Some(
                self.native_query(move || super::discovery::query(&name, args))
                    .await,
            );
        }
        if name == "export_cpu_history" {
            let cpu = self.cpu.clone();
            let wake = self.wake.clone();
            return Some(
                self.native_query(move || {
                    cpu.export(wake, args)
                        .map_err(|e| ApiError::new("cpu_export", e))
                })
                .await,
            );
        }
        if matches!(
            name,
            "read_tweak_module" | "list_tweak_profiles" | "list_tweak_associations"
        ) {
            let services = self.clone();
            let name = name.to_owned();
            return Some(
                self.native_query(move || services.tweak_configuration_read(&name, args))
                    .await,
            );
        }
        if !matches!(
            name,
            "list_processes" | "inspect_process" | "list_runs" | "read_run_output"
        ) {
            return None;
        }
        let result = (|| {
            let a: Args = decode(args)?;
            Ok(a)
        })();
        let a = match result {
            Ok(a) => a,
            Err(e) => return Some(Err(e)),
        };
        let services = self.clone();
        let name = name.to_owned();
        let agent = agent.clone();
        Some(self.native_query(move||{
            let _=&a.request_key;
            match name.as_str(){
                "list_processes"=>{let limit=a.limit.unwrap_or(50);if limit==0||limit>256||a.offset>100000{return Err(ApiError::invalid("Invalid process page"));}
                    let mut rows=OS::get_all_process_names();rows.sort_unstable_by_key(|(pid,_)|*pid);
                    let page:Vec<_>=rows.iter().skip(a.offset).take(limit).map(|(pid,name)|json!({"pid":pid,"name":name,"creation_token":OS::get_process_instance_token(*pid).ok().map(|t|t.to_string())})).collect();
                    Ok(json!({"rows":page,"total":rows.len(),"next_offset":(a.offset+limit<rows.len()).then_some(a.offset+limit),"snapshot":"live on-demand; membership can change between pages; inspect the exact creation token before control"}))},
                "inspect_process"=>{let pid=needed(a.pid,"pid")?;let token=needed(a.creation_token,"creation_token")?.parse().map_err(|_|ApiError::invalid("Invalid creation_token"))?;
                    let p=ExactProcess::open(pid,token,false,false,true).map_err(native)?;let s=p.state().map_err(native)?;
                    let owners=services.running.blocking_read();let owners:Vec<_>=owners.apps.values().filter(|app|app.pid_instance_tokens.get(&pid)==Some(&token)).map(|app|json!({"group_id":app.group_id,"rule_id":app.rule_id})).collect();
                    Ok(json!({"pid":s.pid,"creation_token":s.creation_token.to_string(),"affinity_mask":s.affinity.to_string(),"priority":s.priority,"elevated":s.elevated,"exited":s.exited,"rule_owners":owners,"image":OS::get_process_image_path(pid).ok()}))},
                "list_runs"=>Ok(services.runs.list(&agent.id)),
                "read_run_output"=>services.runs.output(&agent.id,&needed(a.run_id,"run_id")?,a.stream.as_deref().unwrap_or("stdout"),a.after.as_deref().unwrap_or("0").parse().map_err(|_|ApiError::invalid("Invalid output cursor"))?,a.limit.unwrap_or(4096)).map_err(native),
                _=>unreachable!()
            }
        }).await)
    }
    pub(super) async fn mutate(
        &self,
        name: &str,
        args: Value,
        agent: &AgentAccess,
        live: &Arc<LiveAccess>,
        generation: u64,
    ) -> ApiResult {
        if matches!(
            name,
            "create_group"
                | "update_group"
                | "delete_group"
                | "move_group"
                | "create_rule"
                | "update_rule"
                | "delete_rule"
                | "move_rule"
                | "update_preferences"
        ) {
            return self.configure(name, args, agent, live, generation).await;
        }
        if name.starts_with("tweak_") || name == "confirm_display" {
            return self.tweak(name, args, agent, live, generation).await;
        }
        let a: Args = decode(args)?;
        let services = self.clone();
        let name = name.to_owned();
        let agent = agent.clone();
        let live = live.clone();
        self.native_query(move||{
            if !live.current(&agent,generation){return Err(ApiError::new("revoked","Access was revoked before native execution"));}
            let snapshot=services.tuning.snapshot();
            if a.expected_revision.as_ref().is_some_and(|revision|revision!=&snapshot.revision.to_string()){return Err(ApiError::new("revision_conflict","Configuration changed"));}
            match name.as_str(){
                "stop_run"=>{
                    services.runs.stop(&agent.id,&needed(a.run_id,"run_id")?,a.force,true).map_err(native)
                },
                "restart_run"=>{
                    let id=needed(a.run_id,"run_id")?;let (label,spec)=services.runs.restart_spec(&agent.id,&id).map_err(native)?;
                    let mut result=services.runs.launch(&agent.id,label,spec,false,|_|if live.current(&agent,generation){Ok(())}else{Err("Access revoked before resume".into())}).map_err(native)?;result["previous_run_id"]=json!(id);Ok(result)
                },
                "launch_program"|"launch_saved"|"restart_saved"=>{
                    let mut owner=None;
                    let (path,args,cpus,priority,label)=if name!="launch_program"{
                        let group=GroupId(needed(a.group_id,"group_id")?);let rule=RuleId(needed(a.rule_id,"rule_id")?);
                        let g=snapshot.rules.group_index_for_id(&group).ok_or_else(||ApiError::invalid("Unknown group"))?;
                        let r=snapshot.rules.rule_index_for_id(g,&rule).ok_or_else(||ApiError::invalid("Unknown rule"))?;
                        let app=&snapshot.state.groups[g].programs[r];
                        let fields=crate::app::features::system_tweaks::tweaks::scheduling::fields_for_rule(&snapshot.state,&group,&rule);
                        let priority=launch_priority(app.priority,fields.priority)?;
                        let path=match &app.launch_target{
                            LaunchTarget::Path{bin_path,..}=>bin_path.clone(),
                            LaunchTarget::Installed{aumid}=>{
                                if a.elevated||a.capture{return Err(ApiError::new("unsupported","Installed-app activation supports ordinary shell rights and no inherited stdout/stderr capture"));}
                                {
                                    let running=services.running.blocking_read();
                                    if let Some(existing)=running.apps.get(&app.get_key()){
                                        if name=="restart_saved"{
                                            if existing.pid_instance_tokens.is_empty(){return Err(ApiError::new("not_ready","Previous installed-app process identities are unresolved"));}
                                            for (&pid,&token) in &existing.pid_instance_tokens{let old=ExactProcess::open(pid,token,false,false,false).map_err(native)?;if !old.state().map_err(native)?.exited{return Err(ApiError::new("still_running","Confirm the installed app's old exact instances have exited before restart"));}}
                                        }else{return Ok(json!({"status":"already_tracked","instances":existing.pid_instance_tokens.iter().map(|(pid,token)|json!({"pid":pid,"creation_token":token.to_string()})).collect::<Vec<_>>()}));}
                                    }
                                }
                                if !live.current(&agent,generation){return Err(ApiError::new("revoked","Access revoked before installed-app activation"));}
                                os_api::agent::activate_registered_aumid(aumid).map_err(native)?;
                                return Ok(json!({"status":"activation_requested","group_id":group,"rule_id":rule,"pid":null,"creation_token":null,"capture_supported":false,"elevated":false,"settings":"Pending discovery and native readback by the existing saved-rule monitor","note":"Windows may reuse an existing app instance. Read tracked CPU/process data for verified PID and creation token; activation acknowledgement is not proof of process startup."}));
                            }
                        };
                        owner=Some((group,rule,app.get_key(),fields));
                        (path,app.args.clone(),fields.affinity.then(||snapshot.state.groups[g].cores.clone()),priority,app.name.clone())
                    }else{(needed(a.path,"path")?,a.args,a.cpus,a.priority.unwrap_or(PriorityClass::Normal),a.name.unwrap_or_else(||"Agent program".into()))};
                    let priority=launch_priority(priority,true)?;
                    let mut running=services.running.blocking_write();
                    if let Some((_,_,key,_))=&owner{if let Some(existing)=running.apps.get(key){
                        if name=="restart_saved"{
                            if existing.pid_instance_tokens.is_empty(){return Err(ApiError::new("not_ready","Old process identities are unresolved"));}
                            for (&pid,&token) in &existing.pid_instance_tokens{let old=ExactProcess::open(pid,token,false,false,true).map_err(native)?;if !old.state().map_err(native)?.exited{return Err(ApiError::new("still_running","Stop the old exact instances and confirm exit before restarting"));}}
                            running.remove_app(key);
                        }else{let instances:Vec<_>=existing.pid_instance_tokens.iter().map(|(pid,token)|json!({"pid":pid,"creation_token":token.to_string()})).collect();return Ok(json!({"status":"already_tracked","instances":instances,"action":"No new process was launched"}));}
                    }}
                    let mut registered=None;
                    let result=services.runs.launch(&agent.id,label,LaunchSpec{executable:path,args,cwd:a.cwd,cpus,priority,elevated:a.elevated,capture:a.capture,environment:a.environment},owner.is_some(),|child|{
                        if !live.current(&agent,generation){return Err("Access revoked before resume".into());}
                        if let Some((group,rule,key,fields))=&owner{
                            services.scheduling.started(child.pid,child.creation_token,*fields).map_err(|e|format!("{e:?}"))?;
                            running.add_app(key,child.pid,group.clone(),rule.clone());
                            running.mutate_membership(|apps|{apps.apps.get_mut(key).unwrap().pid_instance_tokens.insert(child.pid,child.creation_token);});
                            registered=Some((key.clone(),child.pid,child.creation_token));
                        }Ok(())
                    });
                    if result.is_err(){if let Some((key,pid,token))=registered{rollback_registered_root(&mut running,&key,pid,token);}}
                    let result=result.map_err(native)?;services.wake.request_repaint();Ok(result)
                },
                "set_process_settings"|"stop_process"|"close_process"|"focus_process"=>{
                    let pid=needed(a.pid,"pid")?;let token=needed(a.creation_token,"creation_token")?.parse::<u64>().map_err(|_|ApiError::invalid("Invalid creation_token"))?;
                    let running=services.running.blocking_read();
                    if name=="set_process_settings"&&running.apps.values().any(|app|app.pid_instance_tokens.get(&pid)==Some(&token)){return Err(ApiError::new("owner_conflict","A saved rule owns this process. Edit its rule/group so the reconciler and System Tweaks stay consistent"));}
                    let p=ExactProcess::open(pid,token,name=="set_process_settings",name=="stop_process",true).map_err(native)?;
                    if name=="focus_process" {let alive=!p.state().map_err(native)?.exited;return Ok(json!({"pid":pid,"creation_token":token.to_string(),"focus_requested":alive&&OS::focus_window_by_pid(pid),"foreground_policy":"Windows may refuse foreground activation"}));}
                    if name=="close_process" {return Ok(json!({"pid":pid,"creation_token":token.to_string(),"window_close_requested":p.request_close().map_err(native)?,"forced":false}));}
                    if name=="stop_process"{p.terminate().map_err(native)?;return Ok(json!({"pid":pid,"creation_token":token.to_string(),"termination_requested":true,"exit_confirmed":p.state().ok().map(|s|s.exited)}));}
                    if a.cpus.is_none()&&a.priority.is_none(){return Err(ApiError::invalid("Supply cpus or priority"));}
                    services.scheduling.external_write(pid,token,os_api::gaming::ProcessSettingsFields{affinity:a.cpus.is_some(),priority:a.priority.is_some()}).map_err(|error|ApiError::new("owner_conflict",format!("A System Tweaks scheduling field is reserved: {error:?}")))?;
                    let affinity=a.cpus.as_ref().map(|cpus|p.set_affinity(cpus));let priority=a.priority.map(|priority|p.set_priority(priority));
                    Ok(json!({"pid":pid,"creation_token":token.to_string(),"affinity_result":affinity,"priority_result":priority,"readback":p.state().ok().map(|s|json!({"affinity_mask":s.affinity.to_string(),"priority":s.priority}))}))
                },
                "create_shortcut"=>{
                    let group=GroupId(needed(a.group_id,"group_id")?);let rule=RuleId(needed(a.rule_id,"rule_id")?);
                    let path=crate::app::features::shortcut::create_saved_rule_shortcut(&snapshot.state,&snapshot.rules,group,rule,&mut crate::app::features::shortcut::SystemRuleShortcutPlatform).map_err(|e|ApiError::new("shortcut",e.to_user_message()))?;
                    Ok(json!({"path":path}))
                },
                _=>Err(ApiError::new("unknown_tool","Unsupported command"))
            }
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_launch_priority_rejects_realtime_before_either_target_is_dispatched() {
        assert_eq!(
            launch_priority(PriorityClass::Realtime, true)
                .unwrap_err()
                .code,
            "validation"
        );
        assert_eq!(
            launch_priority(PriorityClass::Normal, true).unwrap(),
            PriorityClass::Normal
        );
        assert_eq!(
            launch_priority(PriorityClass::High, true).unwrap(),
            PriorityClass::High
        );
        assert_eq!(
            launch_priority(PriorityClass::Realtime, false).unwrap(),
            PriorityClass::Normal,
            "A module-reserved field must retain the existing neutral launch policy"
        );
    }
    #[test]
    fn failed_resume_rolls_back_only_the_exact_newly_registered_root() {
        use crate::app::models::RunningApps;
        let input: super::super::super::rules::commands::RuleInput =
            serde_json::from_value(json!({"name":"Child","path":"C:\\Tools\\child.exe"})).unwrap();
        let key = input.into_app().unwrap().get_key();
        let mut running = RunningApps::default();
        running.add_app(&key, 42, GroupId("g".into()), RuleId("r".into()));
        running
            .apps
            .get_mut(&key)
            .unwrap()
            .pid_instance_tokens
            .insert(42, 20);
        rollback_registered_root(&mut running, &key, 42, 10);
        assert!(running.apps.contains_key(&key));
        rollback_registered_root(&mut running, &key, 42, 20);
        assert!(!running.apps.contains_key(&key));
    }
}
