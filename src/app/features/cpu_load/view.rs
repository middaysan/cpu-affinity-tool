use super::history::{Overview, SampleTime};
use super::model::{GraphTarget, RuleTarget};
use super::{
    chart::draw as chart,
    meter::{load_meter, MeterKind, MeterLoad},
};
use crate::app::runtime::AppState;
use crate::app::shared::ids::GroupId;
use crate::app::shell::presenters::shared_elements::{close_button, paint_focus_ring, palette};
use eframe::egui::{self, Align, Layout, RichText};

pub(super) fn percent(value: Option<f32>) -> String {
    value
        .filter(|v| v.is_finite())
        .map(|v| {
            if v > 0.0 && v < 1.0 {
                "<1%".into()
            } else {
                format!("{v:.0}%")
            }
        })
        .unwrap_or_else(|| "—".into())
}

pub(super) fn parking_text(parked: Option<bool>) -> &'static str {
    match parked {
        Some(true) => "Parked",
        Some(false) => "Unparked",
        None => "State —",
    }
}

pub(super) fn measured_percent(value: Option<f32>, partial: bool) -> String {
    let text = percent(value);
    if partial && value.is_some() {
        format!("{text}*")
    } else {
        text
    }
}

fn measured_at(values: &[Option<f32>], partial: &[Option<f32>], cpu: usize) -> (Option<f32>, bool) {
    let full = values.get(cpu).copied().flatten();
    let subtotal = partial.get(cpu).copied().flatten();
    (full.or(subtotal), full.is_none() && subtotal.is_some())
}

struct GridData<'a> {
    values: &'a [Option<f32>],
    partial: &'a [Option<f32>],
    parked: &'a [Option<bool>],
    mask: Option<&'a [usize]>,
    overview: &'a Overview,
    hovered: Option<SampleTime>,
    sample_time: Option<SampleTime>,
    show_app_activity: bool,
}

fn outside_activity(
    values: &[Option<f32>],
    partial: &[Option<f32>],
    mask: Option<&[usize]>,
) -> Option<usize> {
    let mask = mask?;
    let mut count = 0;
    let mut unknown = false;
    for (cpu, _) in values
        .iter()
        .enumerate()
        .filter(|(cpu, _)| !mask.contains(cpu))
    {
        let (value, is_partial) = measured_at(values, partial, cpu);
        unknown |= is_partial;
        match value.filter(|v| v.is_finite()) {
            Some(value) if value > 0.0 => count += 1,
            Some(_) => {}
            None => unknown = true,
        }
    }
    if count > 0 || (!unknown && !values.is_empty()) {
        Some(count)
    } else {
        None
    }
}

const APP_ACTIVITY_COLOR: egui::Color32 = egui::Color32::from_rgb(242, 196, 75);
const APP_ACTIVITY_HELP: &str = "This app has measured CPU work on this logical CPU in the retained history window. The marker includes all retained samples, independently of Live, History or Pinned inspection. It disappears when the last positive sample expires. Missing intervals are not proof of inactivity.";

fn activity_marker(badge: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(
        egui::pos2(badge.right() - 6.5, badge.center().y),
        egui::Vec2::splat(5.0),
    )
}

fn cpu_badge(
    ui: &mut egui::Ui,
    cpu: usize,
    size: f32,
    selected: bool,
    color: Option<egui::Color32>,
    activity: bool,
) -> egui::Response {
    let colors = *palette(ui);
    let badge = egui::Frame::NONE
        .fill(if selected {
            colors.success.fg
        } else {
            colors.primary.fill
        })
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                colors.success.fg
            } else {
                colors.accent.border
            },
        ))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(if size < 10.0 { 4 } else { 6 }, 2))
        .show(ui, |ui| {
            let label = egui::Label::new(
                RichText::new(format!("CPU{cpu}"))
                    .size(size)
                    .strong()
                    .color(if selected {
                        colors.success.fill
                    } else {
                        color.unwrap_or(colors.primary.fg)
                    }),
            )
            .truncate();
            if size < 10.0 {
                // Reserve the marker inside the badge without changing its geometry.
                let height = ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(size)));
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), height),
                    egui::Sense::hover(),
                );
                let mut text_ui =
                    ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(
                            rect.right() - if activity { 8.0 } else { 0.0 },
                            rect.bottom(),
                        ),
                    )));
                response.union(text_ui.add(label.halign(Align::Center)))
            } else {
                ui.add(label)
            }
        })
        .response;
    if activity {
        let marker = activity_marker(badge.rect);
        ui.painter().circle(
            marker.center(),
            2.5,
            APP_ACTIVITY_COLOR,
            egui::Stroke::new(0.5, colors.group),
        );
    }
    badge
}

fn parking_help(time: Option<SampleTime>) -> String {
    let sample = time
        .map(|time| os_api::cpu_load::sample_clock_label(time.wall))
        .unwrap_or_else(|| "time unavailable".into());
    format!("Windows parking state at the displayed sample ({sample}), sampled once per second. \
        Parked means Windows was limiting ordinary scheduling on this CPU at that instant; it does not mean zero work. \
        Load covers the preceding recorded interval, so nonzero load and Parked can coexist. \
        The small graph spans retained history. Unparked CPUs may still be idle. \
        State — means the parking state is unavailable.")
}

fn tile_footer(
    ui: &mut egui::Ui,
    value: Option<f32>,
    partial: bool,
    parked: Option<bool>,
    outside: bool,
    sample_time: Option<SampleTime>,
) {
    let colors = *palette(ui);
    let state_color = if parked == Some(true) {
        colors.warning.fg
    } else {
        colors.text_muted
    };
    let state = ui.painter().layout_no_wrap(
        parking_text(parked).into(),
        egui::FontId::proportional(8.0),
        state_color,
    );
    let load = ui.painter().layout_no_wrap(
        measured_percent(value, partial),
        egui::FontId::proportional(9.0),
        if outside {
            colors.danger.fg
        } else {
            colors.text_primary
        },
    );
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), state.size().y.max(load.size().y)),
        egui::Sense::hover(),
    );
    let width = (state.size().x + 4.0 + load.size().x).min(rect.width());
    let left = rect.center().x - width * 0.5;
    let load_left = left + width - load.size().x;
    let state_clip = egui::Rect::from_min_max(
        egui::pos2(left, rect.top()),
        egui::pos2((load_left - 4.0).max(left), rect.bottom()),
    )
    .intersect(ui.clip_rect());
    ui.painter().with_clip_rect(state_clip).galley(
        egui::pos2(left, rect.center().y - state.size().y * 0.5),
        state,
        state_color,
    );
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(
            egui::pos2(load_left, rect.center().y - load.size().y * 0.5),
            load,
            colors.text_primary,
        );
    response.on_hover_ui(|ui| {
        ui.set_max_width((ui.ctx().content_rect().width() - 24.0).clamp(80.0, 260.0));
        ui.add(egui::Label::new(parking_help(sample_time)).wrap());
    });
}

fn cpu_tile(
    ui: &mut egui::Ui,
    cpu: usize,
    selected: bool,
    width: f32,
    data: &GridData<'_>,
) -> bool {
    let measure_id = egui::Id::new("cpu-tile-measure");
    let key = (
        width.to_bits(),
        ui.pixels_per_point().to_bits(),
        ui.text_style_height(&egui::TextStyle::Small).to_bits(),
        ui.spacing().item_spacing.y.to_bits(),
    );
    type Measure = ((u32, u32, u32, u32), egui::Vec2);
    let cached = ui.ctx().data(|d| d.get_temp::<Measure>(measure_id));
    if let Some((_, size)) = cached.filter(|(old, _)| *old == key) {
        let rect = egui::Rect::from_min_size(ui.cursor().min, size);
        if !ui.is_rect_visible(rect) {
            ui.allocate_exact_size(size, egui::Sense::hover());
            return false;
        }
    }
    crate::render_profile_scope!(GridTile);
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        *data.get_temp_mut_or_default::<usize>(egui::Id::new("cpu-tile-layout-count")) += 1
    });
    let (value, partial) = measured_at(data.values, data.partial, cpu);
    let parked = data.parked.get(cpu).copied().flatten();
    let outside =
        data.mask.is_some_and(|mask| !mask.contains(&cpu)) && value.is_some_and(|v| v > 0.0);
    let colors = *palette(ui);
    let has_activity =
        data.show_app_activity && data.overview.activity.get(cpu).copied().unwrap_or(false);
    let border = if selected {
        colors.success.fg
    } else if outside {
        colors.danger.fg
    } else {
        colors.border_strong
    };
    let mut activity_hit = None;
    let frame = egui::Frame::NONE
        .fill(if selected {
            colors.success.fill
        } else {
            colors.inset
        })
        .stroke(egui::Stroke::NONE)
        .corner_radius(5.0)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.spacing_mut().interact_size.y = 0.0;
                ui.set_width((width - 8.0).max(20.0));
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    let badge_width = ui.available_width() * 0.7;
                    ui.allocate_ui_with_layout(
                        egui::vec2(badge_width, 0.0),
                        Layout::top_down(Align::Center),
                        |ui| {
                            ui.set_width(badge_width);
                            let badge = cpu_badge(
                                ui,
                                cpu,
                                9.0,
                                selected,
                                outside.then_some(colors.danger.fg),
                                has_activity,
                            );
                            if has_activity {
                                activity_hit = Some(
                                    activity_marker(badge.rect)
                                        .expand(2.0)
                                        .intersect(badge.rect),
                                );
                            }
                        },
                    );
                });
                super::mini_chart::draw(ui, data.overview, cpu, data.hovered);
                tile_footer(ui, value, partial, parked, outside, data.sample_time);
            });
        });
    let rect = frame.response.rect;
    if cached != Some((key, rect.size())) {
        ui.ctx()
            .data_mut(|d| d.insert_temp(measure_id, (key, rect.size())));
    }
    // Draw selection inside identical tile geometry, so changing the selected
    // row never moves the graph or changes another tile's hit target.
    ui.painter().rect_stroke(
        rect,
        5.0,
        egui::Stroke::new(if selected { 2.0 } else { 1.0 }, border),
        egui::StrokeKind::Inside,
    );
    if selected {
        if outside {
            ui.painter().rect_stroke(
                rect.shrink(2.0),
                3.0,
                egui::Stroke::new(2.0, colors.danger.fg),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                rect.left_top() + egui::vec2(1.0, 6.0),
                rect.left_bottom() + egui::vec2(4.0, -6.0),
            ),
            2.0,
            colors.success.fg,
        );
    }
    let response = ui.interact(rect, ui.id().with(("cpu-tile", cpu)), egui::Sense::click());
    super::export_ui::menu(ui, &response, rect, cpu);
    crate::debug_mcp_control!(format!("cpu.{cpu}"), response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            selected,
            format!(
                "CPU{cpu}, {}, {}{}",
                measured_percent(value, partial),
                parking_text(parked),
                if has_activity {
                    ", app activity recorded in retained history"
                } else {
                    ""
                }
            ),
        )
    });
    paint_focus_ring(ui, &response);
    let over_activity = activity_hit.is_some_and(|rect| ui.rect_contains_pointer(rect));
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    if let Some(rect) = activity_hit {
        crate::app::features::diagnostics::debug_mcp::rect_control(
            ui.ctx(),
            format!("cpu.activity.{cpu}"),
            rect,
            true,
            serde_json::Value::Null,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(
        if over_activity {APP_ACTIVITY_HELP}
        else if outside {"Activity outside the CURRENT group mask. Click for this CPU's detailed graph. Historical activity is not proof of an earlier mask violation."}
        else if data.show_app_activity {"Click for this CPU's detailed graph. A yellow dot inside the CPU badge marks measured app work in retained history. Mini graphs show a compressed min/max overview; gaps mean missing data."}
        else {"Click for this CPU's detailed graph. Mini graphs show a compressed min/max overview; gaps mean missing data."}
    ).clicked()
}

pub(crate) fn group_indicator(
    app: &mut AppState,
    ui: &mut egui::Ui,
    id: &GroupId,
    name: &str,
    cpus: &[usize],
) {
    if !app.cpu_load_enabled() {
        return;
    }
    let snapshot = app.cpu_load.snapshot();
    let (value, partial) = app.cpu_load.tracked_group_load(cpus);
    let color = palette(ui).accent.fg;
    let response = load_meter(ui, MeterLoad { value, partial }, color, true, MeterKind::Group).on_hover_ui(|ui| {
        ui.label(format!(
            "Open CPU graphs.\nTracked apps: {} of this group's CPU capacity (latest finalized graph interval).\nTracked scope covers unique processes across your saved groups on these CPUs. * means partial measured load; — means unavailable. Use All system processes in the graphs for other programs.",
            measured_percent(value, partial),
        ));
    });
    crate::debug_mcp_control!(format!("meter.group.{}", id.0), response);
    if let Some(error) = &snapshot.error {
        response.clone().on_hover_text(error);
    }
    if let Some(error) = snapshot
        .collector_error
        .as_ref()
        .or(snapshot.detail_error.as_ref())
    {
        response.clone().on_hover_text(error);
    }
    if response.clicked() {
        app.cpu_load.open(GraphTarget::Group {
            id: id.clone(),
            name: name.into(),
            cpus: cpus.to_vec(),
        });
    }
}

pub(crate) fn process_indicator(
    app: &mut AppState,
    ui: &mut egui::Ui,
    target: RuleTarget,
    name: &str,
    running: bool,
    color: egui::Color32,
    status: &str,
) {
    if !app.cpu_load_enabled() {
        return;
    }
    let snapshot = app.cpu_load.snapshot();
    let value = if running {
        snapshot.processes.get(&target).copied().flatten()
    } else {
        None
    };
    let explanation = if running {
        format!("Open CPU graphs.\n{status}\nCPU load across this app's tracked processes, relative to the whole CPU. — means waiting or unavailable.")
    } else {
        format!("{status}\nStart this app to view its CPU graphs.")
    };
    let response = load_meter(ui, value.into(), color, running, MeterKind::Application)
        .on_hover_text(explanation);
    crate::debug_mcp_control!(format!("meter.rule.{}", target.rule.0), response);
    if response.clicked() {
        app.cpu_load.open(GraphTarget::Process {
            target,
            name: name.into(),
        });
    }
}

fn current_group_cpus(app: &AppState, id: &GroupId) -> Option<Vec<usize>> {
    let index = app.rules.group_index_for_id(id)?;
    let state = app.persistent_state.try_read().ok()?;
    state.groups.get(index).map(|group| group.cores.clone())
}

fn contributor_table(
    ui: &mut egui::Ui,
    view: &super::contributors::ContributorView,
    cpu: usize,
    historical: bool,
) {
    crate::render_profile_scope!(Contributors);
    // Stable height prevents modal centering from moving the hovered graph
    // whenever the historical process count or availability changes.
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), contributor_height(ui.ctx())),
        Layout::top_down(Align::Min),
        |ui| {
            ui.set_min_height(contributor_height(ui.ctx()));
            contributor_table_content(ui, view, cpu, historical);
        },
    );
}
fn contributor_table_content(
    ui: &mut egui::Ui,
    view: &super::contributors::ContributorView,
    cpu: usize,
    historical: bool,
) {
    use crate::app::shell::presenters::shared_elements::{
        compact_table_spacing, table_row_separator, COMPACT_TABLE_ROW_HEIGHT,
    };
    compact_table_spacing(ui);
    let colors = *palette(ui);
    use crate::app::shell::presenters::process_table::{self as table, Column, Sort};
    let cell = |ui: &mut egui::Ui, width: f32, align: Align, text: RichText| {
        table::cell(ui, width, align == Align::Max, text)
    };
    let order_id = egui::Id::new("contributor-table-sort");
    let mut order = ui
        .ctx()
        .data(|d| d.get_temp::<table::Order>(order_id))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(RichText::new(if view.partial {"Processes · partial"} else {"Processes"}).small().strong().color(if view.partial {colors.warning.fg} else {colors.text_secondary})).on_hover_text(
            "Scheduler time on the selected logical CPU; 100% means one fully busy logical CPU. The graph and table use the same original interval. Group charts follow the all/tracked filter; app charts include that app's exact tracked instances. Partial means some work could not be attributed. Current whole-machine process percentages use a different counter. Bounded RAM history can become unavailable earlier under exceptional process churn.");
        let mode=if historical {"History"} else {"Live"};
        let time=view.time.map(|time| os_api::cpu_load::sample_clock_label(time.wall)).unwrap_or_else(|| "—".into());
        ui.label(RichText::new(format!("{mode} · {time}")).size(9.0).color(if historical {colors.accent.fg} else {colors.text_secondary}));
    });
    if !view.available {
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        super::debug_api::record_contributors(ui.ctx(), view, cpu, &[]);
        ui.label(
            RichText::new(format!("Process data unavailable for CPU{cpu}."))
                .small()
                .weak(),
        );
        return;
    }
    let width = ui.available_width();
    let pid_width = 52.0;
    let load_width = 48.0;
    let name_width = (width - pid_width - load_width - ui.spacing().item_spacing.x * 2.0).max(40.0);
    ui.horizontal(|ui| {
        let cpu_title = format!("CPU{cpu}");
        for (i, (title, column, width)) in [
            ("Process", Column::Name, name_width),
            ("PID", Column::Pid, pid_width),
            (&cpu_title, Column::Load, load_width),
        ]
        .into_iter()
        .enumerate()
        {
            let response = table::heading(ui, width, title, column, &mut order);
            #[cfg(all(debug_assertions, feature = "debug-mcp"))]
            crate::app::features::diagnostics::debug_mcp::control(
                format!("sort.contributors.{i}"),
                &response,
            );
            #[cfg(not(all(debug_assertions, feature = "debug-mcp")))]
            let _ = (i, response);
        }
    });
    let indices = ui.ctx().data_mut(|d| {
        let cached = d.get_temp_mut_or_default::<table::Order>(order_id.with("indices"));
        cached.sort = order.sort;
        let sort = order.sort.unwrap_or(Sort {
            column: Column::Load,
            descending: true,
        });
        cached.prepare(
            view.revision,
            view.rows.len(),
            |_| true,
            |a, b| {
                let (a, b) = (&view.rows[a], &view.rows[b]);
                let primary = match sort.column {
                    Column::Name => sort.compare(table::name_cmp(&a.process.name, &b.process.name)),
                    Column::Pid => sort.compare(a.process.pid.cmp(&b.process.pid)),
                    _ => sort.load(Some(a.load), Some(b.load)),
                };
                primary
                    .then(a.process.pid.cmp(&b.process.pid))
                    .then(a.process.instance.cmp(&b.process.instance))
            },
        );
        cached.rows.clone()
    });
    ui.ctx().data_mut(|d| d.insert_temp(order_id, order));
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    super::debug_api::record_contributors(ui.ctx(), view, cpu, &indices);
    if view.rows.is_empty() {
        ui.label(
            RichText::new(format!("No measured process load on CPU{cpu}."))
                .small()
                .weak(),
        ).on_hover_text("Only processes with measured work on this selected CPU are listed. Other processes can be idle or working on other CPUs. Hover a point on the large graph to inspect that same recorded interval.");
    } else {
        let scroll = egui::ScrollArea::vertical()
            .id_salt("cpu-contributors")
            .min_scrolled_height(contributor_height(ui.ctx()) - 56.0)
            .max_height(contributor_height(ui.ctx()) - 56.0)
            .auto_shrink([false, false])
            .show_rows(
                ui,
                COMPACT_TABLE_ROW_HEIGHT,
                view.rows.len(),
                |ui, range| {
                    for index in range {
                        let row = &view.rows[indices[index]];
                        let row_response = ui.horizontal(|ui| {
                            cell(
                                ui,
                                name_width,
                                Align::Min,
                                RichText::new(row.process.name.as_ref()).small(),
                            )
                            .on_hover_text(row.process.name.as_ref());
                            cell(
                                ui,
                                pid_width,
                                Align::Max,
                                RichText::new(row.pid_text()).small(),
                            );
                            cell(
                                ui,
                                load_width,
                                Align::Max,
                                RichText::new(row.load_text()).small(),
                            );
                        });
                        table_row_separator(ui, row_response.response.rect);
                    }
                },
            );
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        crate::app::features::diagnostics::debug_mcp::rect_control(
            ui.ctx(),
            "table.contributors",
            scroll.inner_rect.intersect(ui.clip_rect()),
            true,
            serde_json::json!({"offset":scroll.state.offset.y,"rows":view.rows.len(),"content_height":scroll.content_size.y}),
        );
        #[cfg(not(all(debug_assertions, feature = "debug-mcp")))]
        let _ = scroll;
    }
}

fn modal_inner_height(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().height() * 0.8
        - egui::Frame::popup(&ctx.global_style())
            .total_margin()
            .sum()
            .y)
        .max(180.0)
}

fn contributor_height(ctx: &egui::Context) -> f32 {
    (modal_inner_height(ctx) * 0.33).clamp(160.0, 280.0)
}

fn filter_chip(ui: &mut egui::Ui, selected: &mut bool, label: &str) -> egui::Response {
    let colors = *palette(ui);
    let response = ui.add(
        egui::Button::new(RichText::new(label).size(9.0).color(if *selected {
            colors.success.fill
        } else {
            colors.text_secondary
        }))
        .selected(*selected)
        .fill(if *selected {
            colors.success.fg
        } else {
            colors.inset
        })
        .stroke(egui::Stroke::new(
            if *selected { 1.5 } else { 1.0 },
            if *selected {
                colors.success.fg
            } else {
                colors.border_strong
            },
        ))
        .corner_radius(4.0)
        .min_size(egui::vec2(0.0, 17.0)),
    );
    paint_focus_ring(ui, &response);
    if response.clicked() {
        *selected = !*selected;
    }
    response
}

fn process_scope(app: &AppState, ctx: &egui::Context, target: &RuleTarget) -> std::sync::Arc<str> {
    type Scope = (RuleTarget, std::sync::Arc<str>);
    let id = egui::Id::new("app-process-scope");
    if let Some((_, text)) = ctx
        .data(|d| d.get_temp::<Scope>(id))
        .filter(|(key, _)| key == target)
    {
        return text;
    }
    let value = (|| {
        let g = app.rules.group_index_for_id(&target.group)?;
        let r = app.rules.rule_index_for_id(g, &target.rule)?;
        let state = app.persistent_state.try_read().ok()?;
        let app = state.groups.get(g)?.programs.get(r)?;
        Some(scope_text(
            &app.additional_processes,
            app.manage_descendants,
        ))
    })();
    let Some(value) = value else {
        return "Tracked process names unavailable".into();
    };
    let text: std::sync::Arc<str> = value.into();
    ctx.data_mut(|d| d.insert_temp(id, (target.clone(), text.clone())));
    text
}
fn scope_text(names: &[String], descendants: bool) -> String {
    let names = if names.is_empty() {
        "managed launches".into()
    } else {
        names.join(", ")
    };
    format!(
        "Tracks: {names}{}",
        if descendants { " + descendants" } else { "" }
    )
}
fn scope_label(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(9.0)
                .color(palette(ui).text_secondary),
        )
        .truncate(),
    )
    .on_hover_text(text);
}

fn modal_header(ui: &mut egui::Ui, name: &str, application: bool, system: bool) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width() - 60.0, 20.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{name} · {}",
                            if system {
                                "System CPU"
                            } else if application {
                                "Application CPU"
                            } else {
                                "Group CPUs"
                            }
                        ))
                        .strong(),
                    )
                    .truncate(),
                )
            },
        )
        .inner
        .on_hover_text(name);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let response = close_button(ui);
            crate::debug_mcp_control!("graph.close", response);
            close = response.clicked();
        });
    });
    close
}
fn view_switch(ui: &mut egui::Ui, processes: &mut bool) {
    let colors = *palette(ui);
    egui::Frame::NONE
        .fill(colors.group)
        .stroke(egui::Stroke::new(1.0, colors.border_strong))
        .corner_radius(6.0)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                let width = ui.available_width() / 2.0;
                for (label, value) in [("Graphs", false), ("Processes", true)] {
                    let button = egui::Button::new(RichText::new(label).small())
                        .selected(*processes == value)
                        .corner_radius(4.0);
                    let response = ui.add_sized([width, 23.0], button);
                    crate::debug_mcp_control!(
                        if value {
                            "view.processes"
                        } else {
                            "view.graphs"
                        },
                        response
                    );
                    if response.clicked() {
                        *processes = value;
                    }
                }
            });
        });
}

pub(crate) fn draw_modal(app: &mut AppState, ctx: &egui::Context) {
    let Some(target) = app.cpu_load.graph.clone() else {
        super::chart::clear(ctx);
        ctx.data_mut(|d| {
            d.remove::<super::process_list::CurrentOrder>(
                egui::Id::new("current-process-sort").with("cache"),
            );
            d.remove::<(RuleTarget, std::sync::Arc<str>)>(egui::Id::new("app-process-scope"));
        });
        ctx.data_mut(|d| {
            d.remove::<crate::app::shell::presenters::process_table::Order>(
                egui::Id::new("contributor-table-sort").with("indices"),
            )
        });
        return;
    };
    crate::render_profile_scope!(CpuModal);
    let app_scope = if let GraphTarget::Process { target, .. } = &target {
        Some(process_scope(app, ctx, target))
    } else {
        None
    };
    let snapshot = app.cpu_load.snapshot();
    let mut tracked_only = app.cpu_load.tracked_only();
    let system = matches!(target, GraphTarget::System);
    let (name, live_values, error, rule, mask): (&str, _, _, _, _) = match &target {
        GraphTarget::System => (
            "All CPUs",
            if tracked_only {
                snapshot.tracked_cpus.as_slice()
            } else {
                snapshot.cpus.as_slice()
            },
            if tracked_only {
                snapshot.detail_error.as_ref().or(snapshot.error.as_ref())
            } else {
                snapshot
                    .collector_error
                    .as_ref()
                    .or(snapshot.error.as_ref())
            },
            None,
            None,
        ),
        GraphTarget::Group { id, name, cpus } => (
            name,
            if tracked_only {
                snapshot.tracked_cpus.as_slice()
            } else {
                snapshot.cpus.as_slice()
            },
            if tracked_only {
                snapshot.detail_error.as_ref().or(snapshot.error.as_ref())
            } else {
                snapshot
                    .collector_error
                    .as_ref()
                    .or(snapshot.error.as_ref())
            },
            None,
            Some(current_group_cpus(app, id).unwrap_or_else(|| cpus.clone())),
        ),
        GraphTarget::Process { name, target } => {
            let values = snapshot
                .process_cpus
                .get(target)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            (
                name,
                values,
                snapshot.detail_error.as_ref(),
                Some(target),
                current_group_cpus(app, &target.group),
            )
        }
    };
    let process = rule.is_some();
    let live_partial: &[Option<f32>] = if let Some(rule) = rule {
        snapshot
            .process_partial_cpus
            .get(rule)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    } else if tracked_only {
        &snapshot.tracked_partial_cpus
    } else {
        &[]
    };
    let count = snapshot
        .cpus
        .len()
        .max(live_values.len())
        .max(app.ui.group_form.core_selection.len());
    let mut show_all = system || app.cpu_load.show_all_cpus;
    let mut cpus: Vec<_> = if show_all {
        (0..count).collect()
    } else {
        mask.clone().unwrap_or_default()
    };
    cpus.retain(|&cpu| cpu < count);
    cpus.sort_unstable();
    cpus.dedup();
    let mut selected = app
        .cpu_load
        .selected_cpu
        .filter(|cpu| cpus.contains(cpu))
        .or_else(|| cpus.first().copied());
    let mut close = false;
    let series_cpu = selected;
    let mut inspection = app.cpu_load.inspection;
    let displaying_processes = app.cpu_load.show_processes;
    let export_notice = app.cpu_load.export_ui.notice.clone();
    let mut show_processes = displaying_processes;
    let process_rows = displaying_processes.then(|| {
        if process {
            snapshot.live_processes.clone()
        } else {
            app.cpu_load.group_processes(&cpus)
        }
    });
    let data = if displaying_processes {
        None
    } else {
        Some(
            app.cpu_load
                .graph_data(rule, selected.unwrap_or(usize::MAX), |series| {
                    inspection.select(ctx, series)
                }),
        )
    };
    let mut actual_hover = None;
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    if let Some(data) = &data {
        super::debug_api::record_graph(ctx, data, &cpus, selected);
    }
    let response = crate::app::shell::presenters::shared_elements::modal(egui::Id::new("cpu-load-modal")).show(ctx, |ui| {
        let modal_top = ui.cursor().top();
        ui.set_width((ctx.content_rect().width() - 36.0).clamp(200.0, 356.0));
        ui.set_height(modal_inner_height(ctx));
        close = modal_header(ui, name, process, system);
        let subtitle = if tracked_only {
            if show_all {"Tracked apps on all logical CPUs"} else {"Tracked apps on this group's logical CPUs"}
        } else if show_all {"All system processes on all logical CPUs"}
        else {"All system processes on this group's logical CPUs"};
        if let Some(scope) = &app_scope {scope_label(ui, scope);}
        else {ui.label(RichText::new(subtitle).small().weak());}
        ui.horizontal(|ui| {
            ui.set_min_height(18.0);
            ui.spacing_mut().item_spacing.x = 5.0;
            if !system && (!process || !displaying_processes) {
                let response = filter_chip(ui, &mut show_all, "All CPUs").on_hover_text(
                    "Off: use this group's selected CPUs. On: use every logical CPU. Changes the graph CPUs and the scope of the group's process table."
                );
                crate::debug_mcp_control!("graph.all_cpus", response);
            }
            if !process {
                let mut all_processes = !tracked_only;
                let response = filter_chip(ui, &mut all_processes, "All system processes").on_hover_text(
                    "Off: only apps tracked by CPU Affinity Tool across your saved groups. On: include other Windows processes too. Both views use only the displayed CPUs. This changes what is shown; it does not apply CPU settings to other apps."
                );
                crate::debug_mcp_control!("graph.tracked_only", response);
                tracked_only = !all_processes;
            }
            let mode = if let Some(data) = &data {
                data.hovered.map(|time| format!("{} · {}", if inspection.pinned.is_some() {"Pinned"} else {"History"}, os_api::cpu_load::sample_clock_label(time.wall)))
                    .unwrap_or_else(|| if data.pending {
                        data.sample.time.map(|time| format!("Live · {}", os_api::cpu_load::sample_clock_label(time.wall))).unwrap_or_else(|| "Waiting for events…".into())
                    } else {"Live".into()})
            } else {
                process_rows.as_ref().and_then(|rows| rows.time).map(|time| format!("Live · {}", os_api::cpu_load::sample_clock_label(time))).unwrap_or_else(|| "Waiting for data…".into())
            };
            let historical = data.as_ref().is_some_and(|data| data.hovered.is_some());
            let color = if historical {palette(ui).accent.fg} else {palette(ui).text_secondary};
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(mode).size(9.0).color(color).strong()).truncate())
                    .on_hover_text("Graphs: hover to inspect one original interval; click the plot to pin it. Processes: the latest available interval. Group graphs and process rows share the same original sample and filters. Recording continues in both views.");
            });
        });
        if let Some(error) = error {
            ui.add(egui::Label::new(RichText::new(error).small().color(ui.visuals().warn_fg_color)).wrap());
        }
        if let Some(notice) = &export_notice {
            ui.add(egui::Label::new(RichText::new(notice).small().weak()).wrap());
        }
        if let Some(data) = &data {
    let recorded = data.hovered.is_some() || !data.series.points.is_empty() || !data.available;
    let values = if recorded {
        data.sample.values.as_slice()
    } else {
        live_values
    };
    let parked = if recorded {
        data.sample.parked.as_slice()
    } else {
        snapshot.parked.as_slice()
    };
    let partial = if recorded {
        data.sample.partial.as_slice()
    } else {
        live_partial
    };
    let grid = GridData {
        values,
        partial,
        parked,
        mask: if process { mask.as_deref() } else { None },
        overview: data.overview,
        hovered: data.hovered,
        sample_time: if recorded { data.sample.time } else { None },
        show_app_activity: process,
    };
            if process {
                let outside = outside_activity(values, partial, mask.as_deref());
                let message = match outside {Some(0) => "Outside current mask: none".into(), Some(n) => format!("Outside current mask: {n} CPU{}", if n == 1 {""} else {"s"}), None => "Outside current mask: unavailable".into()};
                ui.horizontal(|ui| {
                    ui.set_min_height(18.0);
                    let color = if outside.is_some_and(|n| n > 0) {palette(ui).danger.fg} else {palette(ui).text_muted};
                    ui.label(RichText::new(message).size(9.0).color(color)).on_hover_text("Compared with the CURRENT saved group mask. Earlier masks are not recorded. Red tiles mark observed activity outside that mask, not proof of a past enforcement failure.");
                    if !show_all && outside.is_some_and(|n| n > 0) && ui.small_button("Show").clicked() {show_all = true;}
                });
            }
            if error.is_none() && live_values.is_empty() {ui.label(RichText::new("Collecting the first sample…").small().weak());}
            let body_height = (modal_inner_height(ctx) - (ui.cursor().top() - modal_top) - 70.0).max(100.0);
        let body_scroll = egui::ScrollArea::vertical()
            .min_scrolled_height(body_height)
            .max_height(body_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let colors=*palette(ui);
                egui::Frame::NONE
                    .stroke(egui::Stroke::new(1.0,if data.hovered.is_some(){colors.selected.border}else{egui::Color32::TRANSPARENT}))
                    .fill(if data.hovered.is_some(){colors.selected.fill.gamma_multiply(0.3)}else{egui::Color32::TRANSPARENT})
                    .corner_radius(5.0).inner_margin(3).show(ui,|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                let width = ((ui.available_width() - 24.0) / 4.0).max(42.0);
                for row in cpus.chunks(4) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for &cpu in row {
                            if cpu_tile(ui,cpu,selected==Some(cpu),width,&grid) {
                                selected = Some(cpu);
                            }
                        }
                    });
                }
                if cpus.is_empty() {ui.label("Group CPUs unavailable; select All CPUs.");}
                });
                ui.add_space((8.0-ui.spacing().item_spacing.y).max(0.0));
                if let Some(cpu) = series_cpu {
                    let (value,is_partial) = measured_at(values,partial,cpu);
                    let value = measured_percent(value,is_partial);
                    let parked = parked.get(cpu).copied().flatten();
                    egui::Frame::NONE.fill(colors.group)
                        .stroke(egui::Stroke::new(1.0,colors.success.fg))
                        .corner_radius(6.0).inner_margin(8).show(ui,|ui| {
                            ui.spacing_mut().item_spacing.y=4.0;
                            let header=ui.horizontal(|ui| {
                                cpu_badge(ui,cpu,11.0,true,None,false).on_hover_text("Selected logical CPU. Its load history and process contributions share this panel. Click another CPU tile above to change it.");
                                ui.label(RichText::new(value).size(11.0).strong());
                                if is_partial {ui.label(RichText::new("Partial").size(9.0).color(colors.warning.fg)).on_hover_text("Measured subtotal from known process samples. Some tracked instances are unavailable, so the full total is unknown. Dashed graph segments preserve the measured work.");}
                                ui.label(RichText::new(parking_text(parked)).size(9.0).color(if parked==Some(true){colors.warning.fg}else{colors.text_muted}));
                            });
                            let header_rect=header.response.rect;
                            ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(header_rect.left()-6.0,header_rect.top()),egui::pos2(header_rect.left()-3.0,header_rect.bottom())),2.0,colors.success.fg);
                            actual_hover=chart(ui,data.series,cpu,data.hovered,inspection.pinned.is_some());
                            contributor_table(ui,data.contributors,cpu,data.hovered.is_some());
                        });
                }
            });
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        crate::app::features::diagnostics::debug_mcp::rect_control(ctx, "graph.body", body_scroll.inner_rect, true, serde_json::json!({"offset":body_scroll.state.offset.y,"content_height":body_scroll.content_size.y}));
        #[cfg(not(all(debug_assertions, feature = "debug-mcp")))]
        let _ = body_scroll;
        } else if let Some(rows) = &process_rows {
            let body_height = (modal_inner_height(ctx) - (ui.cursor().top() - modal_top) - 70.0).max(100.0);
            super::process_list::draw(ui, rows, rule, body_height);
        }
        ui.add_space(5.0);
        ui.add(egui::Label::new(RichText::new("History records while CPU monitoring is enabled, including with this graph closed.").small().weak()).wrap());
        view_switch(ui, &mut show_processes);
    });
    let graph_changed = data
        .as_ref()
        .is_some_and(|data| actual_hover != data.hovered);
    if graph_changed
        || selected != app.cpu_load.selected_cpu
        || show_all != app.cpu_load.show_all_cpus
        || tracked_only != app.cpu_load.tracked_only()
        || show_processes != displaying_processes
    {
        ctx.request_discard("Synchronize CPU view and inspection scope");
    }
    app.cpu_load.selected_cpu = selected;
    app.cpu_load.inspection = if show_processes {
        Default::default()
    } else {
        inspection
    };
    app.cpu_load.show_all_cpus = show_all;
    app.cpu_load.show_processes = show_processes;
    app.cpu_load.set_tracked_only(tracked_only);
    if show_processes {
        super::chart::clear(ctx);
    }
    if close || response.should_close() {
        app.cpu_load.close();
        super::chart::clear(ctx);
    }
    app.cpu_load.export_actions(ctx);
}

#[cfg(test)]
mod tests {
    use super::super::history::Series as History;
    use super::*;
    use crate::app::models::AppStateStorage;
    use std::sync::{Arc, RwLock};

    #[test]
    fn parking_help_uses_original_sample_time_and_explains_interval_load() {
        let time = SampleTime {
            seconds: 7.0,
            wall: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
        };
        let help = parking_help(Some(time));
        assert!(help.contains(&os_api::cpu_load::sample_clock_label(time.wall)));
        assert!(help.contains("displayed sample"));
        assert!(help.contains("interval"));
        assert!(help.contains("nonzero load"));
        assert!(!help.contains("last poll"));
        assert!(parking_help(None).contains("time unavailable"));
    }

    #[test]
    fn parked_tile_keeps_neutral_body_and_nonzero_load_in_both_themes() {
        for dark in [true, false] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let overview = Overview::default();
            let data = GridData {
                values: &[Some(0.2)],
                partial: &[],
                parked: &[Some(true)],
                mask: None,
                overview: &overview,
                hovered: None,
                sample_time: None,
                show_app_activity: false,
            };
            let mut tile = egui::Rect::NOTHING;
            let mut colors = None;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                colors = Some(*palette(ui));
                cpu_tile(ui, 0, false, 83.0, &data);
                tile = ctx
                    .read_response(ui.id().with(("cpu-tile", 0usize)))
                    .unwrap()
                    .rect;
            });
            output.textures_delta.clear();
            let colors = colors.unwrap();
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|s| matches!(&s.shape, egui::Shape::Rect(r)
                if r.rect == tile && r.fill == colors.inset)),
                "Parked must not tint the tile body"
            );
            for (text, color) in [("Parked", colors.warning.fg), ("<1%", colors.text_primary)] {
                assert!(
                    output
                        .shapes
                        .iter()
                        .any(|s| matches!(&s.shape, egui::Shape::Text(t)
                    if t.galley.text() == text && t.galley.job.sections[0].format.color == color)),
                    "keep {text} with its own color and the measured load"
                );
            }
        }
    }

    #[test]
    fn cpu_tile_places_activity_in_badge_and_keeps_plot_and_footer_tight() {
        for dark in [true, false] {
            for scale in [1.0, 1.75] {
                let ctx = egui::Context::default();
                ctx.set_fonts(
                    crate::app::shell::presenters::shared_elements::ui_font_definitions(),
                );
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                ctx.set_pixels_per_point(scale);
                let overview = Overview {
                    activity: vec![true],
                    ..Default::default()
                };
                let data = GridData {
                    values: &[Some(0.2)],
                    partial: &[],
                    parked: &[Some(false)],
                    mask: None,
                    overview: &overview,
                    hovered: None,
                    sample_time: None,
                    show_app_activity: true,
                };
                let mut tile = egui::Rect::NOTHING;
                let mut output = egui::FullOutput::default();
                for _ in 0..3 {
                    output = ctx.run_ui(Default::default(), |ui| {
                        ui.spacing_mut().interact_size.y = 28.0;
                        cpu_tile(ui, 0, true, 83.0, &data);
                        tile = ctx
                            .read_response(ui.id().with(("cpu-tile", 0usize)))
                            .unwrap()
                            .rect;
                    });
                    output.textures_delta.clear();
                }
                let text = |name: &str| {
                    output
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(t) if t.galley.text() == name => {
                                Some(t.galley.rect.translate(t.pos.to_vec2()))
                            }
                            _ => None,
                        })
                        .unwrap()
                };
                let state = text("Unparked");
                let load = text("<1%");
                assert!(
                    tile.bottom() - state.bottom().max(load.bottom()) <= 4.0,
                    "no inherited button padding below footer: {tile:?}, {state:?}, {load:?}"
                );
                assert!((state.center().y - load.center().y).abs() < 0.6);
                assert!(
                    (egui::Rect::from_min_max(state.min, load.max).center().x - tile.center().x)
                        .abs()
                        < 0.6,
                    "footer pair must be centered"
                );
                assert!(tile.shrink(2.0).contains_rect(load));
                let baseline = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::LineSegment { points, .. }
                            if (points[0].y - points[1].y).abs() < 0.1
                                && (points[1].x - points[0].x).abs() > 40.0
                                && tile.contains(points[0]) =>
                        {
                            Some(points[0].y)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(
                    state.top().min(load.top()) - baseline <= 3.0,
                    "mini graph must meet its footer closely"
                );
                let dot = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Circle(c)
                            if c.radius == 2.5
                                && c.fill == egui::Color32::from_rgb(242, 196, 75) =>
                        {
                            Some(egui::Rect::from_center_size(
                                c.center,
                                egui::Vec2::splat(5.0),
                            ))
                        }
                        _ => None,
                    })
                    .expect("retained activity needs a yellow dot");
                let cpu = text("CPU0");
                assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Rect(r) if r.rect.contains_rect(cpu) && r.rect.contains_rect(dot) && r.rect.height()<25.0)),"activity belongs inside CPU badge");
                assert!(!dot.intersects(cpu));
                assert!(!output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Rect(r) if r.rect.width()<=3.1 && r.rect.center().x>tile.center().x && tile.contains_rect(r.rect))),"right activity rail is removed");
                let quiet = GridData {
                    show_app_activity: false,
                    ..data
                };
                let mut quiet_rect = egui::Rect::NOTHING;
                let mut quiet_output = ctx.run_ui(Default::default(), |ui| {
                    ui.spacing_mut().interact_size.y = 28.0;
                    cpu_tile(ui, 0, true, 83.0, &quiet);
                    quiet_rect = ctx
                        .read_response(ui.id().with(("cpu-tile", 0usize)))
                        .unwrap()
                        .rect;
                });
                quiet_output.textures_delta.clear();
                assert_eq!(
                    tile.size(),
                    quiet_rect.size(),
                    "activity must not change tile geometry"
                );
            }
        }
    }

    #[test]
    fn application_scope_names_configured_processes_and_descendants_explicitly() {
        assert_eq!(
            scope_text(&["chrome.exe".into(), "helper.exe".into()], true),
            "Tracks: chrome.exe, helper.exe + descendants"
        );
        assert_eq!(scope_text(&[], false), "Tracks: managed launches");
    }

    #[test]
    fn clipped_cpu_tiles_skip_layout_and_retain_row_geometry() {
        let ctx = egui::Context::default();
        let overview = Overview::default();
        let data = GridData {
            values: &[Some(1.0); 3],
            partial: &[],
            parked: &[],
            mask: None,
            overview: &overview,
            hovered: None,
            sample_time: None,
            show_app_activity: false,
        };
        let mut reference = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for cpu in 0..3 {
                reference.push(
                    ui.horizontal(|ui| {
                        cpu_tile(ui, cpu, false, 83.0, &data);
                    })
                    .response
                    .rect,
                );
            }
        });
        output.textures_delta.clear();
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("cpu-tile-layout-count"), 0usize));
        let mut clipped = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_clip_rect(reference[1].shrink(1.0));
            for cpu in 0..3 {
                clipped.push(
                    ui.horizontal(|ui| {
                        cpu_tile(ui, cpu, false, 83.0, &data);
                    })
                    .response
                    .rect,
                );
            }
        });
        output.textures_delta.clear();
        assert_eq!(ctx.data(|d|d.get_temp::<usize>(egui::Id::new("cpu-tile-layout-count"))),Some(1),"only the visible middle row may lay out text: reference={reference:?}, actual={clipped:?}");
        assert_eq!(reference, clipped, "clipping must preserve scroll geometry");
    }

    #[test]
    fn tile_cpu_badge_is_centered_and_matches_detail_in_both_themes() {
        for dark in [true, false] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut tile = egui::Rect::NOTHING;
            let mut detail = egui::Rect::NOTHING;
            let mut primary = egui::Color32::TRANSPARENT;
            let mut border = egui::Color32::TRANSPARENT;
            let overview = Overview::default();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                primary = palette(ui).success.fg;
                border = palette(ui).success.fg;
                let data = GridData {
                    values: &[Some(12.0)],
                    partial: &[],
                    parked: &[Some(true)],
                    mask: None,
                    overview: &overview,
                    hovered: None,
                    sample_time: None,
                    show_app_activity: false,
                };
                ui.horizontal(|ui| {
                    cpu_tile(ui, 0, true, 83.0, &data);
                    tile = ui
                        .ctx()
                        .read_response(ui.id().with(("cpu-tile", 0usize)))
                        .unwrap()
                        .rect;
                });
                detail = cpu_badge(ui, 0, 11.0, true, None, false).rect;
            });
            output.textures_delta.clear();
            let badges = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if r.fill == primary
                            && r.stroke.color == border
                            && r.stroke.width == 1.0 =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            let badge = badges
                .iter()
                .find(|r| tile.contains_rect(**r))
                .expect("CPU tile needs the shared outlined badge");
            assert!(
                (badge.center().x - tile.center().x).abs() < 0.6,
                "CPU name must be centered, including selected tiles"
            );
            assert!(badge.top() < tile.top() + 8.0);
            assert!((badge.width()/(tile.width()-8.0)-0.7).abs()<0.02,
                "tile badge must leave symmetric side space and be about 30% narrower: badge={badge:?}, tile={tile:?}");
            assert!(
                badges.iter().any(|r| r.contains_rect(detail.shrink(1.0))),
                "detail must reuse badge style"
            );
            assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.rect==tile && r.stroke.width==2.0 && r.stroke.color==border)), "selected tile outline must match the green name badge");
            assert!(!output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::LineSegment { points,stroke } if stroke.width==1.5 && tile.contains(points[0]) && tile.contains(points[1]))), "selection must not paint the former check mark");
            let load = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "12%" => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()))
                    }
                    _ => None,
                })
                .unwrap();
            assert!(tile.contains_rect(load));
            assert!(
                !load.intersects(*badge),
                "load must remain readable outside the name badge"
            );
        }
    }

    #[test]
    fn expanded_contributor_table_shows_at_least_eight_rows_at_nine_hundred_height() {
        let ctx = egui::Context::default();
        let view = super::super::contributors::ContributorView {
            revision: 1,
            available: true,
            partial: false,
            time: None,
            rows: (0..40)
                .map(|pid| {
                    super::super::contributors::ContributorRow::new(
                        Arc::new(os_api::cpu_load::CpuProcess {
                            pid,
                            instance: 10,
                            name: format!("row{pid}.exe").into(),
                        }),
                        1.0,
                    )
                })
                .collect(),
        };
        let mut output = egui::FullOutput::default();
        for _ in 0..3 {
            output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| contributor_table(ui, &view, 0, false),
            );
            output.textures_delta.clear();
        }
        let rows = output
            .shapes
            .iter()
            .filter(
                |s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().starts_with("row")),
            )
            .count();
        let positions = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text().starts_with("row") => Some(t.pos.y),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            positions[1] - positions[0] <= 15.1,
            "contributors need compact row pitch: {:?}",
            &positions[..2]
        );
        assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::LineSegment{points,stroke} if (points[0].y-points[1].y).abs()<0.1 && (points[1].x-points[0].x).abs()>200.0 && stroke.width<=1.0)), "contributors need subtle row separators");
        assert!(
            rows >= 8,
            "expanded table needs more visible contributors: {rows}"
        );
    }

    #[test]
    fn contributor_columns_align_names_left_and_numbers_right() {
        let ctx = egui::Context::default();
        let view = super::super::contributors::ContributorView {
            revision: 1,
            time: None,
            available: true,
            partial: false,
            rows: vec![super::super::contributors::ContributorRow::new(
                Arc::new(os_api::cpu_load::CpuProcess {
                    pid: 1234,
                    instance: 1,
                    name: "chrome.exe".into(),
                }),
                27.0,
            )],
        };
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(340.0);
            contributor_table(ui, &view, 14, false);
        });
        output.textures_delta.clear();
        let bounds = |name: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == name => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .unwrap()
        };
        assert!(
            (bounds("Process").left() - bounds("chrome.exe").left()).abs() < 0.5,
            "process names and header need one left edge"
        );
        assert!(
            (bounds("PID").right() - bounds("1234").right()).abs() < 0.5,
            "PID column needs a common right edge: {:?}, {:?}",
            bounds("PID"),
            bounds("1234")
        );
        assert!(
            (bounds("CPU14").right() - bounds("27%").right()).abs() < 0.5,
            "CPU column needs a common right edge"
        );
    }

    #[test]
    fn contributor_table_has_process_pid_cpu_and_explicit_historical_time() {
        let ctx = egui::Context::default();
        let view = super::super::contributors::ContributorView {
            revision: 1,
            time: Some(super::super::history::SampleTime {
                seconds: 1.0,
                wall: std::time::SystemTime::UNIX_EPOCH,
            }),
            rows: vec![super::super::contributors::ContributorRow::new(
                Arc::new(os_api::cpu_load::CpuProcess {
                    pid: 1234,
                    instance: 1,
                    name: "example.exe".into(),
                }),
                27.0,
            )],
            available: true,
            partial: false,
        };
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            contributor_table(ui, &view, 14, true);
        });
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text()),
                _ => None,
            })
            .collect();
        for text in ["Process", "PID", "CPU14", "example.exe", "1234", "27%"] {
            assert!(labels.contains(&text), "{text}: {labels:?}");
        }
        assert!(labels.iter().any(|t| t.starts_with("History · ")));
    }

    #[test]
    fn disabled_monitoring_leaves_no_indicator_space_before_the_name() {
        let ctx = egui::Context::default();
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
                state.cpu_load_monitoring_enabled = false;
            }))),
            20,
        );
        let mut gap = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| {
                let left = ui.cursor().left();
                process_indicator(
                    &mut app,
                    ui,
                    RuleTarget {
                        group: GroupId("g".into()),
                        rule: crate::app::shared::ids::RuleId("r".into()),
                    },
                    "X",
                    false,
                    egui::Color32::GRAY,
                    "Stopped",
                );
                gap = ui.label("X").rect.left() - left;
            });
        });
        output.textures_delta.clear();
        assert_eq!(gap, 0.0, "disabled CPU meter must reserve no space");
    }

    #[test]
    fn group_indicator_only_shows_tracked_interval_load() {
        let ctx = egui::Context::default();
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
            2,
        );
        let mut snapshot = app.cpu_load.snapshot().as_ref().clone();
        snapshot.cpus = vec![Some(70.0), Some(90.0)];
        snapshot.tracked_cpus = vec![Some(99.0); 2];
        *app.cpu_load.published.write().unwrap() = Arc::new(snapshot);
        {
            let mut history = app.cpu_load.history.write().unwrap();
            history.configure(2, 30, &[]).unwrap();
            history.push(
                1.0,
                std::time::SystemTime::UNIX_EPOCH,
                &[Some(90.0); 2],
                &[],
                &std::collections::HashMap::new(),
            );
            history.record_tracked(&[Some(20.0), Some(40.0)]);
        }
        let mut output = ctx.run_ui(Default::default(), |ui| {
            group_indicator(&mut app, ui, &GroupId("g".into()), "Group", &[0, 1]);
        });
        output.textures_delta.clear();
        assert!(output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "30%")
        ));
        assert!(output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "30%")
        ));
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Tracked")));
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "All processes")));
    }

    #[test]
    fn indicators_keep_compact_unlabeled_cpu_strips() {
        let ctx = egui::Context::default();
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
            20,
        );
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            group_indicator(&mut app, ui, &GroupId("g".into()), "Games", &[0, 1]);
            process_indicator(
                &mut app,
                ui,
                RuleTarget {
                    group: GroupId("g".into()),
                    rule: crate::app::shared::ids::RuleId("r".into()),
                },
                "Game",
                true,
                egui::Color32::GRAY,
                "Running",
            );
        });
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(!labels.contains(&"All processes"));
        assert!(!labels.contains(&"Tracked"));
        assert!(labels.contains(&"Group CPU load"));
        let heights: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.rect.width() <= 70.0 => Some(rect.rect.height()),
                _ => None,
            })
            .collect();
        assert!(heights.iter().any(|height| (*height - 13.5).abs() < 0.01));
        assert!(heights.iter().any(|height| (*height - 18.0).abs() < 0.01));
    }

    #[test]
    fn system_modal_has_a_global_scope_without_a_saved_group() {
        let ctx = egui::Context::default();
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
                state.groups.clear()
            }))),
            20,
        );
        app.cpu_load.open(GraphTarget::System);
        let mut output = None;
        for _ in 0..3 {
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| draw_modal(&mut app, ui.ctx()),
            );
            frame.textures_delta.clear();
            output = Some(frame);
        }
        let output = output.unwrap();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        assert!(labels.contains(&"All CPUs · System CPU"));
        assert!(labels.contains(&"All system processes on all logical CPUs"));
        assert!(
            !labels.contains(&"All CPUs"),
            "system CPU scope is fixed, not a group-mask toggle"
        );
    }

    #[test]
    fn modal_uses_eighty_percent_height_with_ten_percent_vertical_margins() {
        let ctx = egui::Context::default();
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
            20,
        );
        app.cpu_load.open(GraphTarget::Group {
            id: GroupId("g".into()),
            name: "Performance".into(),
            cpus: (0..12).collect(),
        });
        for _ in 0..3 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| draw_modal(&mut app, ui.ctx()),
            )
            .textures_delta
            .clear();
        }
        let modal = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("cpu-load-modal")))
            .unwrap();
        assert!(
            (modal.height() - 720.0).abs() < 2.0 && (modal.top() - 90.0).abs() < 2.0,
            "CPU dialog should use80% height with10% top/bottom: {modal:?}"
        );
    }

    #[test]
    fn chart_marks_only_confirmed_parking_and_keeps_missing_load_as_a_gap() {
        let ctx = egui::Context::default();
        let mut history = History::default();
        history.push_sample(1.0, vec![Some(0.0)], vec![Some(false)]);
        history.push_sample(2.0, vec![None], vec![Some(true)]);
        history.push_sample(3.0, vec![Some(0.0)], vec![None]);
        let mut shade = egui::Color32::TRANSPARENT;
        let mut line = egui::Color32::TRANSPARENT;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            shade = palette(ui).warning.fg;
            line = palette(ui).accent.fg;
            chart(ui, &history, 0, None, false);
        });
        output.textures_delta.clear();
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(&shape.shape,egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|v| v.color==shade)))
                .count(),
            1
        );
        let gap = ctx
            .read_response(egui::Id::new("cpu-load-chart"))
            .unwrap()
            .rect
            .center()
            .x;
        for shape in &output.shapes {
            if let egui::Shape::Mesh(mesh) = &shape.shape {
                for triangle in mesh.indices.chunks_exact(3) {
                    let points = triangle
                        .iter()
                        .map(|&i| &mesh.vertices[i as usize])
                        .collect::<Vec<_>>();
                    if points.iter().all(|v| v.color == line) {
                        assert!(
                            !(points.iter().any(|v| v.pos.x < gap)
                                && points.iter().any(|v| v.pos.x > gap)),
                            "unknown samples must remain gaps between isolated measured dots"
                        );
                    }
                }
            }
        }
        assert_eq!(parking_text(Some(true)), "Parked");
        assert_eq!(parking_text(Some(false)), "Unparked");
        assert_eq!(parking_text(None), "State —");
    }

    #[test]
    fn modal_close_control_has_a_readable_word_instead_of_an_unavailable_glyph() {
        let ctx = egui::Context::default();
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
            20,
        );
        app.cpu_load.open(GraphTarget::Group {
            id: GroupId("g".into()),
            name: "Performance".into(),
            cpus: (0..12).collect(),
        });
        for frame in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| draw_modal(&mut app, ui.ctx()),
            );
            output.textures_delta.clear();
            if frame == 1 {
                let text: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert!(!text.iter().any(|text| text.contains('✕')));
                assert_eq!(text.iter().filter(|text| **text == "Close").count(), 1);
            }
        }
    }

    #[test]
    fn modal_fits_compact_viewport_and_closes_with_escape_without_native_work() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut app = AppState::new_for_test(
                Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
                    state.groups.push(crate::app::models::CoreGroup {
                        name: "Many CPUs".into(),
                        cores: (0..192).collect(),
                        programs: vec![],
                        is_hidden: false,
                        run_all_button: false,
                    });
                }))),
                192,
            );
            app.cpu_load.open(GraphTarget::Process {
                target: RuleTarget {
                    group: app.rules.group_id_for_index(0).unwrap(),
                    rule: crate::app::shared::ids::RuleId("r".into()),
                },
                name: "A very long application name that must fit the compact utility".into(),
            });
            for frame in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(400.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| draw_modal(&mut app, ui.ctx()),
                );
                output.textures_delta.clear();
                if frame == 2 {
                    let modal = ctx
                        .memory(|memory| memory.area_rect(egui::Id::new("cpu-load-modal")))
                        .unwrap();
                    assert!(
                        modal.width() <= 400.0 && modal.height() <= 600.0,
                        "modal overflow: {modal:?}"
                    );
                    for shape in &output.shapes {
                        if let egui::Shape::Text(text) = &shape.shape {
                            let visible = text
                                .galley
                                .rect
                                .translate(text.pos.to_vec2())
                                .intersect(shape.clip_rect);
                            assert!(
                                visible.max.x <= 400.0 && visible.min.x >= 0.0,
                                "text outside compact viewport: {visible:?}"
                            );
                        }
                    }
                }
                output.textures_delta.clear();
            }
            ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |ui| draw_modal(&mut app, ui.ctx()),
            )
            .textures_delta
            .clear();
            assert!(app.cpu_load.graph.is_none());
        }
    }

    #[test]
    fn outside_mask_detection_preserves_unknowns_and_small_real_load() {
        assert_eq!(
            outside_activity(&[Some(0.0), Some(0.01), None], &[], Some(&[0])),
            Some(1)
        );
        assert_eq!(outside_activity(&[Some(0.0), None], &[], Some(&[0])), None);
        assert_eq!(
            outside_activity(&[None, Some(0.0)], &[], Some(&[0])),
            Some(0)
        );
        assert_eq!(outside_activity(&[Some(100.0)], &[], None), None);
        assert_eq!(percent(Some(0.01)), "<1%");
        assert_eq!(percent(None), "—");
    }
}
