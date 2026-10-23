//! Explicit debug-only app API. Transport and encoding stay off the GUI thread.
use crate::app::runtime::AppState;
use eframe::egui;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{mpsc, Arc};
use std::time::Instant;

#[path = "debug_mcp_transport.rs"]
mod transport;
#[cfg(test)]
use transport::{authorize, MAX_BODY};
#[path = "debug_wait.rs"]
mod wait;
pub(crate) use wait::WaitObservation;
use wait::{WaitCondition, WaitOptions};
const REGISTRY: &str = "debug-mcp-controls";
const ENABLED: &str = "debug-mcp-enabled";
const CAPTURE: &str = "debug-mcp-capture";
const GRAPH: &str = "debug-mcp-graph-state";
const TABLE: &str = "debug-mcp-table-state";

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    Performance {
        #[serde(default = "performance_window")]
        seconds: u32,
    },
    Quality {
        #[serde(default)]
        offset: usize,
        #[serde(default = "page_size")]
        limit: usize,
    },
    Wait(WaitOptions),
    State {
        #[serde(default)]
        include_series: bool,
    },
    OpenGraph {
        group_id: String,
        #[serde(default)]
        rule_id: Option<String>,
    },
    CloseGraph {},
    SelectCpu {
        cpu: usize,
    },
    GraphOptions {
        #[serde(default)]
        all_cpus: Option<bool>,
        #[serde(default)]
        tracked_only: Option<bool>,
        #[serde(default)]
        view: Option<String>,
    },
    Inspect {
        samples_back: usize,
    },
    Live {},
    Monitor {
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(default)]
        history_seconds: Option<u32>,
    },
    Table {
        kind: String,
        #[serde(default)]
        offset: usize,
        #[serde(default = "page_size")]
        limit: usize,
    },
    Input {
        target: String,
        kind: String,
        #[serde(default = "half")]
        x: f32,
        #[serde(default = "half")]
        y: f32,
        #[serde(default)]
        delta: f32,
    },
    Screenshot {},
    CloseApp {},
}
fn performance_window() -> u32 {
    10
}
fn page_size() -> usize {
    50
}
fn half() -> f32 {
    0.5
}

#[derive(Clone)]
struct Control {
    id: String,
    rect: egui::Rect,
    enabled: bool,
    extra: Value,
}
#[derive(Clone, Default)]
struct Registry {
    entries: Vec<Control>,
}
#[derive(Clone, Copy, Default)]
struct Capture {
    graph: bool,
    series: bool,
    contributors: Option<(usize, usize)>,
    processes: Option<(usize, usize)>,
}
impl Capture {
    fn for_command(command: &Command) -> Self {
        match command {
            Command::State { include_series } => Self {
                graph: true,
                series: *include_series,
                ..Self::default()
            },
            Command::Quality { .. } => Self {
                graph: true,
                ..Self::default()
            },
            Command::Table {
                kind,
                offset,
                limit,
            } if *limit > 0 && *limit <= 100 && *offset <= 100_000 => Self {
                contributors: (kind == "contributors").then_some((*offset, *limit)),
                processes: (kind == "processes").then_some((*offset, *limit)),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}
fn capture(ctx: &egui::Context) -> Capture {
    ctx.data(|d| d.get_temp::<Capture>(egui::Id::new(CAPTURE)))
        .unwrap_or_default()
}
pub(crate) fn control(id: impl Into<String>, response: &egui::Response) {
    rect_control(
        &response.ctx,
        id,
        response.interact_rect,
        response.enabled(),
        Value::Null,
    );
}
pub(crate) fn rect_control(
    ctx: &egui::Context,
    id: impl Into<String>,
    rect: egui::Rect,
    enabled: bool,
    extra: Value,
) {
    if !ctx
        .data(|d| d.get_temp::<bool>(egui::Id::new(ENABLED)))
        .unwrap_or(false)
    {
        return;
    }
    let rect = rect.intersect(ctx.content_rect());
    ctx.data_mut(|d| {
        let registry = d.get_temp_mut_or_default::<Registry>(egui::Id::new(REGISTRY));
        if registry.entries.len() < 256 {
            registry.entries.push(Control {
                id: id.into(),
                rect,
                enabled: enabled && rect.is_positive(),
                extra,
            });
        }
    });
}
pub(crate) fn wants_capture(ctx: &egui::Context) -> bool {
    capture(ctx).graph
}
pub(crate) fn wants_series(ctx: &egui::Context) -> bool {
    capture(ctx).series
}
pub(crate) fn table_page(ctx: &egui::Context, kind: &str) -> Option<(usize, usize)> {
    let plan = capture(ctx);
    if kind == "contributors" {
        plan.contributors
    } else {
        plan.processes
    }
}
pub(crate) fn graph_capture(ctx: &egui::Context, value: Value) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(GRAPH), value));
}
pub(crate) fn table_capture(ctx: &egui::Context, value: Value) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(TABLE), value));
}
fn pointer_events(
    control: &Control,
    kind: &str,
    x: f32,
    y: f32,
    delta: f32,
) -> Result<Vec<egui::Event>, String> {
    if !control.enabled || !control.rect.is_positive() {
        return Err("Target is disabled or outside the viewport".into());
    }
    if !x.is_finite()
        || !y.is_finite()
        || !(0.0..=1.0).contains(&x)
        || !(0.0..=1.0).contains(&y)
        || !delta.is_finite()
        || delta.abs() > 2000.0
    {
        return Err("Invalid normalized pointer or wheel delta".into());
    }
    let pos = control.rect.min + control.rect.size() * egui::vec2(x, y);
    let mut events = vec![egui::Event::PointerMoved(pos)];
    match kind {
        "move" => {}
        "click" | "right_click" => {
            for pressed in [true, false] {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: if kind == "right_click" {
                        egui::PointerButton::Secondary
                    } else {
                        egui::PointerButton::Primary
                    },
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        "scroll" => events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -delta),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }),
        _ => return Err("Use move, click, right_click or scroll".into()),
    }
    Ok(events)
}

struct Request {
    id: u64,
    command: Command,
    reply: mpsc::SyncSender<Reply>,
    deadline: Instant,
}
pub(super) enum Reply {
    Json(Value),
    Image(Arc<egui::ColorImage>, u64),
}
struct Pending {
    request: Request,
    after: u64,
    wait: Option<(WaitOptions, Instant)>,
}
pub(crate) struct Bridge {
    requests: mpsc::Receiver<Request>,
    _transport: Option<transport::Server>,
    pending: Option<Pending>,
    screenshot: Option<Request>,
}
impl Bridge {
    pub(crate) fn start(ctx: &egui::Context) -> Result<Option<Self>, String> {
        let Some(server) = transport::Server::start(ctx.clone())? else {
            return Ok(None);
        };
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(ENABLED), true));
        Ok(Some(Self {
            requests: server.0,
            _transport: Some(server.1),
            pending: None,
            screenshot: None,
        }))
    }
    pub(crate) fn raw_input(
        &mut self,
        app: &mut AppState,
        ctx: &egui::Context,
        input: &mut egui::RawInput,
    ) {
        for event in &input.events {
            if let egui::Event::Screenshot {
                user_data, image, ..
            } = event
            {
                if self.screenshot.as_ref().is_some_and(|r| {
                    user_data
                        .data
                        .as_ref()
                        .and_then(|d| d.downcast_ref::<u64>())
                        == Some(&r.id)
                }) {
                    if let Some(request) = self.screenshot.take() {
                        let _ = request
                            .reply
                            .try_send(Reply::Image(image.clone(), ctx.cumulative_frame_nr()));
                    }
                }
            }
        }
        if self
            .screenshot
            .as_ref()
            .is_some_and(|r| r.deadline < Instant::now())
        {
            self.screenshot = None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|r| r.request.deadline < Instant::now())
        {
            self.pending = None;
        }
        if self.pending.is_some() || self.screenshot.is_some() {
            return;
        }
        let Ok(request) = self.requests.try_recv() else {
            return;
        };
        if request.deadline < Instant::now() {
            return;
        }
        let mut wait = None;
        let result = match &request.command {
            Command::Wait(options) => {
                let mut options = *options;
                options.validate().and_then(|()| {
                    if options.condition == WaitCondition::NewSample
                        && options.after_seconds.is_none()
                    {
                        options.after_seconds =
                            crate::app::features::cpu_load::debug_api::wait_observation(
                                app, false,
                            )?
                            .completed_seconds;
                    }
                    wait = Some((options, Instant::now()));
                    super::render_profile::note("debug_wait_started");
                    Ok(())
                })
            }
            Command::Input {
                target,
                kind,
                x,
                y,
                delta,
            } => {
                let found = ctx
                    .data(|d| d.get_temp::<Registry>(egui::Id::new(REGISTRY)))
                    .unwrap_or_default()
                    .entries
                    .into_iter()
                    .find(|c| &c.id == target);
                found
                    .ok_or_else(|| "Target is absent from the current rendered frame".into())
                    .and_then(|c| pointer_events(&c, kind, *x, *y, *delta))
                    .map(|events| input.events.extend(events))
            }
            Command::Screenshot {} => {
                #[cfg(feature = "render-profiling")]
                super::render_profile::note("debug_screenshot_requested");
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                    request.id,
                )));
                self.screenshot = Some(request);
                ctx.request_repaint();
                return;
            }
            Command::Live {} => {
                input.events.push(egui::Event::PointerGone);
                crate::app::features::cpu_load::debug_api::apply(app, &request.command)
            }
            Command::State { .. }
            | Command::Table { .. }
            | Command::Quality { .. }
            | Command::CloseApp {} => Ok(()),
            _ => crate::app::features::cpu_load::debug_api::apply(app, &request.command),
        };
        if let Err(error) = result {
            let _ = request
                .reply
                .try_send(Reply::Json(json!({"ok":false,"error":error})));
            return;
        }
        self.pending = Some(Pending {
            request,
            after: ctx.cumulative_frame_nr() + 1,
            wait,
        });
        ctx.request_repaint();
    }
    pub(crate) fn begin_ui(&self, ctx: &egui::Context) {
        let frame = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Registry>(egui::Id::new(REGISTRY))
                .entries
                .clear();
            d.insert_temp(
                egui::Id::new(CAPTURE),
                self.pending
                    .as_ref()
                    .filter(|p| p.wait.is_none() && frame >= p.after)
                    .map(|p| Capture::for_command(&p.request.command))
                    .unwrap_or_default(),
            );
            d.remove::<Value>(egui::Id::new(GRAPH));
            d.remove::<Value>(egui::Id::new(TABLE));
        });
    }
    pub(crate) fn end_ui(&mut self, app: &mut AppState, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(pending) = &self.pending else {
            return;
        };
        if ctx.cumulative_frame_nr() < pending.after {
            ctx.request_repaint();
            return;
        }
        if let Some((options, started)) = pending.wait {
            let observed = crate::app::features::cpu_load::debug_api::wait_observation(app, true);
            let elapsed = started.elapsed();
            let satisfied = observed.as_ref().is_ok_and(|o| options.satisfied(o));
            if !satisfied && elapsed < std::time::Duration::from_millis(options.timeout_ms) {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            let pending = self.pending.take().unwrap();
            let _=pending.request.reply.try_send(Reply::Json(json!({"ok":true,"data":{"condition":options.condition,"after_seconds":options.after_seconds,"satisfied":satisfied,"timed_out":!satisfied,"elapsed_ms":elapsed.as_millis(),"observed":observed.as_ref().ok(),"observation_error":observed.err()}})));
            return;
        }
        let pending = self.pending.take().unwrap();
        if pending.request.deadline < Instant::now() {
            return;
        }
        let value = match &pending.request.command {
            Command::Quality { offset, limit } => {
                let rendered = ctx.data(|d| d.get_temp::<Value>(egui::Id::new(GRAPH)));
                crate::app::features::cpu_load::debug_api::quality(
                    app,
                    rendered.as_ref(),
                    *offset,
                    *limit,
                )
            }
            Command::Table {
                kind,
                offset,
                limit,
            } => {
                if *limit == 0 || *limit > 100 || *offset > 100_000 {
                    Err("Use limit 1..100 and offset <=100000".into())
                } else {
                    ctx.data(|d| d.get_temp::<Value>(egui::Id::new(TABLE)))
                        .filter(|v| v["kind"] == *kind)
                        .ok_or_else(|| {
                            "Open the requested contributors/processes view first".into()
                        })
                }
            }
            Command::State { .. } => state(app, ctx),
            _ => Ok(json!({"frame":ctx.cumulative_frame_nr(),"applied":true})),
        };
        let close = matches!(pending.request.command, Command::CloseApp {});
        let body = match value {
            Ok(data) => json!({"ok":true,"data":data}),
            Err(error) => json!({"ok":false,"error":error}),
        };
        let _ = pending.request.reply.try_send(Reply::Json(body));
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
fn state(app: &mut AppState, ctx: &egui::Context) -> Result<Value, String> {
    let groups = crate::app::features::cpu_load::debug_api::groups(app)?;
    let registry = ctx
        .data(|d| d.get_temp::<Registry>(egui::Id::new(REGISTRY)))
        .unwrap_or_default();
    let controls:Vec<_>=registry.entries.iter().map(|c|json!({"id":c.id,"rect":[c.rect.min.x,c.rect.min.y,c.rect.max.x,c.rect.max.y],"enabled":c.enabled,"extra":c.extra})).collect();
    Ok(
        json!({"frame":ctx.cumulative_frame_nr(),"viewport":{"width":ctx.content_rect().width(),"height":ctx.content_rect().height(),"pixels_per_point":ctx.pixels_per_point()},"settings":{"open":app.ui.app_settings_open,"cpu_monitoring":app.cpu_load_enabled(),"history_seconds":app.cpu_load_history_seconds(),"error":app.cpu_load.preference_error},"groups":groups,"graph":crate::app::features::cpu_load::debug_api::graph(app,ctx),"rendered_graph":ctx.data(|d|d.get_temp::<Value>(egui::Id::new(GRAPH))),"controls":controls,"tweaks":tweaks_state(app)}),
    )
}

// Read only the worker's published snapshot, never native settings or raw recovery data.
fn tweaks_state(app: &AppState) -> Value {
    use crate::app::features::system_tweaks::presentation::TuningPageSection;
    let snapshot = app.tuning_snapshot();
    let cards: Vec<_> = snapshot.tweak_cards.iter().map(|card| json!({
        "feature":card.feature.as_str(), "title":card.spec.title,
        "owned":card.owned, "ready":card.ready, "has_target":card.target.is_some(),
        "status":card.status.map(crate::app::features::system_tweaks::presentation::status_label),
        "capability":format!("{:?}",card.capability),
        "observed":card.observed.as_ref().map(|o|o.summary.as_str()),
        "baseline":card.baseline,
    })).collect();
    let previous: Vec<_> = snapshot.profiles.iter().flat_map(|p|p.modules.iter().filter(|m|m.owned).map(move |m|json!({"profile":p.id.get(),"feature":m.feature.as_str(),"status":m.status.map(crate::app::features::system_tweaks::presentation::status_label)}))).collect();
    json!({"revision":snapshot.revision,"configuration":format!("{:?}",snapshot.configuration),
        "operation":format!("{:?}",snapshot.operation), "operation_error":snapshot.operation_error.is_some(),
        "error":snapshot.operation_error.as_ref().map(|e|json!({"kind":format!("{:?}",e.kind),"operation":e.operation,"detail":e.detail(),"native":e.native.as_ref().map(|n|json!({"domain":n.domain.as_str(),"code":n.code}))})),
        "ui_error":app.ui.tuning.error,"has_changes":snapshot.has_tweak_changes,
        "section":match app.ui.tuning.selected_section {TuningPageSection::System=>"system",TuningPageSection::Hardware=>"hardware",TuningPageSection::PcChecks=>"checks"},
        "display_confirmation_ms":snapshot.display_confirmation.map(|t|t.saturating_duration_since(Instant::now()).as_millis()),
        "help":app.ui.tuning.tweak_help.as_ref().map(|(title,_)|title),
        "undo_confirmation":app.ui.tuning.confirm_undo_all,"cards":cards,"previous_owned":previous,
        "background":{"enabled":snapshot.background.enabled,"status":snapshot.background.status,
            "preview":snapshot.background.preview_until.is_some(),"active_games":snapshot.background.active_games,
            "changed_targets":snapshot.background.changed_targets,"selected_games":snapshot.background.config.games.len(),
            "cpu_policy":format!("{:?}",snapshot.background.config.cpu_mode),"inspected":snapshot.background.inspected,
            "topology_cpus":snapshot.background.topology.len(),"topology_error":snapshot.background.topology_error,
            "candidates":snapshot.background.candidates.iter().map(|row|json!({"role":row.role,"pid":row.pid,"status":row.status})).collect::<Vec<_>>()},
        "pc_checks":snapshot.pc_checks.iter().map(|c|json!({"id":c.id,"title":c.title,"summary":c.summary})).collect::<Vec<_>>()})
}

#[cfg(test)]
#[path = "debug_mcp_tests.rs"]
mod tests;
