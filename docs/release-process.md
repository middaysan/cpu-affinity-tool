# Release Process

This document describes the current release contract for CPU Affinity Tool.

It is intentionally narrow and truthful:

- stable releases are Windows-only
- Windows stable tags publish `cpu-affinity-tool.exe` and its matching `cpu_affinity_tool.pdb`
- Linux beta prerelease tags publish a raw Linux binary, a `tar.gz`, and `SHA256SUMS.txt`
- CI validates the Linux beta build/test/clippy path from source on pinned Ubuntu runners; desktop-session smoke remains manual beta validation
- there is no installer, AppImage, Flatpak, code signing, or winget package in the current release contract

## Source of truth

Use these files together:

- `docs/release-checklist.md`
- `docs/linux-beta-release-checklist.md`
- `docs/release-smoke-matrix.md`
- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `.github/workflows/release-linux-beta.yml`
- `scripts/build-windows-release.ps1`
- `scripts/assert-windows-pdb-matches.ps1`
- `scripts/test-windows-pdb-verifier.ps1`
- `scripts/assert-windows-release-manifest.ps1`
- `changelogs/vX.Y.Z.txt`
- `changelogs/linux-beta-vX.Y.Z-N.txt`

## Current automated flow

Release publishing is tag-based.

Normal branch and pull-request CI runs both:

- the Windows release-path validation job
- the Linux beta build/test validation job on `ubuntu-24.04`

Stable Windows publishing and Linux beta publishing are separate workflows with separate tag namespaces.

When a stable tag matching `v*` is pushed:

1. GitHub Actions runs the Windows release job
2. the workflow validates the tag format against `vX.Y.Z`
3. the workflow confirms `Cargo.toml` version matches `X.Y.Z`
4. the workflow requires `changelogs/vX.Y.Z.txt`
5. the workflow runs formatting, `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `libs/os_api` tests, `cargo test --features windows --bin cpu-affinity-tool`, and the line-table production build through `scripts/build-windows-release.ps1`
6. the workflow requires `cpu_affinity_tool.pdb` to match the EXE's embedded PDB basename, CodeView GUID, and age through `scripts/assert-windows-pdb-matches.ps1`, then runs `scripts/assert-windows-release-manifest.ps1` against the built exe and requires `requestedExecutionLevel=requireAdministrator` plus `uiAccess=false`
7. the workflow uploads `cpu-affinity-tool.exe` and its matching `cpu_affinity_tool.pdb`
8. a publish job on `ubuntu-24.04` creates the GitHub Release with the body from `changelogs/vX.Y.Z.txt`

When a Linux beta tag matching `linux-beta-v*` is pushed:

1. GitHub Actions runs the Linux beta prerelease job on `ubuntu-24.04`
2. the workflow validates the tag format against `linux-beta-vX.Y.Z-N`
3. the workflow confirms `Cargo.toml` version matches `X.Y.Z`
4. the workflow requires `changelogs/linux-beta-vX.Y.Z-N.txt`
5. the workflow runs formatting, Linux clippy, `libs/os_api` tests, Linux binary tests, and the Linux release build
6. the workflow publishes a prerelease with:
   - `cpu-affinity-tool-linux-x86_64`
   - `cpu-affinity-tool-linux-x86_64.tar.gz`
   - `SHA256SUMS.txt`

## Release inputs that must be aligned manually

Before pushing a stable Windows tag, align:

- Git tag: `vX.Y.Z`
- `Cargo.toml` version
- `changelogs/vX.Y.Z.txt`
- release-facing docs if platform or process truth changed
- if the release includes the first shipped schema `v8` build, call out that the first explicit save upgrades `state.json` to `v8`; loading pre-`v6` state writes an additional `state.json.pre-v6*` backup before that save, while `v6` or `v7` to `v8` does not

Before pushing a Linux beta tag, align:

- Git tag: `linux-beta-vX.Y.Z-N`
- `Cargo.toml` version: `X.Y.Z`
- `changelogs/linux-beta-vX.Y.Z-N.txt`
- Linux beta release-facing docs if platform or process truth changed
- if the prerelease includes the first shipped schema `v8` build, call out that the first explicit save upgrades `state.json` to `v8`; loading pre-`v6` state writes an additional `state.json.pre-v6*` backup before that save, while `v6` or `v7` to `v8` does not

## Recommended stable release steps

1. Update `Cargo.toml` to the target version.
2. Create or update `changelogs/vX.Y.Z.txt`.
3. Run the release checklist in `docs/release-checklist.md`.
4. Run the manual smoke from `docs/release-smoke-matrix.md`.
5. Push the release commit.
6. Push the stable tag `vX.Y.Z`.
7. Confirm the GitHub Release workflow succeeded and published `cpu-affinity-tool.exe` plus `cpu_affinity_tool.pdb`.

## Recommended Linux beta release steps

1. Confirm `Cargo.toml` already contains the target base version `X.Y.Z`.
2. Create or update `changelogs/linux-beta-vX.Y.Z-N.txt`.
3. Run the Linux beta checklist in `docs/linux-beta-release-checklist.md`.
4. Push the release commit if needed.
5. Push the Linux beta tag `linux-beta-vX.Y.Z-N`.
6. Confirm the prerelease workflow succeeded and published the Linux beta assets.

## Current artifact contract

Stable published artifact:

- `cpu-affinity-tool.exe`
- `cpu_affinity_tool.pdb`

`scripts/build-windows-release.ps1` sets `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` for the stable Windows build. This leaves the shared release profile plus the Linux beta artifact set and debug-information policy unchanged. Windows pull-request CI tests the verifier against matching and deliberately mismatched PDB inputs, while the stable workflow requires matching EXE/PDB basename, CodeView GUID, and age before upload. The Windows CI and stable release workflows also verify the built exe's embedded `RT_MANIFEST` resource with `scripts/assert-windows-release-manifest.ps1`. The manifest script confirms `requireAdministrator` and `uiAccess=false`; actual UAC prompt behavior still belongs to manual Windows smoke.

Linux beta prerelease artifacts:

- `cpu-affinity-tool-linux-x86_64`
- `cpu-affinity-tool-linux-x86_64.tar.gz`
- `SHA256SUMS.txt`

Current non-goals for these workflows:

- installer creation
- AppImage
- Flatpak
- code signing
- winget or choco publication
- Linux stable release parity

Those can be added later, but they are not part of the current release truth.

## Release notes template

Each stable or Linux beta release note file in `changelogs/` should follow this structure:

```md
## [X.Y.Z] - YYYY-MM-DD

### Summary
- Short high-level release summary

### Added
- New capabilities

### Changed
- Behavior changes, docs changes, workflow changes

### Fixed
- Bug fixes

### Known issues
- Current limitations worth calling out

### Download
- Link or note pointing to the GitHub Release artifact
```

## Related docs

- Use [docs/github-metadata.md](github-metadata.md) for manual GitHub UI settings such as repository description, topics, and social preview.
