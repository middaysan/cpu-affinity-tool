# Release Checklist

This repository currently releases a Windows-only desktop binary.
Linux desktop beta code exists in the repository and is part of CI, but it is not part of the published release artifact contract.
Use this checklist as the primary release doc. Use `docs/release-smoke-matrix.md` as the compact manual smoke reference for the actual Windows release path.
Use `docs/release-process.md` for the current automated tag-release flow and release-notes template.

## Before Tagging

- Confirm the release stays Windows-only: `.github/workflows/release.yml` should publish only `cpu-affinity-tool.exe` for `x86_64-pc-windows-msvc`.
- Confirm the CI contract still matches reality: `.github/workflows/ci.yml` runs separate Windows and Linux beta jobs, cancels superseded runs per branch or PR, restores Rust cache, runs shared formatting and `libs/os_api` tests, keeps the Windows release-path checks on `windows-latest`, and verifies the Linux beta binary on `ubuntu-latest`.
- Confirm the tag-release gate matches reality: `.github/workflows/release.yml` runs the same formatting, lint, `libs/os_api`, and root test gates and then builds the Windows artifact in the same Windows job before publishing it.
- Confirm no project docs claim full cross-platform support or Linux release parity.
- Confirm `README.md` and `AGENTS.md` describe Windows as the primary released platform and Linux as a source-only desktop beta without release artifacts.
- Confirm `README.md` documents the administrator/UAC expectation from `app.manifest`.
- Confirm version markers are aligned manually: release tag `vX.Y.Z`, `Cargo.toml`, and `changelogs/vX.Y.Z.txt`.
- Review release-impacting files if they changed: `build.rs`, `app.manifest`, `assets/icon.ico`, `assets/cpu_presets.json`, and `.github/workflows/release.yml`.

## Build Verification

- Run `cargo test --manifest-path libs/os_api/Cargo.toml`.
- Run `cargo test`.
- Run `cargo build --release`.
- Run `cargo test --features linux --bin cpu-affinity-tool-linux`.
- Run `cargo build --release --features linux --bin cpu-affinity-tool-linux`.
- Confirm the expected Windows artifact exists at `target/release/cpu-affinity-tool.exe`.
- If `assets/cpu_presets.json` changed, confirm the binary was rebuilt after that change because presets are embedded via `include_str!`.

## Manual Smoke

- Run the release-path manual checks from `docs/release-smoke-matrix.md`.
- Treat failures in startup, launch, monitoring, tray restore, persistence, logging, or UAC expectation as release-blocking until resolved or explicitly downgraded.

## Release Notes And Distribution

- Confirm the tag format is a stable tag: `vX.Y.Z`.
- Confirm `changelogs/vX.Y.Z.txt` is up to date because `.github/workflows/release.yml` uses it as the published GitHub Release body.
- Confirm installer packaging, code signing, checksums, winget, choco, and published Linux artifacts are still absent, or update docs/workflows in the same change if that contract changed.
