# Windows Explorer file imports

The released GUI requires administrator rights. Winit's default OLE drop target
cannot accept an ordinary Explorer drag across that integrity boundary. Windows
file imports instead use Shell `WM_DROPFILES` on the existing root window;
Winit's OLE registration is disabled for that window. Linux input is unchanged.

The adapter uses `DragAcceptFiles` and `ChangeWindowMessageFilterEx` for exactly
three messages: `WM_DROPFILES`, `WM_COPYDATA` and Shell's
`WM_COPYGLOBALDATA` (0x49) transfer.
The latter system transfer identifier is also used by GLFW; it is not an SDK
application command. No process-wide filter, UIAccess, UAC policy change,
unelevated overlay/helper process or new polling worker is introduced.

Only the current GUI thread's own HWND can register. The guard retains the
previous window procedure, handles normal removal and window-first destruction,
and restores only filters/style it acquired. A bounded single pending batch is
copied from the native drop allocation; every received handle is finished.
Invalid offsets/terminators, releases outside the actual client area, relative paths, batches above
128 files, individual paths above 32767 UTF-16 units and allocations above 1 MiB
are rejected as a whole. Unicode and ANSI Shell lists are supported. File
contents are not read by this adapter, and importing never executes the target.

The adapter takes the drop point directly through `DragQueryPoint`, as GLFW
does, and validates it against the receiving window's client rectangle. On the
tested Explorer cross-integrity path, the raw allocation's area bit is set even
though the point is already relative to the client; using that bit to apply
another screen-to-client conversion would select the wrong group. A native
regression retains the point with that flag set. The later mouse position is
never used to repair or guess the drop location.

Shell captures the physical client point at release. Egui receives that point
at its current pixel scale, and pending imports retain it even if the mouse
moves. The existing group importer handles EXE, supported shortcuts/launchers,
duplicates, resolution errors and asynchronous saving. A release outside a
group is discarded rather than retargeted on a later mouse move. Shell supplies
its acceptance cursor and final drop; it supplies no OLE hover events, so the
old colored group-hover preview is absent on Windows.

Regression tests cover the viewport option, real native HWND/HDROP delivery,
one-shot/bounded queuing, GUI-thread ownership, guard/window destruction order,
Unicode/ANSI lists, malformed input, DPI conversion and the retained group
release point. Same-process native tests do not certify Explorer's cross-process
marshalling. On 2026-10-18 the user verified actual `notepad.exe` transfers from
ordinary Explorer into an elevated isolated debug window on Windows 11 at 175%
scaling, including several different target groups. Native coordinate logs
identified and the regression covers the duplicate-conversion failure before
that successful run. The administrative release manifest and corresponding
release build are checked separately; this does not certify every Windows
version, remote session or launcher format.

Primary references:

- [Microsoft: WM_DROPFILES](https://learn.microsoft.com/en-us/windows/win32/shell/wm-dropfiles)
- [Microsoft: DragAcceptFiles](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-dragacceptfiles)
- [Microsoft: DROPFILES](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/ns-shlobj_core-dropfiles)
- [Microsoft: window-scoped UIPI filtering](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-changewindowmessagefilterex)
- [GLFW: native Windows drop/filter implementation](https://github.com/glfw/glfw/blob/master/src/win32_window.c)
