# Nonshipping canonical tuning metadata probe

`os_api` provides the opt-in `tuning-canonical-probe` example for development and
explicit Windows lab validation. Its default `preflight` only reads the current
process token. It never opens the canonical store, including when already
elevated. Explicit `lab-*` commands can initialize missing protected application
directories and lease metadata. They do not publish, repair, delete or normalize
journal data, change gaming settings, or enable production tuning admission.

The example requires `tuning-test-controls`; stable release builds and uploads
do not select it. It has no UAC launcher, path override, registry continuity
writer or application startup integration. Run Cargo commands from the repository
root. Use a prepared lab context for the commands in the lab section.

## Automated checks and safe preflight

```powershell
cargo test --locked --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls --example tuning-canonical-probe
cargo run --locked --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls --example tuning-canonical-probe -- preflight
```

No arguments also select preflight. Tests exercise the command decisions and
worker lifetime with fakes; they never run canonical lab operations. Preflight
reads actual token elevation and mandatory integrity classification and returns
one bounded NDJSON record:

| Outcome | Meaning |
| --- | --- |
| `read_only_standard` | Observed standard token; the opener was skipped |
| `skipped_elevated` | Observed elevated token; the opener was still skipped |
| `unknown_token` | Token context could not be established; no store operation |
| `unsupported_platform` | Windows native probe is unavailable on this platform |

`store_attempted` is false for all preflight outcomes. Standard/elevated preflight
success is a dispatch check, not canonical ACL acceptance or proof of denied
access to an existing protected object. Unknown or unsupported results exit
nonzero and must not be recorded as an acceptance pass.

Reports contain fixed enums, booleans, lengths and optional native error codes.
They contain no raw machine identity, SID, authentication ID, executable path,
store path, journal bytes or arbitrary native error text. `present_uninterpreted`
means only that a bounded byte buffer was read; it grants no recovery authority.

## Explicit elevated lab checks

These commands require an actual elevated token with recognized High or System
integrity. An unknown token, unknown integrity or standard token is rejected
before opening the store. The real canonical opener performs its own authority
checks. It uses the Known Folder path and accepts no caller-selected replacement.

Record the probe source revision, Windows build and UBR, token/account/session
context and results. Keep any account-specific information in the private lab
record. Do not put credentials or native identities in command arguments.

```powershell
cargo run --locked --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls --example tuning-canonical-probe -- lab-open
```

`lab-open` acquires the canonical lease, validates authority, reads journal
presence once and releases on its worker. A successful terminal record has
`worker_terminal=true`. Existing bytes remain uninterpreted; an oversized or
unreadable journal is a failed observation, with no cleanup fallback.

For contention, start the holder in one prepared elevated lab terminal:

```powershell
cargo run --locked --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls --example tuning-canonical-probe -- lab-hold
```

After its `ready` record, run the contender from another prepared terminal:

```powershell
cargo run --locked --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls --example tuning-canonical-probe -- lab-contend
```

The expected contender outcome is `busy`. An unexpected successful acquisition
fails the contention check and releases that contender's own store. Enter the
exact line `RELEASE` in the holder within its 30-second control window; LF or
CRLF completes the bounded frame. Wait for the holder's terminal record before
repeating `lab-open`. EOF, malformed control or the control deadline closes the
idle holder and reports an incomplete check.

One worker constructs, uses and drops the store. READY follows canonical
validation and the bounded read. If a native operation exceeds the controller's
deadline, a `timeout` report has `worker_terminal=false`; the same probe process
and worker remain alive until that worker finishes or the lab operator terminates
that process. The invocation remains incomplete and exits nonzero even if the
retained worker later finishes successfully. An incomplete report stream also
prevents a successful exit. No replacement worker or claimed lease release follows a timeout.
Any later automated controller must terminate and wait only its own Child handle.

## Remaining acceptance gates

The probe supplies a concrete way to inspect canonical metadata and lease
contention. It does not certify canonical publication, filtered-token object
access, exact machine equality across elevated accounts, moved-executable scope,
protected HKLM continuity, power transitions, or authoritative game reattachment.
Those require their separate native matrix. No probe result alone completes the
recovery foundation or admits a gaming-setting writer.

See [the modular specification](modular-gaming-features-plan.md),
[the module contribution contract](../src/app/features/tuning/README.md) and
[the release smoke matrix](release-smoke-matrix.md).
