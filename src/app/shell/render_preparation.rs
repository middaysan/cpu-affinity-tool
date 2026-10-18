use eframe::egui;
use std::time::Duration;
use std::time::Instant;

const SCROLL_FRAME: Duration = Duration::from_nanos(1_000_000_000 / 60);
const FONT_CHUNK: usize = 32;
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

pub(super) fn scroll_repaint_delay(
    ctx: &egui::Context,
    input: &egui::RawInput,
    modal: bool,
) -> Duration {
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
        return Duration::ZERO;
    }
    let age = ctx
        .data(|d| d.get_temp::<Instant>(id))
        .map(|at| at.elapsed());
    let delay = scroll_delay(age, true, urgent_input(input));
    // The native event loop retains input until the deadline. Never sleep here
    // or advance the budget when a repaint was only deferred.
    if delay.is_zero() {
        ctx.data_mut(|d| d.insert_temp(id, Instant::now()));
    }
    delay
}

#[derive(Clone, Default)]
struct FontWarmup {
    scale: f32,
    options: Option<egui::epaint::TextOptions>,
    font: usize,
    cursor: usize,
    variant: u8,
    frame: Option<u64>,
}
impl FontWarmup {
    fn done(&self) -> bool {
        self.font == FONT_SIZES.len()
    }
    fn next(
        &mut self,
        scale: f32,
        options: &egui::epaint::TextOptions,
        frame: u64,
    ) -> Option<(egui::FontId, &'static str, f32)> {
        if self.scale != scale || self.options.as_ref() != Some(options) {
            *self = Self {
                scale,
                options: Some(*options),
                frame: self.frame,
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
        let offset = f32::from(self.variant) / 4.0 / scale;
        self.variant += 1;
        if self.variant == 4 {
            self.variant = 0;
            self.cursor += end;
            if self.cursor == FONT_TEXT.len() {
                self.cursor = 0;
                self.font += 1;
            }
        }
        Some((font, text, offset))
    }
}

pub(super) fn warm_fonts(ctx: &egui::Context) {
    let id = egui::Id::new("ui-font-preparation");
    let mut state = ctx
        .data(|d| d.get_temp::<FontWarmup>(id))
        .unwrap_or_default();
    let options = ctx.fonts(|fonts| *fonts.fonts.options());
    #[cfg(feature = "render-profiling")]
    if state
        .options
        .as_ref()
        .is_some_and(|previous| previous != &options)
    {
        crate::app::features::diagnostics::render_profile::note("font_options_changed");
    }
    if state.done()
        && state.scale == ctx.pixels_per_point()
        && state.options.as_ref() == Some(&options)
    {
        return;
    }
    if let Some((font, text, offset)) =
        state.next(ctx.pixels_per_point(), &options, ctx.cumulative_frame_nr())
    {
        crate::render_profile_scope!(FontPreparation);
        // Use egui's own glyph/galley caches; do not paint or keep a second copy.
        ctx.fonts_mut(|fonts| {
            // Isolate each glyph so its subpixel position is independent of
            // shaping/advances of the preceding characters in this batch.
            for character in text.chars() {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    character.to_string(),
                    font.clone(),
                    egui::Color32::WHITE,
                );
                job.sections[0].leading_space = offset;
                let _ = fonts.layout_job(job);
            }
        });
    }
    let done = state.done();
    ctx.data_mut(|d| {
        d.insert_temp(id, state);
        d.insert_temp(egui::Id::new("ui-font-preparation-ready"), done);
    });
    if !done {
        ctx.request_repaint_after(Duration::from_nanos(1_000_000_000 / 60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_preparation_bounds_total_paints_without_skipping_variants() {
        let mut state = FontWarmup::default();
        let options = egui::epaint::TextOptions::default();
        let mut variants = 0;
        let mut frames = 0;
        while let Some((_, text, _)) = state.next(1.75, &options, frames) {
            assert!(text.chars().count() <= 32);
            variants += text.chars().count();
            assert!(state.next(1.75, &options, frames).is_none());
            frames += 1;
            assert!(
                frames <= 120,
                "font preparation redraws too many complete frames: {frames}"
            );
        }
        assert!(state.done());
        assert_eq!(variants, FONT_TEXT.chars().count() * FONT_SIZES.len() * 4);
    }

    #[test]
    fn scroll_budget_uses_sixty_frames_per_second() {
        let delay = scroll_delay(Some(Duration::from_millis(3)), true, false);
        assert!(delay > Duration::from_millis(13) && delay < Duration::from_millis(14));
    }

    #[test]
    fn deferred_paint_keeps_wheel_events_and_does_not_move_deadline() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 30.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        let key = egui::Id::new("cpu-scroll-frame-budget");
        assert_eq!(scroll_repaint_delay(&ctx, &input, true), Duration::ZERO);
        let at = ctx.data(|d| d.get_temp::<Instant>(key)).unwrap();
        let delay = scroll_repaint_delay(&ctx, &input, true);
        if !delay.is_zero() {
            assert_eq!(ctx.data(|d| d.get_temp::<Instant>(key)), Some(at));
        }
        assert_eq!(input.events.len(), 1);
        assert_eq!(scroll_repaint_delay(&ctx, &input, false), Duration::ZERO);
        assert!(ctx.data(|d| d.get_temp::<Instant>(key)).is_none());
    }

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
        assert!(delay > Duration::from_millis(13) && delay < Duration::from_millis(14));
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
        let options = egui::epaint::TextOptions::default();
        for frame in 0..512 {
            if let Some((font, text, offset)) = state.next(1.75, &options, frame) {
                assert!(text.chars().count() <= FONT_CHUNK);
                assert!(!text.is_empty());
                if offset == 0.0 {
                    pieces.push((font.size, text));
                }
                assert!(state.next(1.75, &options, frame).is_none());
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
        assert!(state.next(1.75, &options, 600).is_none());
        assert!(state.next(2.0, &options, 601).is_some());
        assert!(!state.done());
    }

    #[test]
    fn font_preparation_finishes_without_a_settled_repaint_loop() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::presenters::shared_elements::ui_font_definitions());
        let mut finished = false;
        for _ in 0..512 {
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

    #[test]
    fn font_preparation_restarts_after_text_options_change() {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        ctx.set_fonts(super::super::presenters::shared_elements::ui_font_definitions());
        for _ in 0..512 {
            let mut output = ctx.run_ui(Default::default(), |ui| warm_fonts(ui.ctx()));
            output.textures_delta.clear();
        }
        assert_eq!(
            ctx.data(|d| d.get_temp::<bool>(egui::Id::new("ui-font-preparation-ready"))),
            Some(true)
        );
        ctx.set_visuals(egui::Visuals::light());
        let mut output = ctx.run_ui(Default::default(), |ui| warm_fonts(ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(
            ctx.data(|d| d.get_temp::<bool>(egui::Id::new("ui-font-preparation-ready"))),
            Some(false)
        );
    }

    #[test]
    fn prepared_glyphs_cover_all_subpixel_positions() {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(1.75);
        ctx.set_fonts(super::super::presenters::shared_elements::ui_font_definitions());
        for _ in 0..512 {
            let mut output = ctx.run_ui(Default::default(), |ui| warm_fonts(ui.ctx()));
            output.textures_delta.clear();
        }
        let mut unchanged = false;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.ctx().fonts_mut(|fonts| {
                let before = fonts.image();
                for (index, size) in FONT_SIZES.into_iter().enumerate() {
                    let family = if index + 1 == FONT_SIZES.len() {
                        super::super::presenters::shared_elements::inter_semibold_family()
                    } else {
                        egui::FontFamily::Proportional
                    };
                    for c in FONT_TEXT.chars() {
                        for bin in 0..4 {
                            let mut job = egui::text::LayoutJob::simple_singleline(
                                c.to_string(),
                                egui::FontId::new(size, family.clone()),
                                egui::Color32::WHITE,
                            );
                            job.sections[0].leading_space = bin as f32 / 4.0 / 1.75;
                            let _ = fonts.layout_job(job);
                        }
                    }
                }
                let after = fonts.image();
                unchanged = before.size == after.size && before.pixels == after.pixels;
            });
        });
        output.textures_delta.clear();
        assert!(unchanged, "prepared glyphs still allocate bitmap variants");
    }
}
