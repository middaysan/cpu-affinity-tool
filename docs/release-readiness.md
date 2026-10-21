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

The responsible lanes reviewed the combined deltas without finding another concrete blocker. Exact-head CI additionally exposed platform-only accessor/test cfg gaps and a newly deprecated Atomic API on Rust 1.99. The cfg gates were corrected; equivalent long-stable CAS loops retain Rust 1.95 compatibility, with concurrent output-budget and lifetime-identity exhaustion characterizations. These are source-review conclusions, not penetration-test or performance measurements. Full local reviewer reports and RED logs remain in the ignored task/evidence workspace.

A fifth focused release/packaging lane inspected workflow gates, manifest/PDB/debug fences, retained vendor licenses and archive provenance without establishing another technical blocker. Its documentation follow-up corrected current smoke instructions for header controls, process modals, tracked-group load/scope, selection cues and activity dots, and dated Winit's initial test deferral. Dated native measurements were preserved.

## Local validation

- Windows application: 792 passed, 10 intentionally ignored opt-in probes; native library: 179 passed, 15 ignored plus one passing doctest.
- Debug MCP/diagnostics configuration: 821 passed, 11 ignored. Ordinary and debug Clippy passed with warnings denied; formatting and the Python MCP adapter's nine tests passed.
- Launcher isolation self-test, all required isolated tuning-recovery scenarios and both real-binary crash-report probes passed. The probes use their own native test objects/files and do not apply the user's Windows settings.
- Local eframe frame tests, Winit DPI rectangle-policy tests and epaint library tests passed. Stable tag builds now run these same three vendor gates before upload.
- Both lockfiles passed cargo-audit against RustSec commit `4b587f00996ebe5c05a73eaef105ef241e876414` without vulnerability ignores. Informational target-reachability findings are recorded in `dependency-advisories.md`; the selected release graph excludes the dev-only schema validator.

The final EXE/PDB identity, manifest, release-code fence and exact-head Windows/Linux CI are checked separately after integration. Historical CPU19 and memory measurements remain tied to their named artifacts in `mcp-validation.md` and `cpu19-performance-probe.md`; the latest review did not rerun a long resource soak.

The expanded Windows CI gate has a 45-minute budget, matching the stable tag workflow. A cold optimized build passed the application/native/debug/vendor/recovery tests and release/PDB checks but reached the former 30-minute job deadline while compiling the crash-probe executable. No checks were removed or relaxed.

## Remaining release decisions and limits

- Select the next version and create its versioned release notes before tagging. `Cargo.toml` still reports the already-published 1.6.3; the consolidated Unreleased section describes the new behavior. These checks do not authorize reusing the old release tag.
- Merge and release-tag/publication remain separate user decisions. A green old-head run is not acceptance of the newly integrated tree; inspect the final MR head's CI.
- Prior Windows 11 mixed-DPI/Explorer/tray and elevated MCP/CPU19 evidence remains host-specific. A fresh final-candidate native smoke/long soak can strengthen acceptance; Windows 10, RDP and other GPU/driver combinations are not certified here.
- The pinned renderer does not recover its graphics device in place during driver replacement. The earlier externally triggered AMD-driver-reset attempt remains excluded from performance evidence. No new renderer recovery stack or automatic relaunch was introduced during this review.

## Follow-up validation — 2026-10-22

The frozen candidate above subsequently completed a read-only CPU19 comparison against the latest retained, previously measured release. The most recent short-run baseline had been overwritten and was explicitly not substituted with an unmeasured build. Both retained releases ran for 32 continuous minutes, with six theme changes each; eight short release starts and separate diagnostic ABBA sessions supplemented the comparison. The final release SHA256 remained `0A0553D1FEB85F2290955FB89486F509ACE9FD804DADBB679620EC10A782E9C6`.

- Mean process CPU was 1.257% versus 1.314% of one logical CPU. Last-ten-minute private commit was 157.375 versus 160.617 MiB; working set was 113.676 versus 115.881 MiB. No monotonic memory growth was observed over the final run.
- Median time to the native window was 350.43 versus 347.29 ms; normal close was 326.27 versus 336.53 ms.
- GPU-completion p95 was approximately 0.47 ms for both. The final release had three rare completion tails, up to 32.572 ms. Their exact cause and connection to application changes remain unproven. Native HAGS capability and PresentMon's queue-based timing limitation were confirmed; the largest tail preceded a Windows presentation-mode transition.
- A separate 20-minute capture using the existing diagnostic binary recorded 15,154 presents without reproducing those GPU tails: GPU-active maximum 1.464 ms, completion maximum 3.172 ms. A different 22.253 ms D3D12 submission interval completed its associated GPU packet in 0.276 ms; driver waiting versus CPU scheduling was not resolved. This is diagnostic localization, not a fix or proof of the historical cause.

The user accepted the rare GPU-tail residual as nonblocking. These are host-specific observations, not a performance guarantee. Exact-release long runs used Graphs/CPU19/all-system-processes on the same eight-CPU group; automated table scrolling belonged to the separate diagnostic sessions. Original settings and binaries were preserved, temporary credentials removed, and owned test processes, traces and helpers closed.

The README follow-up adds seven actual development UI captures and a current System Tweaks user guide. Two focused Astra passes checked the documentation, feature claims and screenshot captions. Its changes are documentation and image assets only; it does not change the runtime source, package version or release status.
