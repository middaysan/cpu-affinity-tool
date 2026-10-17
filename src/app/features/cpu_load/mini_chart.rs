use super::history::{Overview, SampleTime};
use crate::app::shell::presenters::shared_elements::palette;
use eframe::egui::{self, Color32, Rect, Stroke};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(PartialEq)]
struct Key {
    generation: u64,
    rect: Rect,
    color: Color32,
    scale: f32,
}
struct Geometry {
    key: Key,
    mesh: Arc<egui::Mesh>,
}
const CACHE_ID: &str = "cpu-mini-graphs";
type Cache = HashMap<usize, Arc<Geometry>>;

pub(super) fn clear(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Cache>(egui::Id::new(CACHE_ID)));
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    overview: &Overview,
    cpu: usize,
    hovered: Option<SampleTime>,
) -> egui::Response {
    crate::render_profile_scope!(MiniDraw);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let plot = rect.shrink(1.0);
    let colors = *palette(ui);
    ui.painter().line_segment(
        [plot.left_bottom(), plot.right_bottom()],
        Stroke::new(0.5, colors.border_strong),
    );
    let Some(&(start, _)) = overview.times.first() else {
        return response;
    };
    let end = overview.times.last().unwrap().1;
    let span = (end - start).max(1.0);
    let x_at = |seconds: f64| plot.left() + ((seconds - start) / span) as f32 * plot.width();
    let y_at = |value: f32| plot.bottom() - value.clamp(0.0, 100.0) / 100.0 * plot.height();
    let key = Key {
        generation: overview.generation,
        rect: plot,
        color: colors.accent.fg,
        scale: ui.pixels_per_point(),
    };
    let cache_id = egui::Id::new(CACHE_ID);
    let cached = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_default::<Cache>(cache_id)
            .get(&cpu)
            .cloned()
    });
    let geometry = if let Some(cached) = cached.filter(|cached| cached.key == key) {
        cached
    } else {
        crate::render_profile_scope!(MiniRebuild);
        let mut shapes = Vec::new();
        let mut previous = None;
        let bins = overview.times.len();
        if cpu < overview.cpus {
            for (bin, &(from, until)) in overview.times.iter().enumerate() {
                let Some((min, max)) = overview.values[cpu * bins + bin] else {
                    previous = None;
                    continue;
                };
                let point = egui::pos2(x_at((from + until) * 0.5), y_at(max));
                let partial = overview
                    .partial
                    .get(cpu * bins + bin)
                    .copied()
                    .unwrap_or(false);
                // A thin min/max range retains brief peaks in compressed history.
                shapes.push(egui::epaint::ClippedShape {
                    clip_rect: plot,
                    shape: egui::Shape::line_segment(
                        [point, egui::pos2(point.x, y_at(min))],
                        Stroke::new(1.0, colors.accent.fg),
                    ),
                });
                if min == max {
                    shapes.push(egui::epaint::ClippedShape {
                        clip_rect: plot,
                        shape: egui::Shape::circle_filled(point, 1.0, colors.accent.fg),
                    });
                }
                if let Some((at, last, last_partial)) = previous {
                    if from - at <= 2.5 {
                        let stroke = Stroke::new(1.0, colors.accent.fg);
                        let segments = if partial || last_partial {
                            egui::Shape::dashed_line(&[last, point], stroke, 3.0, 2.0)
                        } else {
                            vec![egui::Shape::line_segment([last, point], stroke)]
                        };
                        for shape in segments {
                            shapes.push(egui::epaint::ClippedShape {
                                clip_rect: plot,
                                shape,
                            });
                        }
                    }
                }
                previous = Some((until, point, partial));
            }
        }
        let mut mesh = egui::Mesh::default();
        for primitive in ui.ctx().tessellate(shapes, ui.pixels_per_point()) {
            if let egui::epaint::Primitive::Mesh(part) = primitive.primitive {
                mesh.append(part);
            }
        }
        let geometry = Arc::new(Geometry {
            key,
            mesh: Arc::new(mesh),
        });
        ui.ctx().data_mut(|data| {
            data.get_temp_mut_or_default::<Cache>(cache_id)
                .insert(cpu, geometry.clone())
        });
        geometry
    };
    ui.painter().add(egui::Shape::Mesh(geometry.mesh.clone()));
    if let Some(time) = hovered.filter(|time| time.seconds >= start && time.seconds <= end) {
        let x = x_at(time.seconds);
        ui.painter().line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            Stroke::new(1.0, colors.text_primary),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::features::cpu_load::history::Overview;
    use eframe::egui;
    use std::sync::Arc;

    #[test]
    fn mini_graph_retains_peaks_and_gaps_and_reuses_mesh_during_hover() {
        let ctx = egui::Context::default();
        let overview = Overview {
            revision: 1,
            generation: 1,
            cpus: 1,
            times: (0..5).map(|i| (i as f64, i as f64)).collect(),
            values: vec![
                Some((10.0, 10.0)),
                Some((10.0, 100.0)),
                None,
                Some((20.0, 20.0)),
                Some((30.0, 30.0)),
            ],
            activity: vec![true],
            partial: vec![false; 5],
        };
        let mut previous = None;
        for second in 0..3 {
            let mut rect = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(100.0, 100.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    rect = draw(
                        ui,
                        &overview,
                        0,
                        Some(SampleTime {
                            seconds: second as f64,
                            wall: std::time::SystemTime::UNIX_EPOCH,
                        }),
                    )
                    .rect;
                },
            );
            output.textures_delta.clear();
            let mesh = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh),
                    _ => None,
                })
                .expect("mini graph mesh");
            assert!(
                mesh.vertices.iter().any(|v| v.pos.y <= rect.top() + 2.0),
                "100% peak must survive overview compression"
            );
            let gap = rect.center().x;
            for triangle in mesh.indices.chunks_exact(3) {
                let xs = triangle
                    .iter()
                    .map(|&i| mesh.vertices[i as usize].pos.x)
                    .collect::<Vec<_>>();
                assert!(
                    !(xs.iter().copied().fold(f32::INFINITY, f32::min) < gap
                        && xs.iter().copied().fold(f32::NEG_INFINITY, f32::max) > gap),
                    "mini graph must not bridge missing samples"
                );
            }
            if let Some(previous) = previous {
                assert!(Arc::ptr_eq(&previous, mesh));
            }
            previous = Some(mesh.clone());
        }
    }
}
