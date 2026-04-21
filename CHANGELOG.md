# Changelog

This file is the consolidated high-level project history.

Detailed GitHub Release notes continue to live in `changelogs/vX.Y.Z.txt`.

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
