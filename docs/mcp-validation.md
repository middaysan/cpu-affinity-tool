# Production MCP validation

Host acceptance completed on 2026-10-20. These results describe the artifacts and Windows host below; they are not a published-release or universal compatibility claim.

## Artifact and host

- Windows host with 20 logical CPUs; the application remains BelowNormal on CPU19.
- One production EXE, WGPU/DX12 with `MemoryHints::MemoryUsage`; no companion service.
- Platform acceptance release SHA-256: `77CDAAC2035AC0797A297688723C1F4382E2FB0E5EF44048D65B003623456D45`.
- Matching PDB GUID: `72e8c2ab-ba3f-44e1-a223-9349111b22d3`, age 1.
- Embedded manifest verified as `requireAdministrator`, `uiAccess=false`.
- Debug-only bridge session, control and test-authority strings are absent from this release. The release-profile debug-MCP rejection was also exercised.

## MCP settings UI follow-up

The first modal-layout follow-up EXE is `1B4223DB17FE17F4E477E95D0AACE82657C28269AA0E9F794DFCFE588851C494`, with PDB GUID `c9f8be66-287f-4f6d-8d61-1fc94ca9e3f2`, age 1. It only changes the MCP connection/permission presenter and its help: the shared CPU-style dimmed modal, pinned Close/Save controls, grouped sections and collapsed instructions. General App settings and the HTTP/native/collector implementation are unchanged.

Four focused headless UI tests passed: bounded layout, both themes with long client names and a virtualized app catalog, shared modal backdrop/Escape, pinned controls through scrolling, and copying persisted authority while a draft is open. The modal regressions failed against the preceding Window presenter before implementation. Clippy, formatting, release build, EXE/PDB matching and manifest checks passed. Astra provided the layout recommendation and a focused source spot-check, without a native GUI run. The performance/native measurements below describe the preceding acceptance artifacts; they were not repeated for this UI-only follow-up.

The preceding compact-client follow-up EXE is `26DA44F998AB77B584D2D3829263680ADE866D999757699612165F8565279A81`, with PDB GUID `95a74794-de1e-45e3-84ed-05a13bf9ad9f`, age 1. It distinguishes the shared server from the selected client, uses five main functional scopes, and opens app selection/additional permissions only through explicit actions. Existing independent authority fields and saved app approvals remain intact; no permission checkbox expands the main form. Nine focused UI/state tests passed, including selected-client isolation, stable launch-scope layout, separate-page navigation/virtualization and permission independence. Two new regressions first failed against the earlier presenter. Clippy, debug-feature compilation, formatting, release build, EXE/PDB and manifest checks passed. No elevated/native/performance rerun was made for this presentation change.

## Single-key follow-up (2026-10-21)

The first single-key release is `29E454E6496E53625B12D77772461D38B9560382A1714FE206DA3E4305694C40` (25,998,848 bytes), PDB GUID `334a30ae-3508-4260-b7de-fb5c47403465`, age 1. The single EXE retains its verified administrator manifest. Debug session/test-authority/control strings are absent from the release. It replaces the earlier client/scopes model with one permanent full-access key, schema-2 protected settings and a compact enable/port/copy modal. API version 2 exposes all 59 tools and no permission flags. Legacy tokens retire with a fresh key and forced Off.

TDD regressions for schema, full HTTP discovery and the three-control modal failed on the preceding implementation. The current Windows suite passed 743 tests (9 explicit ignores), and the full debug suite passed 772 (10 ignores). Focused tests cover stable keys across Off/On, port/restart, legacy migration, refusal of invalid/foreign/future authority, full catalog and durable mutation, queued-write revocation, both themes, shared scrim/Escape and copying only persisted connection data. Clippy with `-D warnings`, formatting and EXE/PDB/manifest checks passed.

Final elevated acceptance passed on 2026-10-21 with exit 0 and no test errors. The actual shared modal copied the protected permanent key; Off/On and a port/restart round trip retained it. Full-catalog HTTP configuration writes returned durable receipts and retries did not repeat mutations. Ordinary launch retained ordinary rights and did not inherit CPU19/BelowNormal; explicit administrator launch and exact inspection also passed. Managed output remained available after Off/On. The official Python MCP client discovered all 59 tools. The actual release reopened the same protected connection with the same key. Native schema-1 migration generated a fresh key, preserved the port, forced Off and rejected the old token with HTTP 401 after enabling. Test authority was removed, all test app windows exited normally and the original user-state hash was unchanged. Evidence: `target/mcp-single-key/acceptance-result.json` and `runner-result.json`. The earlier failed attempt was a corrected Python test-driver parameter conflict; it is excluded.

The release's short 10-second check, with CPU collection/enforcement disabled and one configuration query per second, measured 0.619% of one logical CPU, 148.58 MiB private bytes and 94.14 MiB working set. CPU19 affinity and BelowNormal were verified; spawn-to-window was about 570 ms and orderly close about 341 ms. This is a short functional smoke measurement, not a sustained performance comparison or a repetition of the earlier monitored soak. The Desktop artifact matches the tested release SHA-256 above.

The platform measurements below remain tied to the original `77CDAAC2` release and `6140C0F6` debug artifacts. They are not new performance measurements or proof of the retired permission model on the one-key build.

## Footer status and last-100 command journal (2026-10-21)

Preceding footer-journal release: `42A38E0B06F176F7CEA54D20C4C7E9090A497D8BCFE7FE214A79C8EC566A04A9`; matching PDB GUID `84275094-874b-4fb3-8e26-5e723a7c2d69`, age 1. Manifest and debug-control/session/test-directory exclusion checks passed. Release/debug builds, formatting and Clippy with `-D warnings` passed.

The Windows suite passed 750 tests (9 explicitly ignored). A new footer regression first failed because the MCP label was missing. Added coverage proves its placement, both-theme green/red/expiry/click behavior, bounded shared modal, Escape/cache release, exact 100-call retention and eviction safety, terminal operation/retry updates, uncertain cancellation, human labels for all 59 tools and exclusion of arbitrary inputs/unknown supplied names. HTTP tests cover successful reads, validation errors and actual durable mutation results in the GUI journal.

Elevated full-app acceptance passed with exit 0: fresh protected key/copy, port/restart retention, full catalog, ordinary/elevated launch, output, Off/On, official SDK discovery and native legacy migration remained valid. The actual footer opened the shared command modal; after 112 additional read calls its snapshot contained exactly 100 entries (sequences 33..132), newest first with Done results. The native screenshot was inspected. The actual release reopened and exited normally. Test authority was removed and the original user settings hash stayed unchanged. Raw evidence: `target/mcp-activity/acceptance-result.json`, `runner-result.json` and `activity-window.png`.

A 10-second functional release probe with CPU collection/enforcement Off and one configuration query per second measured 1.397% of one logical CPU, 160.68 MiB private bytes and 110.17 MiB working set on CPU19/BelowNormal. Spawn-to-window was about 356 ms and close about 327 ms. This short single-run smoke check includes visible request pulses; it is not a paired performance comparison or another sustained monitored soak. The journal has a fixed 100-entry RAM bound, no raw argument/output retention, no disk writer and no idle repaint timer.

## Unified footer controls (2026-10-21)

Preceding unified-controls release: `6052ED7C0BAF634A6FE9360FCA928F98633A536E91561503FAEA43EAAF0F3C32`, 26,007,040 bytes; matching PDB GUID `acacdff1-6953-4a66-9283-6c795a8ccc12`, age 1. EXE/PDB, embedded manifest, debug-marker exclusion, formatting and Clippy with `-D warnings` passed. Matched delivery files are retained in `target/footer-controls/delivery`.

MCP and Process monitor now share the muted-gray Run-shaped status button renderer. Both open while Off without changing service state. MCP has one Enable / Connection / Logs modal; Process monitor has Enable / Processes / Logs. Process logs select up to 100 recent typed entries from the existing bounded application log, including durable enable/pause transitions; no second collector or log ring was added.

The final Windows suite passed 754 tests (9 explicitly ignored). The new footer regression failed before implementation. A subsequent native screenshot exposed a short viewport when switching from Connection to Logs; a dedicated headless regression reproduced exactly five fully visible rows before the fix. Logs now gets an explicit 80%-height modal and a single bounded list scroll; the regression requires at least twelve visible rows in the 400×700 fixture. Connection retains its compact natural height.

Elevated full-app acceptance passed with exit 0 on the preceding functional candidate `296D78B493376FF4C59FA0B1141C0CA0A6CD19855CE660A858412F5783EEA111` (debug `007E9B23CFC815845E37AFB0D0C88136B23C390196A8F920CC5FAB40CA98BF99`). It exercised both footer controls, MCP Connection/Logs, Process monitor enable/pause and typed logs, and independent CPU recording. Existing key persistence, port/restart, ordinary/elevated launch, output, Off/On, all 59 official-SDK tools and native legacy-key retirement also passed. The journal contained exactly 100 rows after extra reads. Original user settings were unchanged, temporary authority was removed and test windows exited normally. Evidence is in `target/footer-controls/acceptance-result.json` and `runner-result.json`.

Only the Logs viewport fix followed that native run; its exact failing geometry is covered by the final headless regression, without a second elevated run. The native screenshots and short CPU/memory measurements belong to the preceding candidate, not the final Desktop hash. No new sustained performance comparison or soak is claimed for this UI follow-up.

## Service-modal hierarchy follow-up (2026-10-21)

Preceding service-layout release: `F2F01DE843F71F565BFCFF12EDFBC4DD600F04BE7E7E945BFAA76B37E28670BF`, 26,022,912 bytes; matching PDB GUID `025b64ec-8b3b-4f3f-913b-fd6f32843075`, age 1. The matched pair is archived in `target/service-modal-layout/delivery`. Manifest, debug-marker exclusion, formatting, Clippy with `-D warnings`, release build and debug-feature build passed.

The two service dialogs share title/purpose, Enable/actual-status band, full-width segmented tabs and bounded bodies. MCP Connection has one Port/Copy row and concise visible setup/trust help. Process counts remain on Processes. Empty lists have centered explanations and no unused column headings; populated tables retain their virtualized compact rows and existing bounded data. The empty tracked-process message is outside zero-row virtualization. Wrapped MCP errors precede the scroll body; actual endpoint port remains available on the control hover. These are presentation changes, with existing save/copy and service behavior retained.

The Windows suite passed 757 tests (9 explicitly ignored). Two new hierarchy/empty-state regressions failed before implementation; a third reproduced the error-order problem found by the requested Astra source review before correction. Coverage includes both themes, small windows, compact empty dialogs, balanced tab layout, populated journal visibility, error placement and the existing credential/copy/Escape/retention behavior.

A short actual-window debug-MCP smoke opened Connection, empty MCP Logs, empty Processes and empty monitor Logs, captured all four frames and closed normally (exit 0). Astra inspected those screenshots and reported no material layout issues. This ordinary-rights debug run intentionally made no service mutations; its MCP protected-directory privilege error is expected and verified error wrapping. It does not replace elevated production service acceptance or measure performance. The prior native service/HTTP and CPU19 results above remain tied to their recorded artifacts. Raw evidence: `target/service-modal-layout/visual-result.json` and its four PNG captures. No further source change followed the tested final build.

## AI setup prompt and bounded hints (2026-10-21)

Preceding AI-prompt release: `085C8E0CED81B9A05E0D2C3279727344457500174DEAF2DD05489E16BE37CD7B`, 26,075,648 bytes; matching PDB GUID `4229f409-4fd2-4830-ad55-bc0c60f2261d`, age 1. Matched EXE/PDB files are archived in `target/mcp-ai-prompt/delivery`. Manifest, debug-marker exclusion, formatting, Clippy with `-D warnings` and release/debug builds passed.

Connection now has paired Copy connection / Copy AI prompt actions. The original JSON format remains intact. The new prompt includes saved endpoint/Authorization data, client configuration discovery, normal Streamable HTTP initialization/tool discovery, authenticated read-only verification and honest reload/restart limitations. It also explains operation receipts/retries, exact identity, telemetry quality and untrusted output. It does not authorize unrelated app or Windows mutations during setup. A typed deferred format waits for existing persistence/listener completion before generating either clipboard payload. No credentials are shown in labels or logs.

The Windows suite passed 760 tests (9 explicitly ignored). New tests first reproduced the missing prompt action and clipped long tooltip. Both themes now pass the small-viewport wrapped-tooltip and paired-action tests. Prompt clipboard JSON parses to the exact saved URL/key; the key occurs once and never appears in visible labels. Deferred tests cover both formats, no output while busy, updated saved key/port, one-time completion, save failure and closing. Both buttons reject pending keys and unsaved ports. Existing full HTTP and UI regressions remain green.

A short ordinary-rights debug-MCP visual check found the new control and captured its actual layout, then closed normally (exit 0), with no service mutations. Its expected protected-store permission error is the same isolated non-elevated limitation described above. No particular agent client's configuration was edited or certified by these tests; clients may require reload or may not permit self-registration. The prompt explicitly handles those limits rather than promising universal automatic setup. Earlier elevated native/performance results remain tied to their original artifacts. Evidence: `target/mcp-ai-prompt/`.

## Wire contracts and guarded rule replacement (2026-10-21)

Current Desktop release: `F00B87FFAF9F4485DA0ACC508267701D74543542D43E290534DA9148EA7D6290`, 26,268,160 bytes; matching PDB GUID `ea2a198f-0cd8-408a-a43d-85fa3e4a8a40`, age 1. Matched files are retained in `target/mcp-schema-contracts/delivery`. Manifest, EXE/PDB identity, formatting, Clippy with `-D warnings` and release debug/test-probe exclusion passed. `cargo tree --edges normal` confirms the JSON Schema validator is not a release dependency.

All 59 tools publish successful output schemas and descriptions for their defined input fields. Operation result variants, current CPU tuples, original-time pending/completed/lost history and nullable measurements retain their actual wire shapes. Module-owned configuration payloads remain explicitly opaque and use the registered module template/help. Repeated System Tweaks result descriptions reference the fully documented canonical read schema while retaining all validation types. The serialized tool catalog is 244,024 bytes, below the existing 256 KiB budget; it is still built once and reused.

The Windows suite passed 765 tests (10 explicitly ignored). Initial regressions failed on missing output schemas and acceptance of incomplete rule replacements. A pinned Windows-only development validator checks all input/output schemas and representative actual configuration, CPU/history, log, profile and receipt responses. The HTTP regression rejects omission of each editable rule setting before consuming a request sequence, then adds a process name while preserving path, arguments, High priority, startup, descendant handling and RuleId. A subsequent complete request intentionally clears lists/booleans successfully. The service boundary independently refuses incomplete replacements. Shared server/clipboard guidance documents the exact read-to-write mapping and full-list semantics.

The official Python MCP SDK 2.3.0 discovered all 59 tools with output schemas, validated capability/receipt results and correctly handled the partial-rule error. Its bounded test-only localhost server had no native Services attached and exited normally; this is protocol/schema compatibility proof, not another elevated application/System Tweaks or performance run. The first Python attempt stopped at an import before starting a server; processing the installed PyWin32 site paths resolved the test-driver issue. Earlier native/CPU19 results remain artifact-specific. Evidence: `target/mcp-schema-contracts/tests.log`, `catalog-size.log`, `sdk-result.json` and release checks. No source change followed the final tests and build.

## Completed checks

- Windows application: 735 passed, 9 explicitly ignored; Clippy with `-D warnings` passed.
- OS boundary: 176 passed, 15 explicitly ignored, plus the compile-fail doctest; five separately elevated native acceptance tests passed.
- Existing debug adapter: nine Python protocol tests passed. The previous complete diagnostic-feature suite passed 762 tests before the two final API-only regressions were added; those regressions passed in the final ordinary Windows suite.
- Official Python MCP SDK 2.3.0 auto-negotiated and discovered 59 tools.
- The final debug build passed HTTP configuration, durable receipts/retries, ordinary and explicitly permitted administrator launches, scheduling readback, saved-launch deduplication, managed restart/stop, output/exit, original-time run CPU/contributors, installed-app activation, permissions/revocation and MCP Off/On checks.
- Ordinary launch did not inherit CPU19/BelowNormal. Administrator launch was refused without its grant, then verified with the grant and ordinary Normal priority on CPU18. The grant was subsequently removed.
- Profile IDs survive values above JavaScript's exact integer range. HTTP profile creation/read/deletion uses decimal string IDs. An agent Realtime rule request is rejected without changing the saved revision; existing manual data remains supported.
- Protected grant tests proved administrator round trips and ordinary-process read/write/rename refusal, including directory replacement attempts. Exact process tests rejected a wrong creation token and verified scheduling changes/readback.
- Native System Tweaks round trips on the preceding artifact covered seven Apply/Undo operations, restart, manual profile activation while its page was hidden, display confirmation/restart/expiry, tray and hidden restore. Subsequent production edits only changed the MCP wire ID representation, agent Realtime admission and permission explanations; the native controller/adapters are unchanged. Test originals and the user's state were restored and verified.

## Production release comparison

The final artifact passed twelve runs in alternating and reversed order, four runs per mode. Each visible/tray resource window lasted twelve seconds. The comparison used the same isolated copy of saved groups and the same updated AMD driver; enforcement and autorun were disabled.

| Median | Pre-MCP build | Final, MCP Off | Final, MCP On, current query at 1 Hz |
| --- | ---: | ---: | ---: |
| CPU, percent of one logical CPU | 1.302 | 1.041 | 1.237 |
| Private committed memory, MiB | 158.40 | 161.05 | 157.79 |
| Working set, MiB | 103.55 | 105.29 | 107.35 |
| Spawn to discoverable HWND, ms | 315 | 334 | 356 |
| Close to process exit, ms | 317 | 370 | 275 |

These small, variable differences do not establish an acceleration or a negative memory cost. They show no material regression in this host comparison. All minimize/tray/close checks passed, the user's state hash was unchanged, and no owned CPU trace remained between the closed comparison and the next stage.

MCP enabled without active queries used a median 1.042% of one CPU. Tray medians were 0.586% for the pre-MCP build, 0.586% with MCP Off and 0.455% with 1 Hz queries; differences this small should be interpreted with counter quantization and host variation in mind. The pre-MCP reference is the archived build from the same work branch, not a separately published GitHub Release.

## Diagnostic ABBA and 32-minute soak

The final diagnostic build (`6140C0F6E59A3EA93723477EFEBA3EC0285AD16861FB7CBEE57F4880E5B3739E`) passed three Off/On/On/Off cycles, six windows per mode and workload. The API listener, GUI, collector and output ownership all remained in the same application process. The driver and the controlled busy child used CPU18; the application used CPU19.

| Diagnostic median | MCP Off | MCP On |
| --- | ---: | ---: |
| Group view idle, percent of one CPU | 1.301 | 1.216 |
| Process-table scrolling, percent of one CPU | 7.902 | 8.594 |
| Scrolling GUI p95, ms | 0.654 | 0.644 |
| Scrolling private memory, MiB | 170.51 | 170.84 |

Scrolling issued 810 input actions per mode at 15 Hz. The enabled 1 Hz current-query workload used a median 1.301% of one CPU; small counter differences do not establish a speedup. Instrumented values must not be substituted for the production table above.

The subsequent 1920-second soak passed with 384 external resource samples and no test errors:

- Mean process CPU was 1.429% of one logical CPU, approximately 0.071% of this 20-thread machine.
- Private memory began at 173.82 MiB and ended at 168.48 MiB. It ranged from 167.30 to 182.52 MiB; the final ten minutes ranged from 167.46 to 173.58 MiB. Working set moved from 123.05 to 117.25 MiB. Handles began at 461 and ended at 452, with a range of 449–461. There was no monotonic growth in this run; this is not a proof that every workload is leak-free.
- The change feed delivered 1917 observed pending-to-resolved updates at their original times. Ten pinned application graph/contributor checks passed, alongside repeated group/system scope and table checks. The final retained quality snapshot had 298 complete and three pending rows, with no lost collector interval. The full diagnostic log contained two deliberately unavailable `baseline` intervals at initial collection and restart, and no other gap reason or telemetry error. Pending and baseline intervals were not presented as zero load.
- Three MCP Off/On cycles preserved independent CPU collection. Capturing 500,000 bytes per stdout/stderr stream retained the bounded suffix, reported the byte gap and reached EOF.
- Monitoring Off released history to zero bytes; On produced a new sample. The controlled child and temporary rule/group were removed. The application exited with code zero, and the user's source-state hash was unchanged.

| HTTP read, end-to-end on localhost | Calls | p95, ms | Maximum, ms |
| --- | ---: | ---: | ---: |
| Current CPU | 1974 | 1.024 | 4.503 |
| History | 33 | 4.205 | 4.237 |
| Changes | 997 | 1.450 | 7.364 |
| Contributors | 32 | 1.208 | 1.979 |

Rendered GUI time below sums `gui_logic` and `gui_ui` across each frame's rendered passes. These figures cover the whole diagnostic session, including initial setup and the soak:

| View | Frames | p50, ms | p95, ms | p99, ms | Maximum, ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Group view | 1733 | 1.254 | 1.796 | 2.661 | 10.811 |
| CPU graphs | 4983 | 0.529 | 1.631 | 1.981 | 2.595 |
| Process table | 11452 | 0.474 | 0.752 | 0.958 | 1.763 |

No measured rendered GUI frame exceeded 16 ms. GPU execution, presentation waits and input-to-display latency are outside this counter. The expected cold first frame accounts for the group-view maximum; the soak-only group maximum was 2.831 ms.

The private test authority and debug session were removed after the run. The elevated stage helper stopped, no test application or controlled child remained, and an ETW session inventory contained no owned CPU-load trace. Raw logs and matched build artifacts are retained in the ignored local `target/mcp-platform-http/` workspace; no credential is included in this document.

Measurements use external process CPU/private/working-set/handle counters, with the test driver on CPU18 and no concurrent compilation. CPU percentages refer to one logical CPU; divide by 20 for whole-machine equivalents on this host. The production artifact has no diagnostic frame clocks. Instrumented GUI/frame results are reported separately and do not measure GPU execution or input-to-display latency.

Native window creation time is the time to discover its HWND, not the first presented frame. Host load, renderer allocations and working-set residency vary between runs. These checks do not certify FPS effects, Windows 10, other hardware, cross-account execution, or every mixed-DPI configuration.

## Driver-reset limitation

An earlier diagnostic attempt was excluded after a renderer panic during an external AMD driver installation. Windows recorded the new `amdkmdag.sys` service at 16:23:43 UTC; the renderer failed to obtain a staging buffer at 16:23:44 UTC despite sufficient destination-buffer capacity. This strongly points to graphics-device loss during the driver change; the test did not capture a device-lost callback, so the precise native failure is not independently established. Subsequent comparisons use Radeon RX 9070 XT driver `32.0.32015.7029`.

The current egui-wgpu backend can panic after device loss and does not recover the renderer in place. This is an existing [upstream issue](https://github.com/emilk/egui/issues/8450); [the proposed recovery change](https://github.com/emilk/egui/pull/8452) remains under review. No unreviewed recovery stack, buffer fallback or process relaunch was added for this MCP change. Surviving driver replacement without an application restart remains outside this acceptance claim.


## CPU monitoring footer follow-up (2026-10-21)

The global All CPUs summary moved from the Affinity groups toolbar into a third footer service button. The132x36 button is green while CPU recording is enabled and paints the existing shared120x15 summary under its label. Off preserves the target size but paints no load. All three footer buttons share a vertical center. One response owns the CPU button, including its passive meter; it opens compact controls rather than toggling recording as a side effect.

CPU controls reuse the existing preference writer and collector, with Enable, history duration, memory estimate, bounded wrapped errors and an explicit Open CPU graphs action. That action is unavailable while Off; while On it closes controls and opens the System target with all CPUs/all processes. App settings has no duplicate CPU controls. Existing history/missing-versus-zero semantics, Process monitor independence and modal paint/import suppression remain.

Validation: the absent footer and later visual centering issue were each reproduced by failing regressions before their fixes. Windows application suite769passed/10ignored; debug-MCP suite798passed/11ignored. Clippy-Dwarnings and formatting passed. Tests cover enable/disable persistence, read-only opening, process-monitor independence, System scope, one hit target, zero/unknown mean, toolbar scrolling, theme/clip/centering, bounded long errors, retention choices and native modal Escape/outside-click behavior. The debug input test follows the new footer entry and reads the settled frame.

An isolated ordinary-rights debug-MCP smoke captured eight settled dark/light On/Off, controls and System-graph views, toggled CPU recording back Off and closed with exit0. PDH supplied the global summary; detailed ETW reported the expected lack of administrator access. This is UI-path validation, not a new elevated collector, CPU19 performance or memory-soak result. The two user-requested Astra agents reviewed source and settled screenshots; no actionable findings remained. Existing native/performance evidence retains its original artifact qualifications.

Delivered test EXE SHA256 `08606B656B25CDB10DA9E564D6BB9E86EFC67614FD29EF1CEC013BCC26B7E055` (26,266,624 bytes), matched PDB GUID `022290a9-dad7-40b7-b6ec-4bcbc60436e6` age1. Release build, embedded manifest, debug/test marker exclusion and Desktop/archive hash checks passed. Final application source was unchanged after checks/build.


### Compact CPU footer refinement

The later user revision replaces the two-line button with a132x21 control matching its neighbors. CPU monitoring is painted inside the load bar, percentage at right; All CPUs and the second row are removed. Off shows only the neutral title. The shared painter and one click/focus target remain. Height regression failed36vs21 before the change; Windows769passed/10ignored, Clippy/fmt/diff/build/PDB/manifest/debug-exclusion checks passed. Delivered EXE SHA256 `9A9603EEE00F3FACBB965F9BBDCE41B11A262F09B4168585057D343B7281B6DD` (26,265,600 bytes), PDB GUID `80d0d247-e838-4945-a897-54cbc431a1f5` age1. This small layout revision had no new native MCP, elevated/performance test or Astra review.


### Symmetric footer and sole MCP entry

The user-requested Astra follow-up uses three equal21-pixel columns spanning the footer with6-pixel gaps. Borders blend the neutral strong edge with status at35percent; the island has a subtle contour. CPU paints only fill/text over the common native body/frame/focus and reserves measured100% width. The duplicate App settings MCP entry and dead presenter method are removed.

Geometry and menu regressions failed before the fix; final Windows769passed/10ignored and debug798passed/11ignored, Clippy/fmt/diff/build/PDB/manifest/debug-marker exclusion passed. An isolated ordinary-rights debug-MCP run captured ten settled views, verified both themes, On/Off, System graph transition and the simplified App settings, and exited0. Detailed ETW reported expected non-elevated denial; this is UI validation without a new elevated/performance claim. One Astra reviewed source and five actual PNGs with no remaining findings. Delivered EXE SHA256 `AC9708A83B2B4497520D5EB72C244E98DF5A19E14EDEC1DA6563064446EA29F4` (26,266,624 bytes), PDB GUID `dd16dea5-805e-4954-8a4f-16dcf5a5f674` age1; source unchanged after checks/build.


### Header service controls

The later user revision moves MCP / Processes / CPU between App settings and the version. The header centers a preferred240px cluster of equal76x21 controls with6px gaps inside measured remaining width; full names remain in accessibility/help/dialogs. The bottom panel, island and reserved height are removed. The service_controls presenter separates button placement from dialog dispatch; the legacy Footer profiler bucket now covers dialogs only. Monitoring and MCP behavior, IDs, suppression and CPU data sources are unchanged.

Location regression failed before implementation. Windows771passed/10ignored and final debug800passed/11ignored; tests cover dark/light at100/175percent scale, CPU100% clip/gap, no bottom reserve and existing service/dialog behavior. Clippy/fmt/diff/build/PDB/manifest/debug-fence checks passed. An isolated ordinary-rights debug-MCP run captured12settled views, opened all three dialogs, checked CPU On/Off and System graphs, and closed0. Detailed ETW was unavailable without administrator rights; no elevated/performance result is implied. One requested Astra reviewed source and five actual PNGs without remaining findings. The later source delta from those visuals only updates a stale tooltip reference; final tests cover that source.

Delivered EXE SHA256 `ABD10839BA61C1AFA73CBE42F4BF3689D4D39AF91EF04D4EFF7F7D8ADE370D35` (26,273,280 bytes), matched PDB GUID `470df285-20a0-438d-9cbb-9dd87996f405` age1. Desktop filename is `CPU Affinity Tool - Header.exe`. Source unchanged after final checks/build.


### Compact right-aligned header

The three service controls are now half their preceding width: 38×21 logical pixels each, with six-pixel gaps in a right-aligned 126-pixel cluster. Height remains readable. Labels are MCP / Proc / CPU while Off; enabled CPU recording shows a centered percentage over the existing proportional fill. Full service names remain in accessibility metadata, help and modal titles. Version is the final read-only App settings row, retaining the CPU model/thread hover. No footer, collector, history, writer, timer or authority change.

The updated header/version regression failed before implementation. Windows tests: 771 passed/10 ignored; debug-MCP tests: 800 passed/11 ignored. Clippy with denied warnings, formatting, diff check, release build, PDB identity, manifest and debug-marker exclusion passed. Tests cover both themes, 100/175 percent scale, 100% text clipping, dialog actions and no bottom reserve. An isolated ordinary-rights debug-MCP smoke captured 12 settled views, exercised the three dialogs, CPU On/Off and System graph transition, and exited 0. Dark/light header and version screenshots were inspected. Detailed ETW remains unavailable in this ordinary-rights fixture; no new elevated CPU19, performance or memory result is claimed. No additional agent review was requested for this iteration.

Desktop `CPU Affinity Tool - Compact header.exe` matches the archive in `target/compact-header-controls/delivery`: SHA256 `CF061C1B8100E78D8D0CAA52AB05778E322CBC3E96AF43F7E5A5FE427152CBA7`, 26,279,936 bytes. Matching PDB GUID `f9e67c51-cf10-4ac1-a359-9cd0ee39dfcc`, age 1. Previous Header.exe remains available. Application source is unchanged after final tests/build. No Git publication or final MR review.


### Header labels, tooltips and theme selector

User refinement keeps service controls right-aligned and21 pixels high, with smaller9px Inter Medium labels and preferred MCP50 / Process monitor86 / CPU load82 widths separated by6px gaps. CPU load retains its title while enabled, with a right-aligned percentage and the existing proportional fill. Off shows only its neutral title. Header and Linux Process monitor sizing use the same preferred-width owner. Existing native actions, status colors, pulse, sampler and histories are unchanged.

Tooltips explicitly override the inherited no-wrap row style and wrap within the viewport, capped at260 logical pixels. This includes header service help, CPU error text and monitor table/log help. App settings no longer repeats its title and begins with one Theme / Light / Dark / System selector; stored mappings1/2/0 and the existing setter remain unchanged. Version stays last with its CPU-details hover.

TDD: updated header/settings regressions and a new bounded-tooltip regression failed before production changes. Final Windows772passed/10ignored, debug801passed/11ignored, Clippy with denied warnings, formatting, diff check, release/PDB/manifest and debug-marker exclusion passed. Tests cover both themes and100/175 percent scale, full names/100% separation and wrapped long CPU errors. The single-row test allows the native framed text's1.5px baseline difference. Isolated ordinary-rights debug-MCP smoke captured17settled views, checked all three persisted theme choices, CPU On/Off and System graphs, and closed0. Expected detailed-ETW access denial remains qualified; this is UI validation with no new elevated CPU19/performance/memory claim. One user-requested Astra recommended the layout and reviewed final source plus six actual PNGs with no actionable findings.

Desktop `CPU Affinity Tool - Header UI.exe` matches `target/header-labels/delivery`: SHA256 `F18A85A092732B5C2FBF8EA6A8BF0713AFD9F5DA1737233C033B05EB38EABAD1`, 26,260,480bytes; matching PDB `5de7a205-7fd2-477b-b772-867da443db82` age1. Prior Compact header.exe remains. Final application source is unchanged after tests/build. No pending test app/helper/UAC, Git publication or final whole-MR review.

## CPU history export and process-name suggestions delivered — 2026-10-21

PNG and CSV are explicit right-click actions on CPU tiles/detail graphs. CSV schema 1 retains original UTC/elapsed time, CPU/load, process/PID/historical lifetime, source/rule/run identity, native interval metadata and honest gaps/quality. It exports the configured RAM retention, not an unlimited archive. Existing history is copied in pages of at most 128 rows per lock; serialization and disk I/O stay outside the sampler/GUI. One shared preparation gate, one current MCP artifact, 512 MiB cap, exclusive delete-on-close file, 32 KiB UTF-8-safe downloads and lazy 600-second expiry bound resources. HTTP export_cpu_history uses typed input/output contracts and never accepts a remote output path. Range is fixed first; intervals are consistent as copied, not an atomic all-rows-at-request snapshot. PNG uses an explicit screenshot and finite crop/encode/Save As worker with scale/size checks. No new collector or idle polling/timer.

Tracked-name fields now show at most eight case-insensitive running-process matches, prefixes first. A request-driven background name snapshot is cached, with refresh coalesced to a five-second freshness bound. Filtering reuses results for unchanged text/snapshot. Selection fills only the unsaved field; Escape/outside click and arbitrary free text remain supported.

TDD RED evidence covers missing menu, missing MCP tool, matching and the actual foreground-modal popup-layer bug. The latter was found during native dark/light smoke and fixed through egui's owned sublayer relation. Final Windows 782 passed/10 ignored; debug MCP 811 passed/11 ignored. Clippy with denied warnings, formatting, diff check, release build, PDB identity, manifest and debug-marker exclusion passed. Real HTTP MCP regression downloaded a process-history fixture in 37-byte UTF-8 chunks, validated output schema and original Cyrillic process/PID/lifetime/CPU/load. CSV tests cover PID reuse, missing data, scopes, spreadsheet escaping, page revision invalidation and byte cursors; PNG test verifies physical crop/encode round trip. Python debug adapter tests: 9 passed.

Isolated ordinary-rights native debug-MCP smoke exercised real name suggestions/selection, both themes, reopened graph context menus, CPU On/Off and orderly exit 0. Computer-use then saved actual native PNG (24,054 bytes, visible CPU tile inspected) and CSV (329,497 bytes, 2,320 rows across 20 CPUs) through standard Save As; CSV parsed successfully. Early input attempts targeted a helper's 13x13 screenshot; selecting the actual dialog screenshot resolved test automation without production changes. This non-elevated fixture honestly exports unavailable ETW rows; positive process evidence is covered by the real HTTP fixture, not a new elevated CPU19 performance claim. No remaining test app/helper/UAC. No additional agent review or Git publication was requested.

Desktop CPU Affinity Tool - History export.exe and target/cpu-history-export/delivery match SHA256 A22AF447EA2B709E91E21DCA58B121524FAE4CBFBC39B4937BE3106A64B7A409, 26,617,856 bytes; PDB GUID 2dba5622-d702-4db7-810d-163636cfa7e2 age 1. Embedded manifest requireAdministrator/uiAccess=false verified; debug controls/authority strings absent. Prior Header UI.exe is preserved. Application source is unchanged after final checks/build. S53 remains in progress for user feedback; S54 stays done. Final whole-MR Astra review and publication remain deferred.

## Tracked-name CPU recovery and background discovery — 2026-10-25

A transient tracked-membership lock/generation race formerly replaced the persistent CPU target cache with managed-run targets. A later unchanged-membership tick did not rebuild it, so application and tracked-group measurements could stay unavailable while host sampling continued. Target selection now uses a separate reused scratch vector for the uncertain cut. Regression coverage reproduces the original loss, preserves its unavailable interval and verifies recovered application totals, tracked-group means, rule history, exact contributors, current process rows and HTTP read serialization.

An isolated elevated debug-MCP/production-HTTP scenario passed in 25.36 seconds on the Windows 11 host. The diagnostic EXE SHA256 was `83B0C5A89D7D9F15D9E9B07539F5A633D6588CDA957A88799618E09D4ABA1F9A`. It launched one owned test EXE with verified ordinary rights through MCP, detected that source with Process monitor Off, then stopped that managed run. Two separate bounded owned workloads exercised classic tracking: the primary was present before a complete `update_rule` added the second executable name. Six subsequent samples contained both exact processes with positive current CPU and complete scheduler mapping. Application Processes and rule/tracked/system history-contributor reads included the added PID. CPU Off/On resumed measurements without restarting the workloads; dark/light captures showed the application meter at 6% and group capacity meter at 28% in that fixture. Whole-host and per-group capacity percentages have different denominators.

No real background fields changed: the compiled role qualification gate remained empty and the observed changed-target count stayed zero. The application and children closed, test tickets/key files were removed, and only isolated settings were used. This proves the tested discovery/accounting paths, not game/FPS benefit, other hosts or a long performance soak. A subsequent editor-only refinement caches the background dirty-state baseline by immutable configuration identity; its saved-document/rule invalidation and shortcut guard have automated coverage. It does not alter the accepted CPU/discovery sources.

Windows CI and stable release verification now invoke the safe default background protocol/admission runner and strict os_api test-code Clippy. The runner's native owned-process switch is not passed by CI; actual Windows roles still require controlled VM/function/recovery qualification.
### 2026-10-26 — current app-row RAM

Current private resident RAM is shown next to each application CPU meter while CPU monitoring is on. The existing one-second worker reads the exact retained process handles, including additional tracked names and enabled descendants. Exact instances are counted once per rule; shared pages are excluded. Partial sums use an asterisk, unavailable values use a dash, and Off hides the column and stops its reads. No memory history, extra sampler or production MCP response field was added. The native EX2 counter requires updated Windows 10/11; unsupported fields are not replaced with commit estimates.

Local validation passed 876 application tests, 208 default-feature os_api tests plus its documentation test, and 12 headless debug-MCP tests; strict shipping and os_api test-code Clippy, formatting and diff checks passed. The new regressions cover extra identities, duplicates/shared scopes, missing counters, PID reuse, overflow, stale revisions, Off and compact light/dark presentation.

A finite ordinary-rights debug-MCP scenario used only owned 64/96 MiB workloads and isolated settings. Primary-only RAM showed 64 MB; adding the second tracked name showed 160 MB, exactly matching the same native private-working-set sum at comparison. Duplicate names did not duplicate instances. Child exit returned to 64 MB; a new child instance and monitoring Off/On restored the expected sum. Both themes were captured and inspected; all test processes and the session ticket were closed/removed. Scheduler ETW was unavailable at these ordinary rights, so this validates RAM independently of the detailed trace, not elevated CPU attribution.

A separate 500-query direct native check including lifetime reads averaged 2.69 microseconds per process on this host. This is short-run request latency including the Python call boundary, not a full UI/collector CPU measurement, performance soak or other-host guarantee.

Production EXE SHA256: `C7A2AD5B49430489FEEEAA62FEB72C08430D79F3AF32E38BC9C756126A4A6E5A`, 27,060,736 bytes. Matching PDB: `962f2151-6c2d-464f-8174-66fa69faeea2`, age 1. PDB identity, administrator manifest and absence of debug-MCP/diagnostic RAM-control markers passed. The same EXE/PDB pair was delivered locally. Real-role admission remains empty.
