# Release Process

This document describes the current release contract for CPU Affinity Tool.

It is intentionally narrow and truthful:

- releases are Windows-only
- the published artifact is currently `cpu-affinity-tool.exe`
- there is no installer, zip package, code signing, checksum publication, winget package, or Linux artifact in the current release contract

## Source of truth

Use these files together:

- `docs/release-checklist.md`
- `docs/release-smoke-matrix.md`
- `.github/workflows/release.yml`
- `changelogs/vX.Y.Z.txt`

## Current automated flow

The release workflow is tag-based.

When a stable tag matching `v*` is pushed:

1. GitHub Actions runs the Windows release job
2. the workflow runs formatting, clippy, `libs/os_api` tests, root tests, and the Windows release build
3. the workflow uploads `cpu-affinity-tool.exe`
4. GitHub Release is created with the body from `changelogs/vX.Y.Z.txt`

## Release inputs that must be aligned manually

Before pushing a release tag, align:

- Git tag: `vX.Y.Z`
- `Cargo.toml` version
- `changelogs/vX.Y.Z.txt`
- release-facing docs if platform or process truth changed

## Recommended release steps

1. Update `Cargo.toml` to the target version.
2. Create or update `changelogs/vX.Y.Z.txt`.
3. Run the release checklist in `docs/release-checklist.md`.
4. Run the manual smoke from `docs/release-smoke-matrix.md`.
5. Push the release commit.
6. Push the stable tag `vX.Y.Z`.
7. Confirm the GitHub Release workflow succeeded and published `cpu-affinity-tool.exe`.

## Current artifact contract

Current published artifact:

- `cpu-affinity-tool.exe`

Current non-goals for this workflow:

- zip packaging
- installer creation
- code signing
- checksum publishing
- Linux artifacts

Those can be added later, but they are not part of the current release truth.

## Release notes template

Each release note file in `changelogs/` should follow this structure:

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
