use eframe::egui::{self, Color32, Frame, Shadow, Stroke};

pub const BUTTON_FONT_SIZE: f32 = 11.5;

pub fn apply_widget_style(style: &mut egui::Style) {
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(18.0));
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(BUTTON_FONT_SIZE),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(BUTTON_FONT_SIZE),
    );
    style.spacing.item_spacing = egui::vec2(6.0, 2.0);
    style.spacing.button_padding = egui::vec2(7.0, 2.5);
    style.spacing.interact_size.y = 22.0;
}

pub fn apply_widget_visuals(visuals: &mut egui::Visuals) {
    let dark_mode = visuals.dark_mode;
    let radius = egui::CornerRadius::same(6);

    let (inactive_fill, inactive_border, hovered_fill, hovered_border, active_fill) = if dark_mode {
        (
            Color32::from_rgba_unmultiplied(70, 70, 70, 150),
            Color32::from_rgb(82, 82, 82),
            Color32::from_rgba_unmultiplied(88, 88, 88, 190),
            Color32::from_rgb(112, 112, 112),
            Color32::from_rgba_unmultiplied(98, 98, 98, 220),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(224, 226, 230, 170),
            Color32::from_rgb(196, 200, 207),
            Color32::from_rgba_unmultiplied(210, 214, 220, 210),
            Color32::from_rgb(168, 174, 184),
            Color32::from_rgba_unmultiplied(196, 201, 209, 230),
        )
    };

    visuals.widgets.inactive.weak_bg_fill = inactive_fill;
    visuals.widgets.inactive.bg_fill = inactive_fill;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, inactive_border);
    visuals.widgets.inactive.corner_radius = radius;

    visuals.widgets.hovered.weak_bg_fill = hovered_fill;
    visuals.widgets.hovered.bg_fill = hovered_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, hovered_border);
    visuals.widgets.hovered.corner_radius = radius;

    visuals.widgets.active.weak_bg_fill = active_fill;
    visuals.widgets.active.bg_fill = active_fill;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, hovered_border);
    visuals.widgets.active.corner_radius = radius;

    visuals.widgets.open.weak_bg_fill = hovered_fill;
    visuals.widgets.open.bg_fill = hovered_fill;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, hovered_border);
    visuals.widgets.open.corner_radius = radius;

    visuals.selection.bg_fill = if dark_mode {
        Color32::from_rgb(84, 84, 84)
    } else {
        Color32::from_rgb(190, 194, 201)
    };
    visuals.selection.stroke = Stroke::new(
        1.0,
        if dark_mode {
            Color32::from_rgb(226, 226, 226)
        } else {
            Color32::from_rgb(50, 52, 56)
        },
    );
}

pub fn glass_frame(ui: &egui::Ui) -> Frame {
    Frame::NONE
        .fill(surface_fill(ui))
        .stroke(Stroke::new(1.0, subtle_border(ui)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10))
}

pub fn group_frame(ui: &egui::Ui) -> Frame {
    let shadow = Shadow {
        offset: [0, 2],
        blur: 6,
        spread: 0,
        color: Color32::from_black_alpha(if ui.visuals().dark_mode { 35 } else { 16 }),
    };

    Frame::NONE
        .fill(group_fill(ui))
        .stroke(Stroke::new(1.0, strong_border(ui)))
        .shadow(shadow)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10))
        .outer_margin(egui::Margin::symmetric(2, 3))
}

pub fn inset_frame(ui: &egui::Ui) -> Frame {
    Frame::NONE
        .fill(inset_fill(ui))
        .stroke(Stroke::new(1.0, subtle_border(ui)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 5))
}

pub fn success_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(72, 205, 132)
    } else {
        Color32::from_rgb(25, 132, 76)
    }
}

pub fn warning_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(240, 190, 70)
    } else {
        Color32::from_rgb(166, 105, 0)
    }
}

pub fn danger_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(235, 95, 105)
    } else {
        Color32::from_rgb(190, 45, 58)
    }
}

pub fn neutral_emphasis_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(66, 66, 66)
    } else {
        Color32::from_rgb(218, 218, 218)
    }
}

pub fn row_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(42, 42, 42)
    } else {
        Color32::from_rgb(238, 238, 238)
    }
}

pub fn row_hover_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(52, 52, 52)
    } else {
        Color32::from_rgb(228, 228, 228)
    }
}

fn surface_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(29, 29, 29)
    } else {
        Color32::from_rgb(248, 249, 251)
    }
}

fn group_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(32, 32, 32)
    } else {
        Color32::from_rgb(252, 252, 253)
    }
}

fn inset_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(24, 24, 24)
    } else {
        Color32::from_rgb(242, 244, 247)
    }
}

fn subtle_border(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(66, 66, 66)
    } else {
        Color32::from_rgb(205, 209, 217)
    }
}

fn strong_border(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(98, 98, 98)
    } else {
        Color32::from_rgb(163, 170, 183)
    }
}
