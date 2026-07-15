# Release Smoke Matrix

Use this document together with `docs/release-checklist.md`. This is a compact manual smoke reference for the real Windows release path, not a second release checklist.

## Goal

Confirm that the shipped Windows binary starts correctly, applies its core launch behavior, and still matches the documented release contract after the final release build.

## Smoke Scenarios

| Scenario | Expected result | Release-blocking if it fails |
| --- | --- | --- |
| Cold start with existing `state.json` | App starts, loads saved state, and shows the expected groups, apps, and theme without corruption prompts | Yes |
| Add app via **Add installed...** | Selected app is added once, remains visible, and survives restart after save | Yes |
| Add app via **Add file...** | Selected path or launcher is added once, remains visible, and survives restart after save | Yes |
| Add app via drag-drop | Dropped app is added exactly once to the intended group | Yes |
| `.lnk` launch path | Shortcut resolves to the real target and preserves arguments when launched | Yes |
| `.url` / URI-handler path | Registered handler resolves and launches the correct target when such a handler is available on the test machine | Yes, if the release contract depends on supported URI launch flows |
| **Fix** on a mismatched running app | Saved affinity and priority are applied without launching or focusing; protected/green appears only after every setting call succeeds | Yes |
| **Focus** on a protected running app | Existing window is activated without reapplying settings or spawning another process | Yes |
| Single-run and `Run All` | Individual launch and group launch both work without missing or duplicate starts | Yes |
| **Monitoring active** / **Pause monitor** | Monitoring can be paused and resumed, corrections still apply while active, and expected events appear in **Activity** | Yes |
| Tray hide / restore | App can hide to tray and restore cleanly without becoming unresponsive | Yes |
| Theme persistence across restart | Theme changes persist after closing and reopening the app | Yes |
| Theme selector icons | System, light, and dark states use readable painted icons with no missing-glyph square | Yes |
| **Activity** visible and clearable | Events appear in chronological order and can be cleared without breaking later activity reporting | Yes |
| Overview and Activity navigation | The centered navigation switches routes without overlap, clipping, or losing state | Yes |
| Inter rendering and fallback | Latin and Cyrillic text, digits, punctuation, long names, and fallback glyphs remain readable at 100%, 125%, 150%, and 200% display scaling | Yes |
| Compact layout and themes | Group boundaries, controls, statuses, and the full-width monitoring footer remain readable at minimum window size in system, dark, and light themes | Yes |
| CPU-thread selection contrast | Selected Performance and Efficient threads plus **All** use the restrained turquoise primary state and remain distinct from the surrounding surface in dark and light themes | Yes |
| CPU-thread selector wrapping | With scattered threads selected, every control wraps within the fixed-width editor, retains readable core and `thread N` labels, and remains clickable without horizontal overflow | Yes |
| Group reorder by pointer and keyboard | Each input path changes the persisted order once; restart retains order and saved logical IDs still resolve the same rules | Yes |
| Long application status | Full status meaning is discoverable when the visible label does not fit; running-app action remains reachable | Yes |
| UAC / administrator expectation | Launching the built binary requires elevation as documented; a non-elevated start shows the expected UAC prompt, while an already elevated launch starts without contradicting the docs | Yes |
| Built Windows manifest resource | `scripts/assert-windows-release-manifest.ps1` passes against `target/release/cpu-affinity-tool.exe`, confirming `requireAdministrator` and `uiAccess=false` on the built artifact | Yes |

## Shortcut MVP Smoke

Saved-rule desktop shortcuts are Windows-only in the stable release path. They target the current executable path and should be recreated after moving a portable app folder. Shortcut launches may show UAC because they run the same elevated executable. Shortcut creation uses the Desktop for the current elevated Windows token; credential-over-the-shoulder UAC can place shortcuts on the elevated account's Desktop. Linux beta builds do not expose shortcut UI or generate `.desktop` launchers.

| Setup | Action | Pass probe | Fail probe | Expected exit code | Where to verify logs/status | Release-blocking | Notes/N/A rule |
| --- | --- | --- | --- | ---: | --- | --- | --- |
| Saved clean path-target rule, app closed | Create shortcut from rule settings, close tool, launch shortcut | Tool opens and runs only the requested rule | Unrelated autorun rule launches, requested rule not attempted, or GUI fails to open | N/A for `.lnk`; direct exe accepted path should be `0` after forwarding only | In-app **Activity** after startup | Yes | Use direct `Start-Process -Wait -PassThru` only for explicit exe exit-code probes |
| Saved clean path-target rule, app already running | Launch generated shortcut | Existing primary handles command; new process exits | Second long-lived primary appears or command is ignored | `0` for accepted forwarded command | **Activity** of active primary; Task Manager for process count | Yes | UAC prompt may appear before forwarding because the binary requires admin |
| Normal GUI launch with autorun rule | Start tool normally | Existing autorun behavior is unchanged | Autorun skipped or shortcut command path runs | N/A | In-app **Activity** | Yes | Confirms no global single-instance behavior for `NormalGui` |
| Renamed app/group with old shortcut | Rename saved app/group, save, launch old shortcut | Shortcut still resolves by IDs and launches the rule | Shortcut fails because display names changed | `0` if forwarded and accepted | In-app **Activity** | Yes | Filename is cosmetic |
| Reordered app/group with old shortcut | Reorder groups/rules, launch old shortcut | Shortcut still resolves by IDs and launches the rule | Wrong rule launches | `0` if forwarded and accepted | In-app **Activity** | Yes | Rule identity must survive reorder |
| Rule moved to another group | Launch old shortcut | Nothing launches; clear missing-rule behavior | Moved rule launches from old group ID | `21` for direct forwarded exe probe | Active primary activity or cold-start **Activity** | Yes | Old `(group-id, rule-id)` pair is intentionally invalid |
| Deleted group | Launch old shortcut | Nothing launches; clear missing-group behavior | Any target launches | `20` for direct forwarded exe probe | Active primary activity or cold-start **Activity** | Yes | Deleted IDs must not be reused |
| Deleted rule | Launch old shortcut | Nothing launches; clear missing-rule behavior | Any target launches | `21` for direct forwarded exe probe | Active primary activity or cold-start **Activity** | Yes | Deleted IDs must not be reused |
| Path-target rule | Create and inspect generated shortcut | Target is current `cpu-affinity-tool.exe`, args are `--run-rule <group-id> <rule-id>`, working dir is exe dir, icon points to exe | Shortcut stores target app path or raw app args instead of saved IDs | N/A | Shortcut Properties and in-app status | Yes | Current executable path is captured at creation time |
| Installed/AUMID rule | Create shortcut and launch it | Saved installed target launches through the saved rule | AUMID leaks into filename unless it is the saved app name, or installed target cannot launch | `0` if forwarded and accepted | In-app **Activity** | Yes | Saved-rule shortcut docs cover installed targets; scope docs to path-target-only before release if this row cannot be smoked |
| Shortcut filename collision | Create two shortcuts for same saved rule | Second file uses numbered suffix and does not overwrite first | Existing shortcut overwritten or creation panics | N/A | Desktop files and in-app status | Yes | Existing files or directories count as collisions |
| Dirty unsaved rule settings | Edit rule without saving | Button is disabled and no shortcut is written | Unsaved draft is exported | N/A | Rule settings status | Yes | Save first, then create shortcut |
| Secondary normal GUI instance | With a primary Windows GUI already running, open a second normal GUI and view a saved clean rule | Shortcut area remains visible but disabled with an actionable message such as closing the other running instance first; no shortcut is written | Secondary GUI can create a shortcut that forwards to stale primary state | N/A | Rule settings status and Desktop files | Yes | Normal GUI startup is not global-single-instance; only the primary forwarding owner should export shortcuts |
| Stale shortcut result and same-frame dirty state | After a shortcut success/error status is visible, edit the rule and try to create a shortcut before saving | Stale status clears or changes to save-first disabled state; no shortcut is written from the dirty draft | Old success/error remains misleading, button stays enabled, or dirty draft is exported | N/A | Rule settings status and Desktop files | Yes | Automated state/presenter coverage may satisfy this if manual same-frame interaction is impractical |
| Linux beta build | Open rule settings on Linux beta | Shortcut button/status is hidden | Linux UI exposes dead shortcut control or claims `.desktop` launcher parity | N/A | Linux UI | Yes for beta docs | Linux launcher parity is not part of MVP |
| IPC security source audit | Inspect S14 implementation | Explicit descriptor, scoped endpoint, remote rejection, session/user validation, SQOS client open, first-instance pipe are present | Default pipe ACL, unscoped endpoint, or missing auth/session validation | `26` for auth/security failure probe where practical | Source audit plus direct exe probe | Yes | See `docs/shortcut-launch-plan.md` IPC Security Closure |
| Primary starting but pipe absent | Hold/observe primary guard without ready pipe where practical | Forwarding retries briefly and exits non-zero | Second primary cold-starts | `23` | Direct exe probe stderr/exit code | Yes | Manual reproduction may require diagnostic build or controlled probe |
| Protocol error probe | Send invalid/unsupported frame with a local diagnostic client or unit-backed probe | Active primary rejects command and keeps running | Invalid command launches anything or crashes primary | `25` for direct client/exe-equivalent probe | In-app **Activity** and process liveness | Yes | Automated protocol tests cover parser behavior |

## Troubleshooting Notes

- If launch failures happen only for shortcuts or URI handlers, verify the target machine has the expected handler installed and registered.
- If saved-rule shortcuts fail after moving the portable app folder, recreate them; generated shortcuts target the executable path used at creation time.
- For shortcut exit-code probes, launch the executable directly with `--run-rule <group-id> <rule-id>` using PowerShell `Start-Process -Wait -PassThru`; `.lnk` shell activation and process exit-code capture are separate checks.
- If `.url` coverage is not applicable on the smoke machine, mark the scenario as not applicable and note the missing handler explicitly in the release validation notes.
- If tray restore fails, verify that the app is still alive in Task Manager before retrying; a dead process is release-blocking, a one-off interaction miss should be reproduced before classification.
- If saved state appears missing after restart, verify the binary is using the expected active state path: the legacy sidecar `state.json` next to the executable if it already exists, otherwise the platform data directory.
- If UAC behavior differs from expectation, re-check `app.manifest`, the built artifact, and any packaging step that might have replaced resources.
