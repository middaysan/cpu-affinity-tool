# Windows hidden startup

`start_minimized` is read from the selected storage before creating the native
root. The same loaded storage is passed into the application; there is no
second read or preference race. An enabled preference requests an initially
hidden, inactive root without a taskbar entry. Tray initialization happens in
the normal application constructor. If it fails, the first application pass
shows a reachable error window. Failure to start cold shortcut forwarding also
keeps its error visible. Existing shortcut/autorun dispatch is unchanged.

Hiding uses `ViewportCommand::Visible(false)`. Restore uses visibility,
unminimize and focus commands, retaining the window's position. A transient
restore acknowledgement prevents the old `minimized=true` input from hiding
the root again before Windows reports `minimized=false`. Neither transition
moves the root offscreen. The existing exact Winit event-target normalization
is retained.

The shell also retains the renderer's root `Arc<winit::window::Window>` from
`CreationContext`. Hide, Restore and visible startup fallback update both
`WindowExtWindows::set_skip_taskbar` and native taskbar styles. Updating only
styles leaves Winit's initial hidden policy intact: its `TaskbarCreated`
handler would remove a restored window's taskbar button after Explorer
restarts. The existing locked Winit version is an explicit Windows dependency
for this supported API; no second event loop or additional worker is created.

## Dependency correction

Upstream eframe 0.36.2 creates the root hidden for first-paint protection, then
unconditionally shows it before applying application viewport commands. This
overrides an explicit initially hidden request. The local Cargo patch changes
only that first-show decision to respect `NativeOptions.viewport.visible`.
The published source, provenance, licenses and upstream issue are recorded in
[`vendor/eframe/LOCAL-PATCH.md`](../vendor/eframe/LOCAL-PATCH.md).
No renderer or event-loop implementation is changed. Existing Windows hidden
redraw handling continues to run application logic and wake tray/IPC work.

## Verification

The opt-in native regression runs without UAC in an isolated test process:

```powershell
cargo test --features windows --bin cpu-affinity-tool startup_visibility_probe::hidden_root_never_shows_before_explicit_restore -- --ignored --exact --nocapture
```

It installs own-process Windows SHOW/HIDE/foreground hooks before creating the
root and independently polls native visibility. With stock eframe it failed:
the root emitted SHOW before an explicit restore. With the patch there were
zero startup SHOW/foreground events and zero visible polls; explicit show,
hide and second show passed. Ordinary unit coverage checks initial viewport
flags, no offscreen/focus command during hiding, tray-failure visibility,
blocked cold-shortcut visibility and stale minimized input after restoration.

The non-elevated application integration matrix uses isolated sidecar state,
paused enforcement, disabled CPU/Event Log monitoring, no tuning intent and a
short-lived owned child. Real `.lnk` files are written through the native
shortcut writer and opened through ShellExecute. It observes native root
events from before launch, tests cold EXE/shortcut with the preference off/on,
warm forwarding, native minimize/tray double-click callbacks, preserved window
geometry, taskbar styles and a debug-only synthetic tray failure. The failure
control requires both `debug_assertions` and `diagnostics-test-controls` and
does not ship in the ordinary release.

On Windows 11 / 20 logical CPUs, the 2026-10-17 matrix passed all five cold
cases: ordinary visible/hidden, shortcut visible/hidden and requested hidden
startup with synthetic tray failure. Both hidden cases had zero visible polls,
zero root SHOW events and zero root foreground events before Restore. Both
visible cases showed normally; tray failure showed the reachable window.
Each of the four usable-tray cases also passed warm shortcut forwarding and
two native minimize/tray-restore cycles, with unchanged window rectangle and
correct taskbar style. Only the owned child launched; no autorun rule ran.
The first integration attempt exposed the stale-minimize rehide, which now
has a separate failing-before-fix state regression and passed the final matrix.

Independent review additionally found the retained Winit taskbar-policy issue.
A callback-contract regression failed with native-style-only updates, then
passed with both policy/style updates. Read-only UI Automation identifies the
test app's button by its executable AppId, or the owned shortcut's distinct
name and generated AppId. The final five-case matrix confirmed no button while
hidden, including after a root-only `TaskbarCreated` message, and a button
after Restore or visible fallback that survives the same message. Explorer
was not restarted. This is separate from checking native style bits, which
alone cannot prove taskbar membership. Both independent reviewers accepted
the correction with no remaining findings in their final delta reviews.

The final administrator-manifest binary is built and statically verified
separately. Elevated release launch, human UAC behavior and actual login/Task
Scheduler startup still require their release smoke checks; non-elevated
debug integration does not certify them.
