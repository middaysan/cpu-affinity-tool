use eframe::egui::{self, Color32, Frame, Shadow, Stroke};

pub fn glass_frame(ui: &egui::Ui) -> Frame {
    let v = ui.visuals();
    let dark_mode = v.dark_mode;

    let tint = if dark_mode {
        Color32::from_rgba_unmultiplied(255, 255, 255, 14)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 20)
    };

    let fill = tint;

    let stroke_color = if dark_mode {
        Color32::from_rgba_unmultiplied(255, 255, 255, 55)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 60)
    };
    let stroke = Stroke::new(1.0, stroke_color);

    let shadow_alpha = if dark_mode { 40 } else { 30 };
    let shadow = Shadow {
        offset: [0, 10],
        blur: 16,
        spread: 2,
        color: Color32::from_black_alpha(shadow_alpha),
    };

    Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .shadow(shadow)
        .corner_radius(7.0)
        .inner_margin(egui::Margin::same(15))
}
