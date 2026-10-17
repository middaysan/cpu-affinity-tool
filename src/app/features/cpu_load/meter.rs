use crate::app::shell::presenters::shared_elements::{paint_focus_ring, palette};
use eframe::egui::{self, Color32};

/// One compact hit target with horizontal fill behind the percentage. The
/// status-colored outline stays visible at zero or unavailable load too.
pub(super) fn load_meter(
    ui: &mut egui::Ui,
    value: Option<f32>,
    color: Color32,
    interactive: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(47.0, 18.4),
        if interactive {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            interactive,
            format!("CPU load {}. Open graph", super::view::percent(value)),
        )
    });
    let colors = *palette(ui);
    ui.painter().rect_filled(rect, 3.0, colors.inset);
    if response.hovered() && interactive {
        ui.painter()
            .rect_filled(rect, 3.0, color.gamma_multiply(0.1));
    }
    let level = value
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 100.0)
        / 100.0;
    if level > 0.0 {
        let inner = rect.shrink(1.0);
        let fill = egui::Rect::from_min_max(
            inner.min,
            egui::pos2(inner.left() + inner.width() * level, inner.bottom()),
        );
        let left = color.lerp_to_gamma(colors.inset, 0.8);
        let right = color.lerp_to_gamma(
            colors.inset,
            if ui.visuals().dark_mode { 0.5 } else { 0.65 },
        );
        ui.painter()
            .add(egui::Shape::mesh(gradient_fill(fill, left, right)));
    }
    ui.painter().rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(
            if response.hovered() && interactive {
                1.0
            } else {
                0.75
            },
            color,
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        super::view::percent(value),
        egui::FontId::proportional(9.0),
        colors.text_primary,
    );
    if interactive {
        paint_focus_ring(ui, &response);
    }
    response
}

/// A small rounded mesh provides a static gradient without animation or a
/// continuous repaint. Vertex colors interpolate linearly across the fill.
fn gradient_fill(rect: egui::Rect, left: Color32, right: Color32) -> egui::Mesh {
    let mut points = Vec::new();
    egui::epaint::tessellator::path::rounded_rectangle(
        &mut points,
        rect,
        egui::epaint::CornerRadiusF32::same(2.0),
    );
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), left.lerp_to_gamma(right, 0.5));
    for point in &points {
        let fraction = ((point.x - rect.left()) / rect.width().max(f32::EPSILON)).clamp(0.0, 1.0);
        mesh.colored_vertex(*point, left.lerp_to_gamma(right, fraction));
    }
    for index in 0..points.len() as u32 {
        mesh.add_triangle(0, index + 1, (index + 1) % points.len() as u32 + 1);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_is_twenty_percent_shorter_and_fills_horizontally_with_a_gradient() {
        for dark in [true, false] {
            for value in [Some(25.0), Some(50.0), Some(100.0), Some(0.0), None] {
                let ctx = egui::Context::default();
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let color = Color32::from_rgb(5, 180, 80);
                let mut rect = egui::Rect::NOTHING;
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    rect = load_meter(ui, value, color, true).rect;
                });
                output.textures_delta.clear();
                assert_eq!(rect.width(), 47.0);
                assert!(
                    (rect.height() - 23.0 * 0.8).abs() < 0.01,
                    "reduce the meter block height by 20%"
                );
                assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Rect(r) if r.rect.width()<10.0 && r.rect.height()>12.0)),"the old vertical bar must be gone");
                let mesh = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh),
                    _ => None,
                });
                if value.is_some_and(|v| v > 0.0) {
                    let mesh = mesh.expect("nonzero load needs a gradient fill");
                    assert!(
                        mesh.is_valid() && mesh.vertices.len() <= 64,
                        "one small bounded mesh is enough"
                    );
                    assert!(
                        mesh.vertices
                            .iter()
                            .any(|v| v.color != mesh.vertices[0].color),
                        "fill needs a visible gradient"
                    );
                    let min_x = mesh
                        .vertices
                        .iter()
                        .map(|v| v.pos.x)
                        .fold(f32::INFINITY, f32::min);
                    let max_x = mesh
                        .vertices
                        .iter()
                        .map(|v| v.pos.x)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let inner = rect.shrink(1.0);
                    assert!((min_x - inner.left()).abs() < 0.01);
                    assert!(
                        (max_x - (inner.left() + inner.width() * value.unwrap() / 100.0)).abs()
                            < 0.01,
                        "fill width must match CPU load"
                    );
                } else {
                    assert!(mesh.is_none(), "zero and unknown must not invent load");
                }
                let text = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(t) => Some(t),
                        _ => None,
                    })
                    .unwrap();
                let text_rect = text.galley.rect.translate(text.pos.to_vec2());
                assert!(
                    (text_rect.center().x - rect.center().x).abs() < 1.0,
                    "keep percentage centered over the scale"
                );
            }
        }
    }

    #[test]
    fn zero_and_unknown_keep_status_and_whole_indicator_is_clickable() {
        for value in [Some(0.0), None] {
            let ctx = egui::Context::default();
            let color = Color32::from_rgb(5, 180, 80);
            let mut rect = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                rect = load_meter(ui, value, color, true).rect;
            });
            output.textures_delta.clear();
            assert_eq!(rect.width(), 47.0);
            assert!((rect.height() - 23.0 * 0.8).abs() < 0.01);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.color == color)));
            for pressed in [true, false] {
                let mut clicked = false;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events: vec![
                            egui::Event::PointerMoved(rect.right_center() - egui::vec2(2.0, 0.0)),
                            egui::Event::PointerButton {
                                pos: rect.right_center() - egui::vec2(2.0, 0.0),
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| clicked = load_meter(ui, value, color, true).clicked(),
                );
                output.textures_delta.clear();
                if !pressed {
                    assert!(clicked, "percentage must share the bar's hit target");
                }
            }
        }
    }
}
