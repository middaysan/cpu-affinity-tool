//! Opt-in debug-only native renderer attribution, reusing the existing writer.
use crate::app::features::diagnostics::render_profile::{span, Span, Stage};
use std::cell::RefCell;
thread_local! { static SCOPES: RefCell<[Option<Span>; 5]> = const { RefCell::new([None, None, None, None, None]) }; }
pub fn install() {
    eframe::cpu_frame_probe::install(|phase, enter| {
        let stage = match phase {
            0 => Stage::NativeFrame,
            1 => Stage::NativePrepare,
            2 => Stage::NativeUpdate,
            3 => Stage::Tessellate,
            4 => Stage::Paint,
            _ => return,
        };
        SCOPES.with_borrow_mut(|scopes| {
            if enter {
                scopes[phase] = span(stage);
            } else {
                drop(scopes[phase].take());
            }
        });
    });
}
