# Shortcut Launch Plan

Status: MVP implementation is complete in code and docs; release readiness still requires the manual smoke matrix before publishing.

Implemented so far:

- CLI startup parsing for `--run-rule <group-id> <rule-id>`
- non-reusable group/rule allocation counters in `rule_identities`
- saved-rule dispatch outcome for accepted, missing group, missing rule, and immediate launch rejection
- cold-start `RunRule` handling that skips normal autorun
- shortcut command builder for target path, arguments, working directory, and sanitized display name
- Windows `os_api::ShortcutSpec` and `.lnk` creation
- Windows second-instance `RunRule` forwarding through a local named pipe and primary mutex
- explicit named-pipe/mutex security descriptor, remote-client rejection, same-session and same-user client checks, SQOS-protected client open, first-pipe-instance creation, strict versioned JSON protocol, bounded frames, and typed forwarding exit codes
- app-level saved-rule shortcut service with no-overwrite filename allocation and current elevated token Desktop resolution
- rule settings `Create desktop shortcut` button for saved clean rules on Windows

Release gate still required:

- run the release-blocking shortcut smoke matrix in `docs/release-smoke-matrix.md`

Origin: GitHub Discussion #12, "Suggestion: shortcut and Run command line support".

## Summary

Add a Windows-first "Create desktop shortcut" flow for saved app rules.

The shortcut should not encode the full launch configuration. It should only point back to a saved rule by logical identity:

```text
cpu-affinity-tool.exe --run-rule <group-id> <rule-id>
```

The active app instance then resolves the saved rule from `state.json` and reuses the existing launch, affinity, priority, tracking, and correction logic.

Important blocker: saved rule IDs must be non-reusable before this feature is exposed. A deleted rule's old shortcut must never be able to launch a newly created rule after restart.

## Confirmed Facts

- Windows is the primary released platform.
- The Windows binary now accepts the narrow `--run-rule <group-id> <rule-id>` startup intent, but there is still no broad user-facing CLI command contract.
- Saved rules already have logical `GroupId` and `RuleId` identity through `rule_identities`.
- `AppState::run_group_program(group_id, rule_id)` already resolves logical IDs and dispatches the existing launch path.
- `AppState::run_group_program(group_id, rule_id)` now returns `RunRuleOutcome` for accepted, missing group, missing rule, and immediate launch rejection; second-instance IPC translates that outcome into typed responses and forwarding exit codes.
- Path-target rules already store launch args and tracked process names.
- Installed-app rules use AUMID targets and should remain supported by the saved-rule shortcut model.
- The Windows release binary embeds a `requireAdministrator` manifest, so shortcut launches are expected to go through UAC when required by Windows.
- The Windows manifest uses `uiAccess="false"`; the app is not an assistive-technology tool and shortcut IPC must not rely on UIAccess privileges.
- Shortcut creation resolves the Desktop for the current elevated token. If the app is elevated through credential-over-the-shoulder UAC with another administrator account, the shortcut can be created on that elevated account's Desktop instead of the unelevated shell user's Desktop.

## Goals

- Let a user create a desktop shortcut for an already configured app rule.
- Keep the GUI as the source of truth for launch target, args, CPU group, priority, and tracked process names.
- Let shortcut launches work whether the app is already running or not.
- Avoid a broad command-line surface that duplicates the whole rule editor.
- Keep the first implementation small enough to test with unit/state-level coverage plus a focused Windows smoke pass.
- Keep the user-facing shortcut button limited to the completed saved-rule shortcut path: CLI parsing, saved-rule dispatch, shortcut writing, and second-instance forwarding.

## Non-Goals For The First Version

- No `--binarypath`, `--coresetting`, or tracked-process CLI syntax.
- No parsing of CPU labels such as `P0-P1-P2`.
- No command-line mutation of saved state.
- No Linux desktop launcher parity in the first Windows release path.
- No auto-close-after-target-exits behavior in the first version.
- No attempt to avoid UAC while the binary itself still requires administrator privileges.
- No global single-instance behavior for normal no-intent launches in the first version. Single-instance forwarding is required only for `--run-rule`.

## User Flow

1. The user configures an app rule in the normal UI.
2. In that rule's launch settings, the user clicks "Create desktop shortcut" for the saved rule.
3. The app creates a `.lnk` on the Desktop for the current elevated Windows token.
4. The shortcut target is the current `cpu-affinity-tool.exe`.
5. The shortcut args include the saved `GroupId` and `RuleId`.
6. When the shortcut is opened, the tool runs that saved rule.

Shortcut creation must operate on saved persisted state, not on unsaved editor state. If the rule editor has unsaved changes, the UI should either disable shortcut creation with a clear hint or provide an explicit "Save and create shortcut" action.

If the main app is already running:

1. The new process parses the shortcut command.
2. It detects the active instance.
3. It forwards `RunRule { group_id, rule_id }` to the active instance.
4. The active instance runs the saved rule.
5. The new process exits after receiving success or failure.

If the main app is not running:

1. The process starts normally.
2. It loads persisted state.
3. It starts the normal runtime wiring.
4. It skips normal autorun for this shortcut-triggered startup.
5. It dispatches the startup `RunRule` command once the `AppState` is ready.
6. It logs and reports whether the command was accepted or rejected.
7. The GUI remains open unless a future option changes that behavior.

## Command Contract

Initial command:

```text
--run-rule <group-id> <rule-id>
```

Rules:

- `group-id` and `rule-id` are opaque strings.
- Names are not accepted as identity because group names and app names can be renamed or duplicated.
- If the group or rule no longer exists, the app should log a clear error and avoid launching anything.
- The command should not create or edit rules.
- The command should return a non-zero exit code from a forwarding process if the active instance rejects the command.
- The command has exact arity: no missing IDs, no extra positional arguments, and no unknown flags in the first version.
- IDs accepted from CLI or IPC should use a constrained no-whitespace grammar such as `[A-Za-z0-9._:-]{1,128}`. If rule identity generation changes later, it must keep shortcut IDs within that grammar.
- A shortcut binds to the `(group-id, rule-id)` pair. Renaming or reordering a group or app should not break the shortcut, but moving a rule to a different group should invalidate the old shortcut and require creating a new one.

Deferred behaviors are not reserved CLI contract. Exact names and encoding for minimized/background startup or auto-close after target exit are intentionally undecided and should be chosen only after lifecycle behavior is defined and tested.

## Shortcut Contract

The generated Windows shortcut should contain:

- target path: the current executable path
- arguments: argv tokens equivalent to `--run-rule <group-id> <rule-id>`; the Windows OS boundary is responsible for command-line quoting when writing `.lnk`
- working directory: the executable directory
- icon: the current executable icon where possible
- display name: a sanitized name derived from the app and group names

The shortcut file name is only cosmetic. The launch identity must come from the saved IDs in the arguments.

Shortcut creation belongs behind the OS boundary. The Windows backend already owns shortcut parsing and shell integration, so shortcut writing should be added there rather than directly in UI code.

Preferred API shape:

```text
OS::create_shortcut(ShortcutSpec)
```

`ShortcutSpec` should carry typed argument tokens rather than a pre-quoted raw command line. Expose shortcut creation to the app through `src/app/adapters/os`, keeping COM and Windows shell details out of `run_settings`.

## Architecture Plan

Add small, bounded pieces rather than a broad refactor.

1. CLI intent parsing

   Add a small parser near the entrypoint or a narrow app module. It should parse only known startup intents:

   - normal GUI startup
   - run saved rule by `GroupId` and `RuleId`

   A new dependency is not required for the first command. If the command surface grows later, a CLI parser dependency can be reconsidered.

2. Non-reusable shortcut-safe identities

   Before shortcuts can ship, persisted rule identity allocation must stop reusing IDs after delete/save/restart.

   Selected implementation approach:

   - persist next group/rule counters

   Alternatives considered but not chosen for the first implementation:

   - persist tombstones for exported shortcut IDs
   - switch to random UUID-like IDs that stay within the CLI-safe grammar

   This changes the persisted `rule_identities` shape and requires an `AGENTS.md` update in the same implementation change.

3. Saved-rule command dispatch

   Reuse the current launch path with an explicit outcome before IPC is implemented:

   ```text
   RunRuleOutcome::Accepted
   RunRuleOutcome::MissingGroup
   RunRuleOutcome::MissingRule
   RunRuleOutcome::LaunchRejected(String)
   ```

   Callers must be able to distinguish:

   - launched
   - missing group
   - missing rule
   - launch failed before process start

   If full launch success cannot be known synchronously, the IPC response should say "accepted" rather than "completed", and later runtime/process failures should continue to be logged.

4. Shortcut generation

   Add a UI command in the rule launch settings. The UI should request shortcut creation through an adapter/service, not construct Windows shell objects directly.

   The non-UI builder, Windows OS shortcut writer, app service, and user-facing Windows button are implemented.

   Do not expose this UI until the full MVP path works:

   - CLI parsing
   - non-reusable IDs
   - saved-rule dispatch outcome
   - shortcut writing
   - second-instance forwarding

5. Single-instance forwarding

   Use an explicit local IPC command path for second-instance launches. Prefer a Windows named pipe plus a single-instance lock over a localhost TCP port.

   Reasons:

   - a named pipe is a Windows desktop IPC primitive
   - it avoids exposing a network listener
   - the command surface can stay narrow and typed
   - a mutex or equivalent lock can reduce duplicate primary-instance races

   The pipe and lock names must be scoped by app identity plus user/session identity. Do not rely on the default named-pipe security descriptor. Microsoft documents that default named-pipe ACLs grant broad read access, including to Everyone and anonymous accounts, and recommends using the logon SID to prevent access from remote users or other terminal services sessions:

   - https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights

   Required IPC security properties:

   - explicit security descriptor, not default security: implemented in `libs/os_api/src/windows/ipc.rs`
   - scoped to current user/session and app executable identity: implemented through endpoint names plus pipe/mutex ACLs
   - no remote clients: implemented with `PIPE_REJECT_REMOTE_CLIENTS`
   - client identity verification before dispatch: implemented with same-session checks plus named-pipe impersonation and thread-token user SID verification
   - client-side impersonation protection: implemented with `SECURITY_SQOS_PRESENT`, `SECURITY_IDENTIFICATION`, and `SECURITY_EFFECTIVE_ONLY`
   - first-instance anti-squatting: implemented with a primary mutex plus `FILE_FLAG_FIRST_PIPE_INSTANCE`
   - fail closed on authentication or version errors
   - strict maximum IPC message size: implemented at 4096 bytes
   - strict `version == 1`: implemented
   - no unknown commands or unknown fields: implemented with deny-unknown-field serde frames
   - no control characters in logged error text: implemented by sanitizing optional response detail

   The command envelope is versioned JSON using existing `serde_json`:

   ```json
   {
     "version": 1,
     "command": {
       "type": "run_rule",
       "group_id": "group-1",
       "rule_id": "rule-3"
     }
   }
   ```

   The response should also be typed:

   ```json
   {
     "version": 1,
     "code": "accepted"
   }
   ```

   ```json
   {
     "version": 1,
     "code": "missing_rule"
   }
   ```

   Response codes are protocol values, not user-facing text. Optional display/log text is sanitized and is not required for control flow. The protocol rejects unknown fields, unsupported versions, oversized frames, unknown commands, and unknown response codes.

6. Shell integration

   The shell owns app lifecycle and typed event dispatch. A forwarded command needs a reply path, so do not overload the existing monitor-oriented `ShellEvent`.

   Prefer a separate shell-owned command channel:

   ```text
   ShellCommand::RunRule { group_id, rule_id, reply_tx }
   ```

   The shell should drain this channel on the GUI thread, call `AppState`, and send a typed response to the forwarding process.

7. Startup ordering

   Shortcut-triggered cold start should skip normal autorun and run only the requested rule. Normal GUI startup without a command keeps existing autorun behavior.

## Security And Runtime Risks

- Because the release executable requires administrator privileges, the shortcut may trigger UAC even when it only forwards a command.
- Because shortcut creation uses the current elevated token, credential-over-the-shoulder UAC can put the generated shortcut on the elevated administrator account's Desktop.
- Because the active app is elevated, IPC can become an elevation bridge if a lower-integrity or different-session process can connect and trigger saved launches.
- The manifest uses `uiAccess="false"`. Microsoft documents `uiAccess=true` as intended for assistive technology scenarios and says it should not be used by applications that are not assistive technologies:
  - https://learn.microsoft.com/en-us/cpp/build/reference/manifestuac-embeds-uac-information-in-manifest
  - https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview
- IPC must not accept arbitrary executable paths or shell commands.
- IPC should accept only a small allowlist of typed commands.
- Unknown or stale IDs must fail closed.
- Saved state is privileged user intent for this feature. IPC may request only saved IDs; it may not create, mutate, or override target path, args, priority, cores, or tracked process names.
- Log the resolved saved rule target before launching from a shortcut.
- Verify state-file ownership and ACL expectations before treating saved rules as privileged launch intent.
- Race handling matters when two shortcuts are launched at the same time.
- If the active app is starting but the IPC endpoint is not ready yet, the forwarding process retries briefly and then reports failure; it must not become a second primary instance while another process owns the primary guard.
- If a shortcut points to an old moved executable path, Windows will fail before the app can recover; this is normal shortcut behavior. If possible, user-facing docs should tell users to recreate shortcuts after moving the portable app folder.

## Alternatives Considered

PowerShell workaround:

- Good for users who only need one-off launch plus affinity.
- Poor fit for saved rules, tracked helper processes, installed apps, monitoring, logs, and correction loops.

Full CLI rule specification:

- Flexible, but it duplicates the GUI rule editor and creates a larger long-term compatibility contract.
- Higher risk because CPU core syntax, priority, args, installed apps, and tracked process names all need stable parsing and validation.

Localhost TCP:

- Easier to prototype, but it creates a local network listener and needs extra security decisions.
- Not recommended for the first desktop-only command path.

Names as shortcut identity:

- Easier to read, but unsafe because names can be renamed or duplicated.
- Not recommended.

## Implementation Stages

1. Document the design and command contract.
2. Add failing tests for CLI parsing and startup intent handling.
3. Implement `--run-rule <group-id> <rule-id>` parsing.
4. Add failing tests proving deleted shortcut IDs cannot be reused after save/restart.
5. Implement non-reusable shortcut-safe identity allocation.
6. Add state-level tests for running a saved rule by logical IDs and for stale IDs.
7. Add a `RunRuleOutcome`-style dispatch result.
8. Expose a small shortcut command builder that produces executable path plus args.
9. Add builder tests for quoting, ID grammar, and saved-only shortcut semantics.
10. Add Windows shortcut creation behind the OS boundary with tests where practical.
11. Add deterministic IPC tests for command validation, response mapping, retry behavior, lock/race behavior, and rejection exit codes.
12. Add single-instance detection and command forwarding.
13. Add the rule settings UI button and error reporting only after the end-to-end shortcut flow is ready.
14. Update `AGENTS.md`, README or user-facing docs, changelog/release notes, and manual Windows smoke coverage as needed for the runtime, `os_api`, and user-facing behavior changes.

## Test Plan

Automated coverage should be added before production behavior changes whenever technically possible.

Unit and state-level tests:

- parse valid `--run-rule <group-id> <rule-id>`
- reject unknown or incomplete CLI commands
- reject extra args and unknown flags
- reject IDs outside the CLI-safe grammar
- build shortcut args with exact saved IDs
- resolve a saved rule by `GroupId` and `RuleId`
- reject stale group IDs
- reject stale rule IDs
- reject a shortcut after the rule is moved to another group
- prove deleted shortcut IDs are not reused after save/restart
- preserve behavior when groups are reordered or renamed
- return explicit saved-rule dispatch outcomes for accepted, missing group, missing rule, and launch rejection
- skip autorun for shortcut-triggered cold start
- keep autorun for normal no-intent GUI startup
- route a forwarded command into the shell command receiver and then into `AppState`
- map forwarded command responses to forwarding-process exit codes
- retry briefly when the primary instance exists but IPC is not ready
- handle two simultaneous shortcut launches without producing two long-lived GUI instances

Windows-specific tests where practical:

- generated shortcut points to the expected executable
- generated shortcut stores the expected argument string
- shortcut file name sanitization handles invalid Windows filename characters
- named pipe creation does not use default security
- pipe and lock names are scoped by user/session/app identity
- lower-scope or invalid clients fail closed where this can be tested reliably

Manual smoke:

- create shortcut for a path-target rule
- run shortcut when the app is closed
- run shortcut when the app is already open
- run shortcut after renaming the group and app
- verify stale shortcut after deleting the rule shows/logs a clear error
- verify shortcut after moving a rule to another group shows/logs a clear error
- verify expected UAC behavior
- verify no duplicate long-lived GUI instance remains after forwarding
- verify normal no-intent launches keep the existing behavior
- verify shortcut-triggered cold start does not run unrelated autorun rules

## Forwarding Exit Codes

| Result | Exit code |
| --- | ---: |
| Forwarded accepted | 0 |
| Missing group | 20 |
| Missing rule | 21 |
| Launch rejected | 22 |
| Server not ready | 23 |
| Timeout | 24 |
| Protocol error | 25 |
| Auth/security failure | 26 |
| CLI parse error | 2 |
| GUI startup error | 1 |

## IPC Security Closure

| Gate | Evidence | Status |
| --- | --- | --- |
| Explicit pipe/mutex security descriptor | Source audit: `libs/os_api/src/windows/ipc.rs` builds explicit SDDL instead of default descriptors | Closed |
| Scoped endpoint identity | Source audit: endpoint includes user SID, session id, and executable path hash | Closed |
| Remote clients rejected | Source audit: pipe mode uses `PIPE_REJECT_REMOTE_CLIENTS` | Closed |
| Client session validation | Source audit: server and client verify named-pipe session ids | Closed |
| Client user validation | Source audit: server uses named-pipe impersonation and thread-token user SID comparison | Closed |
| Client-side impersonation protection | Source audit: client open uses SQOS flags with identification/effective-only behavior | Closed |
| First-instance anti-squatting | Source audit: primary mutex plus `FILE_FLAG_FIRST_PIPE_INSTANCE` | Closed |
| Strict protocol and frame size | Automated tests: `app::instance_forwarding` protocol tests | Closed |
| Hidden-window drain | Automated tests: shell forwarded-command drain while hidden | Closed |
| Lower-integrity/elevation behavior | Manual release smoke: verify shortcut/UAC behavior and auth failure probes where practical | Release smoke |
| Built manifest `uiAccess` | Artifact probe: `scripts/assert-windows-release-manifest.ps1` reads `RT_MANIFEST` from the built Windows exe and asserts `requireAdministrator` plus `uiAccess="false"` in CI and the stable release workflow | Closed |

Relevant existing verification commands:

```text
cargo test --manifest-path libs/os_api/Cargo.toml
cargo test --features windows --bin cpu-affinity-tool
cargo fmt --all -- --check
cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings
cargo build --release --features windows --bin cpu-affinity-tool
cargo test --features linux --bin cpu-affinity-tool-linux
cargo clippy --features linux --bin cpu-affinity-tool-linux -- -D warnings
cargo build --release --features linux --bin cpu-affinity-tool-linux
```

## Open Decisions

- Manual smoke details for Windows IPC security probes and exit-code capture.
- Whether future focus/window behavior ever needs a manifest privilege change; current shortcut IPC must keep `uiAccess="false"`.
- Whether the first version should support shortcuts for whole groups or only individual rules. Current recommendation: individual rules only.
- Whether minimized startup belongs in the first shortcut UX or a later iteration.
- Whether auto-close should mean "close after the primary launched process exits" or "close after all tracked PIDs for the rule exit".
- Whether Linux beta should eventually create `.desktop` launchers with equivalent saved-rule IDs.

## Suggested Discussion Reply

```text
Thanks for the suggestion. I do not want to duplicate the whole rule editor as command-line flags, because that would create a large and fragile CLI contract for paths, tracked process names, CPU groups, priority, installed apps, and launchers.

The safer direction is a shortcut for an already saved rule: configure the app once in the UI, then use a desktop shortcut that calls the tool with that saved rule ID. If the tool is already running, the new process can forward the command to the active instance and exit. If it is not running, the tool can start normally, load the saved state, and run that rule.

That keeps the GUI as the source of truth while still giving the quick desktop shortcut launch flow you described. I will track this as a staged feature: saved-rule CLI command first, desktop shortcut generation second, and minimized/background or auto-close behavior only after the lifecycle details are clear.
```
