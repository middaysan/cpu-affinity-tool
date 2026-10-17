use super::history::{SampleTime, Series as History, SeriesPoint as Point};
use crate::app::shell::presenters::shared_elements::palette;
use eframe::egui::{self, RichText};
use std::sync::Arc;

/// Choose an actual recorded sample, including unknowns. Never interpolate a
/// value across gaps or use wall time to order samples after a clock change.
fn nearest_sample(history: &History, seconds: f64) -> Option<&Point> {
    let index = history.points.partition_point(|p| p.seconds < seconds);
    let right = history.points.get(index);
    let left = index.checked_sub(1).and_then(|i| history.points.get(i));
    match (left, right) {
        (Some(left), Some(right)) => Some(if seconds - left.seconds <= right.seconds - seconds {
            left
        } else {
            right
        }),
        (left, right) => left.or(right),
    }
}

fn sample_lines(point: &Point, cpu: usize) -> [String; 4] {
    let value = point.value;
    let load = if value.is_some() {
        super::view::measured_percent(value, point.partial)
    } else {
        "No data".into()
    };
    [
        os_api::cpu_load::sample_clock_label(point.timestamp),
        format!("CPU{cpu}"),
        format!(
            "Load: {load}{}",
            if point.partial { " (subtotal)" } else { "" }
        ),
        point
            .parked
            .map(|_| format!("{} at sample", super::view::parking_text(point.parked)))
            .unwrap_or_else(|| "Parking: unknown".into()),
    ]
}

#[derive(Clone, PartialEq)]
struct GeometryKey {
    revision: u64,
    generation: u64,
    cpu: usize,
    plot: egui::Rect,
    accent: egui::Color32,
    parking: egui::Color32,
    scale: f32,
}
struct CachedGeometry {
    key: GeometryKey,
    meshes: Vec<Arc<egui::Mesh>>,
    has_parking: bool,
    has_partial: bool,
    labels: [String; 3],
}
const CACHE_ID: &str = "cpu-load-chart-geometry";
const CHART_ID: &str = "cpu-load-chart";

#[derive(Clone, Copy, Default)]
pub(super) struct Inspection {
    pub pinned: Option<SampleTime>,
    click_frame: Option<u64>,
}
impl Inspection {
    pub fn select(&mut self, ctx: &egui::Context, history: &History) -> Option<SampleTime> {
        let hovered = hovered_sample(ctx, history);
        let frame = ctx.cumulative_frame_nr();
        if ctx.input(|input| input.pointer.primary_clicked()) && self.click_frame != Some(frame) {
            // One physical click toggles once, even if egui repeats the layout.
            // Scroll wheels and scrollbar drags are not click events.
            self.click_frame = Some(frame);
            self.pinned = if self.pinned.is_some() { None } else { hovered };
        }
        self.pinned.or(hovered)
    }
}
fn plot_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        rect.min + egui::vec2(3.0, 13.0),
        rect.max - egui::vec2(3.0, 16.0),
    )
}

fn sample_at(history: &History, plot: egui::Rect, pointer: egui::Pos2) -> Option<SampleTime> {
    let first = history.points.first()?.seconds;
    let last = history.points.last()?.seconds;
    let seconds =
        first + f64::from((pointer.x - plot.left()) / plot.width()) * (last - first).max(1.0);
    nearest_sample(history, seconds).map(|point| SampleTime {
        seconds: point.seconds,
        wall: point.timestamp,
    })
}

/// Inspect the previous stable plot geometry before drawing tiles above it.
/// Normal pointer movement needs no second layout or extra history series.
pub(super) fn hovered_sample(ctx: &egui::Context, history: &History) -> Option<SampleTime> {
    let response = ctx.read_response(egui::Id::new(CHART_ID))?;
    let plot = plot_rect(response.rect);
    let pointer = ctx.pointer_hover_pos()?;
    ctx.rect_contains_pointer(response.layer_id, plot.intersect(response.interact_rect))
        .then(|| sample_at(history, plot, pointer))
        .flatten()
}
pub(super) fn clear(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Arc<CachedGeometry>>(egui::Id::new(CACHE_ID)));
    super::mini_chart::clear(ctx);
}
pub(super) fn draw(
    ui: &mut egui::Ui,
    history: &History,
    cpu: usize,
    inspected: Option<SampleTime>,
    pinned: bool,
) -> Option<SampleTime> {
    crate::render_profile_scope!(DetailDraw);
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), 124.0));
    let response = ui.interact(rect, egui::Id::new(CHART_ID), egui::Sense::click());
    let plot = plot_rect(rect);
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    crate::app::features::diagnostics::debug_mcp::rect_control(
        ui.ctx(),
        "chart.plot",
        plot.intersect(ui.clip_rect()),
        true,
        serde_json::Value::Null,
    );
    let colors = *palette(ui);
    for fraction in [0.0, 0.5, 1.0] {
        let y = plot.bottom() - plot.height() * fraction;
        ui.painter().line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(0.5, colors.border_strong),
        );
    }
    ui.painter().text(
        rect.left_top(),
        egui::Align2::LEFT_TOP,
        "100%",
        egui::FontId::proportional(8.0),
        colors.text_muted,
    );
    let end = history
        .points
        .last()
        .map(|point| point.seconds)
        .unwrap_or(0.0);
    let start = history
        .points
        .first()
        .map(|point| point.seconds)
        .unwrap_or(end);
    let span = (end - start).max(1.0);
    let x_at = |seconds: f64| plot.left() + ((seconds - start) / span) as f32 * plot.width();
    let y_at = |value: f32| plot.bottom() - value.clamp(0.0, 100.0) / 100.0 * plot.height();

    let key = GeometryKey {
        revision: history.revision,
        generation: history.generation,
        cpu,
        plot,
        accent: colors.accent.fg,
        parking: colors.warning.fg,
        scale: ui.pixels_per_point(),
    };
    let cache_id = egui::Id::new(CACHE_ID);
    let cached = ui
        .ctx()
        .data(|data| data.get_temp::<Arc<CachedGeometry>>(cache_id));
    let geometry = if let Some(cached) = cached.filter(|cached| cached.key == key) {
        cached
    } else {
        crate::render_profile_scope!(DetailRebuild);
        let shapes = std::cell::RefCell::new(Vec::new());
        let mut previous_seconds = None;
        let mut has_parking = false;
        let mut has_partial = false;
        let mut line_partial = false;
        let mut line = Vec::new();
        let paint_line = |points: &mut Vec<egui::Pos2>, partial: bool| {
            if points.len() > 1 {
                let stroke = egui::Stroke::new(1.5, colors.accent.fg);
                if partial {
                    for shape in egui::Shape::dashed_line(points, stroke, 4.0, 3.0) {
                        shapes.borrow_mut().push(egui::epaint::ClippedShape {
                            clip_rect: plot,
                            shape,
                        });
                    }
                    points.clear();
                } else {
                    shapes.borrow_mut().push(egui::epaint::ClippedShape {
                        clip_rect: plot,
                        shape: egui::Shape::line(std::mem::take(points), stroke),
                    });
                }
            } else {
                if let Some(point) = points.first() {
                    shapes.borrow_mut().push(egui::epaint::ClippedShape {
                        clip_rect: plot,
                        shape: egui::Shape::circle_filled(*point, 1.75, colors.accent.fg),
                    });
                }
                points.clear();
            }
        };
        for point in &history.points {
            let gap = previous_seconds.is_some_and(|previous| point.seconds - previous > 2.5);
            let parked = point.parked == Some(true);
            if parked {
                has_parking = true;
                // Parking is an instantaneous flag, while load covers an
                // interval. Mark only observed samples below the plot; joining
                // them would imply continuous parking that we did not measure.
                shapes.borrow_mut().push(egui::epaint::ClippedShape {
                    clip_rect: rect,
                    shape: egui::Shape::rect_filled(
                        egui::Rect::from_center_size(
                            egui::pos2(x_at(point.seconds), plot.bottom() + 4.5),
                            egui::vec2(2.0, 3.0),
                        ),
                        0.0,
                        colors.warning.fg,
                    ),
                });
            }
            if gap || point.partial != line_partial {
                paint_line(&mut line, line_partial);
            }
            if let Some(value) = point.value.filter(|value| value.is_finite()) {
                has_partial |= point.partial;
                line_partial = point.partial;
                line.push(egui::pos2(x_at(point.seconds), y_at(value)));
            } else {
                paint_line(&mut line, line_partial);
            }
            previous_seconds = Some(point.seconds);
        }
        paint_line(&mut line, line_partial);

        let meshes = ui
            .ctx()
            .tessellate(shapes.into_inner(), ui.pixels_per_point())
            .into_iter()
            .filter_map(|primitive| match primitive.primitive {
                egui::epaint::Primitive::Mesh(mesh) => Some(Arc::new(mesh)),
                _ => None,
            })
            .collect();
        let labels = [0.0, 0.5, 1.0].map(|fraction| {
            nearest_sample(history, start + span * fraction)
                .map(|point| os_api::cpu_load::sample_clock_label(point.timestamp))
                .unwrap_or_default()
        });
        let cached = Arc::new(CachedGeometry {
            key,
            meshes,
            has_parking,
            has_partial,
            labels,
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(cache_id, cached.clone()));
        cached
    };
    for mesh in &geometry.meshes {
        ui.painter().add(egui::Shape::Mesh(mesh.clone()));
    }
    for ((fraction, align), label) in [
        (0.0, egui::Align2::LEFT_BOTTOM),
        (0.5, egui::Align2::CENTER_BOTTOM),
        (1.0, egui::Align2::RIGHT_BOTTOM),
    ]
    .into_iter()
    .zip(&geometry.labels)
    {
        ui.painter().text(
            egui::pos2(plot.left() + plot.width() * fraction, rect.bottom()),
            align,
            label,
            egui::FontId::proportional(8.0),
            colors.text_muted,
        );
    }
    let hovered = response
        .hover_pos()
        .filter(|pointer| plot.contains(*pointer))
        .and_then(|pointer| sample_at(history, plot, pointer));
    let selected = if pinned { inspected } else { hovered };
    if let Some(time) = selected {
        let index = history
            .points
            .partition_point(|point| point.seconds < time.seconds);
        // A retained cursor never snaps to a different row after ring eviction.
        if let Some(point) = history
            .points
            .get(index)
            .filter(|point| point.seconds == time.seconds && point.timestamp == time.wall)
        {
            let x = x_at(point.seconds);
            ui.painter().line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                egui::Stroke::new(1.0, colors.text_secondary),
            );
            if let Some(value) = point.value {
                ui.painter()
                    .circle_filled(egui::pos2(x, y_at(value)), 2.5, colors.accent.fg);
            }
            if !pinned {
                egui::Tooltip::for_widget(&response)
                    .at_pointer()
                    // Give edge placement a useful size before its first layout;
                    // each field stays whole instead of wrapping near the pointer.
                    .width(140.0)
                    .layout(egui::Layout::top_down(egui::Align::Min))
                    .show(|ui| {
                        for line in sample_lines(point, cpu) {
                            ui.add(egui::Label::new(line).wrap_mode(egui::TextWrapMode::Extend));
                        }
                    });
            }
        }
    } else {
        response.on_hover_text(format!(
            "Up to {} seconds, sampled once per second. Hover for the recorded value; left click the plot to pin that time and scroll the process list. The next left click anywhere releases it. Load covers an interval; amber ticks mark parking only at the sample time. Gaps mean no data.", history.duration_seconds,
        ));
    }
    if geometry.has_parking || geometry.has_partial {
        ui.label(
            RichText::new(if geometry.has_parking && geometry.has_partial {"Dashed/*: measured subtotal · amber: parked"} else if geometry.has_partial {"Dashed/*: measured subtotal · gaps: no data"} else {"Amber ticks: parked at sample · gaps: no data"})
                .size(8.0)
                .color(colors.warning.fg),
        ).on_hover_text("Dashed lines and * show only the known measured work when some tracked processes have no valid sample. This is a lower bound, not a complete CPU total. Missing intervals remain gaps. Amber ticks show parking at the poll and do not cancel work measured during the interval.");
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn partial_curve_is_dashed_and_explained_while_hover_reuses_geometry() {
        let ctx = egui::Context::default();
        let mut history = History::default();
        for second in 0..4 {
            history.push_sample(second as f64, vec![Some(15.0)], vec![]);
            history.points.last_mut().unwrap().partial = true;
        }
        let mut prior = None;
        for hovered in [
            None,
            Some(SampleTime {
                seconds: 1.0,
                wall: history.points[1].timestamp,
            }),
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    draw(ui, &history, 0, hovered, true);
                },
            );
            output.textures_delta.clear();
            let geometry = ctx
                .data(|d| d.get_temp::<Arc<CachedGeometry>>(egui::Id::new(CACHE_ID)))
                .unwrap();
            assert!(geometry.has_partial);
            let plot = geometry.key.plot;
            let mut covered = 0;
            for index in 0..200 {
                let x = plot.left() + (index as f32 + 0.5) / 200.0 * plot.width();
                if geometry.meshes.iter().any(|m| {
                    m.indices.chunks_exact(3).any(|tri| {
                        let xs = tri
                            .iter()
                            .map(|&i| m.vertices[i as usize].pos.x)
                            .collect::<Vec<_>>();
                        xs.iter().any(|v| *v <= x) && xs.iter().any(|v| *v >= x)
                    })
                }) {
                    covered += 1;
                }
            }
            assert!(
                (60..185).contains(&covered),
                "partial measurement needs visible dashes with gaps, coverage={covered}"
            );
            assert!(sample_lines(&history.points[1], 0)[2].contains("subtotal"));
            if let Some(prior) = prior {
                assert!(Arc::ptr_eq(&prior, &geometry));
            }
            prior = Some(geometry);
        }
    }

    #[test]
    fn isolated_measured_samples_are_visible_without_bridging_unknown_intervals() {
        let ctx = egui::Context::default();
        let mut history = History::default();
        for (second, value) in [
            (0.0, None),
            (1.0, Some(15.0)),
            (2.0, None),
            (3.0, Some(1.0)),
            (4.0, None),
        ] {
            history.push_sample(second, vec![value], vec![]);
        }
        let mut color = egui::Color32::TRANSPARENT;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                color = palette(ui).accent.fg;
                draw(ui, &history, 0, None, false);
            },
        );
        output.textures_delta.clear();
        let meshes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Mesh(m) = &s.shape {
                    Some(m)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            meshes
                .iter()
                .any(|m| m.vertices.iter().any(|v| v.color == color)),
            "positive isolated measurements need visible dots"
        );
    }

    #[test]
    fn parking_is_marked_at_samples_without_shading_intervals_or_hiding_load() {
        let context = egui::Context::default();
        let mut history = History::default();
        for (seconds, value, parked) in [
            (0.0, Some(85.0), Some(true)),
            (1.0, Some(40.0), Some(true)),
            (2.0, None, None),
            (3.0, Some(60.0), Some(false)),
            (4.0, Some(25.0), Some(true)),
        ] {
            history.push_sample(seconds, vec![value], vec![parked]);
        }
        let mut plot = egui::Rect::NOTHING;
        let mut marker_color = egui::Color32::TRANSPARENT;
        let mut load_color = marker_color;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                let origin = ui.next_widget_position();
                plot = egui::Rect::from_min_max(
                    origin + egui::vec2(3.0, 13.0),
                    origin + egui::vec2(ui.available_width() - 3.0, 108.0),
                );
                marker_color = palette(ui).warning.fg;
                load_color = palette(ui).accent.fg;
                draw(ui, &history, 0, None, false);
            },
        );
        output.textures_delta.clear();
        let vertices: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.vertices.iter()),
                _ => None,
            })
            .flatten()
            .collect();
        let marks: Vec<_> = vertices
            .iter()
            .filter(|v| v.color == marker_color)
            .collect();
        assert!(!marks.is_empty(), "parking needs visible sample marks");
        for vertex in marks {
            assert!(
                vertex.pos.y > plot.bottom() && vertex.pos.y < plot.bottom() + 8.0,
                "parking is a sample annotation, not a shaded load interval"
            );
            let sample = (vertex.pos.x - plot.left()) / plot.width() * 4.0;
            assert!(
                [0.0_f32, 1.0, 4.0]
                    .iter()
                    .any(|second| (sample - second).abs() < 0.04),
                "unknown/unparked samples and times between polls must not be marked"
            );
        }
        assert!(
            vertices
                .iter()
                .any(|v| v.color == load_color && v.pos.y < plot.center().y),
            "a parked-at-sample flag must not suppress real interval load"
        );
    }

    #[test]
    fn hover_rows_remain_whole_and_inside_window_at_graph_edges() {
        let mut history = History::default();
        for second in 0..3 {
            history.push_sample_at(
                second as f64,
                UNIX_EPOCH + Duration::from_secs(86_400 + second),
                vec![Some(11.0)],
                vec![Some(false)],
            );
        }
        for dark in [true, false] {
            for width in [240.0, 400.0] {
                for top in [24.0, 430.0] {
                    for fraction in [0.0, 0.5, 1.0] {
                        let context = egui::Context::default();
                        context.set_fonts(
                            crate::app::shell::presenters::shared_elements::ui_font_definitions(),
                        );
                        context.set_visuals(if dark {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        });
                        // The chart readout needs its own readable width even if
                        // another UI configures compact general-purpose tooltips.
                        context.all_styles_mut(|style| style.spacing.tooltip_width = 44.0);
                        let screen =
                            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0));
                        let pointer = egui::pos2(3.0 + (width - 6.0) * fraction, top + 50.0);
                        let mut output = None;
                        for _ in 0..4 {
                            let mut frame = context.run_ui(
                                egui::RawInput {
                                    screen_rect: Some(screen),
                                    events: vec![egui::Event::PointerMoved(pointer)],
                                    ..Default::default()
                                },
                                |ui| {
                                    ui.add_space(top);
                                    draw(ui, &history, 0, None, false);
                                },
                            );
                            frame.textures_delta.clear();
                            output = Some(frame);
                        }
                        let output = output.unwrap();
                        let rows: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| {
                                if let egui::Shape::Text(text) = &shape.shape {
                                    if text.galley.text() == "CPU0"
                                        || text.galley.text() == "Load: 11%"
                                        || text.galley.text() == "Unparked at sample"
                                    {
                                        return Some(text);
                                    }
                                }
                                None
                            })
                            .collect();
                        assert_eq!(
                            rows.len(),
                            3,
                            "hover needs separate CPU, load and parking rows"
                        );
                        let cpu_top = rows[0].pos.y;
                        let clock = output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                egui::Shape::Text(text)
                                    if text.galley.text().len() == 8
                                        && text.galley.text().contains(':')
                                        && text.pos.y < cpu_top
                                        && cpu_top - text.pos.y < 30.0 =>
                                {
                                    Some(text)
                                }
                                _ => None,
                            })
                            .expect("sample clock needs its own row above the CPU");
                        let mut last_bottom = None;
                        for text in std::iter::once(clock).chain(rows) {
                            assert_eq!(
                                text.galley.rows.len(),
                                1,
                                "readout text must never split across rows: {}",
                                text.galley.text()
                            );
                            let bounds = text.galley.rect.translate(text.pos.to_vec2());
                            assert!(
                                screen.contains_rect(bounds),
                                "readout must stay inside the window: {bounds:?} at {pointer:?}"
                            );
                            if let Some(bottom) = last_bottom {
                                assert!(bounds.top() >= bottom, "readout rows must not overlap");
                            }
                            last_bottom = Some(bounds.bottom());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn graph_hover_shows_sample_time_and_long_history_uses_batched_geometry() {
        let context = egui::Context::default();
        let mut history = History::default();
        history.set_duration_seconds(1800);
        for second in 0..=1800 {
            history.push_sample_at(
                second as f64,
                UNIX_EPOCH + Duration::from_secs(86_400 + second),
                vec![Some(27.0)],
                vec![Some(true)],
            );
        }
        let mut previous_meshes: Vec<Arc<egui::Mesh>> = Vec::new();
        let mut hovered = false;
        for _ in 0..3 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    events: vec![egui::Event::PointerMoved(egui::pos2(200.0, 50.0))],
                    ..Default::default()
                },
                |ui| {
                    draw(ui, &history, 0, None, false);
                },
            );
            output.textures_delta.clear();
            let paths: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh)
                        if mesh
                            .vertices
                            .iter()
                            .any(|v| v.color != egui::Color32::TRANSPARENT) =>
                    {
                        Some(mesh)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                paths.len(),
                2,
                "load and parking annotations should use two cached batches"
            );
            for (index, path) in paths.iter().enumerate() {
                assert!(!path.vertices.is_empty());
                if let Some(previous) = previous_meshes.get(index) {
                    assert!(
                        Arc::ptr_eq(previous, path),
                        "pointer movement must reuse tessellated geometry"
                    );
                }
            }
            previous_meshes = paths.into_iter().cloned().collect();
            assert!(
                output.shapes.len() < 40,
                "history must not create a paint command per second"
            );
            hovered |= ["CPU0", "Load: 27%", "Parked at sample"].iter().all(|line| output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == *line)));
        }
        assert!(hovered, "hover should expose the original recorded sample");
    }

    #[test]
    fn hover_uses_original_unknown_sample_and_monotonic_order_after_clock_change() {
        let mut history = History::default();
        history.push_sample_at(
            0.0,
            UNIX_EPOCH + Duration::from_secs(86_399),
            vec![Some(15.0)],
            vec![Some(false)],
        );
        history.push_sample_at(
            1.0,
            UNIX_EPOCH + Duration::from_secs(86_400),
            vec![None],
            vec![Some(true)],
        );
        history.push_sample_at(
            2.0,
            UNIX_EPOCH + Duration::from_secs(86_390),
            vec![Some(75.0)],
            vec![None],
        );
        let point = nearest_sample(&history, 0.9).unwrap();
        assert_eq!(point.seconds, 1.0);
        assert_eq!(point.timestamp, UNIX_EPOCH + Duration::from_secs(86_400));
        assert_eq!(
            sample_lines(point, 0),
            [
                os_api::cpu_load::sample_clock_label(point.timestamp),
                "CPU0".into(),
                "Load: No data".into(),
                "Parked at sample".into(),
            ]
        );
        assert_eq!(nearest_sample(&history, 1.9).unwrap().seconds, 2.0);
    }
}
