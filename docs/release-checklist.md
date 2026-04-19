# Release Checklist

This repository currently releases a Windows-only desktop binary.
Linux backend code exists in the repository, but it is not part of the current CI or release contract.

## Before Tagging

- Confirm the release stays Windows-only: `.github/workflows/release.yml` should publish only `cpu-affinity-tool.exe` for `x86_64-pc-windows-msvc`.
- Confirm the CI contract still matches reality: `.github/workflows/ci.yml` runs on `windows-latest` and currently checks `cargo fmt --all -- --check`, `cargo clippy -- -D warnings`, and `cargo build --release`.
- Confirm no project docs claim full cross-platform support or Linux release parity.
- Confirm `README.md` and `AGENTS.md` describe Windows as the only current supported release path and Linux as experimental backend code.
- Confirm version markers are aligned manually: release tag `vX.Y.Z`, `Cargo.toml`, and `changelogs/vX.Y.Z.txt`.
- Review release-impacting files if they changed: `build.rs`, `app.manifest`, `assets/icon.ico`, `assets/cpu_presets.json`, and `.github/workflows/release.yml`.

## Build Verification

- Run `cargo build --release`.
- Confirm the expected Windows artifact exists at `target/release/cpu-affinity-tool.exe`.
- If `assets/cpu_presets.json` changed, confirm the binary was rebuilt after that change because presets are embedded via `include_str!`.

## Release Notes And Distribution

- Confirm the tag format is a stable tag: `vX.Y.Z`.
- Confirm `changelogs/*.txt` are up to date even though GitHub Release notes are generated automatically via `generate_release_notes: true`.
- Confirm installer packaging, code signing, checksums, winget, choco, and Linux artifacts are still absent, or update docs/workflows in the same change if that contract changed.
