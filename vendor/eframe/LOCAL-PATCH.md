# Local eframe compatibility patch

This is the published `eframe` 0.36.2 crate, upstream commit
`49682f8baa058bf49e011035cfbd6e825f88a5ef` (`crates/eframe`). The original
crates.io archive checksum is
`55ad41fac6e3149abb4866e93df37dbd33baa95db525c3368c944584e95a5dc2`.
The upstream MIT and Apache-2.0 licenses are retained here. Cargo uses this
copy through the root `[patch.crates-io]` entry; the user's Cargo cache is not
modified. The published package's standalone lockfile is unnecessary for this
path dependency and is omitted.

The only runtime change is in `src/native/epi_integration.rs`: the initial
automatic show is enabled only when `NativeOptions.viewport.visible` is not
explicitly false. Upstream otherwise unconditionally shows the root after its
first paint, overriding a hidden-start request before processing viewport
commands. Default visible startup is unchanged. Explicit later `Visible(true)`
still shows the window.

Upstream tracking: <https://github.com/emilk/egui/issues/8459>.
Remove this copy and the Cargo patch once the selected upstream release
preserves this behavior. Re-run `startup_visibility_probe` and the application
startup/tray matrix before removing it. No renderer, event-loop, native input,
collector, font or repaint implementation is changed by this patch.
