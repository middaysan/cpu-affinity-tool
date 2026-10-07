# Nonshipping canonical tuning metadata probe

`os_api` provides the opt-in `tuning-canonical-probe` example for development and
explicit Windows lab validation. Its default `preflight` only reads the current
process token. It never opens the canonical store, including when already
elevated. Explicit `lab-*` commands can initialize missing protected application
directories and lease metadata. The two `lab-continuity-*` commands additionally
invoke the protected registry candidate described below. They do not publish,
repair, delete or normalize journal data, change gaming settings, or enable
production tuning admission.

The example requires `tuning-test-controls`; stable release builds and uploads
do not select it. It has no UAC launcher, path override or application startup
integration. Run Cargo commands from the repository
root. Use a prepared lab context for the commands in the lab section.

The separate feature-gated `TuningStore::canonical_continuity_candidate` API is
implemented in `os_api` for explicit native validation. Only this example's two
explicit `lab-continuity-*` commands call it. It borrows the real canonical store/lease
and retains actual token provenance; each guarded operation reobserves exact
machine and token context. Private operation-trace and owned-hive tests do not
certify the canonical registry namespace or elevated access policy.

The candidate uses only the fixed native64 application metadata chain. Existing
components are validated without repair. Initialization requires an actually
absent canonical journal and permits one capsule write only to an actually new
marker. Observation includes a bounded child-creation probe for volatility;
unexpected metadata is retained and closes that guard's observation admission.
This is an explicit metadata operation and must not be added to default token
preflight or described as a read-only command. No actual canonical candidate
execution is certified by this implementation checkpoint. Shipping continuity
and tuning write admission remain unavailable until their native gates pass.
The path-based store refuses ordinary ancestor grants capable of namespace or
security mutation, including `FILE_WRITE_DATA` (the directory `FILE_ADD_FILE`
bit) and `FILE_WRITE_ATTRIBUTES`. Read and add-subdirectory grants retain their
existing policy. The broader handle-relative compatibility work remains separate;
this stricter check never repairs an existing system descriptor.

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
For recognized store refusal predicates, `failure.store_refusal` adds an optional
fixed enum such as `mutation_grant_untrusted`. Only exact allowlisted internal
reasons produce it; unknown text is omitted. The enum identifies the predicate,
not the failing object, repair instructions or recovery authority. All other
failure fields and exit rules remain unchanged.

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

## Explicit continuity candidate checks

These operations require a prepared elevated Windows lab context. Only Windows
x86_64 with 64-bit pointers is admitted; the new commands reject other builds
before token or store access. The actual candidate independently validates the
retained lease, exact primary-token provenance, known High/System integrity and
the full supported machine identity. A standard or unknown token stops before
opening the store. Existing preflight/open/hold/contend commands never invoke it.

To observe an existing candidate:

```powershell
cargo run --locked --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls --example tuning-canonical-probe -- lab-continuity-observe
```

It opens the fixed native64 `HKLM\SOFTWARE\CpuAffinityTool\Tuning\ContinuityV1`
chain without creating missing components. Each observation can request one
bounded child-creation probe after authenticating the existing metadata. A
missing, denied, malformed, redirected or unverified component fails without
repair. Unexpected probe artifacts remain present and close further observation
admission for that guard; the command never deletes them.

Initial creation is separately selected:

```powershell
cargo run --locked --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls --example tuning-canonical-probe -- lab-continuity-initialize
```

Both the command and actual factory require an absent canonical journal. Any
Present journal, including zero bytes, blocks initialization. Missing app-owned
registry components may be created with the fixed protected descriptor; SOFTWARE
is existing-only. Only an actually new marker receives its capsule once. Existing
incomplete or unsafe metadata is retained without overwrite or normalization.

Both commands compare two observations through one retained guard and then drop
the guard and store on the same worker before its terminal report. Reports may
include an optional fixed `continuity` enum; old-command reports omit that field.
`continuity_stable` with `continuity=stable` is successful only with a joined,
terminal worker, complete report output and no controller timeout. Unknown,
Changed, blocked initialization, unsupported build and native failures exit
nonzero. They contain no nonce, hardware ID, SID, authentication ID or key path.

Stable certifies that same-process comparison only. It does not establish
cross-account equality, power-transition behavior, an OS boot UUID, publication
or game recovery. Automated command tests use fakes and shape values, never the
native candidate. Neither command runs implicitly in tests, CI or normal launch.
The initial authorized attempt below stopped before reaching the native candidate.

## Initial native lab evidence

On 2026-10-07, an explicitly authorized UAC launch of the built example on
Windows build `10.0.26200.9445` attempted `lab-continuity-initialize`. It reported
an elevated/High token, then failed at `stage=open` with `native_failure`,
`store_attempted=true`, `authority=not_opened`, `journal=not_read`,
`failure.kind=unsafe` and `worker_terminal=true`; both the probe and capture
wrapper exited `2`. The wrapper skipped `lab-continuity-observe` after this
failure. Continuity, hardware and HKLM producer operations were not reached.

That first artifact predates the optional refusal enum, so its report does not
identify the rejected object or predicate. After the diagnostic-only change passed
33 example tests and independent review, a second authorized initialization
attempt returned the same terminal failure and exit `2`, now with
`failure.store_refusal=mutation_grant_untrusted`. Observation was again skipped;
the diagnostic result identifies the refusal predicate, not its object location.
The store/security policy and continuity implementation were unchanged.
Subsequent fixed-path read-only inspection found no managed application
directories or lease. ProgramData had an ordinary
here-applicable allow mask `0x116`, which intersects the current ancestor mutation
predicate. This is a grounded candidate cause; it does not justify modifying the
system ACL or weakening the store policy. Metadata-attribute access is not barred
by file sharing flags, and reparse setup can admit `FILE_WRITE_ATTRIBUTES` subject
to additional checks. See [CreateFileW sharing semantics](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
and [the file-system reparse operation](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4aeefef8-92c3-4abc-af7a-a610caf8a165).

These failed attempts establish neither canonical admission nor continuity.
Shipping continuity and tuning writers remain unavailable. Further source and
native validation must preserve refusal evidence and address namespace safety.

## Isolated relative-operation characterization

The private `cfg(test)` `relative_io` module characterizes operations rooted in
an actual retained directory handle. Its fixtures own their temporary NTFS
objects; they do not open ProgramData, invoke the canonical candidate, require
UAC, or supply production authority. Fixed component names, native dispositions,
bounded enumeration, exact source-handle deletion and no-replace rename are
tested alongside namespace replacement, junctions, hard links and a conflicting
child-process current directory. Setup failures remain distinct from missing or
incorrect operation behavior.

On 2026-10-07 the 28 module tests passed on the local Windows build: 16 owned
native scenarios, one child helper and 11 pure/regression tests. Nested creation
worked with the original directory access mask. The documented Win32 relative
rename returned error `87` even after correcting the supplied buffer extent;
the fixed no-replace implementation uses the generated `NtSetInformationFile`
binding and `FILE_RENAME_INFORMATION` instead, with no pathname fallback. See
[the native setter contract](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile).

Unexpected pending output retention is covered by a pure ownership trace; no
native asynchronous completion was observed. The private primitive retains its
heap output, and the rename input, without inspection on `STATUS_PENDING`, then
refuses the operation. Any delivered handle remains for process-exit cleanup;
this is not production shutdown or cancellation evidence.

Production storage still uses its existing implementation and ancestor policy.
Adoption requires a complete migration of descendant operations, a separately
reviewed publication/recovery protocol and starting-object eligibility. A
successful temporary-fixture test does not authorize weaker ancestor checks or
close the canonical, cross-account or power-transition gates.

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
