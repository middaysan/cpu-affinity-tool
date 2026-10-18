# Local eframe compatibility patch

This is the published `eframe` 0.36.2 crate, upstream commit
`49682f8baa058bf49e011035cfbd6e825f88a5ef` (`crates/eframe`). The original
crates.io archive checksum is
`55ad41fac6e3149abb4866e93df37dbd33baa95db525c3368c944584e95a5dc2`.
The upstream MIT and Apache-2.0 licenses are retained here. Cargo uses this
copy through the root `[patch.crates-io]` entry; the user's Cargo cache is not
modified. The published package's standalone lockfile is unnecessary for this
path dependency and is omitted.

The initial-visibility change is in `src/native/epi_integration.rs`: the initial
automatic show is enabled only when `NativeOptions.viewport.visible` is not
explicitly false. Upstream otherwise unconditionally shows the root after its
first paint, overriding a hidden-start request before processing viewport
commands. Default visible startup is unchanged. Explicit later `Visible(true)`
still shows the window.

The second, opt-in change is `App::repaint_delay`: the Glow/WGPU native root
checks a requested short delay before consuming queued input. A delayed attempt
uses the existing `RepaintAt`/`ControlFlow::WaitUntil` path; it neither sleeps the
GUI thread nor discards input. Hidden/minimized windows, child viewports and
viewport lifecycle events, resizing, DPI and focus changes bypass it. The default returns zero, preserving the
upstream policy. The wrapper consumes the current repaint entry before painting,
so a deferred deadline cannot merge with an already-due entry and spin.
The Windows application uses this only while scrolling a CPU modal (60 Hz),
with immediate discrete-input bypass. Linux does not opt in. No timer thread,
collector or rendering implementation is added.

Upstream tracking: <https://github.com/emilk/egui/issues/8459>.
Remove this copy and the Cargo patch once the selected upstream release
preserves initial visibility and offers an equivalent native frame-deferral hook.
Re-run `startup_visibility_probe`, the startup/tray matrix, scroll/input/close
smoke and CPU19 timing/quality checks before removing these patches.

The Windows CI runs the two `frame_` library regressions against this copy,
seeding its temporary lock from the root lock and deleting it afterwards. These
check stable-root admission and consumption of due deadlines without removing
future repaints. Native scroll/close/tray smoke still supplements these tests.

The opt-in development profiler also has an optional `cfg(debug_assertions)` callback around full native frame, prepare/input, update, tessellation and paint stages. It is unset unless the application explicitly opts into the existing development profiler. The callback adds no worker or I/O and is compiled out of release builds.
