# Support

## Where to ask for help

Use the repository issue templates:

- **Bug report** for reproducible problems
- **Feature request** for proposed improvements

This repository does not currently use a separate support forum or public discussions area.

## Before opening an issue

Please check:

- [README.md](README.md)
- [docs/why.md](docs/why.md)
- [docs/comparison.md](docs/comparison.md)
- the latest release notes in `changelogs/`

## Launching and monitoring

**Add installed...** shows supported launcher entries, rather than every executable on the PC. Use **Add file...** or drag an executable or supported launcher file onto its group if an app is missing. In the Linux beta, installed-app search uses desktop entries and matching `PATH` executables.

If affinity or priority is not applied, open **Process monitor** and check **Enable process monitor**. Verify the saved executable and extra process names, inspect **Processes**, and check **Logs** for a refused change. New rules manage their verified root process; enable **Manage descendant processes** only when you want their children managed as well. Protected or inaccessible processes can refuse changes even when the tool has administrator rights.

**Process monitor** applies saved rules. **CPU load** records activity independently. A group graph starts with tracked applications, so it can be empty while other programs are using those CPUs; choose **All system processes** to inspect that broader activity. Opening a service button only opens its controls. Existing enable/disable preferences are preserved across restarts.

**Start minimized** changes the next launch into a tray launch. It does not register the utility to start when you sign in to Windows.

## Configuration and saved shortcuts

New Windows configurations use `%LOCALAPPDATA%\CpuAffinityTool\state.json`. If `state.json` already exists beside the EXE, that legacy sidecar remains active. Open **Logs → Data folder** to find the active directory. Keep a backup before testing another build.

The development build saves schema v11. Loading an older supported format does not rewrite it immediately; an explicit save upgrades it. Loading pre-v6 data creates a `state.json.pre-v6*` backup before the first current-format save. Upgrading v6 or later does not create that particular backup. Downgrading after a current-format save is unsupported. MCP connection credentials are stored separately; see [MCP storage and access](docs/mcp.md).

Windows saved-rule shortcuts target the current `cpu-affinity-tool.exe` path. Recreate them after moving the portable folder. They use the same elevated executable and may show UAC. Shortcut creation uses the Desktop of the current elevated Windows account; approving UAC with another administrator's credentials can put the shortcut on that account's Desktop. Linux beta does not create saved-rule `.desktop` launchers.

## Graphics and agent access

The Windows renderer uses WGPU/DirectX 12. If its graphics device is lost, for example during a graphics-driver replacement, restart the application; the current renderer does not rebuild that device in place. Include your GPU, driver and display scaling when reporting a graphics issue.

For MCP connection setup, client reload requirements, the full-access key and command logs, use the [MCP guide](docs/mcp.md). Do not paste connection keys or the copied AI setup prompt into a public issue.

## What to include for diagnostics

For technical problems, include as much of the following as possible:

- Windows version
- CPU model
- CPU Affinity Tool version
- whether the target was added with **Add file...** or **Add installed...**
- the exact target you launched
- clear reproduction steps
- expected behavior
- actual behavior
- screenshots or log output
- a reviewed crash report, if the **Crash reports** page contains one for the incident

The Windows build stores crash reports under the app's active data directory and never uploads them automatically. After the next completed startup scan, **Logs** keeps a summary of the newest validated crash report even if normal activity is cleared. Open **Logs → Crash reports**, choose **Show in Explorer**, and review the full file before attaching it to a public issue. A report can contain local paths or other system details. If redaction is needed, make a copy and redact the copy instead of editing the app-managed original.

Windows Event Log diagnostics are enabled by default, so **Logs** can show an `Unverified Windows Event Log record`. It is a local, read-only lookup of recent `Application Error` records for this executable, not an upload or a root-cause determination. The displayed Record ID, UTC time, exception code, sanitized module basename, module version, faulting offset, and process creation time are useful when Windows supplied valid values. Do not attach raw Event Viewer exports or full event XML without reviewing them for local information. You can disable the lookup from Logs; disabling clears the visible record immediately. If saving the disable fails, the app reports that it is session-only and keeps lookups disabled for the current run. The lookup does not create dumps or modify Windows Error Reporting or the registry.

The app normally retains the newest 20 complete reports, up to 256 KiB each. Cleanup runs in the background. During repeated startup failures, a writer stops after observing 64 managed files; simultaneous instances can slightly exceed that safety ceiling. **Show in Explorer** requires a verified ordinary Windows shell. If one is unavailable, the path stays copyable and the app does not launch an editor as a fallback.

The absence of a crash report does not rule out a native crash, forced termination, out-of-memory termination, anti-cheat action, or another external stop. If the app disappeared without a report, also include:

- whether the process remained visible in Task Manager
- whether the tray icon remained available
- the relevant Reliability Monitor or Event Viewer entry
- the faulting module and exception code, if Windows recorded them

For an unreproducible native access violation, a maintainer may ask for a manually enabled Windows LocalDumps capture. It is never enabled by the application. Follow [docs/windows-native-crash-diagnostics.md](docs/windows-native-crash-diagnostics.md), use the exact release EXE/PDB pair, and review any dump before sharing: memory dumps can contain sensitive application and personal data.

## Security issues

Do not report security issues in public GitHub issues.

See [SECURITY.md](SECURITY.md) for private reporting instructions.
