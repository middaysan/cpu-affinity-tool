# Changelog

This file is the consolidated high-level project history.

Detailed GitHub Release notes continue to live in `changelogs/vX.Y.Z.txt`.

## [Unreleased]

### Added

- Local preview of selected-game background CPU sessions, finite ten-minute preview, bounded applicability/details and optional whole-physical-core CPU Set policy. Native mechanism checks use test-owned processes; the system-role mutation qualification set remains empty pending controlled VM/function validation. See `docs/background-workload.md` for scope and limits.

### Fixed

- Tray Restore shows and unminimizes the native window on the GUI thread before requesting Windows foreground activation, with a short bounded retry that stops on success, hide or exit. Ordinary mouse movement no longer cancels the request.
- Tray double-click Restore waits for the final left-button release so that release cannot invalidate its own activation request. Single clicks and other mouse buttons do not restore the window.
- Group CPU lists use the available header width instead of stopping after eight entries; the hover text retains the full list.
- Parked CPU tiles retain their neutral background. Only the parking status is yellow, and its help explains the displayed sample time and why interval load can be nonzero.

### Changed

- Windows no longer includes the unused Glow/OpenGL renderer; Linux keeps its existing Glow backend.
- Updated the README lead screenshot to the supplied side-by-side groups and CPU selection view.

## [1.7.0] - 2026-10-22

### Added

- Windows System Tweaks with typed Apply/Undo operations, bounded recovery journals, process scheduling ownership and display-change confirmation with automatic rollback. Retained older settings remain available for recovery and Undo.
- An optional localhost Streamable HTTP MCP server inside the same Windows EXE. Its compact controls provide enable/disable, port, Copy connection and a self-contained Copy AI prompt. One permanent protected key grants full access to supported group/rule/settings operations, ordinary or explicitly elevated program launches, exact-instance process control, bounded output, logs, crash reports and CPU telemetry. MCP is disabled by default; share its full-access connection only with a trusted agent.
- A bounded, human-readable journal of the last 100 MCP commands and a separate Process monitor log. Service controls remain accessible while disabled, without toggling a service just by opening its window.
- Continuous CPU history in RAM while monitoring is enabled, including with graphs closed or the window in the tray. Choose 30 seconds, 5 minutes (default), or 30 minutes. Original-time scheduler intervals, PID lifetime identity and explicit unavailable/partial/lost data keep graphs and contributors consistent when events arrive late.
- Shared group/application CPU windows with Graphs / Processes views, exact-time contributors, current tracked process rows, sorting and pinned history. Group graphs default to tracked applications; an explicit All system processes option includes other observed CPU work.
- Right-click CPU graphs to save the visible chart as PNG or all retained CPU/process history as UTF-8 CSV. Agents can download the same bounded CSV through `export_cpu_history`, with immutable artifacts and UTF-8 byte cursors.
- Running-process name suggestions in Application rule tracking fields. Cached background discovery and case-insensitive matching help choose executable names without scanning Windows on each keystroke.
- A manual Memory integrity guide in System Tweaks with a direct Core isolation settings action. The app does not change this security setting or manage its Undo.
- Opt-in nonshipping render profiling and a separate debug-only MCP bridge for isolated GUI checks. Ordinary releases exclude debug automation, authority overrides and profiling hooks; production MCP is an independent optional feature.

### Changed

- Windows renders through WGPU/DX12 with memory-oriented allocation hints. Linux retains Glow and its separate desktop beta scope. Shared theme rasterization, bounded font preparation, cached graph meshes/table sorting and a 60 Hz scroll paint budget reduce repeated GUI work while keeping reactive rendering. Artifact-specific measurements and remaining backend limits are documented separately.
- Compact MCP, Process monitor and CPU load status buttons now sit at the top right. App settings starts with one Light / Dark / System selector and shows the version last. CPU monitoring remains independent from affinity/priority enforcement.
- Align process tables, subtle row separators, modal layouts, stronger dimming and compact CPU tiles. Selected CPUs use a green label/outline; a yellow in-label dot marks retained application activity. Group load shows tracked work on the group's CPUs; the header CPU control shows overall system load.
- Keep the group toolbar fixed while cards scroll. Give application names more room, use compact Run controls and preserve full-name drag targets. Application rule settings use full-width paths and clearer launch/tracking sections.
- Label the group meter **Group CPU load** and refresh the README with current screenshots, setup steps and linked support guides.
- Update compatible Rust dependencies and raise the minimum toolchain to Rust 1.95. Local eframe, epaint and Winit patches retain upstream provenance, licenses, regression tests and removal criteria.

### Fixed

- Preserve logical window sizing when moving between monitors with different scaling, including native rectangle handling and subsequent Explorer drops.
- Restore ordinary Explorer imports into the elevated Windows window through root-scoped Shell drops. Keep the original physical release point so the chosen group receives the file; importing never launches it.
- Honor hidden startup before the first Windows paint, preserve tray restoration geometry and hide the root before graphics teardown. Keep startup errors reachable when the tray or shortcut-forwarding path cannot start.
- Retire ended desktop sessions despite old LSA logon records, without discarding current-session ownership or changing current settings.
- Preserve delayed/final CPU work at its original interval and avoid attributing a reused PID's work to an earlier lifetime. Missing data remains unavailable rather than becoming a zero.
- Keep the MCP native-worker budget across listener restarts and cancelled HTTP callers, so uninterruptible native calls cannot bypass the application-wide bound.
- Reclaim scheduling metadata only after confirmed exact-process exit, keeping alive or uncertain process ownership and previously attempted fields intact.
- Apply MCP's existing Realtime priority restriction before either saved EXE or installed-app activation. Preserve neutral launch scheduling for module-owned fields and retain manual configurations.
- Enforce launch-name limits before MCP admission and bound retained display labels/error text so all managed runs remain inspectable within the response budget. Operational process/run identities and saved names are unchanged.
- Keep graph export menus above reopened modal windows, including after theme changes.

### Compatibility

- Windows remains the stable portable x64 EXE plus matching PDB distribution and still requires administrator approval. CPU recording, System Tweaks and production MCP are Windows-only; Linux beta artifacts remain separate.
- Current saves use schema v11. Existing configuration locations and pre-v6 backup behavior remain; explicit saves upgrade older schemas. Downgrading after a current-schema save is unsupported. MCP authority is stored separately from editable application state.
- CPU history is bounded and memory-only until explicitly exported. CSV contains the retained period and observed positive process work, not an unlimited archive or proof that an absent process was idle. Disabling MCP leaves resumed programs/output and independent CPU monitoring running.

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
