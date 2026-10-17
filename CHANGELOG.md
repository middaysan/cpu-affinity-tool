# Changelog

This file is the consolidated high-level project history.

Detailed GitHub Release notes continue to live in `changelogs/vX.Y.Z.txt`.

## [Unreleased]

### Added

- Opt-in Windows render profiling and a debug-only MCP bridge for graph, table and monitoring-setting checks. Bounded local diagnostics expose CPU/memory, frame work and original-interval quality; ordinary release builds contain no MCP endpoint or handlers and reject the debug feature.
- Process/PID/load contributors below the selected logical CPU graph, for the latest completed sample and the exact historical sample under the pointer. Group graphs include observed host processes; application graphs retain individual tracked instances. Bounded sparse RAM history preserves names of exited processes and distinguishes PID reuse; settings include its memory estimate.
- Windows CPU load indicators for affinity groups and tracked apps, with a centered per-logical-CPU graph. **App settings > CPU load monitoring** saves an on/off preference independently of affinity/priority protection. Per-app/per-logical-CPU history records continuously, including with graphs closed and in the tray. Preallocated bounded memory rings follow saved-app additions/removals; settings show the estimated extra memory and explain monitoring overhead.
- A manual Memory integrity guide at the end of **System Tweaks > System > For games**, with a direct Windows Security Core isolation shortcut. The app does not change this security setting or manage its Undo.

### Fixed

- Complete delayed CPU scheduler data at its original recorded interval, updating the graph, all CPU tiles and process contributors together. Live uses the last resolved interval; real loss remains a gap. Preallocated buffers hold at most 16 pending intervals, preserve original rule/PID lifetime ownership and enter the monitoring memory estimate.
- Keep the final measured scheduler interval of a confirmed exited process and report later intervals as zero instead of letting a stale retired PID make app/tracked-group totals permanently partial. Match the ETW end to a verified native-lifetime QPC observation so buffered ends and reused PIDs cannot retire or charge the wrong instance; startup, delivery gaps and real errors remain unavailable.
- Preserve measured CPU work when another tracked process lacks a sample. App and tracked-group graphs show an explicit dashed subtotal with marked percentages instead of hiding all known activity; fully unknown intervals stay gaps. Render isolated valid samples as dots, retain partial quality through history resizing, and synchronize live/history tile values and process attribution.
- Keep CPU group history continuous at idle by reading busy time as the complement of Windows' Idle Time counter; retain real unavailable samples as gaps. Show explicit Windows parking state on CPU tiles and sampled parking marks in history. Replace the modal's unsupported close glyph with **Close**.
- Preserve CPU runtime-refresh failures during recovery and Undo. Restore absent AC overrides through a scoped Windows restore privilege, without changing registry ACLs or deleting setting containers.
- Remove the leftover minimized Winit helper rectangle when hiding to the tray, including **Start minimized**. Keep the event helper available for paint delivery and leave unrelated windows untouched (issue #20).

### Changed

- Start the Windows GUI with BelowNormal priority and hard affinity to its last available logical CPU before creating background workers. Saved EXE launches receive their own rule mask, or the original pre-restriction mask when affinity belongs to another owner. Startup Logs report success/failure; multi-group or unrepresentable native masks remain unchanged instead of selecting an incorrect CPU.
- Center logical CPU names in a shared rounded, outlined badge on tiles and the detailed graph. Tile badges use 70% of the inner width for symmetric side spacing. Keep tile load right-aligned beside parking status, with selection checks and app-activity rails visible.
- Expand CPU modals to 80% of the window height and enlarge the stable, scrollable contributor table. Group charts offer **Tracked processes only**, switching tiles, history and contributor rows together between all observed host processes and the exact historical tracked union across saved groups. Shared instances count once; application graphs retain their own scope. Include the additional preallocated aggregate ring in the monitoring memory estimate.
- Add a Graphs / Processes switch to app CPU modals with all tracked PIDs, whole-machine CPU time and measured CPU locations, including idle/unavailable instances. Keep current rows separate from graph history and preserve known contributors as partial when a missing sibling makes the app aggregate unavailable.
- Mark application CPU tiles with a green rail when positive work exists in the retained history, independently of hover or pinned time. Remove expired activity markers and add them on new work, including known activity beside missing samples.
- Let a left click pin the CPU graph's exact sample while scrolling historical process contributors; the next left click releases it. Keep recording, cursor, tile values and process attribution synchronized. Align process-table columns and card spacing, shorten the header accent and retain parking color separately from load.
- Make CPU graph selection explicit with a contrasting tile outline, painted check and shared accent rail. Group the selected CPU's large graph and process contributors in one card with a CPU badge; empty and unavailable process messages identify the selected CPU.

- Organize the footer's tracked-process popup as a compact Process / PID / Group table. Clip long cells with horizontal drag/Shift-wheel scrolling; keep a fixed header, vertical list scrolling and full-value hover without widening the popup.
- Move group/application drag targets to their full name columns, including padding after short names. Remove left drag grips, retain keyboard group reordering, and hide all meter spacing when CPU load monitoring is off.
- Unify explicit **Close** buttons with one theme-aware dark-red style; keep a single top-right Close in CPU graphs. Remove group number badges, show the stable Group ID only in Edit, and add subtle borders/padding to load meters.
- Replace separate CPU load/Graph rows with compact percentage blocks and horizontal gradient fill beside group names and processes. Preserve process status colors, keep dialogs compact, and use the shared blue style for the monitor button.
- Reduce repeated CPU-modal rendering work with clipped-tile layout, snapshot-prepared table text and a scroll-only 90 Hz paint budget. Prepare shared font sizes incrementally on existing GUI frames; telemetry remains at 1 Hz. Host measurements and remaining cold-frame/memory limits are documented in `docs/cpu19-performance-probe.md`.
- Add **App settings > CPU history**: 30 seconds, 5 minutes (default), or 30 minutes in memory. Charts show local HH:MM:SS and exact sampled load/parking on hover; retention changes preserve an open process trace and the 1 Hz cadence.
- Add a subtle theme-aware gradient and short selection/hover transitions to the main navigation; keep panel content static and stop decorative repaints after transitions settle.
- Make the main navigation wider and taller than the compact, centered System Tweaks section switch.
- Keep three System Tweaks sections: **System**, **GPU / network**, and **Check PC**. Hide old module profiles; only applied older settings appear at the bottom of System with individual Undo, preserving recovery data across restarts and failed restoration.
- Keep the AC performance tweak to one Windows power-mode preference. Retire CPU frequency, high-clock and parking controls; preserve recovery and Undo for previously applied CPU settings after restarting the app.
- Rename the main tabs to **Affinity groups**, **System Tweaks**, and **Logs**, in that order. Place **New group** at the top right of the group list and remove its repeated heading.
- Simplify the header to **⚙ App settings** on the left and the version on the right. Keep the application name in the window title bar, CPU details on version hover, and the footer for monitoring controls and status.
- Update direct and compatible transitive dependencies, including `eframe` / `egui` 0.36.2, `tray-icon` 0.26.1, Tokio 1.53.2 and Serde/JSON. Raise the minimum Rust toolchain to 1.95 for the GUI stack.

## [1.6.3] - 2026-09-20

### Added

- Application Settings with System/Light/Dark theme selection and an optional Windows **Start minimized** preference. This does not register Windows login startup.
- A scrollable tracked-process list on monitor-status hover, showing verified executable filenames, PIDs, and runtime-owning groups.

### Changed

- Enable monitoring for new configurations while preserving saved preferences; place pause/resume left of the status and move Crash reports access into Activity.
- Polish compact forms, group editing, typography, and Activity controls.
- Trim unused dependency features and reduce monitoring/UI cloning and allocations while preserving process identity checks and GUI wake behavior. No measured runtime performance gain is claimed.

### Fixed

- Support primary-click confirmation in the installed-app picker.
- Wrap long installed-app identifiers without horizontal overflow or overlapping controls.
- Prevent duplicate or incorrectly labeled monitored-process rows and omit inaccessible or reused process IDs.

### Compatibility

- Schema v10, configuration locations, and administrator/UAC requirements remain unchanged. Start minimized defaults to off when absent. Stable distribution remains an unsigned portable Windows x64 EXE plus matching PDB.

## [1.6.1] - 2026-09-02

### Fixed

- Reject reapplying affinity or priority to an already running process when any managed PID cannot accept the settings. The runtime now retains a mismatch state and records the affected PID and Windows failure instead of reporting a false success.
- Identify the Win32 operation that failed while reading or applying process settings. For confirmed protected processes, explain that Windows does not permit affinity or priority changes.
- Keep mismatch status accurate across monitor contention and clear it once the monitor confirms that settings match again.

### Known issues

- Windows-protected processes, including protected audio contexts such as `audiodg.exe`, cannot have hard affinity or priority changed by this application, even when it is elevated. The application does not bypass Windows process protection.
- Stable release artifacts remain Windows-only. Linux continues as a separate desktop beta path without Windows tray, focus, installed-app activation, or saved-rule shortcut parity.
- The release is not code signed and does not include an installer, winget, Chocolatey, AppImage, or Flatpak package.

## [1.6.0] - 2026-09-01

### Added

- Added local Windows crash reports for main-thread Rust panics and native UI-loop errors
- Added a bounded **Crash reports** Activity subpage with a header count, retention of the newest 20 complete reports, confirmed deletion, and **Show in Explorer**
- Added a persistent Activity summary of the newest validated crash report after the next startup, so **Clear** does not hide the last crash context
- Added a non-shipping Windows crash-capture probe for pre-Tokio panic and native-loop error ordering
- Added Windows-only Event Log diagnostics, enabled by default after the first rendered frame, with sanitized Activity evidence
- Added an opt-in Windows native crash-dump guide for failures that cannot be captured by the in-app report writer
- Added per-rule descendant process management, disabled by default for newly created rules

### Changed

- Returned both binaries to the platform system allocator instead of installing `mimalloc` globally
- Stable Windows releases now publish an identity-verified matching PDB with line-table debug information for crash symbolization, and pull-request CI reproduces that symbol build
- Process tracking and actions now bind a PID to verified process identity and creation time before applying affinity, priority, focus, or descendant ownership
- Monitoring uses bounded event delivery and wakes the GUI directly instead of relying on an unbounded bridge or UI-thread sleeps
- Tray commands are routed through the GUI thread; if tray initialization fails, the window remains reachable instead of hiding without a restore path
- Persisted state schema is now v10. Existing pre-v10 rules preserve their previous descendant-management behavior, while newly created rules and missing v10 values default it to off

### Fixed

- Kept overlapped named-pipe operation resources alive through every pending completion state, including cancellation and `ERROR_IO_INCOMPLETE`, preventing timeout-path use-after-free
- Kept the saved-rule primary guard held until its forwarding server has stopped, preventing a replacement cold start from racing the previous named-pipe owner during shutdown
- Prevented stale Event Log worker results from resurfacing after disable/re-enable, and kept Event Log evidence out of chronological Activity entries
- Prevented PID reuse from redirecting runtime actions to a different process instance
- Prevented unverified or older processes from being claimed as descendants of a tracked launch
- Made state replacement crash-safe with validated temporary files and recovery backups instead of allowing an interrupted save to truncate the active state
- Cleaned up suspended Windows launches when process setup fails before resume
- Bounded monitor and IPC worker shutdown so stale workers cannot outlive the runtime state they serve
- Updated `webbrowser` and `event-listener` to patched releases identified during the v1.6.0 dependency audit

### Security

- Crash-report Explorer actions verify and execute through a non-elevated, below-high-integrity Explorer shell process; the existing Activity **Data folder** action keeps its prior direct-launch behavior
- Crash report writes use unique partial files, no-replace publication, bounded UTF-8 content, reparse checks, and file-identity validation before managed deletion
- Windows Event Log diagnostics are read-only, sanitize displayed fields, never upload data, and do not modify Windows Error Reporting or registry settings

## [1.5.0] - 2026-07-16

### Added

- Added the bundled Inter variable font with regular, medium, and semibold UI weights
- Added drag-and-drop group reordering while preserving saved group and rule identities
- Added semantic light and dark color palettes for actions, status, selection, and monitoring states

### Changed

- Redesigned the egui interface around a denser layout with clearer group boundaries and more compact application rows
- Replaced the separate logs entry with centered **Overview** and **Activity** navigation
- Renamed the main add actions to **Add installed...** and **Add file...**
- Reworked monitoring controls around **Monitoring active** / **Monitoring paused** status and the **Pause monitor** / **Resume monitor** action
- Updated the Rust dependency set, including `eframe` / `egui` 0.35, and declared Rust 1.92 as the minimum supported toolchain

### Fixed

- Split application-row actions by intent: **Fix** reapplies affinity and priority without stealing focus, while **Focus** only activates the existing window
- Replaced font-dependent theme glyphs with egui-painted vector icons so the theme control remains readable with the bundled Inter font
- Increased selected CPU-thread contrast with the same restrained turquoise primary palette used by the create action in both themes
- Fixed CPU-thread controls overflowing the fixed-width group editor; all controls now wrap without selected-state shifts and keep their core and thread labels visible
- Improved keyboard access to group reordering after replacing the old arrow controls with drag-and-drop
- Improved small-text contrast and made long application status details discoverable without relying on a clipped label
- Reset stale group form data when starting a new group after editing an existing one

## [1.4.0] - 2026-06-15

### Added

- Added Windows desktop shortcuts for saved rules from rule launch settings
- Added `--run-rule <group-id> <rule-id>` startup handling for saved-rule shortcut launches
- Added Windows local IPC forwarding so shortcut launches can hand off to the active GUI instance

### Changed

- Windows release builds now explicitly use `uiAccess=false` and verify the embedded release manifest in CI and release workflows
- Release docs now include shortcut/UAC/Desktop-placement smoke coverage and forwarding exit-code expectations
- README now documents saved-rule shortcut behavior, UAC expectations, and Linux beta shortcut limitations

### Fixed

- Shortcut cold starts now skip unrelated autorun rules and run only the requested saved rule
- Forwarded shortcut launches now wake the GUI loop and avoid becoming a second primary when another instance owns startup
- Shortcut creation avoids overwriting existing desktop files by allocating numbered filenames

## [1.3.1] - 2026-05-19

### Fixed

- Fixed a regression introduced in `v1.3.0` that accidentally broke dropping external `.exe`, `.lnk`, and `.url` files onto groups
- File drops now remember the hovered group during the OS drag operation, so the target group is still resolved even when the pointer position is missing on the release frame

## [1.3.0] - 2026-05-19

### Added

- Drag-and-drop app moves between CPU groups while preserving each app rule configuration
- Drag-and-drop app reordering inside the same group with placement feedback
- Explicit **Tracked Process Names** controls for transparent process-name rediscovery

### Changed

- Process matching now treats tracked process names as explicit exact-match fallbacks
- The monitoring toggle is now labeled **Auto Re-apply Affinity and Priority**
- Linux beta CI and prerelease automation now use a pinned Ubuntu 24.04 runner

### Fixed

- Store app tracking no longer auto-manages generic Windows host processes such as `backgroundTaskHost.exe`
- Linux launch handling now validates affinity before spawn, applies post-spawn settings best-effort, and reaps child processes
- Duplicate drag-and-drop moves are rejected without mutating saved state

## [1.2.1] - 2026-04-21

### Added

- Broader Windows app discovery through Start Menu shortcut support in **Find Installed**

### Changed

- Installed-app discovery now merges `AppsFolder`, Start Menu shortcuts, and `App Paths`
- The installed-app picker now uses a cleaner full-width row layout for mixed Store and desktop entries

### Fixed

- Desktop apps such as RustDesk can now appear in **Find Installed** even when they are only exposed through Start Menu shortcuts
- Obvious uninstall, remove, and repair shortcuts are filtered out of the installed-app picker

## [1.2.0] - 2026-04-20

### Added

- Windows installed-app support through **Find Installed**
- `AUMID`-based installed targets for Store and MSIX apps
- Release checklist and release smoke documentation

### Changed

- Runtime architecture was split into clearer runtime, model, and view responsibilities
- Persistence internals were decomposed into focused modules
- Windows `os_api` internals were split into focused modules while keeping the public surface stable
- CI and tag-release workflows were aligned with the local verification bundle

### Fixed

- Installed Store and MSIX launch handling now tracks package-local helper processes more reliably
- Windows PowerShell console flicker during installed-app discovery and metadata lookup was removed

## [1.1.5]

### Added

- Current-process priority handling
- Monitoring state for whether running applications still match the assigned settings
- Expanded unit tests for CPU presets and state storage

### Changed

- The application now lowers its own priority to reduce interference with heavier workloads
- CPU preset matching was improved with regex-based rules and broader modern CPU coverage

### Fixed

- Repaint behavior was improved when monitoring detected changes

## [1.1.2] - 2026-01-28

### Added

- Embedded CPU preset system for modern Intel and AMD layouts
- Hierarchical CPU schema model and grouped core visualization
- Preset-format documentation in `CPU_SCHEME_INSTRUCTION`

### Changed

- Group editor was rewritten around cluster-aware CPU selection
- State loading now rebuilds CPU schema more intelligently for the current machine

### Fixed

- Occupied-core display and large-core-count rendering were improved

## [1.0.8]

### Added

- Shared UI elements module for more consistent view code

### Changed

- UI styling was unified around a consistent panel design
- App-removal flow moved out of the main list and into app settings
- Tip-rotation logic was centralized for more reliable view switching

### Fixed

- Borrowing and state-handling issues in app settings rendering were resolved

## [1.0.7] - 2026-01-07

### Added

- Initial Windows system tray integration
- PID tooltip support for running applications
- `mimalloc` as the default allocator

### Changed

- Windows OS integration code was significantly cleaned up and modernized
- Internal comments in core modules were normalized to English

### Fixed

- Code consistency and internal maintainability were improved across the repository
