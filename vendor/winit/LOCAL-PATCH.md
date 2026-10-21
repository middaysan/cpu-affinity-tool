# Local Winit compatibility patch

This is published `winit` 0.30.13, upstream commit
`e9809ef54b18499bb4f2cac945719ecc2a61061b`. The crates.io archive SHA256 is
`a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`.
The original Apache-2.0 license and package sources are retained. The standalone
lockfile and Cargo cache marker are omitted. The root Cargo patch selects this
copy; no change is made to the user's Cargo cache.

The only production change backports the Windows 11 DPI-rectangle policy from
upstream commit `488c036a05d418e13bbdcdc349e2db2f6b7f58e2`:
<https://github.com/rust-windowing/winit/commit/488c036a05d418e13bbdcdc349e2db2f6b7f58e2>.
Related reproduction: <https://github.com/rust-windowing/winit/issues/4600>.

Windows 11 (build 22000+) uses the rectangle supplied by `WM_DPICHANGED` directly.
The older cursor-bias/monitor-nudge calculation can return the window to its old
monitor, causing repeated DPI changes and runaway growth. Known Windows 10 builds
retain the existing calculation. An unavailable version follows upstream's
suggested-rectangle fallback. Dragging, keyboard moves and snapping use the same
policy. The shared version query is moved from `dark_mode` to `util` as upstream
does; dark-mode behavior is unchanged. Linux and other backends are unchanged.

Adaptation to 0.30.13 retains the existing Windows 10 block in place instead of
extracting it. The small `dpi_changed_rect` helper exposes the upstream branch for
pure regression tests: Windows 11/unknown-version paths return the original
rectangle without invoking legacy adjustment; Windows 10 still invokes it.
No new application callback, thread, polling or dependency is added.

Remove this copy and Cargo override after a compatible upstream release includes
the fix. Recheck two-way mixed-DPI dragging from both ends of the title bar,
vertical/negative-coordinate monitor arrangements, keyboard moves/snapping,
fixed logical width and allowed height, CPU-graph pointer alignment, tray restore
and hidden startup. Pure policy tests cannot certify native drag-message order.
Historical note: the initial delivery used a release build only, with tests
deferred at the user's request. That deferral was later superseded. Release
preparation on 2026-10-21 passed the pure `dpi_` regressions locally and in Windows
CI; both Windows CI and the stable release workflow now enforce those tests.
