# Release Smoke Matrix

Use this document together with `docs/release-checklist.md`. This is a compact manual smoke reference for the real Windows release path, not a second release checklist.

## Goal

Confirm that the shipped Windows binary starts correctly, applies its core launch behavior, and still matches the documented release contract after the final release build.

## Smoke Scenarios

| Scenario | Expected result | Release-blocking if it fails |
| --- | --- | --- |
| Cold start with existing `state.json` | App starts, loads saved state, and shows the expected groups, apps, and theme without corruption prompts | Yes |
| Add app via picker | Selected app is added once, remains visible, and survives restart after save | Yes |
| Add app via drag-drop | Dropped app is added exactly once to the intended group | Yes |
| `.lnk` launch path | Shortcut resolves to the real target and preserves arguments when launched | Yes |
| `.url` / URI-handler path | Registered handler resolves and launches the correct target when such a handler is available on the test machine | Yes, if the release contract depends on supported URI launch flows |
| Already-running app launch | Existing window is focused or reused without spawning an unintended duplicate instance | Yes |
| Single-run and `Run All` | Individual launch and group launch both work without missing or duplicate starts | Yes |
| Monitoring toggle and notifications | Monitoring state can be toggled, corrections still apply when enabled, and expected notifications or logs appear | Yes |
| Tray hide / restore | App can hide to tray and restore cleanly without becoming unresponsive | Yes |
| Theme persistence across restart | Theme changes persist after closing and reopening the app | Yes |
| Logs visible and clearable | Logs appear in chronological order and can be cleared without breaking later logging | Yes |
| UAC / administrator expectation | Launching the built binary requires elevation as documented; a non-elevated start shows the expected UAC prompt, while an already elevated launch starts without contradicting the docs | Yes |

## Troubleshooting Notes

- If launch failures happen only for shortcuts or URI handlers, verify the target machine has the expected handler installed and registered.
- If `.url` coverage is not applicable on the smoke machine, mark the scenario as not applicable and note the missing handler explicitly in the release validation notes.
- If tray restore fails, verify that the app is still alive in Task Manager before retrying; a dead process is release-blocking, a one-off interaction miss should be reproduced before classification.
- If saved state appears missing after restart, verify the binary is using the expected active state path: the legacy sidecar `state.json` next to the executable if it already exists, otherwise the platform data directory.
- If UAC behavior differs from expectation, re-check `app.manifest`, the built artifact, and any packaging step that might have replaced resources.
