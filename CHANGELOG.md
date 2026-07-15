# Changelog

This file is the consolidated high-level project history.

Detailed GitHub Release notes continue to live in `changelogs/vX.Y.Z.txt`.

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
