# Release Checklist

This checklist covers the stable Windows desktop release path.
Linux beta prereleases use `docs/linux-beta-release-checklist.md` and `.github/workflows/release-linux-beta.yml`.
Use this checklist as the primary release doc. Use `docs/release-smoke-matrix.md` as the compact manual smoke reference for the actual Windows release path.
Use `docs/release-process.md` for the current automated tag-release flow and release-notes template.

## Before Tagging

- Confirm the release stays Windows-only: `.github/workflows/release.yml` should publish only `cpu-affinity-tool.exe` for `x86_64-pc-windows-msvc`.
- Confirm Linux beta prereleases stay isolated: `.github/workflows/release-linux-beta.yml` should publish only Linux beta prerelease assets for tags matching `linux-beta-v*`.
- Confirm the CI contract still matches reality: `.github/workflows/ci.yml` runs separate Windows and Linux beta jobs, cancels superseded runs per branch or PR, restores Rust cache, runs shared formatting and `libs/os_api` tests, keeps the Windows release-path checks on `windows-latest`, verifies the built Windows artifact manifest, and verifies the Linux beta binary on `ubuntu-24.04`.
- Confirm the tag-release gate matches reality: `.github/workflows/release.yml` validates `vX.Y.Z`, `Cargo.toml`, and `changelogs/vX.Y.Z.txt`, runs the same formatting, lint, `libs/os_api`, and root test gates, builds the Windows artifact, verifies its embedded manifest resource, and then publishes it.
- Confirm no project docs claim full cross-platform support or Linux release parity.
- Confirm `README.md` and `AGENTS.md` describe Windows as the primary stable released platform and Linux as a separate beta prerelease track without stable parity.
- Confirm `README.md` documents the administrator/UAC expectation from `app.manifest`, including that saved-rule shortcut launches may show UAC.
- Confirm `README.md` matches the current interface terminology: **Add installed...**, **Add file...**, **Overview**, **Activity**, **Monitoring active**, and **Pause monitor**.
- Confirm the bundled Inter font and its SIL Open Font License file are present under `assets/fonts` and included in the release commit.
- Confirm shortcut docs explain current elevated token Desktop placement, including credential-over-the-shoulder UAC placing shortcuts on the elevated account's Desktop.
- Confirm version markers are aligned manually before tagging: release tag `vX.Y.Z`, `Cargo.toml`, and `changelogs/vX.Y.Z.txt`. The workflow validates these again after the tag is pushed.
- Confirm the changelog and any release note summary call out the schema `v7` save boundary when applicable:
  - the first explicit save after loading pre-`v6` state writes `state.json.pre-v6*`
  - `v6` to `v7` saves do not write `state.json.pre-v6*`
  - downgrade to older binaries is unsupported after that first current-schema save
- Review release-impacting files if they changed: `build.rs`, `app.manifest`, `assets/icon.ico`, `assets/cpu_presets.json`, `scripts/assert-windows-release-manifest.ps1`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, and `.github/workflows/release-linux-beta.yml`.

## Build Verification

- Run `cargo fmt --all -- --check`.
- Run `cargo test --manifest-path libs/os_api/Cargo.toml`.
- Run `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`.
- Run `cargo test --features windows --bin cpu-affinity-tool`.
- Run `cargo build --release --features windows --bin cpu-affinity-tool`.
- Run `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/assert-windows-release-manifest.ps1 -Path target/release/cpu-affinity-tool.exe`.
- Run `cargo clippy --features linux --bin cpu-affinity-tool-linux -- -D warnings`.
- Run `cargo test --features linux --bin cpu-affinity-tool-linux`.
- Run `cargo build --release --features linux --bin cpu-affinity-tool-linux`.
- Confirm the expected Windows artifact exists at `target/release/cpu-affinity-tool.exe`.
- If `assets/cpu_presets.json` changed, confirm the binary was rebuilt after that change because presets are embedded via `include_str!`.
- Confirm a clean checkout contains `assets/fonts/InterVariable.ttf` before building because the font is embedded with `include_bytes!`.
- Run `cargo audit`; reconcile every vulnerability before release. The two current `quick-xml` exceptions and their build-time reachability assessment are documented in `docs/dependency-advisories.md`; do not ignore any additional advisory.

## Manual Smoke

- Run the release-path manual checks from `docs/release-smoke-matrix.md`.
- Run every release-blocking row in the `Shortcut MVP Smoke` table when saved-rule desktop shortcuts are included in the release notes.
- Smoke the redesigned **Overview** and **Activity** routes in system, dark, and light themes.
- Check Inter rendering at 100%, 125%, 150%, and 200% Windows display scaling, including Latin, Cyrillic, digits, punctuation, long group/app names, and fallback glyphs.
- Check compact layout and clipping at the minimum supported window size and at a typical 1920x1080 work area.
- Reorder groups with both pointer drag-and-drop and the keyboard-accessible reorder path, restart, and verify order plus saved-rule shortcut identity.
- Verify **Fix** on a mismatched running app reapplies affinity and priority without launching or focusing; it may show the protected/green state only after every OS setting call succeeds.
- Verify **Focus** on a correctly configured running app only activates its existing window and does not reapply settings or start another process.
- Verify the theme selector shows the painted system, light, and dark icons without missing-glyph squares in every theme.
- Verify selected CPU threads remain clearly distinguishable with the restrained turquoise primary color in both dark and light themes.
- Select scattered Performance and Efficient threads and verify every CPU control wraps within the fixed-width editor, keeps both its core and `thread N` labels visible, remains reachable, and does not shift or overflow when selected.
- Verify long app statuses expose their full detail and small secondary text remains readable in both dark and light themes.
- Treat the `Installed/AUMID rule` shortcut row as release-blocking while docs describe saved-rule shortcuts broadly; if it cannot be smoked, scope the docs and release notes to path-target shortcuts before release.
- Treat failures in startup, launch, monitoring, tray restore, persistence, logging, shortcut Desktop placement documentation, or UAC expectation as release-blocking until resolved or explicitly downgraded.

## Release Notes And Distribution

- Confirm the tag format is a stable tag: `vX.Y.Z`.
- Confirm `changelogs/vX.Y.Z.txt` is up to date because `.github/workflows/release.yml` uses it as the published GitHub Release body.
- Confirm installer packaging, code signing, winget, choco, AppImage, Flatpak, and Linux stable release artifacts are still absent from the stable release contract, or update docs/workflows in the same change if that contract changed.
