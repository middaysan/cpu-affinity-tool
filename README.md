# CPU Affinity Tool

Save CPU affinity and priority rules for Windows applications. Organize apps into CPU groups, launch them with the settings you chose, and keep those settings applied while they run.

Set up a game and its background apps once, instead of repeating the changes in Task Manager after every launch. Inspect CPU history, adjust selected Windows settings, or connect a local agent to manage the same setup.

**[Download for Windows](https://github.com/middaysan/cpu-affinity-tool/releases/latest)** · [Getting started](#getting-started) · [MCP guide](docs/mcp.md) · [Documentation](#documentation)

**New in [1.7.3](https://github.com/middaysan/cpu-affinity-tool/releases/tag/v1.7.3).** App and group CPU meters use the group's selected CPUs, and readings stay visible when unrelated processes come and go. If a Steam game cannot be detected, **Select game** explains why and opens the process chooser.

![Affinity groups, CPU selection and recorded CPU history in three separate views](assets/screenshot.png)

*Three views of the current Windows interface. Open the full-size [groups](assets/screenshots/main-dark.png), [CPU selection](assets/screenshots/topology.png) or [CPU history](assets/screenshots/cpu-history.png) image for details. [Light theme](assets/screenshots/main-light.png). CPU topology depends on your processor.*

## Getting started

1. Download `cpu-affinity-tool.exe` from the [Windows release](https://github.com/middaysan/cpu-affinity-tool/releases/latest) and run it from a stable folder. Windows asks for administrator approval.
2. Choose **New group**, name the group, and select its logical CPUs.
3. Use **Add installed...**, **Add file...**, or drag and drop to add an application. Save its launch and tracking settings.
4. Select **Run**. Open **Process monitor** and keep **Enable process monitor** checked if you want ongoing affinity and priority corrections.
5. Open **CPU load** to check recording and choose the history duration. Click a group or application load meter to open **Graphs / Processes**.

Both monitors start enabled for new configurations; existing saved preferences are preserved.

Upgrading keeps your saved groups and application rules. If an older configuration has no **CPU load** preference, recording starts enabled with a 5-minute history. **MCP** starts off, and System Tweaks applies changes only when you request them.

**App settings** contains the System / Light / Dark theme selector and **Start minimized**. Start minimized opens the next launch in the tray; it does not register the tool to start at Windows sign-in.

Performance depends on the application and workload. Compare CPU activity and responsiveness before keeping a new configuration. [How affinity helps and where it has limits](docs/why.md) · [Comparison with other tools](docs/comparison.md).

## What you can do

### Keep a repeatable process setup

- Create groups of logical CPUs for foreground work, background apps, or a particular workload.
- Add executables, supported launcher files, or installed apps. Drag and drop from Explorer also works.
- Steam game shortcuts launch through Steam while CPU/RAM, affinity and Focus follow the game. If detection is unavailable or ambiguous, the amber **Select game** warning explains why and opens the process chooser. Select the running game's main EXE and save. Existing Steam shortcuts can switch using **Use Steam game tracking**.
- New entries recognize 20 application families and fill supported app-owned helper names automatically. Helpers stay tied to the selected installation; shared services/runtimes are excluded. [Supported entries and limits](docs/known-app-processes.md).
- Save priority, startup behavior, and extra process names, plus launch arguments for file-based targets. Name suggestions come from running processes.
- Use **Process monitor** to find your saved apps and reapply their affinity and priority. Its **Processes** and **Logs** tabs show what it found and changed.
- Reorder groups and apps, launch a group together, or create a Windows desktop shortcut for a saved executable rule.

Child-process management is an explicit per-rule option and starts off for new rules. After three access denials for a running process, automatic correction pauses for that instance. Use **Retry** to try again. Available CPU/RAM readings stay visible, and failures appear in **Logs**.

### See where CPU time goes

**CPU load** records CPU and process activity independently of Process monitor. Recording continues with graphs closed or the app in the tray. History stays in RAM: choose 30 seconds, 5 minutes (default), or 30 minutes.

Application and group CPU meters use the group's selected logical CPUs as 100% of its capacity. Only work on those CPUs is counted, including configured helpers; shared processes count once per group. Missing scheduler data stays unavailable or explicitly partial. The header CPU meter covers the whole processor. Detailed graphs show scheduler time on each logical CPU, with its own 100% scale and original sample time.

Open a system, group, or application graph to inspect individual logical CPUs, their sampled parking state, and the processes contributing at a selected time. Group graphs start with your tracked apps; **All system processes** expands the scope. The **Processes** tab has sortable columns.

![CPU history with a selected logical CPU and process contributions](assets/screenshots/cpu-history.png)

*Actual recorded activity from owned demonstration processes, captured with the current Windows interface. Values are measured, not injected.*

Right-click a graph to save the visible chart as **PNG** or export all retained CPU/process history as **CSV**. A connected agent can download the same history through MCP. Turning recording off releases the retained history.

[View the process table](assets/screenshots/processes.png) · [View the export menu](assets/screenshots/history-export.png)

### Change individual Windows settings

**System Tweaks** brings together selected Windows controls: AC power mode, all-core parking on Windows 11, Game Mode, background clips, windowed-game settings, display refresh rate, per-app GPU preference, accessibility shortcuts, and mouse acceleration. Hardware and network guidance and an optional read-only PC check are alongside them.

![System Tweaks with current Windows settings and explicit actions](assets/screenshots/system-tweaks.png)

*Current settings are read from Windows. No tweaks were applied to take this screenshot.*

Manual changes remain applied after you close the app. **Undo** uses the recorded original values and preserves later changes made elsewhere. Display changes have a confirmation countdown. See the [System Tweaks guide](docs/system-tweaks.md) for what each control changes.

**Background-load reduction preview:** Application rule includes an optional checkbox, off by default. It currently saves the preference without changing Windows system tasks; controlled VM validation is still pending. The underlying CPU mechanisms have been tested only on owned test processes. [Preview controls and limits](docs/background-workload.md).

### Connect a local agent

The Windows EXE includes an optional **localhost Streamable HTTP MCP server**, disabled by default. Open **MCP**, enable it, then choose **Copy AI prompt** for setup instructions or **Copy connection** for manual configuration.

An agent can manage groups and rules, change monitoring settings, launch programs, inspect exact process instances, read bounded program output and logs, use System Tweaks, and download CPU history. The MCP window keeps a readable journal of the last 100 commands.

**One permanent key gives full access to the supported tools. Share it only with an agent you trust.** Ordinary-user launch is the default for agent-started programs; administrator launch is explicit. No separate server executable is needed.

[Connection window](assets/screenshots/mcp-connection.png) · [Setup and API guide](docs/mcp.md)

## Platforms and data

- **Windows x64** is the stable release platform. The current renderer uses DirectX 12, and the release requires administrator approval. Saved-rule shortcuts may show the same UAC prompt.
- **Linux** source is retained, but support, development, CI and beta releases are paused while the Windows work is completed. It is not a currently supported or validated target. macOS is unsupported.
- New Windows configurations use `%LOCALAPPDATA%\CpuAffinityTool\state.json`. An existing `state.json` beside the EXE keeps its legacy location. **Logs → Data folder** opens the active directory.
- Version 1.7.3 continues to save configuration as schema v14, used since 1.7.1. Keep a backup before upgrading. Loading an older format leaves the saved settings intact until you explicitly save. Downgrading after a current-format save is unsupported.
- Logs and available crash reports stay local and are not uploaded automatically. Review them before sharing. CPU history is retained in memory and saved to a file only when you export it.

## Build from source

Use **Rust 1.95 or newer**. Windows builds need the MSVC C++ build tools and Windows SDK.

```powershell
# Windows production EXE and matching debug symbols
pwsh -NoProfile -File scripts/build-windows-release.ps1
```

The executable is written to `target/release/cpu-affinity-tool.exe`.

See [CONTRIBUTING.md](CONTRIBUTING.md) for dependencies and validation commands, and the [release process](docs/release-process.md) for packaging.

## Documentation

- [Troubleshooting, data locations, shortcuts and crash reports](SUPPORT.md)
- [Local agent setup and MCP API](docs/mcp.md)
- [System Tweaks controls and Undo](docs/system-tweaks.md)
- [Affinity: uses and limits](docs/why.md) · [Tool comparison](docs/comparison.md)
- [Release notes](CHANGELOG.md) · [Reported issues](https://github.com/middaysan/cpu-affinity-tool/issues)
- [Contributing](CONTRIBUTING.md) · [Private security reporting](SECURITY.md)

[MIT license](LICENSE). The interface uses [Inter](assets/fonts/README.md), distributed under the [SIL Open Font License](assets/fonts/LICENSE.txt).
