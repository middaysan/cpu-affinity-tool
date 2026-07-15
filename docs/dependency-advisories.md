# Dependency advisory review

This file records reviewed RustSec findings that remain in the lockfile so release checks do not silently normalize known advisories.

## `quick-xml 0.39.4`

Reviewed: 2026-07-16

Advisories:

- `RUSTSEC-2026-0194` - quadratic duplicate-attribute checking
- `RUSTSEC-2026-0195` - unbounded namespace-declaration allocation in `NsReader`

Dependency path:

`eframe` / `winit` / Wayland crates -> `wayland-scanner 0.31.10` -> `quick-xml 0.39.4`

Release assessment:

- `quick-xml` is not part of the Windows stable runtime dependency graph.
- On the Linux beta path it is used by the `wayland-scanner` build-time procedural macro to parse Wayland protocol XML supplied by pinned crate sources.
- `wayland-scanner 0.31.10` uses `quick_xml::Reader`, not `NsReader`, so the `RUSTSEC-2026-0195` code path is not reached.
- The remaining duplicate-attribute path does not parse user, network, state, launcher, or application input. Its inputs are trusted build inputs from the resolved dependency sources.
- `wayland-scanner 0.31.10` currently constrains `quick-xml` below the patched `0.41.0` line, so Cargo cannot resolve the patched version without vendoring or replacing upstream Wayland components.

Decision:

Accept this as a build-time, non-user-input exposure for `v1.5.0`. Do not remove Wayland beta support or vendor a private scanner fork solely to force the transitive upgrade. Re-evaluate when `wayland-scanner` accepts `quick-xml >=0.41.0`, when the GUI stack is updated, or if the dependency begins parsing untrusted XML.

Audit command for the reviewed lockfile:

```bash
cargo audit --ignore RUSTSEC-2026-0194 --ignore RUSTSEC-2026-0195
```

The unignored audit must still be inspected so new advisories cannot hide behind these two reviewed exceptions.
