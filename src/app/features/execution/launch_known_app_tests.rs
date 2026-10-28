fn s107_discord_rule() -> AppToRun {
    let mut app = AppToRun::new_path(
        r"C:/Fixture/Discord/Update.exe".into(),
        vec!["--processStart".into(), "Discord.exe".into()],
        r"C:/Fixture/Discord/Update.exe".into(),
        PriorityClass::Normal,
        false,
    );
    app.additional_processes = vec!["Discord.exe".into()];
    app.builtin_process_binding = Some(
        serde_json::json!({"catalog_id":"discord-stable","definition_version":1,"generated_names":["Discord.exe"]}),
    );
    app
}

#[test]
fn s108_catalog_launchers_reuse_ordinary_handoff_without_scheduling_the_launcher() {
    for (index, (target, main, id)) in [
        ("EALauncher.exe", "EADesktop.exe", "ea-app"),
        ("LauncherPatcher.exe", "Launcher.exe", "rockstar-launcher"),
        ("Battle.net Launcher.exe", "Battle.net.exe", "battle-net"),
        ("Uplay.exe", "upc.exe", "ubisoft-connect"),
    ]
    .into_iter()
    .enumerate()
    {
        let path = PathBuf::from("C:/OwnedInstall").join(target);
        let mut app = AppToRun::new_path(path.clone(), vec![], path, PriorityClass::Normal, false);
        app.additional_processes = vec![main.into()];
        app.builtin_process_binding = Some(
            serde_json::json!({"catalog_id":id,"definition_version":1,"generated_names":[main]}),
        );
        let runtime = RuntimeRegistry::new();
        let os = FakeLaunchOs::default();
        let mut log = LogManager::default();
        assert_eq!(
            run_launch_decision(
                &runtime,
                &mut log,
                group_id(108),
                rule_id(10800 + index),
                app.clone(),
                vec![0],
                &os
            ),
            super::LaunchDispatchOutcome::Accepted
        );
        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(os.ordinary_calls.borrow().len(), 1);
        assert!(matches!(
            runtime.lookup_running_app_instances(&app.get_key()),
            super::RunningAppInstancesLookup::NotFound
        ));
    }
}

#[test]
fn s108_focus_cannot_create_a_duplicate_of_a_multi_instance_catalog_app() {
    let path = PathBuf::from("C:/OwnedInstall/Common7/IDE/devenv.exe");
    let mut app = AppToRun::new_path(
        path.clone(),
        vec![],
        path.clone(),
        PriorityClass::Normal,
        false,
    );
    app.builtin_process_binding = Some(
        serde_json::json!({"catalog_id":"visual-studio","definition_version":1,"generated_names":["devenv.exe"]}),
    );
    let runtime = RuntimeRegistry::new();
    let mut log = LogManager::default();
    assert!(runtime.add_running_app_with_token(
        &app.get_key(),
        10,
        1010,
        group_id(108),
        rule_id(10809)
    ));
    let os = FakeLaunchOs {
        image_paths: HashMap::from([(10, path)]),
        ..Default::default()
    };
    assert!(matches!(
        super::focus_existing_app(&runtime, &mut log, app, &os),
        super::LaunchDispatchOutcome::Rejected(_)
    ));
    assert!(os.client_activation_calls.borrow().is_empty());
    assert!(os.run_calls.borrow().is_empty());
}
#[test]
fn s107_discord_launch_does_not_schedule_or_record_the_updater() {
    let runtime = RuntimeRegistry::new();
    let mut log = LogManager::default();
    let os = FakeLaunchOs::default();
    let app = s107_discord_rule();
    let outcome = run_launch_decision(
        &runtime,
        &mut log,
        group_id(107),
        rule_id(107),
        app.clone(),
        vec![0],
        &os,
    );
    assert_eq!(outcome, super::LaunchDispatchOutcome::Accepted);
    assert!(
        os.run_calls.borrow().is_empty(),
        "updater must not take the ordinary affinity launch path"
    );
    assert!(os.launch_field_calls.borrow().is_empty());
    assert!(matches!(
        runtime.lookup_running_app_instances(&app.get_key()),
        super::RunningAppInstancesLookup::NotFound
    ));
}
#[test]
fn s107_unknown_binding_cannot_fall_back_to_scheduling_the_updater() {
    let runtime = RuntimeRegistry::new();
    let mut log = LogManager::default();
    let os = FakeLaunchOs::default();
    let mut app = s107_discord_rule();
    app.builtin_process_binding.as_mut().unwrap()["definition_version"] = 99.into();
    assert!(matches!(
        run_launch_decision(
            &runtime,
            &mut log,
            group_id(107),
            rule_id(108),
            app,
            vec![0],
            &os
        ),
        super::LaunchDispatchOutcome::Rejected(_)
    ));
    assert!(os.run_calls.borrow().is_empty());
}

#[test]
fn s107_focus_uses_discord_window_and_never_its_updater_or_foreign_image() {
    let runtime = RuntimeRegistry::new();
    let app = s107_discord_rule();
    let key = app.get_key();
    let mut log = LogManager::default();
    assert!(runtime.add_running_app_with_token(&key, 10, 1010, group_id(107), rule_id(109)));
    let os = FakeLaunchOs {
        image_paths: HashMap::from([(10, r"C:/Fixture/Discord/app-1.0.100/Discord.exe".into())]),
        focus_results: HashMap::from([(10, true)]),
        ..Default::default()
    };
    assert_eq!(
        super::focus_existing_app(&runtime, &mut log, app.clone(), &os),
        super::LaunchDispatchOutcome::Accepted
    );
    assert_eq!(*os.focus_calls.borrow(), [10]);
    let wrong = FakeLaunchOs {
        image_paths: HashMap::from([(10, r"C:/Fixture/Discord/Update.exe".into())]),
        focus_results: HashMap::from([(10, true)]),
        ..Default::default()
    };
    assert!(matches!(
        super::focus_existing_app(&runtime, &mut log, app, &wrong),
        super::LaunchDispatchOutcome::Rejected(_)
    ));
    assert!(wrong.focus_calls.borrow().is_empty());
}
#[test]
fn s107_repeated_discord_launches_coalesce_without_registering_launcher() {
    let runtime = RuntimeRegistry::new();
    let mut log = LogManager::default();
    let os = FakeLaunchOs::default();
    let app = s107_discord_rule();
    for _ in 0..3 {
        assert_eq!(
            run_launch_decision(
                &runtime,
                &mut log,
                group_id(107),
                rule_id(110),
                app.clone(),
                vec![0],
                &os
            ),
            super::LaunchDispatchOutcome::Accepted
        );
    }
    assert_eq!(os.ordinary_calls.borrow().len(), 1);
}

#[test]
fn s107_focus_hidden_owned_client_requests_normal_activation_without_scheduling() {
    let runtime = RuntimeRegistry::new();
    let app = s107_discord_rule();
    let key = app.get_key();
    let mut log = LogManager::default();
    assert!(runtime.add_running_app_with_token(&key, 10, 1010, group_id(107), rule_id(111)));
    let os = FakeLaunchOs {
        image_paths: HashMap::from([(10, r"C:/Fixture/Discord/app-1.0.100/Discord.exe".into())]),
        ..Default::default()
    };
    assert_eq!(
        super::focus_existing_app(&runtime, &mut log, app, &os),
        super::LaunchDispatchOutcome::Accepted
    );
    assert_eq!(os.client_activation_calls.borrow().len(), 1);
    assert!(os.affinity_calls.borrow().is_empty());
    assert!(os.run_calls.borrow().is_empty());
}
