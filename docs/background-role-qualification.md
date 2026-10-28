# Background workload qualification protocol

This protocol separates local mechanism evidence from admission of an actual Windows role. The compiled real-role mutation qualification set remains empty. No VM is available for this delivery, and no real-role functional, service, crash or game-benefit acceptance is claimed. This document does not enable a production override or authorize running its VM steps on a working desktop.

## Local test-owned functional probe

The existing `native_owned_background_round_trip` checks native Priority, EcoQoS and legacy CPU Set mechanisms on a sleeping test-owned child. Its hybrid-topology requirement and setter/readback checks remain useful, but do not show functional work under a changed policy.

The separate `native_owned_background_function_progress` runs on an exact child created from the test executable. It needs no hybrid CPU topology and never selects a role, service or CPU Set. It is compiled only under `cfg(test)` plus `tuning-test-controls`; ordinary application and library builds contain neither child nor harness.

Run the pure protocol checks without native scheduling changes:

```powershell
cargo test --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls windows::background::native_tests::function_progress::workload::tests
```

The repository wrapper `scripts/test-windows-background-workload.ps1` runs its deterministic checks by default. Its optional `-OwnedProcessProbe` switch explicitly adds the ignored functional probe; it does not install a service or select a system-role target.

The following explicitly ignored test changes only its own child. Run it serially after compilation and other measurements have finished:

```powershell
cargo test --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls windows::background::native_tests::function_progress::native_owned_background_function_progress -- --exact --ignored --nocapture --test-threads=1
```

The child computes individual squared integers in ordered batches. The parent checks its cumulative result with the independent closed form `n(n+1)(2n+1)/6`, reduced to the same unsigned 64-bit checksum. Readiness, elapsed time, a heartbeat and reported CPU consumption cannot satisfy this check. Replayed, missing, reordered, oversized or incorrect work is rejected before progress advances.

The test records the exact child PID, creation token, image and native baseline. It then checks 14 batches, totaling 7,000,000 work units:

1. Original native baseline.
2. Priority-only Normal to BelowNormal, then exact Undo.
3. EcoQoS-only Apply and exact Undo from each test-owned SystemManaged, explicit Off and explicit On baseline. An already-On baseline is printed as unchanged. Each fixture returns to the actual original baseline before the next one.
4. Combined Priority/EcoQoS Apply, followed by exact Undo to the original fields.

Fresh identity-checked native query handles independently read priority and all QoS version/control/state fields before and after every batch. Foreign QoS bits must remain equal to the captured values. A changed creation token is refused before the setter callback. Missing EcoQoS readback fails the requested probe; it does not produce a combined success claim.

The protocol admits at most 24 requests of 1,000,000 units and 128 bytes per line. Replies and normal Quit have three-second deadlines, with a 30-second parent deadline. The child exits on stdin EOF and has a separate 32-second watchdog. Successful completion requires normal child exit after exact restoration. On failure, the owner closes stdin and uses the retained `Child` process handle for kill/finite exit polling; it never looks up or kills an arbitrary PID. These bounds cover work, pipes and cleanup; they do not assert a universal deadline for an uninterruptible native Windows call.

The timing is an acceptance deadline, not a throughput or game-performance benchmark. Passing proves that this arithmetic child completed known work while the requested fields were independently observed, and that those fields returned exactly. It proves no Windows-role function, service identity, recovery after utility death, practical benefit or compatibility with another host.

Local acceptance on 2026-10-24: four initial pure protocol tests were observed failing before implementation; five final pure tests passed. An independent code review found no blocker in the bounded work/result, identity or cleanup paths. The explicit unprivileged native run passed all 14 batches and exact restoration from an original Normal/SystemManaged baseline, including the separate Off/On fixtures and combined case. Its child exited normally. This is owned-child evidence only; no real service, role, CPU Set, application configuration or production gate was changed by the probe.

## Evidence required before an actual role test

Use a disposable Windows 11 x64 VM with a restorable snapshot and no personal workload. Record OS edition/build/patches, architecture, VM CPU topology, power/Game mode, installed .NET/Search/handler versions, executable and module file identities/signatures, source commit and lab EXE hash. Use synthetic input created from source; do not download a helper EXE, widen a production gate or modify system task/service configuration to make a role appear eligible.

Choose one exact role/context and one mechanism per case. Save the expected functional result, independent observation method, maximum completion time and cleanup procedure **before** Apply. Set a 60-minute per-role session budget, including a bounded recovery window. If ordinary work cannot complete within its predeclared case budget, record unavailable or failed; do not extend indefinitely or force completion by stopping/restarting a service. Record individual pending cases rather than one family-wide PASS.

There are three independent prerequisites:

- **Native identity:** exact live process instance, signed image/modules, supported user/logon/session and registered task/service/Framework context. Current verification is a starting point, not blanket admission.
- **Functional observability:** a reproducible input, normal processing progress and independently verified result attributable to that role. Generic CPU activity, `SERVICE_RUNNING`, task return code, process exit zero or a successful setter cannot replace this.
- **Recovery authority:** the same exact instance, original owned fields and comparison witness survive the required ownership/restart tests. Reading a saved witness grants no authority to mutate a new instance.

### Scheduled-task association blocker

Current Appraiser/Device Census verification checks a registered task's RUNNING state, executable action and arguments, plus process/image/token evidence. It does not bind the candidate PID/creation token to that task's exact running action instance. This gap must be closed and tested before either role can be admitted, even on a build where its task is present.

The documented [`IRunningTask::EnginePID`](https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nf-taskschd-irunningtask-get_enginepid) identifies the task engine process; treating it as an executable action PID is not a demonstrated association. A future implementation needs a supported, validated action-to-process association for each admitted build, with complete bounded lifecycle evidence. Missing events, ambiguity, PID reuse or a changed definition must refuse admission.

Required negative case: the approved registered task is RUNNING while a second process of the same signed image/path and SYSTEM Session 0 context exists outside that action instance. Only the proved action instance may be eligible. A filename, shared parent, timing coincidence or manual launch of the system EXE must not make the second process eligible. Keep exact task names; a prefix or similar task is not equivalent.

## Role-specific VM cases

Every row below remains **NOT ADMITTED**. The original four roles come first; the three Search roles are separate future cases. The functional observations in this table are requirements to establish in the VM, not implemented automation or claimed stable public result contracts.

| Role/context | Identity and permitted experiment | Required independent function result; unresolved blockers |
| --- | --- | --- |
| Compatibility Appraiser | Exact `Microsoft Compatibility Appraiser` action, supported `CompatTelRunner.exe`/`appraiser.dll` identities and arguments; resolve the action-instance blocker above. | Establish a supported, build-specific observation of a known synthetic compatibility-assessment input, progress and completed assessment output/consumer result. The current project has no such oracle. Task return code, a telemetry file change or process exit alone does not establish correct assessment. If the VM cannot expose a trustworthy result, leave the role unavailable. |
| Device Census | Exact `\Microsoft\Windows\Device Information\Device` action expected by the current resolver and `DeviceCensus.exe` identity; resolve the same action-instance blocker. | Establish a supported, build-specific census input/result and confirm the normal consumer receives the correct device-inventory result. There is no implemented result oracle here. A scheduler completion event or an alive process is insufficient. No oracle means no admission. |
| .NET Framework optimizer | Only the separately verified installed Framework executable/context. The current service variant is refused. Priority mutation remains forbidden because spawner inheritance is unqualified; evaluate EcoQoS separately. | In the VM only, use a small trusted test assembly built from source and supported Framework queue controls. Observe that its exact queued compilation completes, inspect its native-image state, run the assembly with a known result, and independently confirm that the intended native image was used rather than silently falling back to JIT. Attribute this to the exact worker/context and inspect descendants. Existing service or sibling variants gain no admission. |
| Diagnostic telemetry | Exact LocalSystem Session 0 DiagTrack instance, supported signed `svchost.exe`/`diagtrack.dll`, unchanged SCM configuration and sole stable service membership. | Establish an allowed synthetic diagnostic scenario with attributable processing progress and correct downstream diagnostic result through supported local observability. Running status, sent bytes or a generic event-log record is insufficient. No stable public end-to-end oracle has been established here; unknown processing or delivery semantics keeps the role unavailable. Do not generate private user telemetry to fill this gap. |
| Search indexer / WSearch | Exact independently verified WSearch process, account, binary configuration and sole stable SCM membership. | Process a fixed local corpus and its add/edit/rename/delete changes. Verify expected URIs/content tokens and absence of stale results with independent queries; confirm backlog convergence and normal post-session catch-up. Preserve ordinary Windows backoff/Game mode behavior. |
| Search protocol host | Each exact parent/role, account/logon/session and supported handler variant needs a new native proof; the current native resolver refuses helpers. | The same corpus must converge with attributable protocol-host work. Cover supported system/user instances, enumeration failures and recycling; a new helper is a new identity. No family-wide inheritance from indexer qualification. |
| Search filter host | Each exact restricted host/handler variant needs a new native proof; current helper refusal remains. | Validate text/properties from known files handled by the tested filters, their expected query results, completion and normal recycling. Inspect filter errors, retries and timeouts. Signed host identity does not qualify arbitrary third-party filters. |

For Framework, Microsoft documents queued compilation, native-image inspection and fallback behavior in the [`Ngen.exe` reference](https://learn.microsoft.com/en-us/dotnet/framework/tools/ngen-exe-native-image-generator). These APIs support designing a VM oracle; they do not validate our `mscorsvw` attribution or service variant automatically. Do not drain unrelated queued jobs or change the host's global queue state to create a passing case.

For Search, [`GetCatalogStatus`](https://learn.microsoft.com/en-us/windows/win32/api/searchapi/nf-searchapi-isearchcatalogmanager-getcatalogstatus) reports status/pause reason, and [`NumberOfItemsToIndex`](https://learn.microsoft.com/en-us/windows/win32/api/searchapi/nf-searchapi-isearchcatalogmanager-numberofitemstoindex) exposes incremental, notification and high-priority queue counts. Record those alongside expected corpus results; counters reaching zero alone do not prove correct content. Microsoft's [indexing architecture](https://learn.microsoft.com/en-us/windows/win32/search/-search-indexing-process-overview) describes separate helper roles, and [prioritization documentation](https://learn.microsoft.com/en-us/windows/win32/search/indexing-prioritization-and-rowset-events) explains work that can bypass backoff. Do not force urgent indexing, reset/reindex the catalog, or disable backoff to manufacture a benefit.

## Bounded native and recovery sequence

1. Restore the VM snapshot, establish ordinary functional baseline and record the exact target's native priority and full QoS control/state/version. Record task/service configuration and result observability before selecting the case. No active work or unreadable baseline means unavailable.
2. Run matched baseline, priority-only, EcoQoS-only and combined cases from equivalent input/snapshot conditions, only where that role permits the mechanism. Repeat each eligible functional case three times within its fixed budget. Do not raise an already lower priority or count an equal baseline as an applied optimization. Test SystemManaged, explicit Off and On preservation on owned fixtures; do not manufacture these baselines on a system role.
3. Before every native write, revalidate exact identity and current task/service context. Apply through the existing application writer/controller in a separately prepared nonshipping VM fixture. Independently open/query the live target after Apply and while work is progressing. Verify normal completion/result and errors, then Undo only if current fields still equal the expected owned values; independently verify the exact original fields and foreign bits afterward.
4. Repeat restoration for trigger exit, checkbox Off, normal utility shutdown, overlapping compatible trigger owners and utility restart. One owner leaving must retain changes needed by another; final-owner release must restore the first baseline. A rule moved to another group must keep the same source/owner. Conflicting requests must refuse.
5. Use explicit nonshipping fault injection in the VM to terminate only the utility at durable-persistence, attempted-write and verification boundaries. Capture independent target fields and original journal evidence before restart; verify startup recovery without inventing completion or discarding uncertain evidence. A restored snapshot is cleanup after the case, not proof that application recovery worked.
6. Exercise target exit, replacement/reused PID, changed image/task/service configuration, shared/pending service hosts, inaccessible/critical targets, another user/logon and external priority/QoS changes. A reused or uncertain identity must receive no restore; an external field change must remain pending without being overwritten. Reacquire proof for any newly started target.
7. Stop within the budget, export bounded logs/results and restore the snapshot. Record outcomes separately as identity, native Apply/readback, function, exact Undo, crash/restart and resource/usefulness evidence. Any unknown required cell leaves that role/context/mechanism unqualified.

Before actual system-service cases, a VM-only dedicated test-service fixture should exercise the same SCM/process boundary with a known work/result protocol. It must use a unique exact name, a locally built source-owned binary and an explicit VM opt-in, retain its registration/configuration and PID/creation identity, and provide finite lifetime and conditional cleanup. Refuse an existing name or changed configuration. Delete only the exact registration created by that fixture, and terminate only its retained owned process. Include shared-host and configuration-change refusals without reconfiguring a production `svchost`. No such service executable or installer is supplied or run by this delivery; the arithmetic child does not substitute for SCM evidence.

## Evidence record and promotion decision

For each case retain a bounded manifest containing source/lab artifact hashes, VM snapshot/build/context, exact role/native identity, mechanism, original/desired/read-back/restored fields, input and expected result, observed function result, monotonic deadlines, errors, recovery/owner/conflict outcomes and cleanup result. Keep original raw evidence separate from the decision. Do not store personal payloads, credentials or unrelated diagnostic records.

An actual result record must label evidence precisely: **owned-child mechanism/function**, **dedicated test-service boundary**, or **actual role/build/context/mechanism**. Never turn an unexecuted checklist or a test-owned setter result into a role PASS. Promotion requires resolving identity/result blockers and every required native/function/recovery cell, followed by a narrow reviewed code change. Tests cannot open gates automatically.

Before that first promotion, replace the all-or-nothing role admission with a consistent role/mechanism/build/context contract across candidate presentation, planning and the native write boundary. The current false boolean safely rejects every real role, but making it true would not express a priority-only or EcoQoS-only result. Define conditional Undo/recovery separately from admission of a new Apply: narrowing qualification after an update must not silently strand previously owned fields. Do not add an unused qualification scaffold or a user bypass in advance of actual evidence.

Practical value is a further physical-hardware experiment: matched real contention, foreground latency/frametime distributions, background completion time and utility CPU/I/O/memory overhead. A VM functional pass or reduced task-manager CPU percentage proves no game benefit. CPU placement, download adapters and changes to other programs remain outside this protocol.
