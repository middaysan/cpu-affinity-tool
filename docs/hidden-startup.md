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

An accepted Close or tray Quit hides that same native root synchronously before
background shutdown starts. The corresponding queued visibility command keeps
eframe's viewport state consistent. Hidden logic continues waiting for the CPU
worker and tuning shutdown as before; no forced process termination is added.
`App::on_exit` repeats the native hide before graphics teardown, covering native
exit paths too. A viewport command queued only in `on_exit` would not be applied
before the backend stops pumping messages.

The close investigation observed intermittent waits of about two seconds after
the application state, tray and forwarding resources had already been dropped,
inside graphics-backend destruction. Previously the cleared root remained
visible throughout that wait. This correction removes the visible-window
lifecycle defect; it does not claim to eliminate driver cleanup latency.

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

The close regression also has an isolated native test:

```powershell
cargo test --features windows --bin cpu-affinity-tool app::shell::app::tests::restored_root_is_hidden_before_graphics_teardown -- --ignored --exact --nocapture
```

It minimizes, hides and restores a real Winit root, then checks native visibility
immediately after the production `on_exit` callback and before backend cleanup.
It failed before the correction because the root was still visible. A state
regression separately requires accepted Quit to hide while draining, without
requesting final Close before background shutdown has completed.

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

The administrator-manifest binary is built and statically verified separately.
Four subsequent real UAC launches passed on this host: ordinary EXE and saved-rule
shortcut, each with Start minimized off/on. A medium-integrity ShellExecute
launcher requested elevation, the user approved it, and an elevated observer
verified the exact executable and token. Hidden cases emitted no root SHOW or
foreground event before Restore; visible cases showed normally. Warm forwarding,
taskbar reconstruction, two minimize/restore cycles and normal exit also passed.

The close correction passed six elevated isolated release runs with paused
enforcement and a copy of the saved app list, with autorun/tuning disabled.
Direct close and close after minimize/restore were tested with CPU monitoring
off/on. Native visibility disappeared within the first 0–15 ms observation;
processes exited with code 0 after 0.19–0.41 seconds. EXE/PDB identity and the
administrator manifest were verified after the final build. These observations
do not guarantee driver cleanup timing
on every machine. Ordinary UAC launches alone do not certify login behavior.

## Actual reboot and login observation

On 2026-10-17 the user rebooted and logged in with two temporary interactive,
highest-privilege Task Scheduler fixtures prepared in advance. Each task
started its observer before directly launching the same final release EXE,
using an isolated state with Start minimized off or on. Both observers verified
an elevated application token and a new authentication lifetime.

With Start minimized on, the original login run completed: zero visible root
polls, zero SHOW/foreground events before Restore, two minimize/restore cycles,
normal exit code 0 and task removal. With it off, the root emitted one SHOW and
one foreground event and was observed visible. That observer then stopped on
an incorrect test assumption that the helper's outer rectangle must already
be zero; its restore/exit assertions were not reached. The original failed
report is retained rather than relabeled as a complete run.

The default Winit event target can have a 14-by-14 nonclient rectangle with a
zero-sized client area before the first hide. Native inspection reproduced
this in both the development build and an elevated final-release follow-up.
After hiding, its outer and client rectangles were both zero. The corrected
observer checks the client area, required helper styles and nonminimized state;
it requires a zero outer rectangle only after normalization. Six predicate
checks cover this distinction, real client content and minimized/wrong-style
helpers. The separate elevated release follow-up passed two minimize/restore
cycles and normal exit code 0 after login.

Both temporary task removals and termination of their exact application PIDs
were independently verified. These observations cover startup visibility for
both preferences on this host, with the visible-mode restore/exit evidence
coming from the separate follow-up. They do not claim two uninterrupted,
fully passing original login-observer runs or broader hardware/login coverage.
