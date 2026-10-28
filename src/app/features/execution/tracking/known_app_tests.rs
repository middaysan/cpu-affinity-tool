fn s107_bound_steam() -> AppStateStorage {
    let mut state = sample_path_program_state();
    let app = AppToRun::new_path(
        r"C:/Steam/steam.exe".into(),
        vec![],
        r"C:/Steam/steam.exe".into(),
        PriorityClass::Normal,
        false,
    );
    let mut value = serde_json::to_value(app).unwrap();
    value["additional_processes"] = serde_json::json!(["steam.exe", "steamwebhelper.exe"]);
    value["builtin_process_binding"] = serde_json::json!({
        "catalog_id":"steam", "definition_version":1,
        "generated_names":["steamwebhelper.exe"]
    });
    state.groups[0].programs[0] = serde_json::from_value(value).unwrap();
    state
}

#[test]
fn s107_binding_roundtrips_without_expanding_saved_names() {
    let mut state = s107_bound_steam();
    let app = &mut state.groups[0].programs[0];
    app.additional_processes.retain(|name| name == "steam.exe");
    let value = serde_json::to_value(&app).unwrap();
    assert_eq!(value["builtin_process_binding"]["catalog_id"], "steam");
    let reloaded: AppToRun = serde_json::from_value(value).unwrap();
    assert_eq!(reloaded.additional_processes, ["steam.exe"]);
}

#[test]
fn s116_catalog_alternate_mains_witness_helper_only_membership() {
    for (catalog_id, imported, running, helper, main_names) in [
        (
            "obs-studio",
            "bin/32bit/obs32.exe",
            "bin/32bit/obs32.exe",
            "obs-plugins/32bit/obs-browser-page.exe",
            ["obs64.exe", "obs32.exe"],
        ),
        (
            "obs-studio",
            "bin/64bit/obs64.exe",
            "bin/32bit/obs32.exe",
            "obs-plugins/32bit/obs-browser-page.exe",
            ["obs64.exe", "obs32.exe"],
        ),
        (
            "ubisoft-connect",
            "upc.exe",
            "UbisoftConnect.exe",
            "UbisoftWebCore.exe",
            ["upc.exe", "UbisoftConnect.exe"],
        ),
        (
            "logitech-g-hub",
            "lghub.exe",
            "system_tray/lghub_system_tray.exe",
            "lghub_agent.exe",
            ["lghub.exe", "lghub_system_tray.exe"],
        ),
    ] {
        let mut state = sample_path_program_state();
        let root = std::path::Path::new("C:/OwnedInstall");
        let mut app = AppToRun::new_path(
            root.join(imported),
            vec![],
            root.join(imported),
            PriorityClass::Normal,
            false,
        );
        let helper_name = helper.rsplit('/').next().unwrap();
        app.additional_processes = vec![helper_name.into()];
        app.builtin_process_binding = Some(serde_json::json!({
            "catalog_id": catalog_id,
            "definition_version": 1,
            "generated_names": [main_names[0], main_names[1], helper_name]
        }));
        let key = app.get_key();
        state.groups[0].programs[0] = app;
        let mut os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                names: HashMap::from([
                    (1, running.rsplit('/').next().unwrap().into()),
                    (2, helper_name.into()),
                ]),
                ..Default::default()
            }),
            image_paths: HashMap::from([(1, root.join(running)), (2, root.join(helper))]),
            ..Default::default()
        };
        let mut apps = RunningApps::default();
        run_iteration(&mut apps, collect_configured_programs(&state), &os);
        let tracked = apps
            .apps
            .get(&key)
            .expect("a live qualified main admits its helper");
        assert_eq!(tracked.pid_instance_tokens, HashMap::from([(2, 2)]));
        assert_eq!(tracked.primary_instances, HashMap::from([(1, 1)]));
        assert_eq!(state.groups[0].programs[0].additional_processes, [helper_name]);

        os.snapshot.as_mut().unwrap().names.remove(&1);
        run_iteration(&mut apps, collect_configured_programs(&state), &os);
        assert!(!apps.apps.contains_key(&key), "an orphan helper must retire");
    }
}

#[test]
fn s110_identical_app_rules_in_distinct_groups_keep_independent_membership() {
    let mut state = s107_bound_steam();
    let mut duplicate = state.groups[0].clone();
    duplicate.name = "Other group".into();
    duplicate.cores = vec![1];
    state.groups.push(duplicate);
    state.rule_identities = None;
    let mut apps = RunningApps::default();
    run_iteration(&mut apps,collect_configured_programs(&state),&s107_fake_steam());
    assert_eq!(apps.apps.len(),2,"different logical rules must not collapse through one launch key");
    let mut ids:Vec<_> = apps.apps.values().map(|app|app.rule_id.clone()).collect();
    ids.sort_by(|a,b|a.0.cmp(&b.0));ids.dedup();assert_eq!(ids.len(),2);
}

#[test]
fn s107_generated_helpers_belong_only_to_the_selected_installation() {
    let state = s107_bound_steam();
    let key = state.groups[0].programs[0].get_key();
    let mut apps = RunningApps::default();
    let os = s107_fake_steam();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(
        apps.apps[&key].pid_instance_tokens,
        HashMap::from([(1, 1), (2, 2)])
    );
}

fn s107_fake_steam() -> FakeRunningAppsOs {
    FakeRunningAppsOs {
        snapshot: Ok(ProcessSnapshot {
            names: HashMap::from([
                (1, "steam.exe".into()),
                (2, "steamwebhelper.exe".into()),
                (3, "steamwebhelper.exe".into()),
                (4, "steamwebhelper.exe".into()),
                (5, "steam.exe".into()),
                (6, "steamwebhelper.exe".into()),
            ]),
            ..Default::default()
        }),
        image_paths: HashMap::from([
            (1, r"C:/Steam/steam.exe".into()),
            (2, r"C:/Steam/bin/cef/cef.win64/steamwebhelper.exe".into()),
            (3, r"D:/Steam/bin/cef/cef.win64/steamwebhelper.exe".into()),
            (4, r"C:/Steam/steamapps/Game/steamwebhelper.exe".into()),
            (5, r"D:/Steam/steam.exe".into()),
            (6, r"C:/Steam/bin/cef/cef.win64/steamwebhelper.exe".into()),
        ]),
        denied_steam: HashSet::from([6]),
        ..Default::default()
    }
}

pub(super) fn s107_fixture_membership() -> (AppStateStorage, RunningApps) {
    let mut state = s107_bound_steam();
    let mut duplicate = state.groups[0].programs[0].clone();
    duplicate.args = vec!["-silent".into()];
    state.groups[0].programs.push(duplicate);
    let mut apps = RunningApps::default();
    run_iteration(
        &mut apps,
        collect_configured_programs(&state),
        &s107_fake_steam(),
    );
    (state, apps)
}
#[test]
fn s107_removed_helpers_retire_without_refill_and_manual_names_keep_existing_meaning() {
    let mut state = s107_bound_steam();
    let key = state.groups[0].programs[0].get_key();
    let mut apps = RunningApps::default();
    let os = s107_fake_steam();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    state.groups[0].programs[0].additional_processes = vec!["steam.exe".into()];
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.apps[&key].pids, [1]);
    // Re-entering the same generated name keeps its installation scope.
    state.groups[0].programs[0]
        .additional_processes
        .push("STEAMWEBHELPER".into());
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.apps[&key].pids, [1, 2]);
    state.groups[0].programs[0].builtin_process_binding = None;
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(
        apps.apps[&key].pids.len(),
        5,
        "unbound manually authored names retain global matching"
    );
}
#[test]
fn s107_orphan_helpers_do_not_keep_a_client_running_and_reused_pids_are_rechecked() {
    let state = s107_bound_steam();
    let key = state.groups[0].programs[0].get_key();
    let mut apps = RunningApps::default();
    let mut os = s107_fake_steam();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    os.snapshot.as_mut().unwrap().names.remove(&1);
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(!apps.apps.contains_key(&key));
    os.snapshot
        .as_mut()
        .unwrap()
        .names
        .insert(1, "steam.exe".into());
    os.image_instance_tokens.insert(2, 999);
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.apps[&key].pids, [1]);
}
#[test]
fn s107_discord_tracks_real_versioned_images_not_update_or_another_channel() {
    let mut state = s107_bound_steam();
    let app = &mut state.groups[0].programs[0];
    app.launch_target = crate::app::models::LaunchTarget::Path {
        dropped_path: r"C:/Fixture/Discord/Discord.lnk".into(),
        bin_path: r"C:/Fixture/Discord/Update.exe".into(),
    };
    app.args = vec!["--processStart".into(), "Discord.exe".into()];
    app.additional_processes = vec!["Discord.exe".into()];
    app.builtin_process_binding = Some(
        serde_json::json!({"catalog_id":"discord-stable","definition_version":1,"generated_names":["Discord.exe"]}),
    );
    let key = app.get_key();
    let os = FakeRunningAppsOs {
        snapshot: Ok(ProcessSnapshot {
            names: HashMap::from([
                (1, "Update.exe".into()),
                (2, "Discord.exe".into()),
                (3, "Discord.exe".into()),
                (4, "Discord.exe".into()),
            ]),
            ..Default::default()
        }),
        image_paths: HashMap::from([
            (1, r"C:/Fixture/Discord/Update.exe".into()),
            (2, r"C:/Fixture/Discord/app-1.0.100/Discord.exe".into()),
            (3, r"C:/Fixture/Discord/app-1.0.101/Discord.exe".into()),
            (
                4,
                r"C:/Fixture/DiscordCanary/app-1.0.101/Discord.exe".into(),
            ),
        ]),
        ..Default::default()
    };
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.apps[&key].pids, [2, 3]);
    assert_eq!(
        apps.apps[&key].primary_instances,
        HashMap::from([(2, 2), (3, 3)])
    );
}

#[test]
fn s110_duplicate_namespace_change_preserves_exact_rule_lifetime_evidence() {
    let state=s107_bound_steam();let mut apps=RunningApps::default();run_iteration(&mut apps,collect_configured_programs(&state),&s107_fake_steam());
    let old=state.groups[0].programs[0].get_key();let primary=apps.apps[&old].primary_instances.clone();
    let mut next=state.clone();next.groups.push(state.groups[0].clone());next.rule_identities=None;let configured=collect_configured_programs(&next);let key=configured[0].key.clone();
    assert!(super::rekey_rule_membership(&mut apps,&configured,&Arc::new(RwLock::new(Default::default()))));
    assert!(!apps.apps.contains_key(&old));assert_eq!(apps.apps[&key].primary_instances,primary);
}
