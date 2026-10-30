fn s103_candidate(pid: u32, path: &str) -> os_api::steam::GameProcess {
    os_api::steam::GameProcess {
        pid,
        creation_token: pid as u64,
        path: path.into(),
        has_window: true,
    }
}
fn s103_os(id: u32, candidates: Vec<os_api::steam::GameProcess>) -> FakeRunningAppsOs {
    let images: HashMap<_, _> = candidates.iter().map(|p| (p.pid, p.path.clone())).collect();
    FakeRunningAppsOs {
        snapshot: Ok(ProcessSnapshot {
            names: images
                .iter()
                .map(|(&p, path)| (p, path.file_name().unwrap().to_string_lossy().into()))
                .collect(),
            ..Default::default()
        }),
        steam: HashMap::from([(id, candidates)]),
        steam_root: Some(r"C:\Game".into()),
        instance_tokens: images.keys().map(|&p| (p, p as u64)).collect(),
        image_paths: images,
        ..Default::default()
    }
}

#[test]
fn steam_missing_installation_requests_selection_and_exact_choice_recovers() {
    let mut state = steam_state(570);
    state.groups[0].programs[0].additional_processes = vec!["game.exe".into()];
    let rule = collect_configured_programs(&state)[0].rule_id.clone();
    let mut apps = RunningApps::default();
    let mut os = FakeRunningAppsOs::default();
    os.steam_directory_error = Some("Game is installed in multiple Steam libraries; choose its process".into());
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(apps.steam_unresolved.contains_key(&rule), "Missing metadata must offer process selection, not silently look stopped/starting");
    assert!(apps.apps.is_empty());
    assert!(apps.steam_unresolved.get(&rule).unwrap().reason().contains("multiple Steam libraries"));

    let game = s103_candidate(2, r"C:\Game\game.exe");
    state.groups[0].programs[0].launch_target = crate::app::models::LaunchTarget::Steam {
        app_id: 570,
        executable: Some(game.path.clone()),
    };
    os = s103_os(570, vec![game]);
    os.steam_root = None;
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(!apps.steam_unresolved.contains_key(&rule));
    assert_eq!(apps.apps.values().next().unwrap().primary_instances, HashMap::from([(2, 2)]));
    assert_eq!(state.groups[0].programs[0].additional_processes, ["game.exe"]);
}

#[test]
fn steam_warning_explains_helper_only_and_ambiguous_candidates_without_admitting_them() {
    use crate::app::features::execution::steam_detection::SteamDetectionIssue;
    let mut state = steam_state(570);
    let rule = collect_configured_programs(&state)[0].rule_id.clone();
    state.groups[0].programs[0].additional_processes = vec!["game.exe".into()];
    let mut os = s103_os(570, vec![s103_candidate(2, r"C:\Game\game.exe")]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.steam_unresolved.get(&rule), Some(&SteamDetectionIssue::HelperOnly));
    assert!(apps.apps.is_empty());
    state.groups[0].programs[0].additional_processes.clear();
    os.steam.get_mut(&570).unwrap().push(s103_candidate(3, r"C:\Game\second.exe"));
    os.instance_tokens.insert(3, 3);
    os.image_paths.insert(3, r"C:\Game\second.exe".into());
    os.snapshot.as_mut().unwrap().names.insert(3, "second.exe".into());
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.steam_unresolved.get(&rule), Some(&SteamDetectionIssue::AmbiguousProcesses));
    assert!(apps.apps.is_empty());
}

#[test]
fn s103_windowed_helper_cannot_reseed_main_after_game_exit() {
    let mut state = steam_state(570);
    state.groups[0].programs[0].additional_processes = vec!["helper.exe".into()];
    let main = s103_candidate(2, r"C:\Game\game.exe");
    let helper = s103_candidate(3, r"C:\Game\helper.exe");
    let mut os = s103_os(570, vec![main]);
    os.image_paths.insert(3, helper.path.clone());
    os.snapshot
        .as_mut()
        .unwrap()
        .names
        .insert(3, "helper.exe".into());
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    os.image_paths.remove(&2);
    os.snapshot.as_mut().unwrap().names.remove(&2);
    os.steam.insert(570, vec![helper]);
    for _ in 0..2 {
        run_iteration(&mut apps, collect_configured_programs(&state), &os);
        assert!(
            apps.apps.is_empty(),
            "A helper window must not become main on this or the next tick"
        );
    }
}

#[test]
fn s103_bootstrap_window_is_not_a_game_and_renderer_handoff_is_detected() {
    let state = steam_state(570);
    let launcher = s103_candidate(1, r"C:\Game\GameLauncher.exe");
    let game = s103_candidate(2, r"C:\Game\Game-Win64-Shipping.exe");
    let mut os = s103_os(570, vec![launcher.clone()]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(
        apps.apps.is_empty(),
        "A recognizable launcher is not the game"
    );
    os.image_paths.insert(2, game.path.clone());
    os.snapshot
        .as_mut()
        .unwrap()
        .names
        .insert(2, "Game-Win64-Shipping.exe".into());
    os.steam.insert(570, vec![launcher, game]);
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(
        apps.apps.values().next().unwrap().primary_instances,
        HashMap::from([(2, 2)])
    );
}

#[test]
fn s103_new_ambiguity_withdraws_confident_automatic_roster() {
    let state = steam_state(570);
    let mut os = s103_os(570, vec![s103_candidate(2, r"C:\Game\first.exe")]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(!apps.apps.is_empty());
    let second = s103_candidate(3, r"C:\Game\second.exe");
    os.image_paths.insert(3, second.path.clone());
    os.snapshot
        .as_mut()
        .unwrap()
        .names
        .insert(3, "second.exe".into());
    os.steam.get_mut(&570).unwrap().push(second);
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(
        apps.apps.is_empty(),
        "Ambiguous fresh evidence must not silently retain an old guessed main"
    );
}

#[test]
fn s103_explicit_other_app_blocks_automatic_claim() {
    let mut state = steam_state(570);
    let mut other = steam_state(571).groups[0].programs[0].clone();
    other.launch_target = crate::app::models::LaunchTarget::Steam {
        app_id: 571,
        executable: Some(r"C:\Game\game.exe".into()),
    };
    state.groups[0].programs.push(other);
    let game = s103_candidate(2, r"C:\Game\game.exe");
    let mut os = s103_os(570, vec![game.clone()]);
    os.steam.insert(571, vec![game]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(!apps
        .apps
        .contains_key(&state.groups[0].programs[0].get_key()));
    assert!(apps
        .apps
        .contains_key(&state.groups[0].programs[1].get_key()));
}

#[test]
fn s103_ambiguous_raw_evidence_still_blocks_overlapping_app() {
    let mut state = steam_state(570);
    state.groups[0]
        .programs
        .push(steam_state(571).groups[0].programs[0].clone());
    let game = s103_candidate(2, r"C:\Game\game.exe");
    let extra = s103_candidate(3, r"C:\Game\other.exe");
    let mut os = s103_os(570, vec![game.clone(), extra.clone()]);
    os.steam.insert(570, vec![game.clone()]);
    os.steam.insert(571, vec![game, extra]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(apps.apps.is_empty());
}

#[test]
fn s103_candidate_newer_than_snapshot_does_not_publish_empty_game() {
    let state = steam_state(570);
    let mut os = s103_os(570, vec![s103_candidate(2, r"C:\Game\game.exe")]);
    os.snapshot.as_mut().unwrap().names.clear();
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(
        apps.apps.is_empty(),
        "No empty RunningApp may be published as startup completion"
    );
}

#[test]
fn s103_hidden_previous_main_and_fresh_other_image_are_unresolved() {
    let state = steam_state(570);
    let first = s103_candidate(2, r"C:\Game\first.exe");
    let second = s103_candidate(3, r"C:\Game\second.exe");
    let mut os = s103_os(570, vec![first]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    os.image_paths.insert(3, second.path.clone());
    os.snapshot
        .as_mut()
        .unwrap()
        .names
        .insert(3, "second.exe".into());
    os.steam.insert(570, vec![second]);
    for _ in 0..3 {
        run_iteration(&mut apps, collect_configured_programs(&state), &os);
        assert!(
            apps.apps.is_empty(),
            "Ambiguity must survive repeated identical observations"
        );
    }
}

#[test]
fn s103_hidden_previous_main_cannot_escape_explicit_other_app_claim() {
    let mut state = steam_state(570);
    let game = s103_candidate(2, r"C:\Game\game.exe");
    let mut os = s103_os(570, vec![game.clone()]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    let mut other = steam_state(571).groups[0].programs[0].clone();
    other.launch_target = crate::app::models::LaunchTarget::Steam {
        app_id: 571,
        executable: Some(game.path.clone()),
    };
    state.groups[0].programs.push(other);
    os.steam.insert(570, vec![]);
    os.steam.insert(571, vec![game]);
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert!(!apps
        .apps
        .contains_key(&state.groups[0].programs[0].get_key()));
}

#[test]
fn s103_transient_token_denial_does_not_erase_descendant_helper_role() {
    let mut state = steam_state(570);
    state.groups[0].programs[0].manage_descendants = true;
    let main = s103_candidate(2, r"C:\Game\game.exe");
    let helper = s103_candidate(3, r"C:\Game\companion.exe");
    let mut os = s103_os(570, vec![main]);
    os.image_paths.insert(3, helper.path.clone());
    let snapshot = os.snapshot.as_mut().unwrap();
    snapshot.names.insert(3, "companion.exe".into());
    snapshot.children_of.insert(2, vec![3]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.apps.values().next().unwrap().pids.len(), 2);
    os.denied_tokens.insert(3);
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    os.denied_tokens.clear();
    os.snapshot.as_mut().unwrap().names.remove(&2);
    os.image_paths.remove(&2);
    os.steam.insert(570, vec![helper]);
    for _ in 0..3 {
        run_iteration(&mut apps, collect_configured_programs(&state), &os);
        assert!(apps.apps.is_empty());
    }
}
#[test]
fn s103_named_helper_must_pass_interactive_logon_boundary() {
    let mut state = steam_state(570);
    state.groups[0].programs[0].additional_processes = vec!["companion.exe".into()];
    let mut os = s103_os(570, vec![s103_candidate(2, r"C:\Game\game.exe")]);
    os.image_paths.insert(3, r"C:\Game\companion.exe".into());
    os.snapshot
        .as_mut()
        .unwrap()
        .names
        .insert(3, "companion.exe".into());
    os.denied_steam.insert(3);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.apps.values().next().unwrap().pids, vec![2]);
}

#[test]
fn s103_unreadable_hidden_main_still_blocks_a_different_image() {
    let state = steam_state(570);
    let mut os = s103_os(570, vec![s103_candidate(2, r"C:\Game\first.exe")]);
    let mut apps = RunningApps::default();
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    let second = s103_candidate(3, r"C:\Game\second.exe");
    os.image_paths.insert(3, second.path.clone());
    os.snapshot
        .as_mut()
        .unwrap()
        .names
        .insert(3, "second.exe".into());
    os.steam.insert(570, vec![second]);
    os.denied_tokens.insert(2);
    for _ in 0..3 {
        run_iteration(&mut apps, collect_configured_programs(&state), &os);
        assert!(
            apps.apps.is_empty(),
            "Unknown lifetime is not proof of exit"
        );
    }
    os.snapshot.as_mut().unwrap().names.remove(&2);
    run_iteration(&mut apps, collect_configured_programs(&state), &os);
    assert_eq!(apps.apps.values().next().unwrap().pids, vec![3]);
}
