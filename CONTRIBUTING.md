# Contributing

Thanks for your interest in improving CPU Affinity Tool.

Windows is the primary stable release platform. Linux has a separate x86_64 glibc desktop beta, with CI build/test coverage and manual X11/Wayland smoke checks. CPU recording, System Tweaks and production MCP remain Windows-only.

## Build basics

Use Rust **1.95 or newer**. On Windows, install the MSVC C++ build tools and Windows SDK, then build the production EXE and matching PDB with:

```powershell
pwsh -NoProfile -File scripts/build-windows-release.ps1
```

For a Windows development session:

```powershell
cargo run --features windows --bin cpu-affinity-tool
```

For the Linux desktop beta:

```sh
cargo run --features linux --bin cpu-affinity-tool-linux
cargo build --release --features linux --bin cpu-affinity-tool-linux
```

Debian/Ubuntu-like systems may need `libclang-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev`. Linux CI and beta prerelease builds use pinned Ubuntu 24.04 runners. Tray, taskbar and focus behavior does not have full Windows parity.

## Validation

Run formatting and the native library tests on the relevant platform:

```sh
cargo fmt --all -- --check
cargo test --manifest-path libs/os_api/Cargo.toml
```

On Windows:

```powershell
cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings
cargo test --features windows --bin cpu-affinity-tool
pwsh -NoProfile -File scripts/build-windows-release.ps1
pwsh -NoProfile -File scripts/test-windows-pdb-verifier.ps1 -ExePath target/release/cpu-affinity-tool.exe -PdbPath target/release/cpu_affinity_tool.pdb
pwsh -NoProfile -File scripts/assert-windows-release-manifest.ps1 -Path target/release/cpu-affinity-tool.exe
```

On Linux:

```sh
cargo clippy --features linux --bin cpu-affinity-tool-linux -- -D warnings
cargo test --features linux --bin cpu-affinity-tool-linux
cargo build --release --features linux --bin cpu-affinity-tool-linux
```

The [release checklist](docs/release-checklist.md) adds native crash/recovery probes, dependency audits, vendor regressions and manual Windows smoke. The [Linux beta checklist](docs/linux-beta-release-checklist.md) covers its separate prerelease path. See [release process](docs/release-process.md) for version, tag and artifact requirements.

See [testing](docs/testing.md) for comparable test-build measurements and the
synthetic built-app MCP suite. Live scenarios supplement the deterministic
identity, ownership, CPU/RAM, persistence and recovery regressions.

## Reporting bugs

Please use the bug report issue template and include:

- Windows version
- CPU model
- CPU Affinity Tool version
- exact reproduction steps
- expected behavior
- actual behavior
- screenshots or logs when possible

If the problem involves launch, monitoring, or affinity correction, include whether the target was added with **Add file...** or **Add installed...**.

## Requesting features

Please use the feature request template.

Feature requests should explain:

- the real problem
- the proposed behavior
- alternatives you considered

Do not open large implementation pull requests for new features without discussing them first.

## Pull request expectations

- Keep changes scoped
- Avoid mixing refactors with feature work unless they are required
- Update documentation when behavior, platform truth, release process, or repository structure changes
- Preserve truthful Windows-first positioning
- Do not add claims about unsupported platforms or unverified performance gains

## Coding and review expectations

- Keep comments in English
- Keep user-facing repository text in English
- Prefer targeted changes over broad rewrites
- Follow TDD for behavior changes: add or update the failing or characterization test before changing production code whenever technically possible
- Include a regression test for every bug fix, especially for UI state transitions, drag and drop, storage migrations, process tracking, and launch behavior
- If an OS or GUI interaction cannot be tested deterministically, add the closest reliable unit or state-level test and document the manual smoke validation that remains
- Run the relevant checks before asking for review
