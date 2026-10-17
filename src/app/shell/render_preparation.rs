use eframe::egui;
use std::time::Duration;
use std::time::Instant;

const SCROLL_FRAME: Duration = Duration::from_nanos(1_000_000_000 / 90);
const FONT_CHUNK: usize = 8;
const FONT_TEXT: &str =
    "CPU0123456789%*—: .,-/()abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ<>…·";
const FONT_SIZES: [f32; 9] = [8.0, 9.0, 9.5, 10.0, 10.5, 11.0, 12.0, 19.0, 16.0];

fn scroll_delay(age: Option<Duration>, scrolling: bool, urgent: bool) -> Duration {
    if !scrolling || urgent {
        return Duration::ZERO;
    }
    age.map(|age| SCROLL_FRAME.saturating_sub(age))
        .unwrap_or_default()
}

fn urgent_input(input: &egui::RawInput) -> bool {
    input
        .viewports
        .values()
        .any(|v| v.events.contains(&egui::ViewportEvent::Close))
        || input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::PointerButton { .. }
                    | egui::Event::Key { .. }
                    | egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Cut
            )
        })
}

pub(super) fn pace_scroll(ctx: &egui::Context, input: &egui::RawInput, modal: bool) {
    let id = egui::Id::new("cpu-scroll-frame-budget");
    let scrolling = modal
        && input.viewport_id == egui::ViewportId::ROOT
        && (input
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::MouseWheel { .. }))
            || ctx.input(|i| i.is_scrolling() || i.pointer.is_decidedly_dragging()));
    if !scrolling {
        ctx.data_mut(|d| d.remove::<Instant>(id));
        return;
    }
    let age = ctx
        .data(|d| d.get_temp::<Instant>(id))
        .map(|at| at.elapsed());
    let delay = scroll_delay(age, true, urgent_input(input));
    // Bound the existing GUI thread's paint cadence, without an idle timer or
    // dropping/coalescing input. Discrete actions bypass this short wait.
    if !delay.is_zero() {
        std::thread::sleep(delay);
    }
    ctx.data_mut(|d| d.insert_temp(id, Instant::now()));
}

#[derive(Clone, Default)]
struct FontWarmup {
    scale: f32,
    font: usize,
    cursor: usize,
    frame: Option<u64>,
}
impl FontWarmup {
    fn done(&self) -> bool {
        self.font == FONT_SIZES.len()
    }
    fn next(&mut self, scale: f32, frame: u64) -> Option<(egui::FontId, &'static str)> {
        if self.scale != scale {
            *self = Self {
                scale,
                ..Default::default()
            };
        }
        if self.done() || self.frame == Some(frame) {
            return None;
        }
        self.frame = Some(frame);
        let rest = &FONT_TEXT[self.cursor..];
        let end = rest
            .char_indices()
            .nth(FONT_CHUNK)
            .map_or(rest.len(), |(i, _)| i);
        let text = &rest[..end];
        let family = if self.font + 1 == FONT_SIZES.len() {
            super::presenters::shared_elements::inter_semibold_family()
        } else {
            egui::FontFamily::Proportional
        };
        let font = egui::FontId::new(FONT_SIZES[self.font], family);
        self.cursor += end;
        if self.cursor == FONT_TEXT.len() {
            self.cursor = 0;
            self.font += 1;
        }
        Some((font, text))
    }
}

pub(super) fn warm_fonts(ctx: &egui::Context) {
    let id = egui::Id::new("ui-font-preparation");
    let mut state = ctx
        .data(|d| d.get_temp::<FontWarmup>(id))
        .unwrap_or_default();
    if state.done() && state.scale == ctx.pixels_per_point() {
        return;
    }
    if let Some((font, text)) = state.next(ctx.pixels_per_point(), ctx.cumulative_frame_nr()) {
        // Use egui's own glyph/galley caches; do not paint or keep a second copy.
        ctx.fonts_mut(|fonts| {
            let _ = fonts.layout_no_wrap(text.into(), font, egui::Color32::WHITE);
        });
    }
    let done = state.done();
    ctx.data_mut(|d| {
        d.insert_temp(id, state);
        d.insert_temp(egui::Id::new("ui-font-preparation-ready"), done);
    });
    if !done {
        ctx.request_repaint_after(Duration::from_millis(8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_budget_is_bounded_and_bypasses_idle_and_discrete_input() {
        assert_eq!(scroll_delay(None, true, false), Duration::ZERO);
        assert_eq!(
            scroll_delay(Some(Duration::ZERO), false, false),
            Duration::ZERO
        );
        assert_eq!(
            scroll_delay(Some(Duration::ZERO), true, true),
            Duration::ZERO
        );
        let delay = scroll_delay(Some(Duration::from_millis(3)), true, false);
        assert!(delay > Duration::from_millis(8) && delay < Duration::from_millis(9));
        assert_eq!(
            scroll_delay(Some(Duration::from_millis(20)), true, false),
            Duration::ZERO
        );
    }

    #[test]
    fn wheel_keeps_budget_while_buttons_keys_and_close_bypass() {
        let mut input = egui::RawInput::default();
        assert!(!urgent_input(&input));
        input.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 30.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        assert!(!urgent_input(&input));
        input.events.push(egui::Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        assert!(urgent_input(&input));
        input.events.clear();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        assert!(urgent_input(&input));
        input.events.clear();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        assert!(urgent_input(&input));
    }

    #[test]
    fn font_preparation_is_incremental_once_per_frame_and_restarts_at_new_scale() {
        let mut state = FontWarmup::default();
        let mut pieces = Vec::new();
        for frame in 0..256 {
            if let Some((font, text)) = state.next(1.75, frame) {
                assert!(text.chars().count() <= FONT_CHUNK);
                assert!(!text.is_empty());
                pieces.push((font.size, text));
                assert!(state.next(1.75, frame).is_none());
            } else {
                break;
            }
        }
        assert!(state.done());
        for size in [8.0, 9.0, 9.5, 10.0, 10.5, 11.0, 12.0, 19.0] {
            let joined: String = pieces
                .iter()
                .filter(|(s, _)| *s == size)
                .map(|(_, t)| *t)
                .collect();
            assert_eq!(joined, FONT_TEXT);
        }
        assert!(state.next(1.75, 300).is_none());
        assert!(state.next(2.0, 301).is_some());
        assert!(!state.done());
    }

    #[test]
    fn font_preparation_finishes_without_a_settled_repaint_loop() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::presenters::shared_elements::ui_font_definitions());
        let mut finished = false;
        for _ in 0..256 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| warm_fonts(ui.ctx()));
            output.textures_delta.clear();
            if ctx.data(|d| d.get_temp::<bool>(egui::Id::new("ui-font-preparation-ready")))
                == Some(true)
            {
                finished = true;
                break;
            }
        }
        assert!(finished);
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| warm_fonts(ui.ctx()));
            output.textures_delta.clear();
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                    > Duration::from_secs(1)
            );
        }
    }
}
