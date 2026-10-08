# Managed Windows modules

Each module owns its typed configuration, editor, observation, resource projection and exact token codec. Constructors and editors perform no native reads. Schemes and CPU topology are cached by worker observations. `os_api::gaming` supplies narrow typed reads and conditional writes; application/controller code does not interpret their configuration payloads.

| Module | Owned field tokens | Scope |
| --- | --- | --- |
| Power plan | Exact GUID of `active_power_scheme`; only existing schemes | Verified interactive user |
| Power mode | Separate GUIDs of `user_power_mode_ac` and `user_power_mode_dc` | Verified interactive user; documented Windows 11 exports required |
| Mouse acceleration | Complete three signed integers of `mouse_acceleration_tuple` | Verified interactive user; session SPI operation |
| Accessibility hotkeys | HOTKEYACTIVE and CONFIRMHOTKEY flags separately for Sticky, Filter and Toggle | Verified interactive user; enabled features skipped |
| Process QoS | Original execution-speed control and state bits | Exact tracked process instance |
| CPU Sets | Complete default ID list, including original empty assignment | Exact tracked process instance; actual topology and reservation flags |
| Process priority | Exact original class; configuration has no Realtime choice | Exact tracked process instance; coordinated legacy writer required |
| Process timer policy | Original ignore-timer-resolution control and state bits | Explicit experiment; honor existing requests only |

Process conditional reads/writes retain one opened handle and recheck creation time, executable path and liveness. QoS and timer writes merge only their respective owned bits into the freshly read structure, preserving each other and unrelated bits. Accessibility writes retain the complete current native structure, so enabled state, timing, sounds and unrelated flags survive. User operations reject missing Explorer, impersonation, SID/session/logon mismatch and changed context. SPI writes are session-only and do not overwrite persisted Control Panel preferences.

These source adapters are not native acceptance evidence. The application must keep setting activation closed until durable baseline and lifecycle guards pass. Stage 3 performs no host setting mutations.

Required stage 4 native smoke, with exact before/apply/read/disable/restore observations:

- Existing power scheme GUID; separate AC and DC mode GUIDs, plan/mode interaction and genuine unsupported/denied contexts.
- Complete mouse tuple; foreground withdrawal. An enabled accessibility feature is untouched; each inactive selected feature restores hotkey bits while unrelated flags and Filter Keys timings remain exact.
- QoS and timer together: changing/restoring either preserves the other's owned bits. The timer experiment requires requesting/non-requesting and visible/occluded process evidence and makes no latency claim.
- CPU Set IDs across actual groups, original empty/nonempty default assignments, unavailable reservations, and hard-affinity/thread-selected constraints.
- Priority captured before any relevant launch/monitor write, coordinated single writer, exact last-owner restore. PID reuse, wrong image, denied reads/writes and actual exit remain distinct.

Primary API references: [Power schemes](https://learn.microsoft.com/en-us/windows/win32/power/managing-power-schemes), [user-configured AC power mode](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powersetuserconfiguredacpowermode), [process power policy and timer semantics](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation), [CPU Sets](https://learn.microsoft.com/en-us/windows/win32/procthread/cpu-sets), [Filter Keys structure](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-filterkeys).
