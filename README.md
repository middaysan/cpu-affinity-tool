# CPU Affinity Tool

Windows utility for managing CPU affinity of games and background applications.

- Officially supported platform: Windows
- Download: [Latest Release](https://github.com/middaysan/cpu-affinity-tool/releases/latest)
- License: [MIT](./LICENSE)

![CPU Affinity Tool main window](assets/screenshot.png)

## What problem it solves

Games, browsers, launchers, Discord, overlays, recording tools, and work apps can compete for the same CPU time. Windows can handle many workloads well on its own, but some systems behave better when the foreground workload and background workload are kept apart.

CPU Affinity Tool lets you save and apply repeatable launch rules instead of reopening Task Manager or re-running command-line commands every time.

It is meant to give you explicit control over:

- which CPU cores an app can use
- which priority class an app starts with
- whether those settings should be re-applied while the app is running

This is a control tool, not a promise of better FPS.

## Who this is for

- Gamers who keep browsers, Discord, launchers, overlays, or recording tools open in the background
- Users with hybrid CPUs, multi-chiplet CPUs, or other workloads where core placement matters
- Users who want repeatable launch rules instead of one-off Task Manager changes
- Users who want saved affinity and priority rules for a fixed set of games or applications

## Features

- Save CPU core groups for different workloads
- Launch apps with saved affinity and priority rules
- Add apps from `.exe`, `.lnk`, and `.url` targets with **Open App**
- Add supported installed Windows apps with **Find Installed**
- Re-apply affinity and priority while monitoring is enabled
- Autorun selected apps with the tool
- Add targets by drag and drop
- Inspect launch and monitoring activity in the built-in log view
- Open the active data folder directly from the log screen
- Switch between light, dark, and system theme modes

## When it helps

- A game is CPU-bound and background apps are competing for the same cores
- You have heavy background activity such as browsers, voice chat, launchers, or recording tools
- Your CPU layout is asymmetric or segmented, such as hybrid-core or multi-chiplet designs
- You want a repeatable launch layout that stays consistent across runs

## When it does not

- The real bottleneck is the GPU
- Your background load is already light enough that contention is negligible
- The game or application does not respond well to affinity pinning
- Windows scheduling already behaves well for your workload

For a longer explanation, see [docs/why.md](docs/why.md).

## Quick Start

1. Download the latest Windows release and run `cpu-affinity-tool.exe`.
2. Accept the UAC prompt. The current Windows build requires administrator rights.
3. Create a CPU core group for the workload you want to isolate.
4. Add an app with **Open App**, **Find Installed**, or drag and drop.
5. Set the desired affinity and priority, then save the rule.
6. Launch the app from the tool and keep monitoring enabled if you want settings re-applied automatically.

## Comparison

| Tool | Saved launch rules | Monitoring / re-apply | Complexity | Best fit |
| --- | --- | --- | --- | --- |
| CPU Affinity Tool | Yes | Yes | Low | Focused Windows affinity workflows with saved rules |
| Task Manager | No | No | Low | One-off manual changes |
| Process Lasso | Yes | Yes | Medium | Broader third-party process automation on Windows |
| PowerShell / CLI methods | Script-dependent | Script-dependent | High | Users who want fully manual or scripted control |

See [docs/comparison.md](docs/comparison.md) for a fuller breakdown.

## FAQ

### Does it improve FPS?

Not by itself. It can help when CPU contention is real, but it is not a guaranteed FPS booster.

### Does it work for every game?

No. Some games benefit, some show little change, and some do not react well to manual affinity pinning.

### Do I need administrator privileges?

Yes. The current Windows release is built with `requireAdministrator`, so you should expect a UAC prompt on launch.

### Can Windows or the application override affinity settings?

Yes. Some applications or helper processes may change affinity or priority after launch. That is why the tool includes monitoring and re-apply behavior.

### Is this an alternative to Process Lasso?

It overlaps with a narrower part of that use case. CPU Affinity Tool is a focused Windows utility for saved affinity and priority workflows, not a full replacement for every Process Lasso feature.

### Is Linux or macOS supported?

No official Linux or macOS release is currently supported. The repository contains an experimental Linux path, but only Windows is officially supported and released at the moment.

### Where is the configuration stored?

If `state.json` already exists next to the executable, the app keeps using that legacy sidecar file. Otherwise, new Windows installs default to `%LOCALAPPDATA%\CpuAffinityTool\state.json`.

## Troubleshooting

### The app I want is not listed in Find Installed

Use **Open App** instead. The installed-app picker is a launch-safe Windows subset, not a full inventory of every app on the system.

### Affinity or priority is not applied

- Make sure you launched the target from CPU Affinity Tool
- Keep monitoring enabled if the app changes settings after launch
- Verify that you accepted the UAC prompt and the tool is running elevated

### The application overrides the configured settings

Some applications spawn helper processes or reset their own settings. Monitoring is designed to correct that, but it may not cover every application behavior.

### Windows security software or antivirus prompts appear

The current release is a Windows executable that requests elevation. Verify that you downloaded it from the official release page and that the prompt matches the expected executable path.

### Shortcut or launch path behavior looks wrong

Try **Open App** with the direct executable path. For installed apps, try **Find Installed** first and fall back to **Open App** if the target is not listed correctly.

## Build from source

Supported build target for the official product path:

- Windows
- Rust stable toolchain

Basic release build:

```bash
cargo build --release --bin cpu-affinity-tool
```

Useful verification commands:

```bash
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test --manifest-path libs/os_api/Cargo.toml
cargo test
cargo build --release
```

Expected Windows artifact:

```text
target/release/cpu-affinity-tool.exe
```

For release-process details, see [docs/release-process.md](docs/release-process.md).

## Contributing, Support, Security, License

- Contributing guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Support and diagnostics: [SUPPORT.md](SUPPORT.md)
- Security reporting: [SECURITY.md](SECURITY.md)
- License: [LICENSE](LICENSE)
