use super::*;

#[test]
fn input_ack_does_not_capture_or_clone_graph_history() {
    use crate::app::models::AppStateStorage;
    use std::sync::RwLock;
    let mut app = AppState::new_for_test(
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
        4,
    );
    let ctx = egui::Context::default();
    let (_, rx) = mpsc::sync_channel(1);
    let (reply, result) = mpsc::sync_channel(1);
    let mut bridge = Bridge {
        requests: rx,
        _transport: None,
        pending: Some(Pending {
            request: Request {
                id: 1,
                command: Command::Live {},
                reply,
                deadline: Instant::now() + std::time::Duration::from_secs(5),
            },
            after: 0,
            wait: None,
        }),
        screenshot: None,
    };
    let mut output = ctx.run_ui(Default::default(), |ui| {
        bridge.begin_ui(ui.ctx());
        assert!(
            !wants_capture(ui.ctx()),
            "input must not materialize history"
        );
        bridge.end_ui(&mut app, ui.ctx());
    });
    output.textures_delta.clear();
    let Reply::Json(body) = result.try_recv().unwrap() else {
        panic!("expected acknowledgement")
    };
    assert_eq!(body["data"]["applied"], true);
    assert!(body["data"].get("rendered_graph").is_none());
}

#[test]
fn explicit_series_capture_is_opt_in() {
    assert!(
        serde_json::from_value::<Command>(json!({"action":"state","include_series":true})).is_ok()
    );
}

#[test]
fn read_capture_is_request_scoped_and_bounded() {
    let light = Capture::for_command(&Command::State {
        include_series: false,
    });
    assert!(
        light.graph && !light.series && light.contributors.is_none() && light.processes.is_none()
    );
    assert!(
        Capture::for_command(&Command::State {
            include_series: true
        })
        .series
    );
    let page = Capture::for_command(&Command::Table {
        kind: "contributors".into(),
        offset: 99,
        limit: 2,
    });
    assert_eq!(page.contributors, Some((99, 2)));
    assert!(!page.graph && !page.series && page.processes.is_none());
    let invalid = Capture::for_command(&Command::Table {
        kind: "processes".into(),
        offset: 0,
        limit: 101,
    });
    assert!(invalid.processes.is_none());
}

#[test]
fn wait_timeout_returns_observed_failure_and_releases_the_request() {
    use crate::app::models::AppStateStorage;
    use std::sync::RwLock;
    let mut app = AppState::new_for_test(
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
        4,
    );
    let ctx = egui::Context::default();
    let (_, rx) = mpsc::sync_channel(4);
    let (reply, result) = mpsc::sync_channel(1);
    let options = WaitOptions {
        condition: WaitCondition::NewSample,
        timeout_ms: 1,
        after_seconds: Some(10.0),
        enabled: None,
        history_seconds: None,
    };
    let mut bridge = Bridge {
        requests: rx,
        _transport: None,
        pending: Some(Pending {
            request: Request {
                id: 1,
                command: Command::Wait(options),
                reply,
                deadline: Instant::now() + std::time::Duration::from_secs(8),
            },
            after: 0,
            wait: Some((
                options,
                Instant::now() - std::time::Duration::from_millis(5),
            )),
        }),
        screenshot: None,
    };
    let mut output = ctx.run_ui(Default::default(), |ui| bridge.end_ui(&mut app, ui.ctx()));
    output.textures_delta.clear();
    let Reply::Json(body) = result.try_recv().unwrap() else {
        panic!("Expected wait result")
    };
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["satisfied"], false);
    assert_eq!(body["data"]["timed_out"], true);
    assert!(body["data"]["observed"]["completed_seconds"].is_null());
    assert!(bridge.pending.is_none());
}

#[test]
fn wait_conditions_preserve_unknowns_strict_freshness_and_commit_state() {
    let mut observed = WaitObservation::default();
    let options = WaitOptions {
        condition: WaitCondition::NewSample,
        timeout_ms: 100,
        after_seconds: Some(10.0),
        enabled: None,
        history_seconds: None,
    };
    assert!(!options.satisfied(&observed));
    observed.completed_seconds = Some(10.0);
    assert!(!options.satisfied(&observed));
    observed.completed_seconds = Some(10.01);
    assert!(options.satisfied(&observed));
    let settings = WaitOptions {
        condition: WaitCondition::SettingsApplied,
        enabled: Some(false),
        history_seconds: Some(30),
        ..options
    };
    observed.enabled = false;
    observed.history_seconds = 30;
    assert!(!settings.satisfied(&observed));
    observed.settings_committed = true;
    assert!(settings.satisfied(&observed));
    assert!(WaitOptions {
        timeout_ms: 6001,
        ..settings
    }
    .validate()
    .is_err());
    assert!(WaitOptions {
        condition: WaitCondition::CollectionReleased,
        enabled: Some(true),
        ..settings
    }
    .validate()
    .is_err());
}

#[test]
fn graph_commands_resolve_saved_identity_close_settings_and_reject_invalid_views() {
    use crate::app::features::cpu_load::debug_api;
    use crate::app::models::{AppStateStorage, CoreGroup};
    use std::sync::RwLock;
    let mut app = AppState::new_for_test(
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|s| {
            s.groups.push(CoreGroup {
                name: "group".into(),
                cores: vec![2, 3],
                programs: vec![],
                is_hidden: false,
                run_all_button: false,
            });
        }))),
        4,
    );
    let id = app.rules.group_id_for_index(0).unwrap().0;
    app.ui.app_settings_open = true;
    app.ui.cpu_monitor_open = true;
    debug_api::apply(
        &mut app,
        &Command::OpenGraph {
            group_id: id,
            rule_id: None,
        },
    )
    .unwrap();
    assert!(!app.ui.app_settings_open);
    assert!(!app.ui.cpu_monitor_open);
    let before = debug_api::graph(&app, &egui::Context::default());
    assert_eq!(before["target"]["kind"], "group");
    assert!(debug_api::apply(
        &mut app,
        &Command::GraphOptions {
            all_cpus: Some(true),
            tracked_only: None,
            view: Some("invalid".into())
        }
    )
    .is_err());
    assert!(
        !app.cpu_load.show_all_cpus,
        "invalid request cannot partially mutate graph state"
    );
    debug_api::apply(
        &mut app,
        &Command::GraphOptions {
            all_cpus: Some(true),
            tracked_only: None,
            view: Some("processes".into()),
        },
    )
    .unwrap();
    assert!(app.cpu_load.show_all_cpus && app.cpu_load.show_processes);
    assert!(debug_api::apply(
        &mut app,
        &Command::OpenGraph {
            group_id: "removed".into(),
            rule_id: None
        }
    )
    .is_err());
    assert_eq!(
        debug_api::graph(&app, &egui::Context::default())["target"],
        before["target"]
    );
}

#[test]
fn http_gate_requires_exact_local_host_origin_bearer_and_bounded_json() {
    let request = |auth: &str, origin: &str, length: usize| {
        format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nAuthorization: Bearer {auth}\r\nContent-Type: application/json\r\n{origin}Content-Length: {length}\r\n\r\n")
    };
    assert_eq!(
        authorize(&request("secret", "", 4), 1234, "secret").unwrap(),
        4
    );
    assert!(authorize(&request("wrong", "", 4), 1234, "secret").is_err());
    assert!(authorize(
        &request("secret", "Origin: https://foreign.example\r\n", 4),
        1234,
        "secret"
    )
    .is_err());
    assert!(authorize(&request("secret", "", MAX_BODY + 1), 1234, "secret").is_err());
    assert!(authorize(
        &request("secret", "", 4).replace("POST /rpc", "GET /rpc"),
        1234,
        "secret"
    )
    .is_err());
    assert!(authorize(
        &request("secret", "", 4).replace("Host: 127.0.0.1:1234", "Host: foreign.example"),
        1234,
        "secret"
    )
    .is_err());
}

#[test]
fn input_targets_are_current_visible_and_normalized_and_click_has_real_edges() {
    let target = Control {
        id: "test".into(),
        rect: egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 30.0)),
        enabled: true,
        extra: Value::Null,
    };
    let events = pointer_events(&target, "click", 0.5, 0.5, 0.0).unwrap();
    assert!(matches!(events[0],egui::Event::PointerMoved(p) if p==egui::pos2(60.0,35.0)));
    assert!(matches!(
        events[1],
        egui::Event::PointerButton { pressed: true, .. }
    ));
    assert!(matches!(
        events[2],
        egui::Event::PointerButton { pressed: false, .. }
    ));
    assert!(pointer_events(&target, "move", f32::NAN, 0.5, 0.0).is_err());
    assert!(pointer_events(&target, "move", 1.1, 0.5, 0.0).is_err());
    assert!(pointer_events(
        &Control {
            enabled: false,
            ..target
        },
        "click",
        0.5,
        0.5,
        0.0
    )
    .is_err());
}

#[test]
fn debug_command_parser_exposes_no_launch_shell_or_tweak_operations() {
    for action in ["run_rule", "shell", "apply_affinity", "tweak", "write_file"] {
        assert!(serde_json::from_value::<Command>(serde_json::json!({"action":action})).is_err());
    }
    assert!(
        serde_json::from_value::<Command>(serde_json::json!({"action":"state","unknown":1}))
            .is_err()
    );
}

#[test]
fn system_tweaks_mcp_reads_snapshot_and_registers_actual_ui_controls() {
    use crate::app::{
        models::AppStateStorage, shell::presenters::system_tweaks::draw_system_tweaks,
    };
    use std::sync::RwLock;
    let mut app = AppState::new_for_test(
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
        4,
    );
    let ctx = egui::Context::default();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(ENABLED), true));
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| draw_system_tweaks(&mut app, ui),
        );
        output.textures_delta.clear();
    }
    let value = state(&mut app, &ctx).unwrap();
    assert_eq!(value["tweaks"]["section"], "system");
    assert_eq!(value["tweaks"]["background"]["enabled"], false);
    assert_eq!(value["tweaks"]["background"]["changed_targets"], 0);
    assert!(value["tweaks"]["background"].get("proof").is_none());
    assert!(value["tweaks"]["cards"].is_array());
    for id in [
        "tweaks.section.system",
        "tweaks.section.hardware",
        "tweaks.section.checks",
        "tweaks.body",
        "tweaks.undo_all",
    ] {
        assert!(
            value["controls"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == id),
            "Missing {id}"
        );
    }
    let registry = ctx
        .data(|d| d.get_temp::<Registry>(egui::Id::new(REGISTRY)))
        .unwrap();
    let control = registry
        .entries
        .iter()
        .find(|c| c.id == "tweaks.section.hardware")
        .unwrap();
    let events = pointer_events(control, "click", 0.5, 0.5, 0.0).unwrap();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| draw_system_tweaks(&mut app, ui),
    );
    output.textures_delta.clear();
    assert_eq!(
        state(&mut app, &ctx).unwrap()["tweaks"]["section"],
        "hardware"
    );
    app.ui.tuning.tweak_help = Some(("Test help", "Cached help".into()));
    for _ in 0..2 {
        let mut output = ctx.run_ui(Default::default(), |ui| draw_system_tweaks(&mut app, ui));
        output.textures_delta.clear();
    }
    assert_eq!(
        state(&mut app, &ctx).unwrap()["tweaks"]["help"],
        "Test help"
    );
    let registry = ctx
        .data(|d| d.get_temp::<Registry>(egui::Id::new(REGISTRY)))
        .unwrap();
    let control = registry
        .entries
        .iter()
        .rev()
        .find(|c| c.id == "tweaks.help.close")
        .unwrap();
    let events = pointer_events(control, "click", 0.5, 0.5, 0.0).unwrap();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| draw_system_tweaks(&mut app, ui),
    );
    output.textures_delta.clear();
    assert!(app.ui.tuning.tweak_help.is_none());
}

#[test]
fn egui_input_updates_real_settings_then_reply_reads_the_settled_frame() {
    use crate::app::models::AppStateStorage;
    use std::sync::RwLock;
    let mut app = AppState::new_for_test(
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
        4,
    );
    let ctx = egui::Context::default();
    crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(ENABLED), true));
    let (tx, rx) = mpsc::sync_channel(4);
    let mut bridge = Bridge {
        requests: rx,
        _transport: None,
        pending: None,
        screenshot: None,
    };
    let frame = |bridge: &mut Bridge, app: &mut AppState| {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 700.0),
            )),
            ..Default::default()
        };
        bridge.raw_input(app, &ctx, &mut input);
        let mut output = ctx.run_ui(input, |ui| {
            bridge.begin_ui(ui.ctx());
            crate::app::shell::presenters::header::draw_top_panel(app, ui);
            crate::app::shell::presenters::service_controls::draw_modals(app, ui.ctx());
            bridge.end_ui(app, ui.ctx());
        });
        output.textures_delta.clear();
    };
    for _ in 0..3 {
        frame(&mut bridge, &mut app);
    }
    let (reply, result) = mpsc::sync_channel(1);
    tx.send(Request {
        id: 1,
        command: Command::Input {
            target: "settings.open".into(),
            kind: "click".into(),
            x: 0.5,
            y: 0.5,
            delta: 0.0,
        },
        reply,
        deadline: Instant::now() + std::time::Duration::from_secs(5),
    })
    .unwrap();
    frame(&mut bridge, &mut app);
    assert!(
        result.try_recv().is_err(),
        "never reply from the input's pre-settlement frame"
    );
    frame(&mut bridge, &mut app);
    let Reply::Json(body) = result.try_recv().unwrap() else {
        panic!("expected acknowledgement")
    };
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["applied"], true);
    assert!(body["data"].get("settings").is_none());
    let snapshot = state(&mut app, &ctx).unwrap();
    assert_eq!(
        snapshot["settings"]["open"], true,
        "input must hit the actual widget"
    );
    assert!(snapshot["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "settings.theme.1"));
    assert!(!snapshot["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "settings.history"));
    let (reply, result) = mpsc::sync_channel(1);
    tx.send(Request {
        id: 2,
        command: Command::Input {
            target: "cpu.monitor.open".into(),
            kind: "click".into(),
            x: 0.5,
            y: 0.5,
            delta: 0.0,
        },
        reply,
        deadline: Instant::now() + std::time::Duration::from_secs(5),
    })
    .unwrap();
    frame(&mut bridge, &mut app);
    assert!(result.try_recv().is_err());
    frame(&mut bridge, &mut app);
    let Reply::Json(body) = result.try_recv().unwrap() else {
        panic!("expected acknowledgement")
    };
    assert_eq!(body["ok"], true);
    assert!(app.ui.cpu_monitor_open);
    assert!(state(&mut app, &ctx).unwrap()["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "settings.history"));
}
