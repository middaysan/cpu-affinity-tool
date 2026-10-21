# MR22 release preparation — 2026-10-21

This records the local preparation of the original `docs/modular-gaming-features-plan` branch. It is not a published release or an assertion that every supported Windows/driver configuration was exercised.

## Review results

Three independent Astra lanes reviewed correctness/lifecycle, performance/resources and architecture/integration. A fourth reviewed the elevated localhost MCP/native authority boundaries. Each inspected the current proposed tree, including untracked implementation files and the earlier MR commits; none claimed an exhaustive line-by-line audit of every vendor or native branch. No P0/P1 issue was established. Four concrete P2 issues were reproduced or characterized and corrected:

| Finding | Correction | Regression evidence |
| --- | --- | --- |
| Restarting MCP recreated its native-worker semaphore while old blocking calls could remain alive | One lazily allocated application-owned native budget is cloned into listener adapters; actual native closures retain their permits | Four blocked native closures, cancelled callers, recreated services, shared identity, refused fifth worker, then release and successful recovery |
| Scheduling provenance permanently filled after 1024 distinct process lifetimes | Bounded pressure-triggered native expiry probes outside the metadata lock; remove only exact keys positively known exited/replaced | Full-map reclamation, preserved alive/unknown reservations and attempted fields, concurrent PID reuse, no probes below pressure, native current-instance/error classification |
| Saved AUMID activation returned before the common Realtime guard | Resolve the effective saved-rule priority before either launch target or restart side effects | Realtime rejection, allowed normal/high priority and unchanged neutral launch for module-reserved priority |
| Oversized run labels bypassed the published input bound and could make all retained runs exceed the response cap | Validate launch names before admission; bound only retained display labels and exit summaries by Unicode count and escaped JSON bytes | Rejected empty/257-character names without advancing admission, accepted valid Unicode, exact operational IDs, full 64-row structured/text wrapper including mixed escaped strings |

The responsible lanes reviewed the combined deltas without finding another concrete blocker. These are source-review conclusions, not penetration-test or performance measurements. Full local reviewer reports and RED logs remain in the ignored task/evidence workspace.

## Local validation

- Windows application: 791 passed, 10 intentionally ignored opt-in probes; native library: 178 passed, 15 ignored plus one passing doctest.
- Debug MCP/diagnostics configuration: 820 passed, 11 ignored. Ordinary and debug Clippy passed with warnings denied; formatting and the Python MCP adapter's nine tests passed.
- Launcher isolation self-test, all required isolated tuning-recovery scenarios and both real-binary crash-report probes passed. The probes use their own native test objects/files and do not apply the user's Windows settings.
- Local eframe frame tests, Winit DPI rectangle-policy tests and epaint library tests passed. Stable tag builds now run these same three vendor gates before upload.
- Both lockfiles passed cargo-audit against RustSec commit `4b587f00996ebe5c05a73eaef105ef241e876414` without vulnerability ignores. Informational target-reachability findings are recorded in `dependency-advisories.md`; the selected release graph excludes the dev-only schema validator.

The final EXE/PDB identity, manifest, release-code fence and exact-head Windows/Linux CI are checked separately after integration. Historical CPU19 and memory measurements remain tied to their named artifacts in `mcp-validation.md` and `cpu19-performance-probe.md`; the latest review did not rerun a long resource soak.

## Remaining release decisions and limits

- Select the next version and create its versioned release notes before tagging. `Cargo.toml` still reports the already-published 1.6.3; the consolidated Unreleased section describes the new behavior. These checks do not authorize reusing the old release tag.
- Merge and release-tag/publication remain separate user decisions. A green old-head run is not acceptance of the newly integrated tree; inspect the final MR head's CI.
- Prior Windows 11 mixed-DPI/Explorer/tray and elevated MCP/CPU19 evidence remains host-specific. A fresh final-candidate native smoke/long soak can strengthen acceptance; Windows 10, RDP and other GPU/driver combinations are not certified here.
- The pinned renderer does not recover its graphics device in place during driver replacement. The earlier externally triggered AMD-driver-reset attempt remains excluded from performance evidence. No new renderer recovery stack or automatic relaunch was introduced during this review.
