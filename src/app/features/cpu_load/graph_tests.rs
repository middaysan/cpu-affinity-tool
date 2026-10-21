use super::*;
use crate::app::models::CoreGroup;
use crate::app::runtime::AppState;
use crate::app::shared::ids::RuleId;
use eframe::egui::{self, Pos2};

fn fixture() -> (AppState, RuleTarget) {
    let mut app = AppState::new_for_test(
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
            state.groups.push(CoreGroup {
                name: "Background".into(),
                cores: (12..20).collect(),
                programs: vec![],
                is_hidden: false,
                run_all_button: false,
            });
        }))),
        20,
    );
    let target = RuleTarget {
        group: app.rules.group_id_for_index(0).unwrap(),
        rule: RuleId("chrome".into()),
    };
    app.cpu_load.open(GraphTarget::Process {
        target: target.clone(),
        name: "Chrome".into(),
    });
    let mut values = vec![Some(0.0); 20];
    values[0] = Some(12.0);
    values[12] = Some(33.0);
    *app.cpu_load.published.write().unwrap() = Arc::new(Snapshot {
        cpus: vec![Some(99.0); 20],
        parked: vec![Some(false); 20],
        process_cpus: HashMap::from([(target.clone(), values)]),
        ..Snapshot::default()
    });
    (app, target)
}
fn frame(app: &mut AppState, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(400.0, 900.0),
            )),
            events,
            ..Default::default()
        },
        |ui| super::super::view::draw_modal(app, ui.ctx()),
    );
    output.textures_delta.clear();
    output
}
fn label(output: &egui::FullOutput, name: &str) -> Option<Pos2> {
    if let Some((cpu, load)) = name.split_once("  ").filter(|(cpu, _)| {
        cpu.strip_prefix("CPU")
            .is_some_and(|v| v.parse::<usize>().is_ok())
    }) {
        return output.shapes.iter().find_map(|shape|match &shape.shape {
            egui::Shape::Text(text) if text.galley.text()==cpu => {
                let center=text.galley.rect.translate(text.pos.to_vec2()).center();
                let tile=tile_at(output,center)?;
                output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()==load && tile.contains(t.galley.rect.translate(t.pos.to_vec2()).center()))).then_some(center)
            }
            _=>None,
        });
    }
    let labels = output.shapes.iter().filter_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == name => {
            Some(text.galley.rect.translate(text.pos.to_vec2()).center())
        }
        _ => None,
    });
    if name
        .strip_prefix("CPU")
        .is_some_and(|v| v.parse::<usize>().is_ok())
    {
        // Detail badge follows tiles; exclude the plain table column header.
        labels.filter(|pos|output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Rect(r) if r.rect.contains(*pos) && r.rect.height()<30.0 && r.stroke.width==1.0 && r.fill!=egui::Color32::TRANSPARENT))).last()
    } else {
        labels.into_iter().next()
    }
}
fn tile_at(output: &egui::FullOutput, position: Pos2) -> Option<egui::Rect> {
    output.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Rect(r)
            if r.rect.contains(position)
                && r.rect.height() >= 40.0
                && r.rect.height() < 100.0
                && r.rect.width() < 130.0
                && r.stroke.width >= 1.0 =>
        {
            Some(r.rect)
        }
        _ => None,
    })
}
fn click(app: &mut AppState, ctx: &egui::Context, pos: Pos2) -> egui::FullOutput {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    frame(app, ctx, vec![])
}
fn tiles(output: &egui::FullOutput) -> Vec<usize> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text)
                if tile_at(
                    output,
                    text.galley.rect.translate(text.pos.to_vec2()).center(),
                )
                .is_some() =>
            {
                text.galley.text().strip_prefix("CPU")?.parse().ok()
            }
            _ => None,
        })
        .collect()
}

fn has_red_border(shape: &egui::Shape, position: Pos2) -> bool {
    match shape {
        egui::Shape::Rect(rect) => {
            rect.rect.contains(position)
                && rect.stroke.color.r() > rect.stroke.color.g()
                && rect.stroke.color.a() > 0
        }
        egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_red_border(shape, position)),
        _ => false,
    }
}

#[test]
fn measured_partial_work_is_visible_in_app_and_tracked_group_tiles_and_history() {
    for dark in [true, false] {
        let (mut app, target) = fixture();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
        snapshot.process_cpus.insert(target.clone(), vec![None; 20]);
        snapshot.tracked_cpus = vec![None; 20];
        let mut current = vec![None; 20];
        current[12] = Some(3.0);
        snapshot
            .process_partial_cpus
            .insert(target.clone(), current.clone());
        snapshot.tracked_partial_cpus = current;
        *app.cpu_load.published.write().unwrap() = Arc::new(snapshot);
        {
            let mut store = app.cpu_load.history.write().unwrap();
            store
                .configure(20, 30, std::slice::from_ref(&target))
                .unwrap();
            for (second, value) in [(1, 1.25), (2, 2.0), (3, 3.0)] {
                let mut partial = vec![None; 20];
                partial[12] = Some(value);
                let time = SampleTime {
                    seconds: second as f64,
                    wall: SystemTime::UNIX_EPOCH + Duration::from_secs(second),
                };
                store.push(
                    time.seconds,
                    time.wall,
                    &[Some(90.0); 20],
                    &[],
                    &HashMap::new(),
                );
                store.record_partial(
                    &partial,
                    &HashMap::from([(target.clone(), partial.clone())]),
                );
                store.contributors.push(
                    time,
                    &[Contribution {
                        run: None,
                        process: Arc::new(os_api::cpu_load::CpuProcess {
                            pid: 42,
                            instance: 10,
                            name: "busy.exe".into(),
                        }),
                        rule: Some(Arc::new(target.rule.clone())),
                        cpu: 12,
                        load: value,
                    }],
                    true,
                );
            }
        }
        let mut output = frame(&mut app, &ctx, vec![]);
        for _ in 0..3 {
            output = frame(&mut app, &ctx, vec![]);
        }
        assert!(
            label(&output, "CPU12  3%*").is_some(),
            "live tile must expose its measured subtotal"
        );
        assert!(label(&output, "Partial").is_some());
        assert!(label(&output, "busy.exe").is_some());
        let rect = ctx
            .read_response(egui::Id::new("cpu-load-chart"))
            .unwrap()
            .rect;
        let output = frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(
                rect.left() + 4.0,
                rect.center().y,
            ))],
        );
        assert!(
            label(&output, "CPU12  1%*").is_some(),
            "history tile must use that exact partial sample"
        );
        assert_eq!(output.platform_output.num_completed_passes, 1);
        app.cpu_load.open(GraphTarget::Group {
            id: target.group.clone(),
            name: "Background".into(),
            cpus: (12..20).collect(),
        });
        app.cpu_load.set_tracked_only(true);
        let mut output = frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(2.0, 2.0))],
        );
        for _ in 0..3 {
            output = frame(&mut app, &ctx, vec![]);
        }
        assert!(label(&output, "CPU12  3%*").is_some());
        assert!(label(&output, "Processes · partial").is_some());
        let data = app.cpu_load.graph_data(None, 12, |_| None);
        assert!(data
            .series
            .points
            .iter()
            .all(|p| p.value.is_some() && p.partial));
    }
}

#[test]
fn group_scope_checkbox_switches_load_and_contributors_together_in_both_themes() {
    for dark in [true, false] {
        let (mut app, target) = fixture();
        app.persistent_state.write().unwrap().groups[0].cores = vec![0];
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
        snapshot.cpus = vec![Some(90.0); 20];
        snapshot.tracked_cpus = vec![Some(30.0); 20];
        *app.cpu_load.published.write().unwrap() = Arc::new(snapshot.clone());
        {
            let mut store = app.cpu_load.history.write().unwrap();
            store
                .configure(20, 30, std::slice::from_ref(&target))
                .unwrap();
            store.push(
                1.0,
                SystemTime::UNIX_EPOCH,
                &snapshot.cpus,
                &snapshot.parked,
                &snapshot.process_cpus,
            );
            store.record_tracked(&snapshot.tracked_cpus);
            let p = |pid, name: &str, rule, load| Contribution {
                run: None,
                process: Arc::new(os_api::cpu_load::CpuProcess {
                    pid,
                    instance: 10,
                    name: name.into(),
                }),
                rule,
                cpu: 0,
                load,
            };
            store.contributors.push(
                SampleTime {
                    seconds: 1.0,
                    wall: SystemTime::UNIX_EPOCH,
                },
                &[
                    p(1, "tracked.exe", None, 30.0),
                    p(2, "foreign.exe", None, 60.0),
                    p(1, "tracked.exe", Some(Arc::new(target.rule.clone())), 30.0),
                ],
                true,
            );
        }
        app.cpu_load.open(GraphTarget::Group {
            id: target.group,
            name: "Performance".into(),
            cpus: vec![0],
        });
        let mut output = frame(&mut app, &ctx, vec![]);
        for _ in 0..3 {
            output = frame(&mut app, &ctx, vec![]);
        }
        assert!(app.cpu_load.tracked_only());
        assert!(label(&output, "CPU0  30%").is_some());
        assert!(label(&output, "foreign.exe").is_none());
        let filter = label(&output, "All system processes")
            .expect("visible opt-in for other Windows processes");
        let plot = ctx
            .read_response(egui::Id::new("cpu-load-chart"))
            .unwrap()
            .rect;
        let mode = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().starts_with("Live") => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()))
                }
                _ => None,
            })
            .expect("visible live mode");
        assert!((mode.center().y - filter.y).abs() < 2.0);
        assert!(
            mode.right() >= plot.right() - 1.0,
            "inspection mode must align to the row's right edge"
        );
        let output = click(&mut app, &ctx, filter);
        assert!(!app.cpu_load.tracked_only());
        assert!(label(&output, "CPU0  90%").is_some());
        assert!(label(&output, "foreign.exe").is_some());
        assert_eq!(
            ctx.read_response(egui::Id::new("cpu-load-chart"))
                .unwrap()
                .rect,
            plot,
            "scope change must not move plot geometry"
        );
        let output = click(
            &mut app,
            &ctx,
            label(&output, "All system processes").unwrap(),
        );
        assert!(app.cpu_load.tracked_only());
        assert!(label(&output, "CPU0  30%").is_some() && label(&output, "foreign.exe").is_none());
        // The contributor card also has a Processes heading; the footer switch is last.
        let switch = output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Processes" => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .expect("group process view switch");
        let output = click(&mut app, &ctx, switch);
        assert!(app.cpu_load.show_processes);
        assert!(label(&output, "tracked.exe").is_some());
        assert!(label(&output, "foreign.exe").is_none());
        assert!(tiles(&output).is_empty());
        let output = click(
            &mut app,
            &ctx,
            label(&output, "All system processes").unwrap(),
        );
        assert!(label(&output, "foreign.exe").is_some());
        let output = click(&mut app, &ctx, label(&output, "Graphs").unwrap());
        assert!(!app.cpu_load.show_processes && !tiles(&output).is_empty());
        let modal = ctx
            .memory(|m| m.area_rect(egui::Id::new("cpu-load-modal")))
            .unwrap();
        assert!((modal.height() - 720.0).abs() < 2.0);
    }
}

#[test]
fn all_group_scopes_surface_collector_failures_without_relabeling_host_load() {
    let (mut app, target) = fixture();
    let ctx = egui::Context::default();
    let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
    snapshot.tracked_cpus = vec![None; 20];
    snapshot.detail_error = Some("Scheduler attribution is unavailable".into());
    snapshot.collector_error = snapshot.detail_error.clone();
    {
        let mut store = app.cpu_load.history.write().unwrap();
        store
            .configure(20, 30, std::slice::from_ref(&target))
            .unwrap();
        let time = SampleTime {
            seconds: 1.0,
            wall: SystemTime::UNIX_EPOCH,
        };
        store.push_pending(time, &snapshot.parked);
        store.mark_lost_reason(time, super::super::history::GapReason::TraceUnavailable);
    }
    *app.cpu_load.published.write().unwrap() = Arc::new(snapshot);
    app.cpu_load.open(GraphTarget::Group {
        id: target.group,
        name: "Background".into(),
        cpus: (12..20).collect(),
    });
    let mut output = frame(&mut app, &ctx, vec![]);
    for _ in 0..3 {
        output = frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(
        app.cpu_load.snapshot().cpus[12],
        Some(99.0),
        "healthy PDH is a separate metric"
    );
    assert!(label(&output, "CPU12  —").is_some());
    assert!(label(&output, "Scheduler attribution is unavailable").is_some());
    let output = click(
        &mut app,
        &ctx,
        label(&output, "All system processes").unwrap(),
    );
    assert!(label(&output, "CPU12  —").is_some());
    assert!(label(&output, "Scheduler attribution is unavailable").is_some());
}

#[test]
fn host_group_does_not_show_tracked_membership_only_errors() {
    let (mut app, target) = fixture();
    let ctx = egui::Context::default();
    let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
    snapshot.detail_error = Some("Tracked processes changed; waiting for the next interval".into());
    *app.cpu_load.published.write().unwrap() = Arc::new(snapshot);
    app.cpu_load.open(GraphTarget::Group {
        id: target.group,
        name: "Background".into(),
        cpus: (12..20).collect(),
    });
    let mut output = frame(&mut app, &ctx, vec![]);
    for _ in 0..3 {
        output = frame(&mut app, &ctx, vec![]);
    }
    assert!(label(
        &output,
        "Tracked processes changed; waiting for the next interval"
    )
    .is_some());
    let output = click(
        &mut app,
        &ctx,
        label(&output, "All system processes").unwrap(),
    );
    assert!(label(
        &output,
        "Tracked processes changed; waiting for the next interval"
    )
    .is_none());
}

#[test]
fn current_process_table_is_compact_scrollable_and_keeps_scroll_on_snapshot_refresh() {
    for dark in [true, false] {
        let (mut app, target) = fixture();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let instances: Vec<_> = (0..80).map(|i| (1000 + i, 10)).collect();
        let names = instances
            .iter()
            .enumerate()
            .map(|(i, &(pid, token))| {
                (
                    (pid, token),
                    Arc::new(os_api::cpu_load::CpuProcess {
                        pid,
                        instance: token,
                        name: if i == 0 {
                            "a very long helper process executable name.exe".into()
                        } else {
                            format!("process-{i}.exe").into()
                        },
                    }),
                )
            })
            .collect();
        let list = super::super::process_list::ProcessList::from_tracking(
            &HashMap::from([(target.clone(), instances.clone())]),
            &HashMap::new(),
            &names,
        );
        let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
        snapshot.live_processes = Arc::new(list);
        *app.cpu_load.published.write().unwrap() = Arc::new(snapshot.clone());
        app.cpu_load.show_processes = true;
        let render = |app: &mut AppState, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(400.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| super::super::view::draw_modal(app, ui.ctx()),
            );
            output.textures_delta.clear();
            output
        };
        let mut output = render(&mut app, vec![]);
        for _ in 0..3 {
            output = render(&mut app, vec![]);
        }
        let modal = ctx
            .memory(|m| m.area_rect(egui::Id::new("cpu-load-modal")))
            .unwrap();
        assert!(
            modal.width() <= 400.0 && modal.height() <= 600.0,
            "current table must fit compact viewport: {modal:?}"
        );
        assert!(
            label(&output, "process-79.exe").is_none(),
            "virtual list must not render every PID"
        );
        let pos = label(&output, "1001").unwrap();
        render(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -2200.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Default::default(),
                },
            ],
        );
        for _ in 0..50 {
            output = render(&mut app, vec![]);
        }
        let bottom = label(&output, "1079").expect("last PID must be reachable by scrolling");
        let fresh = super::super::process_list::ProcessList::from_tracking(
            &HashMap::from([(target, instances)]),
            &(1000..1080).map(|pid| ((pid, 10), Some(0.0))).collect(),
            &names,
        );
        snapshot.live_processes = Arc::new(fresh);
        snapshot.at = Instant::now();
        *app.cpu_load.published.write().unwrap() = Arc::new(snapshot);
        let output = render(&mut app, vec![]);
        assert_eq!(
            label(&output, "1079"),
            Some(bottom),
            "1Hz publication must preserve table scroll and geometry"
        );
        render(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(app.cpu_load.graph.is_none());
        assert!(!app.cpu_load.show_processes);
    }
}

#[test]
fn app_modal_switches_to_all_current_pids_without_mask_or_activity_filtering() {
    for dark in [true, false] {
        let (mut app, target) = fixture();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let names = HashMap::from([
            (
                (101, 10),
                Arc::new(os_api::cpu_load::CpuProcess {
                    pid: 101,
                    instance: 10,
                    name: "busy.exe".into(),
                }),
            ),
            (
                (102, 20),
                Arc::new(os_api::cpu_load::CpuProcess {
                    pid: 102,
                    instance: 20,
                    name: "idle.exe".into(),
                }),
            ),
            (
                (103, 30),
                Arc::new(os_api::cpu_load::CpuProcess {
                    pid: 103,
                    instance: 30,
                    name: "unknown.exe".into(),
                }),
            ),
        ]);
        let mut list = super::super::process_list::ProcessList::from_tracking(
            &HashMap::from([(target.clone(), vec![(101, 10), (102, 20), (103, 30)])]),
            &HashMap::from([((101, 10), Some(2.0)), ((102, 20), Some(0.0))]),
            &names,
        );
        list.record_cores(
            20,
            &[
                os_api::cpu_load::ProcessCpuSample {
                    pid: 101,
                    token: 10,
                    cpus: (0..20)
                        .map(|cpu| if cpu == 0 { Some(40.0) } else { Some(0.0) })
                        .collect(),
                },
                os_api::cpu_load::ProcessCpuSample {
                    pid: 102,
                    token: 20,
                    cpus: vec![Some(0.0); 20],
                },
            ],
        );
        let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
        snapshot.live_processes = Arc::new(list);
        *app.cpu_load.published.write().unwrap() = Arc::new(snapshot);
        let mut output = frame(&mut app, &ctx, vec![]);
        for _ in 0..3 {
            output = frame(&mut app, &ctx, vec![]);
        }
        let switch = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Processes" => {
                    Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                }
                _ => None,
            })
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .expect("bottom Processes switch");
        let output = click(&mut app, &ctx, switch);
        for value in [
            "busy.exe",
            "idle.exe",
            "unknown.exe",
            "101",
            "102",
            "103",
            "2%",
            "0%",
            "Idle",
            "Unavailable",
            "CPUs",
        ] {
            assert!(
                label(&output, value).is_some(),
                "current process table missing {value}"
            );
        }
        assert!(
            tiles(&output).is_empty(),
            "processes mode replaces CPU tiles/plot"
        );
        assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().starts_with("3 tracked processes"))));
        assert!(app.cpu_load.show_processes);
        let output = click(&mut app, &ctx, label(&output, "Graphs").unwrap());
        assert!(!tiles(&output).is_empty());
        assert!(!app.cpu_load.show_processes);
    }
}

#[test]
fn app_activity_dots_follow_the_retained_window_independent_of_hover_or_pin() {
    for dark in [true, false] {
        let (mut app, target) = fixture();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let yellow = egui::Color32::from_rgb(242, 196, 75);
        {
            let mut history = app.cpu_load.history.write().unwrap();
            history
                .configure(20, 30, std::slice::from_ref(&target))
                .unwrap();
            for second in 0..=2 {
                let mut values = vec![Some(0.0); 20];
                if second == 0 {
                    values[12] = Some(0.001);
                }
                history.push(
                    second as f64,
                    SystemTime::UNIX_EPOCH + Duration::from_secs(second),
                    &[Some(99.0); 20],
                    &[Some(true); 20],
                    &HashMap::from([(target.clone(), values)]),
                );
            }
        }
        let dots = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Circle(c) if c.fill == yellow && c.radius == 2.5 => Some(
                        egui::Rect::from_center_size(c.center, egui::Vec2::splat(5.0)),
                    ),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let mut output = frame(&mut app, &ctx, vec![]);
        for _ in 0..3 {
            output = frame(&mut app, &ctx, vec![]);
        }
        let markers = dots(&output);
        assert_eq!(
            markers.len(),
            1,
            "only CPU12 has measured app work, not the host's busy CPUs or live snapshot"
        );
        let tile_label = label(&output, "CPU12  0%").unwrap();
        let tile = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(r)
                    if r.rect.contains(tile_label)
                        && r.rect.height() < 100.0
                        && r.stroke.width >= 2.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .unwrap();
        assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Rect(r) if r.rect.height()<25.0 && tile.contains_rect(r.rect) && r.rect.contains(tile_label) && r.rect.contains_rect(markers[0]))),"dot must be inside the CPU12 name badge");
        ctx.all_styles_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        let _ = frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(markers[0].center())],
        );
        let tooltip = frame(&mut app, &ctx, vec![]);
        assert!(tooltip.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("This app has measured CPU work"))),"yellow dot must explain its retained-history meaning on hover");
        let plot = ctx
            .read_response(egui::Id::new("cpu-load-chart"))
            .unwrap()
            .rect;
        let output = click(
            &mut app,
            &ctx,
            egui::pos2(plot.right() - 3.0, plot.center().y),
        );
        assert!(app.cpu_load.inspection.pinned.is_some());
        assert_eq!(
            dots(&output),
            markers,
            "pinning a zero-load sample does not erase older measured work"
        );
        {
            let mut history = app.cpu_load.history.write().unwrap();
            for second in 3..=31 {
                history.push(
                    second as f64,
                    SystemTime::UNIX_EPOCH + Duration::from_secs(second),
                    &[Some(99.0); 20],
                    &[],
                    &HashMap::from([(target.clone(), vec![Some(0.0); 20])]),
                );
            }
        }
        let output = frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(2.0, 2.0))],
        );
        assert!(
            dots(&output).is_empty(),
            "last positive sample expired even though a time remains pinned"
        );
        let mut values = vec![Some(0.0); 20];
        values[13] = Some(0.01);
        app.cpu_load.history.write().unwrap().push(
            32.0,
            SystemTime::UNIX_EPOCH + Duration::from_secs(32),
            &[Some(99.0); 20],
            &[],
            &HashMap::from([(target.clone(), values)]),
        );
        let output = frame(&mut app, &ctx, vec![]);
        assert_eq!(
            dots(&output).len(),
            1,
            "new work adds a marker despite the frozen inspection"
        );
        let cpu13 = label(&output, "CPU13  0%").unwrap();
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Rect(r) if r.rect.height()<100.0 && r.rect.contains(cpu13) && r.rect.contains_rect(dots(&output)[0]))),"marker belongs to CPU13");
        app.cpu_load.open(GraphTarget::Group {
            id: target.group,
            name: "Background".into(),
            cpus: (12..20).collect(),
        });
        let output = frame(&mut app, &ctx, vec![]);
        assert!(
            dots(&output).is_empty(),
            "group host activity is not an app-presence marker"
        );
    }
}

#[test]
fn pinned_graph_keeps_exact_time_while_scrolling_and_unpins_on_the_next_click() {
    for dark in [true, false] {
        let (mut app, target) = fixture();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        {
            let mut history = app.cpu_load.history.write().unwrap();
            history
                .configure(20, 30, std::slice::from_ref(&target))
                .unwrap();
            for second in 1..=3 {
                let mut values = vec![Some(0.0); 20];
                values[12] = Some(10.0 + second as f32);
                history.push(
                    second as f64,
                    SystemTime::UNIX_EPOCH + Duration::from_secs(second),
                    &[Some(1.0); 20],
                    &[Some(false); 20],
                    &HashMap::from([(target.clone(), values)]),
                );
                history.contributors.push(
                    SampleTime {
                        seconds: second as f64,
                        wall: SystemTime::UNIX_EPOCH + Duration::from_secs(second),
                    },
                    &(0..40)
                        .map(|i| Contribution {
                            run: None,
                            process: Arc::new(os_api::cpu_load::CpuProcess {
                                pid: 1000 + i,
                                instance: 1,
                                name: format!("sample-{second}-process-{i}.exe").into(),
                            }),
                            rule: Some(Arc::new(target.rule.clone())),
                            cpu: 12,
                            load: (40 - i) as f32 * 0.02,
                        })
                        .collect::<Vec<_>>(),
                    true,
                );
            }
        }
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let plot = ctx
            .read_response(egui::Id::new("cpu-load-chart"))
            .unwrap()
            .rect;
        let first = egui::pos2(plot.left() + 3.0, plot.center().y);
        let time =
            os_api::cpu_load::sample_clock_label(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        let output = click(&mut app, &ctx, first);
        assert!(
            label(&output, &format!("Pinned · {time}")).is_some(),
            "left click should pin the recorded sample"
        );
        assert!(label(&output, "CPU12  11%").is_some());
        let table = label(&output, "sample-1-process-0.exe").unwrap();
        let output = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(table)]);
        assert!(label(&output, &format!("Pinned · {time}")).is_some());
        assert!(
            label(&output, "CPU12  11%").is_some(),
            "moving into the table must not restore live data"
        );
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::LineSegment{points,..} if (points[0].x-points[1].x).abs()<0.1 && (points[0].y-points[1].y).abs()>80.0 && plot.contains(points[0]) && plot.contains(points[1]))),"pinned cursor must remain after the pointer leaves");
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -150.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            }],
        );
        let mut output = frame(&mut app, &ctx, vec![]);
        for _ in 0..5 {
            output = frame(&mut app, &ctx, vec![]);
        }
        assert!(
            label(&output, "sample-1-process-0.exe").is_none(),
            "contributor table should actually scroll"
        );
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(t) if t.galley.text().starts_with("sample-1-process-"))),"scroll must keep the pinned contributor frame");
        assert!(
            label(&output, &format!("Pinned · {time}")).is_some(),
            "wheel must not unpin"
        );
        let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
        snapshot.process_cpus.get_mut(&target).unwrap()[12] = Some(88.0);
        *app.cpu_load.published.write().unwrap() = Arc::new(snapshot.clone());
        app.cpu_load.history.write().unwrap().push(
            4.0,
            SystemTime::UNIX_EPOCH + Duration::from_secs(4),
            &snapshot.cpus,
            &snapshot.parked,
            &snapshot.process_cpus,
        );
        let output = frame(&mut app, &ctx, vec![]);
        assert!(
            label(&output, "CPU12  11%").is_some(),
            "new samples must not move the pinned time"
        );
        let output = click(
            &mut app,
            &ctx,
            egui::pos2(plot.right() - 3.0, plot.center().y),
        );
        let latest =
            os_api::cpu_load::sample_clock_label(SystemTime::UNIX_EPOCH + Duration::from_secs(4));
        assert!(
            label(&output, &format!("History · {latest}")).is_some(),
            "second plot click must unpin into ordinary hover, not re-pin on relayout"
        );
        let output = frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(2.0, 2.0))],
        );
        assert!(label(&output, "Live").is_some());
        assert!(label(&output, "CPU12  88%").is_some());
        let output = click(&mut app, &ctx, first);
        assert!(label(&output, &format!("Pinned · {time}")).is_some());
        let output = click(
            &mut app,
            &ctx,
            label(&output, "Chrome · Application CPU").unwrap(),
        );
        assert!(
            label(&output, "Live").is_some(),
            "a click outside the plot should unpin"
        );
        assert!(
            app.cpu_load.graph.is_some(),
            "unpinning inside the dialog must not close it"
        );
        click(&mut app, &ctx, first);
        let target_graph = app.cpu_load.graph.clone().unwrap();
        app.cpu_load.close();
        app.cpu_load.open(target_graph);
        let output = frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(2.0, 2.0))],
        );
        assert!(
            label(&output, "Live").is_some(),
            "opening a graph must reset pin state"
        );
    }
}

#[test]
fn expired_pin_stays_unavailable_and_other_controls_still_work() {
    let (mut app, target) = fixture();
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
    {
        let mut history = app.cpu_load.history.write().unwrap();
        history
            .configure(20, 30, std::slice::from_ref(&target))
            .unwrap();
        for second in 1..=3 {
            history.push(
                second as f64,
                SystemTime::UNIX_EPOCH + Duration::from_secs(second),
                &[Some(5.0); 20],
                &[Some(false); 20],
                &HashMap::from([(target.clone(), vec![Some(5.0); 20])]),
            );
        }
    }
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let plot = ctx
        .read_response(egui::Id::new("cpu-load-chart"))
        .unwrap()
        .rect;
    let pos = egui::pos2(plot.left() + 3.0, plot.center().y);
    click(&mut app, &ctx, pos);
    let time = app.cpu_load.inspection.pinned.expect("pinned sample");
    // Simulate ring capacity eviction while the reader remains on the old row.
    {
        let mut history = app.cpu_load.history.write().unwrap();
        for second in 4..=40 {
            history.push(
                second as f64,
                SystemTime::UNIX_EPOCH + Duration::from_secs(second),
                &[Some(80.0); 20],
                &[Some(false); 20],
                &HashMap::from([(target.clone(), vec![Some(80.0); 20])]),
            );
        }
    }
    let output = frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(2.0, 2.0))],
    );
    assert_eq!(app.cpu_load.inspection.pinned, Some(time));
    assert!(
        label(&output, "CPU12  —").is_some(),
        "expired time must not become latest or nearest values"
    );
    assert!(label(&output, "Process data unavailable for CPU12.").is_some());
    assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::LineSegment{points,..} if (points[0].x-points[1].x).abs()<0.1 && (points[0].y-points[1].y).abs()>80.0 && plot.contains(points[0]) && plot.contains(points[1]))),"evicted pin must not mark a different recorded sample");
    let output = click(&mut app, &ctx, label(&output, "CPU13  —").unwrap());
    assert!(app.cpu_load.inspection.pinned.is_none());
    assert_eq!(
        app.cpu_load.selected_cpu,
        Some(13),
        "unpin click should keep the ordinary CPU button action"
    );
    assert!(label(&output, "Live").is_some());
    let plot = ctx
        .read_response(egui::Id::new("cpu-load-chart"))
        .unwrap()
        .rect;
    click(&mut app, &ctx, plot.center());
    assert!(app.cpu_load.inspection.pinned.is_some());
    app.cpu_load.configure(false, true);
    assert!(
        app.cpu_load.inspection.pinned.is_none(),
        "turning monitoring off clears inspection state"
    );
}

#[test]
fn cpu_selection_links_the_tile_graph_and_table_even_when_parked_or_outside() {
    for dark in [true, false] {
        let (mut app, target) = fixture();
        let context = egui::Context::default();
        context.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        context.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        context.all_styles_mut(|style| style.animation_time = 0.0);
        let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
        snapshot.parked.fill(Some(true));
        snapshot.process_cpus.get_mut(&target).unwrap()[12] = Some(0.0);
        snapshot.process_cpus.get_mut(&target).unwrap()[13] = Some(25.0);
        *app.cpu_load.published.write().unwrap() = Arc::new(snapshot.clone());
        {
            let mut store = app.cpu_load.history.write().unwrap();
            store
                .configure(20, 30, std::slice::from_ref(&target))
                .unwrap();
            store.push(
                1.0,
                SystemTime::UNIX_EPOCH,
                &snapshot.cpus,
                &snapshot.parked,
                &snapshot.process_cpus,
            );
            store.contributors.push(
                SampleTime {
                    seconds: 1.0,
                    wall: SystemTime::UNIX_EPOCH,
                },
                &[Contribution {
                    run: None,
                    process: Arc::new(os_api::cpu_load::CpuProcess {
                        pid: 1234,
                        instance: 1,
                        name: "chrome.exe".into(),
                    }),
                    rule: Some(Arc::new(target.rule.clone())),
                    cpu: 13,
                    load: 25.0,
                }],
                true,
            );
        }
        let mut accent = egui::Color32::TRANSPARENT;
        let mut check_fg = accent;
        context
            .run_ui(egui::RawInput::default(), |ui| {
                let colors = crate::app::shell::presenters::shared_elements::palette(ui);
                accent = colors.success.fg;
                check_fg = colors.primary.fg;
            })
            .textures_delta
            .clear();
        let mut output = frame(&mut app, &context, vec![]);
        for _ in 0..3 {
            output = frame(&mut app, &context, vec![]);
        }
        let tile = label(&output, "CPU12  0%").unwrap();
        assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.rect.contains(tile) && r.rect.height()<100.0 && r.stroke.width>=2.0 && r.stroke.color==accent)),"selected parked CPU needs a strong accent outline: {:?}, accent={accent:?}, tile={tile:?}, rects={:?}",app.cpu_load.selected_cpu,output.shapes.iter().filter_map(|s| match &s.shape {egui::Shape::Rect(r) if r.rect.contains(tile)=>Some((r.rect,r.stroke)),_=>None}).collect::<Vec<_>>());
        let tile_rect = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.rect.contains(tile)
                        && r.rect.height() < 100.0
                        && r.stroke.width >= 2.0
                        && r.stroke.color == accent =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(output.shapes.iter().filter(|s| matches!(&s.shape,egui::Shape::LineSegment {points,stroke} if stroke.color==check_fg && tile_rect.contains(points[0]) && tile_rect.contains(points[1]))).count(),0,"selection uses the green badge and outline without a painted check");
        assert!(
            label(&output, "No measured process load on CPU12.").is_some(),
            "zero must name the selected CPU"
        );
        let badge = label(&output, "CPU12").unwrap();
        let plot = context
            .read_response(egui::Id::new("cpu-load-chart"))
            .unwrap()
            .rect;
        let table = label(&output, "Processes").unwrap();
        assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.stroke.width>0.0 && r.rect.contains(badge) && r.rect.contains_rect(plot) && r.rect.contains(table))),"CPU badge, plot and process table need one visible owner container");
        let output = click(&mut app, &context, label(&output, "CPU13  25%").unwrap());
        assert_eq!(app.cpu_load.selected_cpu, Some(13));
        assert!(label(&output, "CPU13").is_some());
        assert!(label(&output, "chrome.exe").is_some());
        assert!(label(&output, "No measured process load on CPU12.").is_none());
        let output = click(&mut app, &context, label(&output, "All CPUs").unwrap());
        let output = click(&mut app, &context, label(&output, "CPU0  12%").unwrap());
        assert_eq!(app.cpu_load.selected_cpu, Some(0));
        let tile = label(&output, "CPU0  12%").unwrap();
        assert!(
            output.shapes.iter().any(|s| has_red_border(&s.shape, tile)),
            "outside warning must survive selection"
        );
        assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.rect.contains(tile) && r.rect.height()<100.0 && r.stroke.width>=2.0 && r.stroke.color==accent)),"selected outside CPU must also keep the selection outline");
    }
}

#[test]
fn process_graph_defaults_to_group_cpus_and_keeps_outside_activity_reachable() {
    for dark in [true, false] {
        let (mut app, _) = fixture();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        frame(&mut app, &ctx, vec![]);
        let output = frame(&mut app, &ctx, vec![]);
        assert_eq!(
            tiles(&output),
            (12..20).collect::<Vec<_>>(),
            "process grid must default to its assigned group"
        );
        assert!(
            label(&output, "Outside current mask: 1 CPU").is_some(),
            "filter must not hide observed outside-mask load"
        );
        let all = label(&output, "All CPUs").expect("CPU scope switch");
        let output = click(&mut app, &ctx, all);
        assert_eq!(tiles(&output), (0..20).collect::<Vec<_>>());
        let outside_label = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "CPU0" => Some(t),
                _ => None,
            })
            .unwrap();
        let color = outside_label.galley.job.sections[0].format.color;
        assert!(color.r() > color.g(), "outside activity needs a red label");
        assert!(
            output
                .shapes
                .iter()
                .any(|s| has_red_border(&s.shape, outside_label.pos)),
            "outside activity needs a visible tile border"
        );
        let output = click(&mut app, &ctx, label(&output, "All CPUs").unwrap());
        assert_eq!(tiles(&output), (12..20).collect::<Vec<_>>());
    }
}

#[test]
fn hovering_detail_updates_every_tile_at_one_time_and_leaving_restores_live_values() {
    let (mut app, target) = fixture();
    {
        let mut history = app.cpu_load.history.write().unwrap();
        history.configure(20, 300, &[target.clone()]).unwrap();
        for second in 1..=3 {
            let mut values = vec![Some(0.0); 20];
            values[12] = Some(20.0 + second as f32);
            values[13] = Some(70.0 + second as f32);
            values[14] = None;
            let mut parked = vec![Some(false); 20];
            parked[12] = Some(true);
            history.push(
                second as f64,
                SystemTime::UNIX_EPOCH + Duration::from_secs(second),
                &[Some(90.0); 20],
                &parked,
                &HashMap::from([(target.clone(), values)]),
            );
        }
    }
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let rect = ctx
        .read_response(egui::Id::new("cpu-load-chart"))
        .unwrap()
        .rect;
    let output = frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(
            rect.left() + 3.0,
            rect.center().y,
        ))],
    );
    assert!(
        label(&output, "CPU12  21%").is_some(),
        "selected tile must use historical process load"
    );
    assert!(
        label(&output, "CPU13  71%").is_some(),
        "all tiles must use the same historical row"
    );
    assert!(
        label(&output, "CPU14  —").is_some(),
        "missing data must not become live data or zero"
    );
    assert!(
        label(&output, "Outside current mask: none").is_some(),
        "warning must describe the historical row, not current activity"
    );
    let time =
        os_api::cpu_load::sample_clock_label(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
    assert!(label(&output, &format!("History · {time}")).is_some());
    assert_eq!(
        ctx.read_response(egui::Id::new("cpu-load-chart"))
            .unwrap()
            .rect,
        rect,
        "history inspection must not move the plot under the pointer"
    );
    assert_eq!(
        output.platform_output.num_completed_passes, 1,
        "stable hover must not render the entire modal twice"
    );
    let output = frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(
            rect.right() - 3.0,
            rect.center().y,
        ))],
    );
    assert!(label(&output, "CPU12  23%").is_some());
    assert!(label(&output, "CPU13  73%").is_some());
    assert_eq!(output.platform_output.num_completed_passes, 1);
    let output = frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(Pos2::new(2.0, 2.0))],
    );
    assert!(label(&output, "Live").is_some());
    assert!(label(&output, "CPU12  23%").is_some());
    let output = click(&mut app, &ctx, label(&output, "CPU13  73%").unwrap());
    assert_eq!(
        app.cpu_load.selected_cpu,
        Some(13),
        "clicking a tile chooses its larger detailed graph"
    );
    assert!(
        label(&output, "CPU13").is_some(),
        "detail panel must identify the selected CPU"
    );
    assert!(
        label(&output, "Process data unavailable for CPU13.").is_some(),
        "missing contributor data must name the same selected CPU"
    );
}

#[test]
fn historical_tile_read_under_contention_never_reuses_another_time_or_rule() {
    let (mut app, target) = fixture();
    app.cpu_load
        .history
        .write()
        .unwrap()
        .configure(20, 300, &[target.clone()])
        .unwrap();
    app.cpu_load.history.write().unwrap().push(
        1.0,
        SystemTime::UNIX_EPOCH,
        &[Some(1.0); 20],
        &[],
        &HashMap::from([(target.clone(), vec![Some(42.0); 20])]),
    );
    let time = super::super::history::SampleTime {
        seconds: 1.0,
        wall: SystemTime::UNIX_EPOCH,
    };
    assert_eq!(
        app.cpu_load
            .graph_data(Some(&target), 12, |_| Some(time))
            .sample
            .values[12],
        Some(42.0)
    );
    let history = app.cpu_load.history.clone();
    let _writer = history.write().unwrap();
    let data = app.cpu_load.graph_data(Some(&target), 12, |_| Some(time));
    assert!(data.sample.time.is_none());
    assert!(data.sample.values.iter().all(Option::is_none));
}
