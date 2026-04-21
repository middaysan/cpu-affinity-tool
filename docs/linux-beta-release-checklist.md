# Linux Beta Release Checklist

This checklist covers the Linux desktop beta prerelease path.
Stable Windows releases continue to use `docs/release-checklist.md` and `.github/workflows/release.yml`.
Use this checklist with `docs/release-process.md`.

## Before Tagging

- Confirm the Linux beta release stays isolated: `.github/workflows/release-linux-beta.yml` should trigger only for tags matching `linux-beta-v*` and should publish only Linux beta prerelease assets.
- Confirm the stable Windows release path remains separate: `.github/workflows/release.yml` should still react only to stable tags matching `v*`.
- Confirm the CI contract still matches reality: `.github/workflows/ci.yml` runs separate Windows and Linux beta jobs, and the Linux beta job still verifies formatting, Linux clippy, `libs/os_api` tests, Linux binary tests, and the Linux release build.
- Confirm no project docs claim full cross-platform support, Linux stable parity, AppImage support, or Flatpak support.
- Confirm `README.md` and `AGENTS.md` describe Linux as a desktop beta path with prerelease artifacts under `linux-beta-v*` tags.
- Confirm the base version in `Cargo.toml` matches the `X.Y.Z` segment of the Linux beta tag you plan to push.
- Confirm the beta changelog exists at `changelogs/linux-beta-vX.Y.Z-N.txt`.
- Review release-impacting files if they changed: `assets/cpu_presets.json`, `.github/workflows/ci.yml`, `.github/workflows/release-linux-beta.yml`, `README.md`, `docs/release-process.md`, and `AGENTS.md`.

## Build Verification

- Run `cargo fmt --all -- --check`.
- Run `cargo clippy --features linux --bin cpu-affinity-tool-linux -- -D warnings`.
- Run `cargo test --manifest-path libs/os_api/Cargo.toml`.
- Run `cargo test --features linux --bin cpu-affinity-tool-linux`.
- Run `cargo build --release --features linux --bin cpu-affinity-tool-linux`.
- Confirm the expected Linux artifact exists at `target/release/cpu-affinity-tool-linux`.
- If `assets/cpu_presets.json` changed, confirm the binary was rebuilt after that change because presets are embedded via `include_str!`.

## Manual Smoke

- Launch the built Linux binary on a supported `x86_64` `glibc` desktop session running `X11` or `Wayland`.
- Confirm the window opens and the detected CPU core count looks correct.
- Add an app target by path or `.desktop` entry and confirm it appears once.
- Launch the target and confirm affinity and priority are applied as expected.
- If monitoring is expected for the beta, confirm re-apply behavior and logs still work.
- Treat startup failures, incorrect CPU topology, launch failures, missing logs, or broken affinity application as beta-blocking until resolved or explicitly downgraded.

## Release Notes And Distribution

- Confirm the tag format is `linux-beta-vX.Y.Z-N`.
- Confirm `changelogs/linux-beta-vX.Y.Z-N.txt` is up to date because `.github/workflows/release-linux-beta.yml` uses it as the published GitHub prerelease body.
- Confirm the published asset set stays minimal:
  - `cpu-affinity-tool-linux-x86_64`
  - `cpu-affinity-tool-linux-x86_64.tar.gz`
  - `SHA256SUMS.txt`
- Confirm AppImage, Flatpak, `.deb`, `.rpm`, installer packaging, code signing, and Linux stable release artifacts are still absent, or update docs/workflows in the same change if that contract changed.
