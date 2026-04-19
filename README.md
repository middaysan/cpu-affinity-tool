# CPU Affinity Tool

A simple and convenient program for managing CPU cores (CPU affinity) on Windows. It allows you to manually choose which cores your applications will run on, helping to improve performance, optimize resource distribution, and make the system more responsive.

![CPU Affinity Tool](assets/screenshot.png)

## Current Platform Status

- **Windows** is the only platform that is currently treated as the supported, CI-validated, and published release path.
- The current GitHub Release workflow publishes only `cpu-affinity-tool.exe` for `x86_64-pc-windows-msvc`.
- The repository also contains a feature-gated Linux entrypoint (`cpu-affinity-tool-linux`), but that code path is still experimental backend work. It does not currently have CI coverage, published release artifacts, or runtime parity with the Windows path.
- The Windows binary embeds `requestedExecutionLevel=requireAdministrator`, so the tool requires elevated launch; on a non-elevated start, users should expect the normal UAC prompt.
- Manual pre-release checks for the current contract are documented in `docs/release-checklist.md`.

## Why do you need this?

- **Improve Performance**: Isolate resource-heavy programs on separate cores.
- **Gaming Stability**: Dedicate specific cores to games to avoid micro-stutters caused by background tasks.
- **System Smoothness**: Assign background processes to some cores and important work tools to others.
- **Flexible Tuning**: Experiment with load distribution to achieve maximum efficiency.

## Key Features

- **Core Groups**: Create sets of cores (e.g., "Gaming Cores", "Background Cores") and easily switch between them.
- **App Management**: Launch programs tied to specific cores with the desired priority.
- **Open App**: Add traditional `.exe`, `.lnk`, or `.url` targets directly from the file picker.
- **Find Installed (Windows)**: Search a Start-backed list of installed apps, including Microsoft Store / MSIX apps that do not expose an obvious `exe` path.
- **Automatic Monitoring**: The program tracks running processes and automatically restores core and priority settings if an app changes them.
- **Autorun**: Configure programs to start automatically together with the tool.
- **Drag & Drop**: Simply drag an `.exe` file or a shortcut into the program window to add it to a group.
- **Themes**: Choose between Light, Dark, or System themes.
- **Event History (Logs)**: View the history of launches and applied settings.

## How to Use

### 1. Creating a Core Group
Click the **"+ Create Group"** button at the top of the window. Enter a group name (e.g., "Work") and select the cores you want to include. Click "Save".

### 2. Adding Applications
Select the desired group from the list. You can add an application in two ways:
- Click **"Open App"** to add an `.exe`, `.lnk`, or `.url` target from disk.
- Click **"Find Installed"** to search the Windows Start-backed installed-app list. This is the recommended path for Microsoft Store / MSIX apps such as Spotify. If the app is not listed there, use **Open App** instead.
- Simply drag the application file (`.exe`) or its shortcut directly onto the group area in the program window.

### 3. App Settings
Click the gear icon next to the application name to:
- Change the process priority (e.g., set to "High" for games).
- Add command-line arguments for file-based targets.
- Enable **Autorun** so the app starts automatically when CPU Affinity Tool is launched.

For installed app targets added through **Find Installed**, the tool stores the Windows app identity (`AUMID`) instead of a manual executable path. In the current version, those installed targets do not expose editable command-line arguments.

### 4. Launching
- Click the blue button with the application name to launch it.
- If the app is already running, the program will simply switch focus to its window.
- If the **"Run All"** button is enabled in the group settings, you can launch all apps in the group with a single click.

### 5. Process Monitoring
At the bottom of the window (footer), there is a monitoring toggle button. 
- When it is **ACTIVE**, the program will automatically ensure that apps do not "reset" their core and priority settings. 
- This is useful for games and programs that tend to change their own parameters while running.

## License

This project is licensed for non-commercial use. You may use it for free for personal purposes. For commercial use, please contact the author.
