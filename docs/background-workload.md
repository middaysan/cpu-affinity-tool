# Background workload control (local preview)

This development branch adds **Reduce background load while this app runs** to each **Application rule** in Affinity groups. It is off by default. The app's own affinity and priority remain separate. There is no standalone System Tweaks card.

**System-role changes are not admitted in this preview.** No real Windows role has completed controlled VM/function validation. Checking an image, signature, task or service proves its identity, not that changing it is qualified. The current compiled mutation qualification set is empty; there is no user override or configuration switch that bypasses it. Native mechanisms have been exercised on child processes owned by the tests, not on Windows services.

## Controls

1. Add the app's EXE to an affinity group and open its settings. Installed-app activation targets are not supported as triggers.
2. Enable **Reduce background load while this app runs**, then **Save Changes**. Cancel/Close discard the unsaved option together with the rest of the form. App edits and the background policy save atomically through the existing writer.
3. The session follows the exact running app instance, including while you Alt-Tab or launch it outside the utility. The app itself is excluded from background targets. Read-only discovery works while Process monitor is paused; it does not change the app's affinity or insert a classic monitoring entry.
4. After the app exits, or its saved option is disabled, retained session changes are conditionally restored. Normal utility closure also restores them; recovery after an abrupt termination runs at the next start. No separate service guarantees immediate restoration after a crash.

Policies are independent per logical app rule and follow it between groups. Existing shared preview selections remain readable; editing one selected app splits only its own settings and preserves the other apps. Plain MCP rule replacement preserves these separate settings. Existing internal finite-preview/controller recovery data remains supported, but the removed standalone preview/role-selection controls are not shown in the app editor. There is no Advanced section, CPU-list action or manual core selector.

In this local preview, enabling the option demonstrates discovery and session state. It will not lower a system process's priority or change its CPU Sets. Saved enabled intent is distinct from changed and verified native fields.

## Fixed CPU policy

The one checkbox requests Priority + EcoQoS only. For a future independently qualified target, Normal priority may become BelowNormal where that role permits it. Existing Idle/BelowNormal values remain unchanged. Unexpected higher priorities exclude the whole target. EcoQoS owns only the execution-speed bit, retaining its original control/state and other bits. An unavailable getter skips that mechanism rather than inventing a baseline. The app's own priority and affinity are independent.

New sessions do not apply CPU Sets or choose E-cores/manual cores. Previously saved placement options are ignored when deriving active policy; saving the app clears those options for that rule. Reading an old document does not rewrite it. Old CPU Set codecs, witnesses and conditional restoration remain available for recovery of previously recorded ownership; retaining them does not authorize a new placement write.

## Catalogue and exclusions

Base candidates are Windows compatibility assessment, Device Census, .NET Framework optimization and diagnostic telemetry. Each requires exact live role evidence in addition to the expected signed image. Service admission requires a sole, stable running service in its host. Unknown task actions, shared hosts, changed configuration, critical processes, the foreground process, this utility and tracked game/app processes are excluded. Search roles are detected but remain optional, unadmitted future work.

This does not stop services, suspend processes, cap CPU through jobs, create IFEO rules, alter service startup, block updates or limit downloads. Delivery Optimization/BITS adapters, extra vendor/search families, GPU/I/O experiments and paired game/latency qualification remain separate work. Use Windows or the download application's settings for bandwidth limits. No FPS or latency improvement is promised.

## Ownership, resource use and validation limits

The existing System Tweaks worker owns policy saves and reconciliation; there is no additional thread, CPU sampler or ETW session. Off with no retained ownership performs no catalogue scan. Armed policies reuse the existing once-per-second execution observations; active/explicit applicability checks use a bounded, lazy five-second catalogue refresh and an offline bounded signature cache. The app editor edits only a local checkbox draft and starts no catalogue/topology read. At most 128 target candidates, 129 trigger/pool records, the controller's existing 256-owner budget and 32 recent session events are retained. An exceeded bound refuses further admission visibly.

Fields retain exact original values, expected/attempted writes and a bounded native comparison witness. Equal policies can share the first baseline, including when another session has already set the desired value; an unowned suitable value creates no Undo record. Moving a selected logical rule between groups keeps the same running trigger and pool. Incompatible changes conflict. Game lifetime and background-target lifetime are different: game exit restores live targets, while positive target-exit evidence retires only the dead instance. Reused PIDs are never restoration authority. Later changes made by another program are retained and shown as recovery needing attention.

Local acceptance includes deterministic policy/controller/UI regressions, offline signed/unsigned image checks, a read-only SCM inventory, joined GLPI/CPU Set inspection on a Windows 11 x64 host with 20 logical CPUs/14 physical cores, and owned-child Apply/readback/Undo of priority, EcoQoS and process-default CPU Sets (including an empty original set). The preceding standalone-card build's isolated GUI-MCP acceptance checked saved-game discovery while Process monitor was off, complete SMT selection/save, preview/Undo, both themes and ordinary-release restart/persistence; those historical captures do not certify this later app-editor layout. Test windows closed normally and working settings stayed unchanged. Artifact identities, short CPU19 overhead measurements and their limits are recorded in [MR #24](https://github.com/middaysan/cpu-affinity-tool/pull/24).

These checks do not qualify real roles, other hardware/OS builds, thread-override races, service functional progress, VM crash/reload recovery or game performance. The qualification gate is deliberately still closed.

## Remaining work before real system-role changes

1. On a disposable Windows 11 x64 VM with a snapshot, run a dedicated test service through the native service path. Verify exact service/process identity and refusal of shared hosts, inaccessible/critical targets and changed service configuration. This extends the existing owned-child tests without reconfiguring production svchost.
2. Qualify each actual intended role independently: Windows compatibility assessment, Device Census, .NET Framework optimization and diagnostic telemetry. Read its baseline, apply only that role's permitted priority/EcoQoS fields and independently read the actual native values back. Roles absent from that VM/build remain unavailable.
3. Let each background function make normal progress and complete; verify its result and Windows errors. A setter/readback success is insufficient. Do not stop/restart a service merely to imitate successful application.
4. Repeat exact Undo on app exit, checkbox Off, normal utility shutdown, overlapping app sessions and utility restart. Force utility termination between persistence/write/verification phases, then check startup recovery. Also exercise target exit/PID reuse and external priority/QoS changes; restoration must not overwrite someone else's later value.
5. Record the tested OS/build/architecture and role evidence, then add only qualified roles/conditions to mutation admission. `Role::mutation_qualified` currently returns false for every role; tests alone do not flip it automatically. Run the native/recovery and app-level acceptance again with those narrowly admitted roles.
6. Separately measure the utility's overhead and paired real-game CPU/frametime behavior on physical hardware. This establishes whether the policy is useful; successful native mutation by itself proves no FPS/latency gain and creates no CPU-percent cap.

These steps have not been performed for real roles. The user's accepted scope so far is local builds and test-owned processes; no VM/service qualification is claimed. Manual CPU placement, Search/vendor extensions, bandwidth adapters and GPU/I/O experiments are outside this checkbox-only scope.

Native API references: [CPU Sets](https://learn.microsoft.com/en-us/windows/win32/procthread/cpu-sets), [physical-core topology](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getlogicalprocessorinformationex), [CPU Set metadata](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-system_cpu_set_information), [process power throttling](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation).
