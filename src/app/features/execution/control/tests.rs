use super::policy::{Choice, Policy};
use super::*;
use crate::app::{
    features::rules::RulesContext,
    models::{AppStateStorage, AppToRun, CoreGroup, RunningApps},
    shared::ids::RuleId,
};
use os_api::{gaming::ProcessSettingsFields, PriorityClass};
use std::path::Path;
pub(crate) fn fixture() -> AppStateStorage {
    let mut state = AppStateStorage::test_fixture(|state| {
        state.groups = (0..2)
            .map(|index| {
                let path = format!(r"C:\Apps\app{index}.exe");
                let mut app = AppToRun::new_path(
                    path.clone().into(),
                    vec![],
                    path.into(),
                    PriorityClass::Normal,
                    false,
                );
                app.name = format!("App {index}");
                app.additional_processes.push("helper.exe".into());
                CoreGroup {
                    name: format!("Group {index}"),
                    cores: vec![index],
                    programs: vec![app],
                    is_hidden: false,
                    run_all_button: true,
                }
            })
            .collect();
        state.rule_identities = None;
    });
    let rules = RulesContext::from_storage(&state);
    state.rule_identities = Some(rules.to_persisted_identities());
    state
}
fn write(
    control: &Authority,
    index: usize,
    pid: u32,
    token: u64,
    image: &str,
) -> Result<bool, String> {
    let spec = control.view().rules[index].clone();
    control
        .write(
            WriteRequest {
                rule: &spec.rule,
                key: &spec.key,
                pid,
                token,
                image: Some(Path::new(image)),
                mask: spec.mask,
                priority: spec.app.priority,
                fields: spec.fields,
                change: true,
            },
            |allowed| Ok(allowed),
        )
        .map(|(allowed, _)| allowed)
}
pub(crate) fn observe(
    control: &Authority,
    state: &AppStateStorage,
    ids: &[usize],
    token: u64,
    image: &str,
) {
    let view = control.view();
    let mut apps = RunningApps::default();
    for &index in ids {
        let spec = &view.rules[index];
        apps.add_app(&spec.key, 42, spec.group.clone(), spec.rule.clone());
        apps.apps
            .get_mut(&spec.key)
            .unwrap()
            .pid_instance_tokens
            .insert(42, token);
    }
    control.observe(&apps, Policy::read(state).unwrap().generation(), |_, _| {
        Some(image.into())
    });
}
#[test]
fn s110_transfer_owner_keeps_lists_and_survives_restart() {
    let mut state = fixture();
    let groups = serde_json::to_value(&state.groups).unwrap();
    let control = Authority::default();
    control.configure(&state);
    assert!(write(&control, 0, 42, 1, r"C:\Shared\helper.exe").is_err());
    observe(&control, &state, &[0, 1], 1, r"C:\Shared\helper.exe");
    let owner = control.view().rules[1].rule.clone();
    let choice = Choice {
        image: r"C:\Shared\helper.exe".into(),
        owner: owner.clone(),
    };
    control
        .validate_choices(&state, std::slice::from_ref(&choice))
        .unwrap();
    Policy::choose(&mut state, choice).unwrap();
    let restarted = Authority::default();
    restarted.configure(&state);
    assert!(!write(&restarted, 0, 42, 1, r"C:\Shared\helper.exe").unwrap());
    assert!(write(&restarted, 1, 42, 1, r"C:\Shared\helper.exe").unwrap());
    assert_eq!(serde_json::to_value(&state.groups).unwrap(), groups);
    assert!(restarted
        .view()
        .shared_elsewhere(&restarted.view().rules[0].rule));
}
#[test]
fn s110_existing_rule_wins_new_overlap_equal_settings_need_no_choice() {
    let mut before = fixture();
    before.groups.remove(1);
    before.rule_identities = None;
    let ids = RulesContext::from_storage(&before);
    before.rule_identities = Some(ids.to_persisted_identities());
    let mut next = before.clone();
    next.groups.push(fixture().groups[1].clone());
    let mut ids = RulesContext::from_storage(&next);
    ids.reconcile_with_storage(&next);
    next.rule_identities = Some(ids.to_persisted_identities());
    Policy::prepare_save(&before, &mut next).unwrap();
    let control = Authority::default();
    control.configure(&next);
    assert!(write(&control, 0, 42, 1, r"C:\Shared\helper.exe").unwrap());
    assert!(!write(&control, 1, 42, 1, r"C:\Shared\helper.exe").unwrap());
    let mut equal = fixture();
    equal.groups[1].cores = vec![0];
    let equal_control = Authority::default();
    equal_control.configure(&equal);
    assert!(write(&equal_control, 0, 42, 1, r"C:\Shared\helper.exe").unwrap());
    assert!(!write(&equal_control, 1, 42, 1, r"C:\Shared\helper.exe").unwrap());
    assert!(!equal_control
        .view()
        .needs_choice(&equal_control.view().rules[1].rule));
}
#[test]
fn s110_exact_images_stale_keys_and_reused_pids_do_not_transfer_control() {
    let state = fixture();
    let control = Authority::default();
    control.configure(&state);
    observe(&control, &state, &[0, 1], 1, r"C:\Shared\helper.exe");
    // The same PID with another lifetime and another image must not retain the old claimants.
    assert!(write(&control, 0, 42, 2, r"C:\Apps\app0.exe").unwrap());
    assert!(write(&control, 0, 0, 1, r"C:\Shared\helper.exe").is_err());
    assert!(write(&control, 0, 42, 0, r"C:\Shared\helper.exe").is_err());
    let mut updated = state.clone();
    updated.groups[0].cores = vec![3];
    let old = control.view().rules[0].clone();
    control.configure(&updated);
    let mut called = false;
    assert!(control
        .write(
            WriteRequest {
                rule: &old.rule,
                key: &old.key,
                pid: 42,
                token: 2,
                image: Some(Path::new(r"C:\Apps\app0.exe")),
                mask: old.mask,
                priority: old.app.priority,
                fields: ProcessSettingsFields::default(),
                change: true
            },
            |_| {
                called = true;
                Ok(())
            }
        )
        .is_err());
    assert!(!called);
    let unrelated = Choice {
        image: r"C:\Other\random.exe".into(),
        owner: RuleId("rule-0".into()),
    };
    assert!(control.validate_choices(&updated, &[unrelated]).is_err());
}
#[test]
fn s110_legacy_prelaunch_conflict_requires_choice_and_explicit_launch_uses_owner() {
    let mut state = fixture();
    let image = r"C:\Apps\app1.exe";
    state.groups[0].programs[0]
        .additional_processes
        .push("app1.exe".into());
    let control = Authority::default();
    control.configure(&state);
    let rule = control.view().rules[1].clone();
    let mut called = false;
    assert!(control
        .launch(&rule.rule, &rule.key, Path::new(image), |_| {
            called = true;
            Ok(())
        })
        .is_err());
    assert!(!called);
    let choice = Choice {
        image: image.into(),
        owner: control.view().rules[0].rule.clone(),
    };
    control
        .validate_choices(&state, std::slice::from_ref(&choice))
        .unwrap();
    Policy::choose(&mut state, choice).unwrap();
    control.configure(&state);
    control
        .launch(&rule.rule, &rule.key, Path::new(image), |owner| {
            assert_eq!(owner.mask, 1);
            Ok(())
        })
        .unwrap();
    assert!(control
        .launch(
            &rule.rule,
            &rule.key,
            Path::new(r"C:\Bad\other.exe"),
            |_| Ok(())
        )
        .is_err());
}

#[test]
fn s110_stale_observation_cannot_revive_removed_rule_and_preference_falls_back() {
    let mut state = fixture();
    for (i, group) in state.groups.iter_mut().enumerate() {
        group.programs[0].launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 1000 + i as u32,
            executable: None,
        };
    }
    let control = Authority::default();
    control.configure(&state);
    observe(&control, &state, &[0, 1], 1, r"C:\Games\game.exe");
    let choice = Choice {
        image: r"C:\Games\game.exe".into(),
        owner: control.view().rules[1].rule.clone(),
    };
    Policy::choose(&mut state, choice).unwrap();
    control.configure(&state);
    observe(&control, &state, &[0], 1, r"C:\Games\game.exe");
    assert!(write(&control, 0, 42, 1, r"C:\Games\game.exe").unwrap());
    observe(&control, &state, &[0, 1], 1, r"C:\Games\game.exe");
    assert!(!write(&control, 0, 42, 1, r"C:\Games\game.exe").unwrap());
    assert!(write(&control, 1, 42, 1, r"C:\Games\game.exe").unwrap());
    let before = state.clone();
    state.groups.remove(1);
    let mut ids = RulesContext::from_storage(&state);
    ids.reconcile_with_storage(&state);
    state.rule_identities = Some(ids.to_persisted_identities());
    Policy::prepare_save(&before, &mut state).unwrap();
    control.configure(&state);
    observe(&control, &before, &[0], 1, r"C:\Games\game.exe");
    assert!(control.view().notices.is_empty());
    assert!(Policy::read(&state).unwrap().choices.is_empty());
}
#[test]
fn s110_native_write_and_durable_transfer_are_serialized() {
    use std::{
        sync::{mpsc, Arc},
        time::Duration,
    };
    let mut state = fixture();
    state.groups[1].cores = vec![0];
    let control = Arc::new(Authority::default());
    control.configure(&state);
    let spec = control.view().rules[0].clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let held = control.clone();
    let writer = std::thread::spawn(move || {
        held.write(
            WriteRequest {
                rule: &spec.rule,
                key: &spec.key,
                pid: 42,
                token: 1,
                image: Some(Path::new(r"C:\Shared\helper.exe")),
                mask: spec.mask,
                priority: spec.app.priority,
                fields: spec.fields,
                change: true,
            },
            |_| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            },
        )
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        control.gate.try_lock().is_err(),
        "save cannot publish a new owner while the old native write is active"
    );
    release_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
    assert!(control.gate.try_lock().is_ok());
}

#[test]
fn s110_same_basename_in_distinct_installations_is_only_a_name_hint() {
    let mut state = fixture();
    for (i, group) in state.groups.iter_mut().enumerate() {
        let app = &mut group.programs[0];
        app.launch_target = crate::app::models::LaunchTarget::Path {
            dropped_path: format!(r"C:\Install{i}\client.exe").into(),
            bin_path: format!(r"C:\Install{i}\client.exe").into(),
        };
        app.additional_processes = vec!["client.exe".into()];
    }
    let control = Authority::default();
    control.configure(&state);
    assert!(write(&control, 0, 42, 1, r"C:\Install0\client.exe").unwrap());
    assert!(write(&control, 1, 43, 1, r"C:\Install1\client.exe").unwrap());
    assert!(control.view().notices.is_empty());
    assert_eq!(
        control
            .view()
            .duplicate_labels(&control.view().rules[0].rule, &state.groups[0].programs[0])
            .len(),
        1
    );
}
#[cfg(target_os = "windows")]
#[test]
#[ignore = "explicit owned-child native acceptance; no real applications or services"]
fn s110_owned_child_transfer_changes_only_the_selected_writer() {
    use os_api::OS;
    use std::{
        os::windows::process::CommandExt,
        process::{Child, Command},
    };
    struct Owned(Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let path = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap())
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let child = Owned(
        Command::new(&path)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 45",
            ])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap(),
    );
    let pid = child.0.id();
    let token = OS::get_process_instance_token(pid).unwrap();
    let available = OS::get_process_affinity(pid).unwrap();
    let cpus: Vec<_> = (0..usize::BITS as usize)
        .filter(|cpu| available & (1usize << cpu) != 0)
        .take(2)
        .collect();
    assert!(!cpus.is_empty());
    let mut state = fixture();
    for (i, group) in state.groups.iter_mut().enumerate() {
        group.cores = vec![cpus[i % cpus.len()]];
        group.programs[0] = AppToRun::new_path(
            path.clone(),
            vec![],
            path.clone(),
            if i == 0 {
                PriorityClass::Normal
            } else {
                PriorityClass::BelowNormal
            },
            false,
        );
    }
    let control = Authority::default();
    control.configure(&state);
    let image = OS::get_process_image_path(pid).unwrap();
    let apply = |index: usize| {
        let spec = control.view().rules[index].clone();
        control.write(
            WriteRequest {
                rule: &spec.rule,
                key: &spec.key,
                pid,
                token,
                image: Some(&image),
                mask: spec.mask,
                priority: spec.app.priority,
                fields: spec.fields,
                change: true,
            },
            |allowed| {
                let result = OS::apply_process_fields_if_instance(
                    pid,
                    token,
                    spec.mask,
                    spec.app.priority,
                    spec.fields,
                    allowed,
                )?;
                Ok(result)
            },
        )
    };
    assert!(apply(0).is_err());
    assert!(apply(1).is_err());
    assert_eq!(OS::get_process_affinity(pid).unwrap(), available);
    for index in [0, 1, 0] {
        let choice = Choice {
            image: image.to_string_lossy().into_owned(),
            owner: control.view().rules[index].rule.clone(),
        };
        Policy::choose(&mut state, choice).unwrap();
        control.configure(&state);
        let caller = control.view().rules[1 - index].clone();
        control
            .initial(
                WriteRequest {
                    rule: &caller.rule,
                    key: &caller.key,
                    pid,
                    token,
                    image: Some(&image),
                    mask: caller.mask,
                    priority: caller.app.priority,
                    fields: caller.fields,
                    change: true,
                },
                |owner| {
                    OS::apply_process_fields_if_instance(
                        pid,
                        token,
                        owner.mask,
                        owner.app.priority,
                        owner.fields,
                        true,
                    )
                    .map(|_| ())
                },
            )
            .unwrap();
        assert!(apply(index).unwrap().1);
        assert!(!apply(1 - index).unwrap().1);
        assert_eq!(
            OS::get_process_affinity(pid).unwrap(),
            control.view().rules[index].mask
        );
        assert_eq!(
            OS::get_process_priority(pid).unwrap(),
            control.view().rules[index].app.priority
        );
    }
}

#[test]
fn s110_a_lone_instance_does_not_hide_another_shared_instance_of_same_image() {
    let mut state = fixture();
    for (index, group) in state.groups.iter_mut().enumerate() {
        group.programs[0].launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 100 + index as u32,
            executable: None,
        };
    }
    let control = Authority::default();
    control.configure(&state);
    let rules = control.view();
    let mut apps = RunningApps::default();
    for rule in rules.rules.iter() {
        apps.add_app(&rule.key, 42, rule.group.clone(), rule.rule.clone());
        apps.apps
            .get_mut(&rule.key)
            .unwrap()
            .pid_instance_tokens
            .insert(42, 1);
    }
    apps.apps
        .get_mut(&rules.rules[0].key)
        .unwrap()
        .pid_instance_tokens
        .insert(43, 2);
    control.observe(&apps, 0, |_, _| Some(r"C:\Games\common.exe".into()));
    assert!(control.view().needs_choice(&rules.rules[1].rule));
}

#[test]
fn s110_unrelated_save_cannot_temporarily_drop_shared_game_claims() {
    let state = fixture();
    let mut games = state.clone();
    for (index, group) in games.groups.iter_mut().enumerate() {
        group.programs[0].launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 100 + index as u32,
            executable: None,
        };
    }
    let control = Authority::default();
    control.configure(&games);
    observe(&control, &games, &[0, 1], 1, r"C:\Games\game.exe");
    assert!(write(&control, 0, 42, 1, r"C:\Games\game.exe").is_err());
    let mut changed = games.clone();
    changed.theme_index = 2;
    Policy::prepare_save(&games, &mut changed).unwrap();
    control.configure(&changed);
    assert!(
        write(&control, 0, 42, 1, r"C:\Games\game.exe").is_err(),
        "a preference save must not open a write window until the next discovery"
    );
}

#[test]
fn s112_disjoint_lifetimes_do_not_create_shared_image_conflicts() {
    let mut state = fixture();
    for group in &mut state.groups {
        group.programs[0]
            .additional_processes
            .retain(|name| name != "helper.exe");
        group.programs[0].manage_descendants = true;
    }
    let control = Authority::default();
    control.configure(&state);
    let view = control.view();
    let mut apps = RunningApps::default();
    for (index, spec) in view.rules.iter().enumerate() {
        let pid = 42 + index as u32;
        apps.add_app(&spec.key, pid, spec.group.clone(), spec.rule.clone());
        apps.apps
            .get_mut(&spec.key)
            .unwrap()
            .pid_instance_tokens
            .insert(pid, 100 + pid as u64);
    }
    control.observe(&apps, 0, |_, _| Some(r"C:\Shared\helper.exe".into()));
    assert!(
        control.view().notices.is_empty(),
        "the same image is not proof of a shared lifetime"
    );
}

#[test]
fn s112_disjoint_shared_sets_keep_independent_control_preferences() {
    let original = fixture();
    let mut state = original.clone();
    state.groups = vec![
        original.groups[0].clone(),
        original.groups[0].clone(),
        original.groups[1].clone(),
        original.groups[1].clone(),
    ];
    for (index, group) in state.groups.iter_mut().enumerate() {
        group.programs[0].launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 60000 + index as u32,
            executable: None,
        };
    }
    state.rule_identities = None;
    let ids = RulesContext::from_storage(&state);
    state.rule_identities = Some(ids.to_persisted_identities());
    let control = Authority::default();
    control.configure(&state);
    let publish = |state: &AppStateStorage| {
        let mut apps = RunningApps::default();
        for (index, spec) in control.view().rules.iter().enumerate() {
            let pid = 42 + (index / 2) as u32;
            apps.add_app(&spec.key, pid, spec.group.clone(), spec.rule.clone());
            apps.apps
                .get_mut(&spec.key)
                .unwrap()
                .pid_instance_tokens
                .insert(pid, 100 + pid as u64);
        }
        control.observe(&apps, Policy::read(state).unwrap().generation(), |_, _| {
            Some(r"C:\Shared\helper.exe".into())
        });
    };
    publish(&state);
    assert_eq!(control.view().notices.len(), 2);
    assert!(control
        .view()
        .notices
        .iter()
        .all(|notice| !notice.different && notice.owner.is_some()));
    state.groups[1].cores = vec![2];
    state.groups[3].cores = vec![3];
    control.configure(&state);
    publish(&state);
    let rules = control.view();
    for index in [0, 2] {
        Policy::choose(
            &mut state,
            Choice {
                image: r"C:\Shared\helper.exe".into(),
                owner: rules.rules[index].rule.clone(),
            },
        )
        .unwrap();
    }
    control.configure(&state);
    publish(&state);
    let view = control.view();
    assert!(
        view.notices.iter().all(|notice| notice.owner.is_some()),
        "choosing the second independent pair must preserve the first choice"
    );
    assert_eq!(view.notices[0].owner, Some(view.rules[0].rule.clone()));
    assert_eq!(view.notices[1].owner, Some(view.rules[2].rule.clone()));
}

#[test]
fn s116_initialized_gui_path_does_not_wait_for_an_observation() {
    let state = fixture();
    let control = std::sync::Arc::new(Authority::default());
    control.configure(&state);
    let lease = control
        .observation(Policy::read(&state).unwrap().generation(), None)
        .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let cloned = control.clone();
    let worker = std::thread::spawn(move || {
        cloned.initialize(&state);
        tx.send(()).unwrap();
    });
    let immediate = rx.recv_timeout(std::time::Duration::from_secs(1)).is_ok();
    drop(lease);
    worker.join().unwrap();
    assert!(
        immediate,
        "an already initialized GUI action waited behind inventory"
    );
}

#[test]
fn s116_unknown_control_policy_does_not_block_read_only_observation() {
    let mut state = fixture();
    state.set_process_control_raw(r#"{"version":99}"#.into());
    let control = Authority::default();
    control.configure(&state);
    assert!(control.view().error.is_some());
    assert!(
        control.observation(u64::MAX, None).is_ok(),
        "write refusal disabled CPU/RAM discovery"
    );
}
