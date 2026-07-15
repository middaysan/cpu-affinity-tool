use eframe::egui::{self, Color32, Frame, Shadow, Stroke};
use std::sync::Arc;

pub const BUTTON_FONT_SIZE: f32 = 10.5;
pub const INTER_REGULAR_FONT_NAME: &str = "Inter Regular";
pub const INTER_MEDIUM_FONT_NAME: &str = "Inter Medium";
pub const INTER_SEMIBOLD_FONT_NAME: &str = "Inter Semibold";
pub const INTER_MEDIUM_FAMILY_NAME: &str = "Inter Medium";
pub const INTER_SEMIBOLD_FAMILY_NAME: &str = "Inter Semibold";

const INTER_VARIABLE_FONT: &[u8] = include_bytes!("../../../assets/fonts/InterVariable.ttf");

fn inter_font_data(weight: f32) -> egui::FontData {
    egui::FontData::from_static(INTER_VARIABLE_FONT).tweak(egui::FontTweak {
        coords: egui::epaint::text::VariationCoords::new([(b"wght", weight)]),
        ..Default::default()
    })
}

pub fn inter_medium_family() -> egui::FontFamily {
    egui::FontFamily::Name(INTER_MEDIUM_FAMILY_NAME.into())
}

pub fn inter_semibold_family() -> egui::FontFamily {
    egui::FontFamily::Name(INTER_SEMIBOLD_FAMILY_NAME.into())
}

pub fn ui_font_definitions() -> egui::FontDefinitions {
    let mut definitions = egui::FontDefinitions::default();
    let default_fallbacks = definitions
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    for (name, weight) in [
        (INTER_REGULAR_FONT_NAME, 400.0),
        (INTER_MEDIUM_FONT_NAME, 500.0),
        (INTER_SEMIBOLD_FONT_NAME, 600.0),
    ] {
        definitions
            .font_data
            .insert(name.to_owned(), Arc::new(inter_font_data(weight)));
    }

    let family_with_fallbacks = |primary: &str| {
        let mut family = Vec::with_capacity(default_fallbacks.len() + 1);
        family.push(primary.to_owned());
        family.extend(default_fallbacks.iter().cloned());
        family
    };

    definitions.families.insert(
        egui::FontFamily::Proportional,
        family_with_fallbacks(INTER_REGULAR_FONT_NAME),
    );
    definitions.families.insert(
        inter_medium_family(),
        family_with_fallbacks(INTER_MEDIUM_FONT_NAME),
    );
    definitions.families.insert(
        inter_semibold_family(),
        family_with_fallbacks(INTER_SEMIBOLD_FONT_NAME),
    );

    definitions
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToneTokens {
    pub fg: Color32,
    pub fill: Color32,
    pub hover_fill: Color32,
    pub active_fill: Color32,
    pub border: Color32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneRole {
    Selected,
    Primary,
    Warning,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPalette {
    pub surface: Color32,
    pub group: Color32,
    pub inset: Color32,
    pub row: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,
    pub neutral_status: Color32,
    pub accent: ToneTokens,
    pub selected: ToneTokens,
    pub primary: ToneTokens,
    pub visibility: ToneTokens,
    pub success: ToneTokens,
    pub warning: ToneTokens,
    pub danger: ToneTokens,
    pub drag_idle: Color32,
    pub drag_active: Color32,
    pub drop_target: Color32,
    pub monitoring_surface: Color32,
    pub monitoring_border: Color32,
    pub core_performance: Color32,
    pub core_efficiency: Color32,
    pub core_hyper_thread: Color32,
    pub core_other: Color32,
    pub core_all: Color32,
}

const DARK_PALETTE: UiPalette = UiPalette {
    surface: Color32::from_rgb(15, 17, 19),
    group: Color32::from_rgb(21, 24, 27),
    inset: Color32::from_rgb(27, 31, 35),
    row: Color32::from_rgb(30, 34, 38),
    text_primary: Color32::from_rgb(237, 241, 243),
    text_secondary: Color32::from_rgb(163, 171, 178),
    text_muted: Color32::from_rgb(132, 141, 149),
    border_subtle: Color32::from_rgb(43, 48, 53),
    border_strong: Color32::from_rgb(57, 65, 72),
    neutral_status: Color32::from_rgb(132, 141, 149),
    accent: ToneTokens {
        fg: Color32::from_rgb(128, 153, 164),
        fill: Color32::from_rgb(20, 30, 35),
        hover_fill: Color32::from_rgb(27, 42, 49),
        active_fill: Color32::from_rgb(17, 25, 29),
        border: Color32::from_rgb(57, 88, 102),
    },
    selected: ToneTokens {
        fg: Color32::from_rgb(237, 241, 243),
        fill: Color32::from_rgb(25, 39, 46),
        hover_fill: Color32::from_rgb(32, 50, 58),
        active_fill: Color32::from_rgb(20, 30, 35),
        border: Color32::from_rgb(57, 88, 102),
    },
    primary: ToneTokens {
        fg: Color32::from_rgb(231, 238, 241),
        fill: Color32::from_rgb(41, 67, 79),
        hover_fill: Color32::from_rgb(49, 77, 89),
        active_fill: Color32::from_rgb(34, 57, 67),
        border: Color32::from_rgb(57, 88, 102),
    },
    visibility: ToneTokens {
        fg: Color32::from_rgb(163, 171, 178),
        fill: Color32::from_rgb(27, 31, 35),
        hover_fill: Color32::from_rgb(37, 42, 47),
        active_fill: Color32::from_rgb(21, 24, 27),
        border: Color32::from_rgb(57, 65, 72),
    },
    success: ToneTokens {
        fg: Color32::from_rgb(131, 187, 156),
        fill: Color32::from_rgb(23, 36, 29),
        hover_fill: Color32::from_rgb(29, 48, 38),
        active_fill: Color32::from_rgb(18, 29, 24),
        border: Color32::from_rgb(53, 91, 69),
    },
    warning: ToneTokens {
        fg: Color32::from_rgb(198, 167, 108),
        fill: Color32::from_rgb(42, 35, 23),
        hover_fill: Color32::from_rgb(55, 46, 28),
        active_fill: Color32::from_rgb(34, 29, 19),
        border: Color32::from_rgb(109, 89, 47),
    },
    danger: ToneTokens {
        fg: Color32::from_rgb(207, 137, 144),
        fill: Color32::from_rgb(41, 26, 29),
        hover_fill: Color32::from_rgb(55, 35, 39),
        active_fill: Color32::from_rgb(34, 22, 24),
        border: Color32::from_rgb(112, 65, 72),
    },
    drag_idle: Color32::from_rgb(112, 121, 129),
    drag_active: Color32::from_rgb(128, 153, 164),
    drop_target: Color32::from_rgb(96, 126, 139),
    monitoring_surface: Color32::from_rgb(21, 24, 27),
    monitoring_border: Color32::from_rgb(43, 48, 53),
    core_performance: Color32::from_rgb(20, 30, 35),
    core_efficiency: Color32::from_rgb(21, 35, 30),
    core_hyper_thread: Color32::from_rgb(40, 33, 22),
    core_other: Color32::from_rgb(27, 31, 35),
    core_all: Color32::from_rgb(25, 39, 46),
};

const LIGHT_PALETTE: UiPalette = UiPalette {
    surface: Color32::from_rgb(242, 244, 245),
    group: Color32::from_rgb(250, 251, 252),
    inset: Color32::from_rgb(233, 237, 239),
    row: Color32::from_rgb(236, 239, 241),
    text_primary: Color32::from_rgb(29, 37, 43),
    text_secondary: Color32::from_rgb(86, 99, 108),
    text_muted: Color32::from_rgb(94, 105, 113),
    border_subtle: Color32::from_rgb(203, 210, 215),
    border_strong: Color32::from_rgb(174, 185, 192),
    neutral_status: Color32::from_rgb(94, 105, 113),
    accent: ToneTokens {
        fg: Color32::from_rgb(59, 89, 101),
        fill: Color32::from_rgb(224, 232, 235),
        hover_fill: Color32::from_rgb(203, 218, 221),
        active_fill: Color32::from_rgb(189, 206, 213),
        border: Color32::from_rgb(138, 160, 169),
    },
    selected: ToneTokens {
        fg: Color32::from_rgb(29, 37, 43),
        fill: Color32::from_rgb(214, 225, 229),
        hover_fill: Color32::from_rgb(203, 218, 221),
        active_fill: Color32::from_rgb(189, 206, 213),
        border: Color32::from_rgb(138, 160, 169),
    },
    primary: ToneTokens {
        fg: Color32::from_rgb(247, 251, 252),
        fill: Color32::from_rgb(68, 100, 114),
        hover_fill: Color32::from_rgb(78, 113, 128),
        active_fill: Color32::from_rgb(56, 86, 99),
        border: Color32::from_rgb(138, 160, 169),
    },
    visibility: ToneTokens {
        fg: Color32::from_rgb(86, 99, 108),
        fill: Color32::from_rgb(233, 237, 239),
        hover_fill: Color32::from_rgb(226, 231, 234),
        active_fill: Color32::from_rgb(214, 221, 225),
        border: Color32::from_rgb(174, 185, 192),
    },
    success: ToneTokens {
        fg: Color32::from_rgb(36, 109, 73),
        fill: Color32::from_rgb(227, 238, 231),
        hover_fill: Color32::from_rgb(213, 229, 219),
        active_fill: Color32::from_rgb(213, 229, 219),
        border: Color32::from_rgb(145, 179, 159),
    },
    warning: ToneTokens {
        fg: Color32::from_rgb(119, 87, 29),
        fill: Color32::from_rgb(241, 234, 219),
        hover_fill: Color32::from_rgb(231, 219, 194),
        active_fill: Color32::from_rgb(231, 219, 194),
        border: Color32::from_rgb(196, 173, 120),
    },
    danger: ToneTokens {
        fg: Color32::from_rgb(150, 62, 72),
        fill: Color32::from_rgb(242, 226, 228),
        hover_fill: Color32::from_rgb(232, 210, 213),
        active_fill: Color32::from_rgb(232, 210, 213),
        border: Color32::from_rgb(201, 151, 156),
    },
    drag_idle: Color32::from_rgb(123, 135, 143),
    drag_active: Color32::from_rgb(73, 104, 117),
    drop_target: Color32::from_rgb(88, 118, 130),
    monitoring_surface: Color32::from_rgb(250, 251, 252),
    monitoring_border: Color32::from_rgb(203, 210, 215),
    core_performance: Color32::from_rgb(224, 232, 235),
    core_efficiency: Color32::from_rgb(225, 236, 230),
    core_hyper_thread: Color32::from_rgb(239, 230, 214),
    core_other: Color32::from_rgb(233, 237, 239),
    core_all: Color32::from_rgb(214, 225, 229),
};

pub const fn palette_for_dark_mode(dark_mode: bool) -> &'static UiPalette {
    if dark_mode {
        &DARK_PALETTE
    } else {
        &LIGHT_PALETTE
    }
}

pub fn palette(ui: &egui::Ui) -> &'static UiPalette {
    palette_for_dark_mode(ui.visuals().dark_mode)
}

pub fn tone(ui: &egui::Ui, role: ToneRole) -> ToneTokens {
    let palette = palette(ui);
    match role {
        ToneRole::Selected => palette.selected,
        ToneRole::Primary => palette.primary,
        ToneRole::Warning => palette.warning,
        ToneRole::Danger => palette.danger,
    }
}

pub fn apply_widget_style(style: &mut egui::Style) {
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(16.0, inter_semibold_family()),
    );
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(9.5));
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(BUTTON_FONT_SIZE),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(BUTTON_FONT_SIZE),
    );
    style.spacing.item_spacing = egui::vec2(4.0, 1.0);
    style.spacing.button_padding = egui::vec2(6.0, 2.0);
    style.spacing.interact_size.y = 21.0;
}

pub fn apply_widget_visuals(visuals: &mut egui::Visuals) {
    let dark_mode = visuals.dark_mode;
    let palette = palette_for_dark_mode(dark_mode);
    let radius = egui::CornerRadius::same(5);

    let (inactive_fill, inactive_border, hovered_fill, hovered_border, active_fill) = if dark_mode {
        (
            Color32::from_rgb(27, 31, 35),
            Color32::from_rgb(57, 65, 72),
            Color32::from_rgb(37, 42, 47),
            Color32::from_rgb(91, 103, 112),
            Color32::from_rgb(21, 24, 27),
        )
    } else {
        (
            Color32::from_rgb(233, 237, 239),
            Color32::from_rgb(174, 185, 192),
            Color32::from_rgb(226, 231, 234),
            Color32::from_rgb(123, 135, 143),
            Color32::from_rgb(214, 221, 225),
        )
    };

    visuals.panel_fill = palette.surface;
    visuals.window_fill = palette.group;
    visuals.window_stroke = Stroke::new(1.0, palette.border_strong);
    visuals.faint_bg_color = palette.row;
    visuals.extreme_bg_color = palette.inset;
    visuals.code_bg_color = palette.inset;
    visuals.widgets.noninteractive.fg_stroke.color = palette.text_primary;

    visuals.widgets.inactive.weak_bg_fill = inactive_fill;
    visuals.widgets.inactive.bg_fill = inactive_fill;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, inactive_border);
    visuals.widgets.inactive.fg_stroke.color = palette.text_secondary;
    visuals.widgets.inactive.corner_radius = radius;

    visuals.widgets.hovered.weak_bg_fill = hovered_fill;
    visuals.widgets.hovered.bg_fill = hovered_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, hovered_border);
    visuals.widgets.hovered.fg_stroke.color = palette.text_primary;
    visuals.widgets.hovered.corner_radius = radius;

    visuals.widgets.active.weak_bg_fill = active_fill;
    visuals.widgets.active.bg_fill = active_fill;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, hovered_border);
    visuals.widgets.active.fg_stroke.color = palette.text_primary;
    visuals.widgets.active.corner_radius = radius;

    visuals.widgets.open.weak_bg_fill = hovered_fill;
    visuals.widgets.open.bg_fill = hovered_fill;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, hovered_border);
    visuals.widgets.open.fg_stroke.color = palette.text_primary;
    visuals.widgets.open.corner_radius = radius;

    visuals.selection.bg_fill = palette.selected.fill;
    visuals.selection.stroke = Stroke::new(1.0, palette.selected.fg);
}

fn apply_tone_to_visuals(visuals: &mut egui::Visuals, tokens: ToneTokens) {
    for (widget, fill) in [
        (&mut visuals.widgets.inactive, tokens.fill),
        (&mut visuals.widgets.hovered, tokens.hover_fill),
        (&mut visuals.widgets.active, tokens.active_fill),
        (&mut visuals.widgets.open, tokens.hover_fill),
    ] {
        widget.weak_bg_fill = fill;
        widget.bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, tokens.border);
        widget.fg_stroke.color = tokens.fg;
    }
    visuals.selection.bg_fill = tokens.fill;
    visuals.selection.stroke = Stroke::new(1.0, tokens.fg);
}

fn apply_ghost_to_visuals(visuals: &mut egui::Visuals, colors: &UiPalette) {
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke.color = colors.text_secondary;

    visuals.widgets.hovered.weak_bg_fill = colors.row;
    visuals.widgets.hovered.bg_fill = colors.row;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.fg_stroke.color = colors.text_primary;

    visuals.widgets.active.weak_bg_fill = colors.inset;
    visuals.widgets.active.bg_fill = colors.inset;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.fg_stroke.color = colors.text_primary;
}

pub fn toned_button(ui: &mut egui::Ui, button: egui::Button<'_>, role: ToneRole) -> egui::Response {
    let tokens = tone(ui, role);
    toned_button_with_tokens(ui, button, tokens)
}

pub fn toned_button_with_tokens(
    ui: &mut egui::Ui,
    button: egui::Button<'_>,
    tokens: ToneTokens,
) -> egui::Response {
    let response = ui
        .scope(|ui| {
            apply_tone_to_visuals(&mut ui.style_mut().visuals, tokens);
            ui.add(button)
        })
        .inner;
    paint_focus_ring(ui, &response);
    response
}

pub fn toned_sized_button(
    ui: &mut egui::Ui,
    size: impl Into<egui::Vec2>,
    button: egui::Button<'_>,
    role: ToneRole,
) -> egui::Response {
    let tokens = tone(ui, role);
    toned_sized_button_with_tokens(ui, size, button, tokens)
}

pub fn toned_sized_button_with_tokens(
    ui: &mut egui::Ui,
    size: impl Into<egui::Vec2>,
    button: egui::Button<'_>,
    tokens: ToneTokens,
) -> egui::Response {
    let response = ui
        .scope(|ui| {
            apply_tone_to_visuals(&mut ui.style_mut().visuals, tokens);
            ui.add_sized(size, button)
        })
        .inner;
    paint_focus_ring(ui, &response);
    response
}

pub fn ghost_button(ui: &mut egui::Ui, button: egui::Button<'_>) -> egui::Response {
    let colors = *palette(ui);
    let response = ui
        .scope(|ui| {
            apply_ghost_to_visuals(&mut ui.style_mut().visuals, &colors);
            ui.add(button)
        })
        .inner;
    paint_focus_ring(ui, &response);
    response
}

pub fn paint_focus_ring(ui: &egui::Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            7.0,
            Stroke::new(2.0, palette(ui).drop_target),
            egui::StrokeKind::Outside,
        );
    }
}

pub fn selected_tone_feedback_stroke(
    tokens: ToneTokens,
    hovered: bool,
    pressed: bool,
) -> Option<Stroke> {
    if pressed {
        Some(Stroke::new(2.0, tokens.fg))
    } else if hovered {
        Some(Stroke::new(1.5, tokens.fg))
    } else {
        None
    }
}

pub fn paint_selected_tone_feedback(ui: &egui::Ui, response: &egui::Response, tokens: ToneTokens) {
    if let Some(stroke) = selected_tone_feedback_stroke(
        tokens,
        response.hovered(),
        response.is_pointer_button_down_on(),
    ) {
        ui.painter().rect_stroke(
            response.rect.shrink(stroke.width * 0.5),
            5.0,
            stroke,
            egui::StrokeKind::Inside,
        );
    }
}

pub fn glass_frame(ui: &egui::Ui) -> Frame {
    Frame::NONE
        .fill(surface_fill(ui))
        .stroke(Stroke::new(1.0, subtle_border(ui)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(8))
}

pub fn group_frame(ui: &egui::Ui) -> Frame {
    let shadow = Shadow {
        offset: [0, 2],
        blur: 4,
        spread: 0,
        color: Color32::from_black_alpha(if ui.visuals().dark_mode { 24 } else { 12 }),
    };

    Frame::NONE
        .fill(group_fill(ui))
        .stroke(Stroke::new(1.0, strong_border(ui)))
        .shadow(shadow)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(7))
        .outer_margin(egui::Margin::symmetric(1, 2))
}

pub fn inset_frame(ui: &egui::Ui) -> Frame {
    Frame::NONE
        .fill(inset_fill(ui))
        .stroke(Stroke::new(1.0, subtle_border(ui)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(6, 4))
}

pub fn success_color(ui: &egui::Ui) -> Color32 {
    palette(ui).success.fg
}

pub fn warning_color(ui: &egui::Ui) -> Color32 {
    palette(ui).warning.fg
}

pub fn danger_color(ui: &egui::Ui) -> Color32 {
    palette(ui).danger.fg
}

pub fn row_fill(ui: &egui::Ui) -> Color32 {
    palette(ui).row
}

pub fn drag_grip(ui: &mut egui::Ui, accessible_label: &'static str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(12.0, 20.0), egui::Sense::drag());
    let response = response
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text(accessible_label);
    let colors = palette(ui);
    let color = if response.hovered() || response.dragged() {
        colors.drag_active
    } else {
        colors.drag_idle
    };
    for x in [-2.0, 2.0] {
        for y in [-3.0, 0.0, 3.0] {
            ui.painter()
                .circle_filled(rect.center() + egui::vec2(x, y), 1.0, color);
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, accessible_label)
    });
    paint_focus_ring(ui, &response);
    response
}

fn surface_fill(ui: &egui::Ui) -> Color32 {
    palette(ui).surface
}

fn group_fill(ui: &egui::Ui) -> Color32 {
    palette(ui).group
}

fn inset_fill(ui: &egui::Ui) -> Color32 {
    palette(ui).inset
}

fn subtle_border(ui: &egui::Ui) -> Color32 {
    palette(ui).border_subtle
}

fn strong_border(ui: &egui::Ui) -> Color32 {
    palette(ui).border_strong
}

#[cfg(test)]
mod tests {
    use super::{
        apply_tone_to_visuals, apply_widget_style, palette_for_dark_mode,
        selected_tone_feedback_stroke, ui_font_definitions, BUTTON_FONT_SIZE,
        INTER_MEDIUM_FAMILY_NAME, INTER_MEDIUM_FONT_NAME, INTER_REGULAR_FONT_NAME,
        INTER_SEMIBOLD_FAMILY_NAME, INTER_SEMIBOLD_FONT_NAME,
    };
    use eframe::egui::{Color32, FontFamily, Stroke, Style, TextStyle};

    fn relative_luminance(color: Color32) -> f32 {
        let linear = |component: u8| {
            let value = f32::from(component) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    fn contrast_ratio(left: Color32, right: Color32) -> f32 {
        let (bright, dark) = {
            let left = relative_luminance(left);
            let right = relative_luminance(right);
            if left >= right {
                (left, right)
            } else {
                (right, left)
            }
        };
        (bright + 0.05) / (dark + 0.05)
    }

    #[test]
    fn test_semantic_palettes_use_exact_muted_dark_and_light_accents() {
        let dark = palette_for_dark_mode(true);
        let light = palette_for_dark_mode(false);

        assert_eq!(dark.surface, Color32::from_rgb(15, 17, 19));
        assert_eq!(dark.group, Color32::from_rgb(21, 24, 27));
        assert_eq!(dark.accent.fill, Color32::from_rgb(20, 30, 35));
        assert_eq!(dark.accent.border, Color32::from_rgb(57, 88, 102));
        assert_eq!(light.surface, Color32::from_rgb(242, 244, 245));
        assert_eq!(light.accent.fill, Color32::from_rgb(224, 232, 235));
        assert_eq!(light.accent.border, Color32::from_rgb(138, 160, 169));
        assert_ne!(dark.success.fg, dark.warning.fg);
        assert_ne!(light.success.fg, light.warning.fg);
    }

    #[test]
    fn test_widget_style_matches_approved_compact_density() {
        let mut style = Style::default();

        apply_widget_style(&mut style);

        assert_eq!(BUTTON_FONT_SIZE, 10.5);
        assert_eq!(
            style.text_styles.get(&TextStyle::Heading).unwrap().size,
            16.0
        );
        assert_eq!(
            style.text_styles.get(&TextStyle::Heading).unwrap().family,
            FontFamily::Name(INTER_SEMIBOLD_FAMILY_NAME.into())
        );
        assert_eq!(style.text_styles.get(&TextStyle::Small).unwrap().size, 9.5);
        assert_eq!(style.spacing.item_spacing, eframe::egui::vec2(4.0, 1.0));
        assert_eq!(style.spacing.button_padding, eframe::egui::vec2(6.0, 2.0));
        assert_eq!(style.spacing.interact_size.y, 21.0);
    }

    #[test]
    fn test_inter_is_primary_proportional_font_with_named_weights() {
        let fonts = ui_font_definitions();

        assert_eq!(
            fonts
                .families
                .get(&FontFamily::Proportional)
                .and_then(|family| family.first())
                .map(String::as_str),
            Some(INTER_REGULAR_FONT_NAME)
        );
        assert!(
            !fonts.font_data[INTER_REGULAR_FONT_NAME]
                .variation_axes()
                .is_empty(),
            "bundled Inter font must remain a readable variable font"
        );

        for (family_name, font_name, expected_weight) in [
            (INTER_MEDIUM_FAMILY_NAME, INTER_MEDIUM_FONT_NAME, 500.0),
            (INTER_SEMIBOLD_FAMILY_NAME, INTER_SEMIBOLD_FONT_NAME, 600.0),
        ] {
            let family = FontFamily::Name(family_name.into());
            assert_eq!(
                fonts
                    .families
                    .get(&family)
                    .and_then(|fonts| fonts.first())
                    .map(String::as_str),
                Some(font_name)
            );
            assert_eq!(
                fonts.font_data[font_name]
                    .tweak
                    .coords
                    .as_ref()
                    .first()
                    .map(|(_, weight)| *weight),
                Some(expected_weight)
            );
        }
    }

    #[test]
    fn test_semantic_palette_tokens_are_opaque() {
        for palette in [palette_for_dark_mode(true), palette_for_dark_mode(false)] {
            for color in [
                palette.accent.fg,
                palette.accent.fill,
                palette.accent.hover_fill,
                palette.accent.active_fill,
                palette.accent.border,
                palette.success.fg,
                palette.warning.fg,
                palette.danger.fg,
                palette.drop_target,
            ] {
                assert_eq!(color.a(), 255);
            }
        }
    }

    #[test]
    fn test_semantic_tone_text_contrast_meets_small_text_threshold() {
        for palette in [palette_for_dark_mode(true), palette_for_dark_mode(false)] {
            for tone in [
                palette.accent,
                palette.selected,
                palette.primary,
                palette.visibility,
                palette.success,
                palette.warning,
                palette.danger,
            ] {
                for fill in [tone.fill, tone.hover_fill, tone.active_fill] {
                    assert!(
                        contrast_ratio(tone.fg, fill) >= 4.5,
                        "insufficient contrast for {:?} on {:?}",
                        tone.fg,
                        fill
                    );
                }
            }
        }
    }

    #[test]
    fn test_muted_and_neutral_text_contrast_meets_small_text_threshold() {
        for palette in [palette_for_dark_mode(true), palette_for_dark_mode(false)] {
            for foreground in [palette.text_muted, palette.neutral_status] {
                for background in [palette.surface, palette.group, palette.inset, palette.row] {
                    assert!(
                        contrast_ratio(foreground, background) >= 4.5,
                        "insufficient contrast for {:?} on {:?}",
                        foreground,
                        background
                    );
                }
            }
        }
    }

    #[test]
    fn test_tone_application_updates_all_widget_states() {
        let tokens = palette_for_dark_mode(true).accent;
        let mut visuals = eframe::egui::Visuals::dark();

        apply_tone_to_visuals(&mut visuals, tokens);

        assert_eq!(visuals.widgets.inactive.bg_fill, tokens.fill);
        assert_eq!(visuals.widgets.hovered.bg_fill, tokens.hover_fill);
        assert_eq!(visuals.widgets.active.bg_fill, tokens.active_fill);
        assert_eq!(visuals.widgets.open.bg_fill, tokens.hover_fill);
        assert_eq!(visuals.widgets.active.bg_stroke.color, tokens.border);
        assert_eq!(visuals.widgets.inactive.fg_stroke.color, tokens.fg);
        assert_eq!(visuals.selection.bg_fill, tokens.fill);
        assert_eq!(visuals.selection.stroke.color, tokens.fg);
    }

    #[test]
    fn test_selected_tone_feedback_distinguishes_idle_hover_and_pressed() {
        let tokens = palette_for_dark_mode(true).primary;

        assert_eq!(selected_tone_feedback_stroke(tokens, false, false), None);
        assert_eq!(
            selected_tone_feedback_stroke(tokens, true, false),
            Some(Stroke::new(1.5, tokens.fg))
        );
        assert_eq!(
            selected_tone_feedback_stroke(tokens, true, true),
            Some(Stroke::new(2.0, tokens.fg))
        );
    }
}
