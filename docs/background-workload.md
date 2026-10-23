# Background workload control (local preview)

This development branch adds **Reduce background impact** under **System Tweaks → System**. It is off by default and separate from the affinity and priority rules in Affinity groups.

**System-role changes are not admitted in this preview.** No real Windows role has completed controlled VM/function validation. Checking an image, signature, task or service proves its identity, not that changing it is qualified. The current compiled mutation qualification set is empty; there is no user override or configuration switch that bypasses it. Native mechanisms have been exercised on child processes owned by the tests, not on Windows services.

## Controls

1. Add a game's EXE to Affinity groups, then select it in this card. Installed-app activation targets are not supported as game triggers.
2. **Enable for selected games** saves the policy. A session follows the exact running game instance, including while you Alt-Tab. The game itself is excluded from background targets. The read-only discovery also works while Process monitor is paused; it does not change the game's affinity or insert a classic monitoring entry.
3. **Check applicability** inspects currently running catalogue candidates and reads physical-core topology. Missing, shared, inaccessible, excluded and unqualified roles are shown without starting services or maintenance tasks for a demonstration.
4. **Preview for 10 min** starts one finite session without requiring a game. Reopening the page does not extend it. **Undo now** disables this background policy and conditionally restores retained changes. Normal application closure restores session changes; after an abrupt termination, recovery runs at the next start. There is no separate recovery service guaranteeing immediate restoration after a crash.

In this local preview, enabling or previewing a policy can demonstrate discovery, session state and the explanations for skipped candidates. It will not lower a system process's priority or change its CPU Sets. A saved **Enabled** policy is distinct from **Changed and verified** native fields.

## CPU policies

- **Priority + EcoQoS** is the default. For a future independently qualified target, only Normal priority may become BelowNormal. Existing Idle/BelowNormal values remain unchanged. Unexpected higher priorities exclude the whole target. EcoQoS owns only the execution-speed bit, retaining its original control/state and other bits. An unavailable getter skips that mechanism rather than inventing a baseline.
- **Prefer efficient cores** additionally prepares a stable process-default CPU Set pool for the session when GLPI physical-core data and CPU Set data agree on a heterogeneous topology. It does not choose arbitrary last CPUs on a homogeneous processor.
- **Manual whole-core pool** selects every SMT sibling of a physical core together. The UI shows processor group, logical CPUs, efficiency class, cache and NUMA metadata. Parking does not make a CPU permanently unavailable. Reserved/foreign-allocated sets cannot be selected.

CPU Sets are optional soft placement. They do not reserve exclusive cores, cap CPU percentage, prevent kernel activity or change the game's CPU selection. Existing default sets cannot be widened. Hard affinity, thread-selected sets and target availability are checked separately; ambiguous or conflicting restrictions skip placement while priority/EcoQoS remain independent. This preview conservatively refuses native placement for a target spanning multiple processor groups or a physical core spanning groups; topology inspection retains group identities and never flattens IDs into one `u64` mask. Topologies above 512 logical CPUs or evidence exceeding the retained context budget skip this optional path. Changed topology never authorizes writing old IDs onto a new topology.

## Catalogue and exclusions

Base candidates are Windows compatibility assessment, Device Census, .NET Framework optimization and diagnostic telemetry. Each requires exact live role evidence in addition to the expected signed image. Service admission requires a sole, stable running service in its host. Unknown task actions, shared hosts, changed configuration, critical processes, the foreground process, this utility and tracked game/app processes are excluded. Search roles are detected but remain optional, unadmitted future work.

This does not stop services, suspend processes, cap CPU through jobs, create IFEO rules, alter service startup, block updates or limit downloads. Delivery Optimization/BITS adapters, extra vendor/search families, GPU/I/O experiments and paired game/latency qualification remain separate work. Use Windows or the download application's settings for bandwidth limits. No FPS or latency improvement is promised.

## Ownership, resource use and validation limits

The existing System Tweaks worker owns policy saves and reconciliation; there is no additional thread, CPU sampler or ETW session. Off with no retained ownership performs no catalogue scan. Armed policies reuse the existing once-per-second execution observations; active/explicit applicability checks use a bounded, lazy five-second catalogue refresh and an offline bounded signature cache. Opening the card reads cached snapshots only. At most 128 target candidates, 129 trigger/pool records, the controller's existing 256-owner budget and 32 recent session events are retained. An exceeded bound refuses further admission visibly.

Fields retain exact original values, expected/attempted writes and a bounded native comparison witness. Equal policies can share the first baseline; incompatible changes conflict. Game lifetime and background-target lifetime are different: game exit restores live targets, while positive target-exit evidence retires only the dead instance. Reused PIDs are never restoration authority. Later changes made by another program are retained and shown as recovery needing attention.

Local acceptance includes deterministic policy/controller/UI regressions, offline signed/unsigned image checks, a read-only SCM inventory, joined GLPI/CPU Set inspection on a Windows 11 x64 host with 20 logical CPUs/14 physical cores, and owned-child Apply/readback/Undo of priority, EcoQoS and process-default CPU Sets (including an empty original set). Isolated GUI-MCP acceptance also checked saved-game discovery while Process monitor was off, complete SMT selection/save, preview/Undo, both themes and ordinary-release restart/persistence. Test windows closed normally and working settings stayed unchanged. Artifact identities, short CPU19 overhead measurements and their limits are recorded in [MR #24](https://github.com/middaysan/cpu-affinity-tool/pull/24).

These checks do not qualify real roles, other hardware/OS builds, thread-override races, service functional progress, VM crash/reload recovery or game performance. Those gates must pass before production role promotion.

Native API references: [CPU Sets](https://learn.microsoft.com/en-us/windows/win32/procthread/cpu-sets), [physical-core topology](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getlogicalprocessorinformationex), [CPU Set metadata](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-system_cpu_set_information), [process power throttling](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation).
