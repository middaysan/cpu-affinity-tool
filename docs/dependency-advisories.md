# Dependency advisory review

This file records reviewed RustSec findings that remain in the lockfile so release checks do not silently normalize known advisories.

## Release 1.7.1 audit (2026-10-28)

Both lockfiles pass without advisory ignores against RustSec database commit `40f5be6fb12ed93fc591bf4ab23b7d793b3f2239` (2026-10-28), with zero vulnerabilities. The five informational findings in the 2026-10-21 table below remain unchanged; the separate OS API lockfile has no warnings.

The 1.7.2 lockfiles were rechecked against the same revision with the same results. No dependency versions changed for that patch release.

The release check found two new items and resolved them with targeted patch updates:

- `tokio 1.53.2` was reported by [RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html). The affected `AsyncFd` constructors are Unix-only, so this is not a Windows-path exposure. The manifest minimum and lockfile now select patched `1.53.3`, retaining the existing feature set; see the [upstream I/O-safety change](https://github.com/tokio-rs/tokio/pull/8587).
- Yanked `clipboard-win 5.4.1`, selected through arboard on Windows, was replaced by [5.4.3](https://crates.io/crates/clipboard-win/5.4.3). The small source diff corrects HTML range copying and clipboard-lock retry yielding, with no license or feature change. The application uses the existing Unicode clipboard integration.

No advisory ignore, broad dependency refresh or platform claim was introduced. Existing Windows/Linux CI and the stable release gates validate the final locked graph.

## Current audit review (2026-10-21)

Both lockfiles were rechecked against RustSec database commit `4b587f00996ebe5c05a73eaef105ef241e876414` (2026-10-21). Both audits pass without advisory ignores and report zero vulnerabilities. The separate `libs/os_api` lockfile has no informational warnings. No dependency or lockfile was changed for this review.

The application lockfile reports these informational findings:

| Package | Advisory | Kind | Windows stable graph | Linux beta graph |
| --- | --- | --- | --- | --- |
| `proc-macro-error 1.0.4` | `RUSTSEC-2024-0370` | Unmaintained | Absent | Absent |
| `glib 0.18.5` | `RUSTSEC-2024-0429` | Unsound | Absent | Absent |
| `zbus_names 4.3.4` | `RUSTSEC-2026-0371` | Unmaintained | Absent | Present |
| `zvariant 5.15.0` | `RUSTSEC-2026-0370` | Unmaintained | Absent | Present |
| `zvariant_derive 5.15.0` | `RUSTSEC-2026-0372` | Unmaintained | Absent | Present |

The three new findings were published on 2026-10-16. These standalone crates moved into `zbus 6`: `names`, `wire`, and its derive macros. They remain transitive dependencies of the selected Linux beta graph; their maintenance notices do not report a vulnerability. Retain the locked versions for this change and follow the upstream dependency stack's migration rather than forcing a major transitive upgrade or adding advisory ignores. Re-evaluate when that stack supports `zbus 6`, when supported target features change, or when a security advisory affects the retained versions. The unused `proc-macro-error` and `glib` findings remain subject to the same target-reachability check.

Audit and selected normal/build graph checks:

```bash
cargo audit
cargo audit --file libs/os_api/Cargo.lock
cargo tree --locked --features windows --target x86_64-pc-windows-msvc -e normal,build
cargo tree --locked --features linux --target x86_64-unknown-linux-gnu -e normal,build
```

The former `quick-xml` vulnerabilities remain resolved by `wayland-scanner 0.31.11` with `quick-xml 0.41.0`. The dated dependency-update and advisory assessments below are historical and apply to the versions recorded there.

## Dependency update (2026-10-13)

- Rechecked both updated lockfiles against RustSec database commit `6aed84668c23d7f535077a9fe918bf1fa0f6a5bc` (2026-10-13).
- The application and separate `libs/os_api` audits pass without advisory exceptions and report zero vulnerabilities. Both include the yanked-package lookup.
- `wayland-scanner` is now `0.31.11` and accepts the patched `quick-xml 0.41.0`; both previously accepted XML advisories are resolved.
- The application lockfile still reports `proc-macro-error 1.0.4` (`RUSTSEC-2024-0370`, unmaintained) and `glib 0.18.5` (`RUSTSEC-2024-0429`, unsound) as informational warnings. Neither package is present in the selected Windows stable or Linux beta dependency graph, as checked with `cargo tree --locked --features windows --target x86_64-pc-windows-msvc -i <package>` and the equivalent Linux feature/target command. They remain in Cargo's all-target/optional dependency resolution; do not replace upstream GTK components with private forks to alter an unused graph.
- The separate OS API lockfile reports no informational warnings.
- All direct registry dependencies match their latest non-yanked stable releases at the check date. A final `cargo update --dry-run --verbose` proposes no compatible changes. Three older TOML-family versions remain constrained by the unused GTK build graph's upstream exact requirements (`proc-macro-crate 2.0.2` pins `toml_datetime =0.6.3` and `toml_edit =0.20.2`); no application override or vendored fork is introduced.

## v1.6.3 recheck (2026-09-20)

- Rechecked against RustSec database commit `d5c17953a895cf19e8d3ce66eaa42b6fcfe1fb16` (2026-09-19).
- The unignored application audit reports only the two documented `quick-xml` advisories below; the existing build-time assessment remains applicable to the unchanged dependency versions.
- The audit passes with only `RUSTSEC-2026-0194` and `RUSTSEC-2026-0195` ignored. Informational findings remain `proc-macro-error` (`RUSTSEC-2024-0370`, unmaintained) and `glib` (`RUSTSEC-2024-0429`, unsound); the latter remains absent from both supported target graphs.
- The separate `libs/os_api/Cargo.lock` advisory scan reports no findings. Its yanked-package lookup was not requested; the successful application audit included that lookup after retrying an initial registry timeout.

## `quick-xml 0.39.4`

Reviewed: 2026-09-01

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

Accept this as a build-time, non-user-input exposure for `v1.6.0`. Do not remove Wayland beta support or vendor a private scanner fork solely to force the transitive upgrade. Re-evaluate when `wayland-scanner` accepts `quick-xml >=0.41.0`, when the GUI stack is updated, or if the dependency begins parsing untrusted XML.

Historical audit command for that lockfile:

```bash
cargo audit --ignore RUSTSEC-2026-0194 --ignore RUSTSEC-2026-0195
```

The unignored audit must still be inspected so new advisories cannot hide behind these two reviewed exceptions.

## Resolved during the `v1.6.0` review

- Updated `webbrowser` from 1.2.1 to 1.2.2, resolving `RUSTSEC-2026-0257` in the Unix `BROWSER` handling path.
- Updated `event-listener` from 5.4.1 to 5.4.2, resolving the `RUSTSEC-2026-0221` thread-safety warning in the Linux accessibility dependency path.
- `cargo audit --ignore RUSTSEC-2026-0194 --ignore RUSTSEC-2026-0195` reports no remaining vulnerabilities. Informational unmaintained warnings remain in resolved transitive dependencies; the GTK3/GLib entries reported from the all-target lockfile are not present in either the Windows stable or Linux beta target graph.
