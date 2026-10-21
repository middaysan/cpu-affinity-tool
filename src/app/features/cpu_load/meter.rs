use crate::app::shell::presenters::shared_elements::{
    inter_medium_family, paint_focus_ring, palette, status_button, STATUS_BUTTON_FONT_SIZE,
};
use eframe::egui::{self, Color32};

#[derive(Default)]
pub(super) struct MeterLoad {
    pub value: Option<f32>,
    pub partial: bool,
}

impl From<Option<f32>> for MeterLoad {
    fn from(value: Option<f32>) -> Self {
        Self {
            value,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MeterKind {
    System,
    Group,
    Application,
}

impl MeterKind {
    fn height(self) -> f32 {
        match self {
            Self::System => 21.0,
            Self::Group => 30.0,
            Self::Application => 18.0,
        }
    }

    fn meter_rect(self, button: egui::Rect) -> egui::Rect {
        if self == Self::Group {
            egui::Rect::from_min_size(button.min + egui::vec2(4.0, 3.0), egui::vec2(70.0, 13.5))
        } else {
            button
        }
    }
}

/// A compact inline group/application graph button.
pub(super) fn load_meter(
    ui: &mut egui::Ui,
    load: MeterLoad,
    color: Color32,
    interactive: bool,
    kind: MeterKind,
) -> egui::Response {
    let (button_rect, response) = if kind == MeterKind::System {
        let response = ui
            .add_enabled_ui(interactive, |ui| {
                status_button(ui, [ui.available_width(), kind.height()], "", color)
            })
            .inner;
        (response.rect, response)
    } else {
        ui.allocate_exact_size(
            egui::vec2(
                match kind {
                    MeterKind::System => ui.available_width().max(1.0),
                    MeterKind::Group => 78.0,
                    MeterKind::Application => 44.0,
                },
                kind.height(),
            ),
            if interactive {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        )
    };
    response.widget_info(|| {
        let value = super::view::measured_percent(load.value, load.partial);
        let label = match kind {
            MeterKind::System => {
                format!("All logical CPUs, all programs: {value}. Open system CPU graphs")
            }
            MeterKind::Group => format!("Tracked apps CPU load {value}. Open group CPU graphs"),
            MeterKind::Application if interactive => {
                format!("App CPU load {value}. Open CPU graphs")
            }
            MeterKind::Application => format!("App CPU load {value}. Application is stopped"),
        };
        egui::WidgetInfo::labeled(egui::WidgetType::Button, interactive, label)
    });
    paint_meter(
        ui,
        button_rect,
        load,
        color,
        interactive,
        kind,
        (response.hovered(), response.is_pointer_button_down_on()),
    );
    if interactive {
        paint_focus_ring(ui, &response);
        return response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    response
}

/// Reuse the summary painting without introducing a nested button or focus target.
pub(super) fn paint_system_summary(
    ui: &egui::Ui,
    rect: egui::Rect,
    value: Option<f32>,
    color: Color32,
    feedback: (bool, bool),
) {
    paint_meter(
        ui,
        rect,
        value.into(),
        color,
        true,
        MeterKind::System,
        feedback,
    );
}

fn paint_meter(
    ui: &egui::Ui,
    button_rect: egui::Rect,
    load: MeterLoad,
    color: Color32,
    interactive: bool,
    kind: MeterKind,
    feedback: (bool, bool),
) {
    let rect = kind.meter_rect(button_rect);
    let colors = *palette(ui);
    let hovered = interactive && feedback.0;
    let pressed = interactive && feedback.1;
    if kind == MeterKind::Group {
        ui.painter().rect_filled(
            button_rect,
            5.0,
            colors.group.lerp_to_gamma(
                color,
                if pressed {
                    0.14
                } else if hovered {
                    0.08
                } else {
                    0.02
                },
            ),
        );
        ui.painter().rect_stroke(
            button_rect,
            5.0,
            egui::Stroke::new(
                0.75,
                if pressed {
                    color
                } else if hovered {
                    colors.border_strong
                } else {
                    colors.border_subtle
                },
            ),
            egui::StrokeKind::Inside,
        );
    }
    let background = if kind == MeterKind::System {
        if pressed {
            colors.visibility.active_fill
        } else if hovered {
            colors.visibility.hover_fill
        } else {
            colors.visibility.fill
        }
    } else if !interactive {
        colors.row.lerp_to_gamma(colors.inset, 0.25)
    } else {
        colors.inset.lerp_to_gamma(
            color,
            if pressed {
                0.24
            } else if hovered {
                0.16
            } else if interactive {
                0.08
            } else {
                0.0
            },
        )
    };
    let summary = kind != MeterKind::Application;
    let radius = match kind {
        MeterKind::System => 5.0,
        MeterKind::Group => rect.height() * 0.5,
        MeterKind::Application => 3.0,
    };
    if kind != MeterKind::System {
        ui.painter().rect_filled(rect, radius, background);
    }
    let row = rect.shrink(1.0);
    let level = load
        .value
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 100.0)
        / 100.0;
    if level > 0.0 {
        let fill =
            egui::Rect::from_min_size(row.min, egui::vec2(row.width() * level, row.height()));
        let fill_color =
            color.lerp_to_gamma(background, if ui.visuals().dark_mode { 0.55 } else { 0.7 });
        if summary {
            ui.painter()
                .with_clip_rect(fill.intersect(ui.clip_rect()))
                .rect_filled(row, (radius - 1.0).max(0.0), fill_color);
        } else {
            ui.painter().rect_filled(fill, 2.0, fill_color);
        }
    }
    let painter = ui.painter().with_clip_rect(row.intersect(ui.clip_rect()));
    if kind == MeterKind::System {
        painter.text(
            egui::pos2(row.left() + 4.0, row.center().y),
            egui::Align2::LEFT_CENTER,
            "CPU load",
            egui::FontId::new(STATUS_BUTTON_FONT_SIZE, inter_medium_family()),
            color,
        );
    }
    let compact = kind == MeterKind::Application;
    painter.text(
        if compact {
            row.center()
        } else {
            egui::pos2(row.right() - 4.0, row.center().y)
        },
        if compact {
            egui::Align2::CENTER_CENTER
        } else {
            egui::Align2::RIGHT_CENTER
        },
        super::view::measured_percent(load.value, load.partial),
        egui::FontId::proportional(if kind == MeterKind::Group { 9.0 } else { 9.5 }),
        if interactive {
            colors.text_primary
        } else {
            colors.text_muted.lerp_to_gamma(colors.row, 0.3)
        },
    );
    if kind != MeterKind::System {
        ui.painter().rect_stroke(
            rect,
            radius,
            egui::Stroke::new(
                if pressed {
                    1.5
                } else if hovered {
                    1.25
                } else {
                    0.75
                },
                if interactive {
                    color
                } else {
                    colors.border_subtle
                },
            ),
            egui::StrokeKind::Inside,
        );
    }
    if kind == MeterKind::Group {
        ui.painter().text(
            egui::pos2(
                button_rect.center().x,
                (rect.bottom() + button_rect.bottom() - 2.0) * 0.5,
            ),
            egui::Align2::CENTER_CENTER,
            "Group CPU load",
            egui::FontId::proportional(8.5),
            colors.text_muted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_meter_caption_and_pill_share_one_outlined_button() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.set_width(320.0);
            let response = load_meter(
                ui,
                MeterLoad::default(),
                Color32::GRAY,
                true,
                MeterKind::Group,
            );
            assert_eq!(response.rect.size(), egui::vec2(78.0, 30.0));
            assert_eq!(
                MeterKind::Group.meter_rect(response.rect).size(),
                egui::vec2(70.0, 13.5)
            );
        });
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        assert!(!labels.contains(&"Tracked"));
        assert!(!labels.contains(&"All processes"));
        assert!(labels.contains(&"Group CPU load"));
        assert_eq!(labels.iter().filter(|&&label| label == "—").count(), 1);
    }

    #[test]
    fn system_and_group_share_fill_policy_with_context_specific_corners() {
        for kind in [MeterKind::System, MeterKind::Group] {
            for dark in [true, false] {
                let ctx = egui::Context::default();
                crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut rect = egui::Rect::NOTHING;
                let mut output = ctx.run_ui(Default::default(), |ui| {
                    ui.set_width(if kind == MeterKind::System {
                        82.0
                    } else {
                        70.0
                    });
                    let button =
                        load_meter(ui, Some(100.0).into(), Color32::GREEN, true, kind).rect;
                    rect = kind.meter_rect(button);
                });
                output.textures_delta.clear();
                assert_eq!(
                    rect.width(),
                    if kind == MeterKind::System {
                        82.0
                    } else {
                        70.0
                    }
                );
                let background = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(shape)
                            if shape.rect == rect && shape.fill != Color32::TRANSPARENT =>
                        {
                            Some(shape)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(
                    (f32::from(background.corner_radius.nw)
                        - if kind == MeterKind::System {
                            5.0
                        } else {
                            rect.height() * 0.5
                        })
                    .abs()
                        <= 0.5
                );
                let texts: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if !text.galley.text().is_empty() => Some(text),
                        _ => None,
                    })
                    .collect();
                let value = texts
                    .iter()
                    .find(|text| text.galley.text() == "100%")
                    .unwrap();
                if kind == MeterKind::System {
                    assert_eq!(rect.height(), 21.0);
                    assert_eq!(texts.len(), 2);
                    let title = texts
                        .iter()
                        .find(|text| text.galley.text() == "CPU load")
                        .unwrap();
                    assert!(title.pos.x + title.galley.size().x + 3.0 < value.pos.x);
                    assert!(value.pos.x + value.galley.size().x < rect.right());
                } else {
                    assert_eq!(rect.height(), 13.5);
                    assert_eq!(texts.len(), 2);
                    let caption = texts
                        .iter()
                        .find(|text| text.galley.text() == "Group CPU load")
                        .unwrap();
                    assert!(caption.pos.y > rect.bottom());
                }
            }
        }
    }

    #[test]
    fn meter_keeps_compact_size_with_full_height_fill_and_centered_percentage() {
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
                    ui.set_width(320.0);
                    rect = load_meter(ui, value.into(), color, true, MeterKind::Application).rect;
                });
                output.textures_delta.clear();
                assert_eq!(rect.width(), 44.0);
                assert!(
                    (rect.height() - 18.0).abs() < 0.01,
                    "application strips have one compact lane"
                );
                let lane = rect;
                let inner = lane.shrink(1.0);
                let fill = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(shape)
                        if (shape.rect.height() - inner.height()).abs() < 0.01 =>
                    {
                        Some(shape.rect)
                    }
                    _ => None,
                });
                if let Some(value) = value.filter(|value| *value > 0.0) {
                    let fill = fill.expect("positive CPU load must fill the button");
                    assert!((fill.left() - inner.left()).abs() < 0.01);
                    assert!((fill.top() - inner.top()).abs() < 0.01);
                    assert!((fill.width() - inner.width() * value / 100.0).abs() < 0.01);
                } else {
                    assert!(
                        fill.is_none(),
                        "zero and unavailable load must not invent fill"
                    );
                }
                assert!(
                    !output
                        .shapes
                        .iter()
                        .any(|shape| matches!(&shape.shape, egui::Shape::Mesh(_))),
                    "load lanes use flat fills"
                );
                let text = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(t)
                            if t.galley.text() == super::super::view::percent(value) =>
                        {
                            Some(t)
                        }
                        _ => None,
                    })
                    .unwrap();
                let text_rect = text.galley.rect.translate(text.pos.to_vec2());
                assert!(
                    (text_rect.center().x - lane.center().x).abs() < 1.0,
                    "keep compact app percentage centered"
                );
            }
        }
    }

    #[test]
    fn stopped_app_meter_uses_muted_neutral_decoration() {
        for dark in [true, false] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut rect = egui::Rect::NOTHING;
            let mut colors = None;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                colors = Some(*palette(ui));
                let response = load_meter(
                    ui,
                    None.into(),
                    Color32::GREEN,
                    false,
                    MeterKind::Application,
                );
                rect = response.rect;
                assert_eq!(response.sense, egui::Sense::hover());
            });
            output.textures_delta.clear();
            let colors = colors.unwrap();
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(shape) if shape.rect == rect && shape.stroke.color == colors.border_subtle)));
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "—" && text.fallback_color == colors.text_muted.lerp_to_gamma(colors.row, 0.3))));
        }
    }

    #[test]
    fn zero_and_unknown_keep_status_and_whole_indicator_is_clickable() {
        for value in [Some(0.0), None] {
            let ctx = egui::Context::default();
            let color = Color32::from_rgb(5, 180, 80);
            let mut rect = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(320.0);
                rect = load_meter(ui, value.into(), color, true, MeterKind::Application).rect;
            });
            output.textures_delta.clear();
            assert_eq!(rect.width(), 44.0);
            assert!((rect.height() - 18.0).abs() < 0.01);
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
                    |ui| {
                        ui.set_width(320.0);
                        clicked = load_meter(ui, value.into(), color, true, MeterKind::Application)
                            .clicked()
                    },
                );
                output.textures_delta.clear();
                if !pressed {
                    assert!(clicked, "percentage must share the bar's hit target");
                }
            }
        }
    }
}
