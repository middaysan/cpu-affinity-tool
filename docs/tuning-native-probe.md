# Nonshipping canonical tuning metadata probe

`os_api` provides the opt-in `tuning-canonical-probe` example for development and
explicit Windows lab validation. Its default `preflight` only reads the current
process token. It never opens the canonical store, including when already
elevated. Explicit `lab-*` commands can initialize missing protected application
directories and lease metadata. The explicit `lab-continuity-*` commands additionally
invoke the protected registry candidate described below. They do not publish,
repair, delete or normalize journal data, change gaming settings, or enable
production tuning admission.

The example requires `tuning-test-controls`; stable release builds and uploads
do not select it. It has no UAC launcher, path override or application startup
integration. Run Cargo commands from the repository
root. Use a prepared lab context for the commands in the lab section.

The separate feature-gated `TuningStore::canonical_continuity_candidate` API is
implemented in `os_api` for explicit native validation. Only this example's
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
preflight or described as a read-only command. The native attempts below record
their exact acceptance limits. Shipping continuity
and tuning write admission remain unavailable until their native gates pass.
The production store uses handle-relative descendant operations. Only an actual
retained, relative-open ancestor can supply the borrowed capability described
below. Bootstrap and managed objects retain strict security checks; no command
repairs an existing system descriptor.

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

Known continuity refusal classes may similarly add an optional
`failure.continuity_refusal` enum. Only exact recognized `Unsupported` literals
are classified; arbitrary text and other native error classes remain omitted.
The six private candidate classes are malformed capsule, query bound, unsafe
security, unproven context, missing component and refused initialization. The
existing aggregate observation-unknown and publisher/machine source/value
refusals also have fixed classes. They preserve the original failure kind/code
and nonzero exit; they do not enable recovery or change token, machine, registry
or store admission. `missing_component` names an uncertain component/namespace
proof and is not positive absence or permission to initialize. An Unknown
observation need not supply a failure class. Optional-field absence preserves
the previous report shape; no raw diagnostic text enters a report.

The diagnostic addition passed seven pure candidate-mapping tests and four
probe tests within the 260-test OS library and 37-test example runs. Pre-fix
tests exposed six lost candidate categories and three missing report/category
results. The tests also preserve native errors, exact previous JSON shapes,
same-worker terminal cleanup, bounded output and failure exit `2`. These are
diagnostic behavior checks and supply no native continuity acceptance.

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
The first two authorized attempts below stopped before reaching the native
candidate; a subsequent attempt reached its continuity boundary and refused.

## Exact same-account process comparison

The explicit comparator observes the existing candidate in two serial owned
processes; it never initializes the marker or publishes a journal:

```powershell
cargo run --locked --manifest-path libs/os_api/Cargo.toml --features tuning-test-controls --example tuning-canonical-probe -- lab-continuity-compare
```

Each child's single worker validates its actual elevated context and canonical
store, then requires two equal observations through one ObserveExisting guard.
The child copies the exact machine tuple retained by that verified context and
the full continuity producer/version/domain/view/32-byte nonce. Its guard and
store are dropped on the worker before a joined terminal sample is sent. The
readonly machine accessor is not a fresh observation or shipping permit.

Private binary frames travel only through owned redirected child pipes, after
actual stdin/stdout pipe checks and an exact fixed handshake followed by input
EOF. Frames are capped at 4352 bytes, including at most 4096 machine bytes and
64 bytes per continuity label. No raw tuple, nonce, hash or baseline is written
to disk, public stdout or Debug. The internal sample entry is not a public lab
command and refuses console/file handles or malformed handshakes.

The parent requires complete framing, no stderr, joined-worker provenance,
actual child exit0 and both pipe EOFs before starting the next process. One
10-second deadline covers both actors. Timeout retains the same actual child,
streams and worker until completion; it never kills or starts a replacement and
remains exit2 afterward. Public reports contain only schema, command, a fixed
`equal`/`different`/`failed`/`timed_out` result, nullable machine/continuity equality
booleans and `worker_terminal`. Only equal full tuples with terminal evidence
exit0. Failed sampling supplies no positive equality or terminal-worker proof.

Eight targeted tests (including one conditional child helper) passed after
actual RED runs for parsing/framing/full-tuple comparisons and serial child
admission. They cover last-byte/source/domain/view differences, malformed and
extra frames, actual fixture process exit/EOF, and timeout retention/no second
actor. They never invoke canonical or hardware sampling. Full acceptance passed
266 OS tests, all 45 example tests, 482 Windows application tests and four OS
doctests, both Clippy gates, formatting and unchanged lockfiles. The OS Clippy
gate uses only its same five existing allowances; the shipping gate adds none.

On 2026-10-07 the reviewed, rebuilt comparator SHA256
`853B97CD2400B0F71866E31B68CD627BC1B666024D357F2974C5F969E2BB1F56`
was run through an owned elevated wrapper on Windows build `26200.9445`.
Its single bounded report returned `equal`, both equality booleans true and
`worker_terminal=true`. The public parent and actual owning wrapper exited `0`;
independent review verified the fixed report and terminal status. The private
observations and comparison retained no raw identities outside their memory
channels. This establishes full machine and continuity equality across those
two serial sample processes in the same-account live context.
Cross-account, boot/power, journal publication and shipping recovery remain
separate gates. Old commands and their JSON report shapes are unchanged.

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
here-applicable allow mask `0x116`, which intersected the then-current ancestor mutation
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

At this private-only checkpoint production storage still used its existing
implementation and ancestor policy. Adoption required a complete migration of
descendant operations, a separately
reviewed publication/recovery protocol and starting-object eligibility. A
successful temporary-fixture test does not authorize weaker ancestor checks or
close the canonical, cross-account or power-transition gates.

## Isolated publication protocol

The private `cfg(test)` publication fixture uses an exclusive lease and retained
root, old-journal and staged-file handles. It syncs the staged bytes, moves the
old journal to a fixed previous name without replacement, then moves the stage
to the journal name without replacement. This deliberately leaves a gap between
the two calls. A stage or previous artifact blocks a fresh read from reporting
Absent, including when the journal name is missing. Failed operations poison the
owner and retain remaining artifacts without rollback, retry or repair.

Before each rename, the source name must still identify the retained object and
complete bytes. Old-object cleanup requires a fresh match of the journal name
to the staged object and bytes, and deletes only the retained old handle.
These operations do not reproduce `ReplaceFileW` metadata preservation or prove
namespace durability across power loss.

Subprocess fixtures use a parent-created temporary root. A private borrowed
selector checks its fixed temporary-name boundary, actual NTFS/type and original
object ID; it cannot create a root, own its cleanup or supply canonical authority.
The parent terminates and waits for its actual child before inspecting recovery.
Synthetic pending tests retain the whole fixture context and lease until process
exit. They do not demonstrate actual native pending completion or implement a
production worker reaper.

On 2026-10-07 all 23 private publication tests passed: 14 owned native scenarios,
one conditional child helper and eight pure tests. Forced termination covers all
five replacement boundaries, two initial-publication boundaries, and before/after
clean publication. When staging or previous remains, fresh owners preserve
interrupted object IDs and full bytes and refuse both clean absence and another
publication. Separate tests cover
rename collisions, same-byte object substitution before cleanup, exact old-handle
deletion, root pinning, capacity refusal and actual lease exclusion. A regression
also verifies that rejected requests cannot downgrade unresolved retention and
release its lease. The conditional helper alone supplies no native evidence.

At this private-only checkpoint production storage, canonical admission and
shipping write policy were unchanged. Complete migration required the production
journal capacity, security, recovery and worker-lifetime contracts.

## Production relative-store contract

`store_io` supplies the production native boundary. Descendant opens, managed
directory creation, exclusive lease acquisition and metadata rechecks use one
component relative to the retained parent handle. Artifact enumeration, bounded
journal reads, staged writes/sync, both no-replace renames and exact old-handle
cleanup use those retained objects. There is no absolute descendant reopen,
pathname publication or deletion fallback. Journal capacity remains 4 MiB; the
component limit is 255 UTF-16 units. Opaque journal bytes and the public capacity
error contract are preserved.

Canonical bootstrap derives its drive from the actual OS Known Folder. It opens
the global DOS-device root and cross-checks the local root using complete final
volume-GUID root paths, full object identities and local fixed NTFS eligibility.
Drive classification uses the captured GUID root and brackets retained identity
and complete GUID-path rechecks rather than querying a mutable drive letter.
A substituted directory or different local drive mapping refuses rather than
minting canonical authority. Descendants use only the retained global handle.
This is a selection proof, not a relaxation of ancestor ACLs or evidence of
successful canonical initialization. See [the NT namespace](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file),
[local/global device names](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/local-and-global-ms-dos-device-names)
and [final handle paths](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew).
The classification follows [the drive-root query contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getdrivetypew).

Publication deliberately has a missing-journal interval between the two renames.
Readers require the same exclusive lease; concurrent contenders receive Busy.
Cold staging/previous evidence blocks clean absence and subsequent publication,
including continuity initialization. No interrupted version is automatically
selected, repaired, overwritten or removed. This reduces automatic recovery
availability and does not preserve `ReplaceFileW` physical atomicity or metadata.

The native call result is checked before output inspection. Unexpected pending
or ambiguous completion retains the complete native-used request and outer
store/opening context on that exact worker until process exit. Timeout and caller
Drop cannot manufacture completion or release/replace its lease. Synthetic
request and owned child tests exercise this policy; they do not demonstrate
actual SDK pending or cancellation completion. Normal confirmed terminal paths
still close their owned objects. The complete migration initially preserved the
strict ancestor security policy. Shipping continuity remains unavailable and
tuning write admission remains closed.

On 2026-10-07 the complete migration passed 13 isolated store tests (12 actual
native scenarios and one conditional child helper) and eight production-used
decision/request-ownership tests within the full 243-test OS library run. The
33 probe-example tests also passed. Actual tests cover all five replacement
boundaries, exact 4 MiB limits, source/destination substitution, exact old-handle
cleanup, late read-name/orphan changes and store/partial-opening lease retention.
The latter outcomes are injected nonterminal fixtures, not observed SDK pending.
Canonical bootstrap remains source-reviewed and awaits its actual scope matrix;
the strict ancestor mutation-grant predicate at that checkpoint still refused the previously
observed ProgramData permissions. No ACL repair or gaming-setting write is added.

## Capability-bound ancestor compatibility

The subsequent compatibility change binds the ancestor policy to a borrowed,
nontransferable capability over the actual relative-open node. It requires a
retained directory, ancestor scope, the existing read-only access plan and
sharing without delete. Bootstrap, managed directories, files, renameable or
unscoped nodes cannot supply it. Identity, type and attributes are checked before
and after the complete owner/DACL validation. Descendant traversal continues
through that retained handle with no reparse following or path fallback.

Only this capability permits the additional `0x112` rights: directory add-file /
write-data, write-EA and write-attributes. Existing read, add-subdirectory and
standard-read rights retain their policy. Ordinary delete-child, delete,
write-DACL, write-owner and remaining broad raw generic rights still refuse.
Bootstrap and managed security remain strict. All observed encrypted objects
refuse before metadata writes; the tests do not invoke EFS or change encryption.

Windows can map a requested generic right into concrete file rights before the
DACL is observed. An actual owned fixture requested `GENERIC_WRITE` and observed
`0x00120116`; the strict and managed checks refuse it, while the eligible relative
ancestor capability accepts that concrete mask. Raw generic bits remain refused.
The policy evaluates observed rights and cannot recover the original request
spelling. See [generic access rights](https://learn.microsoft.com/en-us/windows/win32/secauthz/generic-access-rights),
[file-right mappings](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights)
and [MapGenericMask](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-mapgenericmask).

On 2026-10-07 the focused production boundary suite passed 17 tests within a
253-test OS library run; the 33 probe-example tests also passed. Genuine pre-fix
failures covered encrypted directory/file admission and the actual relative
`0x116` fixture. A separate test-assumption correction records the native generic
mapping above; it is characterization evidence, not a new production exception.

Two owned native race fixtures intervene after the final parent check and before
the real child-open call. The empty ancestor accepts an actual junction tag; the
test requires either refusal or traversal confined to the exact original objects,
with the external owned sentinel unchanged. The nonempty
ancestor's actual tag installation returns directory-not-empty (`145`), and
original-handle traversal remains intact. Both fixtures compare full sentinel
bytes and identities before cleanup, remove only an authenticated matching tag,
and clean only captured owned objects. Attribute rights are therefore treated as
available despite sharing restrictions. These fixtures provide no canonical,
cross-account, boot or power-transition acceptance.

## Native canonical open after compatibility

On 2026-10-07 the already authorized UAC initialization was repeated with the
reviewed `2dab14e` compatibility source on Windows `10.0.26200.9445`. The built
example SHA256 was
`CE922A84E5A85BE9DBC0B279B98F44929999301E362CF7A25BA44BC957B32217`.
Its terminal record reported an elevated/High token, `authority=canonical`,
`journal.kind=absent` and `store_attempted=true`. Thus the actual canonical
opening, lease validation and absent-journal read succeeded in this context.

The same invocation then reported `stage=continuity`, `outcome=native_failure`,
`failure.kind=unsupported`, no native code and `worker_terminal=true`. Both the
probe and its retained elevated wrapper exited `2`; the wrapper skipped
observation. No replacement process followed the earlier observation wait.
This is a successful store-access slice and a failed continuity attempt; it
does not establish continuity initialization, HKLM progress, cross-account or
power behavior. The broad failure category does not identify its exact cause.
Shipping continuity and tuning write admission remain unavailable.

## Fixed continuity refusal after canonical open

A second authorized UAC attempt on 2026-10-07 used reviewed source `c524203`
and example SHA256
`39FC00E15A4638B58B2A95DD39C6302583EF1709C7E9A0E2458B3F40CDB27AAF`.
It again reported elevated/High, canonical authority and an absent journal, then
failed at continuity with `failure.kind=unsupported` and the fixed optional
`continuity_refusal=unsafe_security`. The worker and retained elevated wrapper
both terminated with exit `2`; observation was skipped.

An independent read-only native64 SOFTWARE owner/DACL inspection found an
Administrators owner and six ordinary allow ACEs, all with container-inherit
flag `2`. The first full-access ACE used the exact well-known Creator Owner
placeholder; the other full-access ACEs were SYSTEM and Administrators, with
three read-only ACEs. No inherit-only ACE was observed. This inspection did not
read the integrity label or identify the precise production failed handle.
It therefore supplies a concrete parser/policy investigation, rather than proof
of full continuity initialization or an unsafe ordinary-user grant. Microsoft
describes [Creator Owner as an inheritance placeholder](https://learn.microsoft.com/en-us/windows/win32/secauthz/well-known-sids),
distinct from the current-owner `OWNER_RIGHTS` identity. No system ACL repair
or gaming-setting change was performed.

## Exact registry inheritance placeholder

The registry parser now distinguishes exact Creator Owner (`S-1-3-0`) from
ordinary principals. Only its observed plain allow ACE with raw `KEY_ALL` and
exact container-inherit flag `2` is accepted on the already trusted-owner
SOFTWARE ancestor. It does not supply owner trust, SYSTEM/Administrators grants
or any managed-key permission. Creator Group, Owner Rights, SID aliases, other
flags or masks and ordinary mutation grants retain refusal. No general
inherit-only exception is added.

New managed keys still request and validate an actual protected DACL, exact
SYSTEM/Administrators full access and High/no-write-up label. Protection excludes
parent inheritance; existing keys are independently inspected rather than
repaired. See [new-object DACL construction](https://learn.microsoft.com/en-us/windows/win32/secauthz/dacl-for-a-new-object).

The regression tests first exposed the old rejection of the observed shape
and the loss of its distinct semantics as an ordinary read-only principal.
Two actual Windows API characterizations use only descriptors in memory:
`AccessCheck` denies four concrete mutation rights for the Creator Owner
placeholder while an identical Everyone positive control grants them, and
`CreatePrivateObjectSecurity` excludes the parent placeholder from an explicitly
protected child DACL. No descriptor is applied to a registry key, file or token.
The child test uses token-default owner/group and makes no canonical-owner or
integrity-label claim. These tests establish the scoped semantic correction;
they do not prove canonical continuity or complete the native scope matrix.

On 2026-10-07 all six focused tests, the full 266-test OS library suite,
37 probe-example tests, 482 Windows application tests and four OS doctests
passed. The OS all-target Clippy check uses its same five previously documented
allowances; the shipping application Clippy check adds none. The corrected
source at that checkpoint still awaited a new canonical UAC attempt.

## Successful canonical candidate after the correction

On 2026-10-07 the previously authorized pair was run using reviewed source
`aca9a86` and rebuilt example SHA256
`9E4C90DF60BC91EC528292C26C19114A7D43CCB63B9FBC3EBE586B67D4AA0058`.
Both initialization and observation reported elevated/High, canonical authority,
an absent journal, `continuity_stable`, no failure and a terminal worker.
Both exited `0`; the retained elevated wrapper also completed with exit `0`.
Direct review confirmed one bounded record per invocation and no pending probe.

This establishes protected metadata access, successful candidate initialization
and two equal full observations within each invocation. The output contains no
raw identity data and does not compare the identities between those separate
processes. Canonical journal publication, cross-account and power-transition
behavior remain separate acceptance gates.
Shipping continuity and tuning write admission remain closed.

## Same-account lease and moved-executable check

On 2026-10-07 the same reviewed build passed a four-process elevated/High lab
sequence. The original `lab-hold` reported canonical authority and an absent
journal before READY. Both the original `lab-contend` and a byte-identical,
hash-pinned copy launched from a different executable and working directory
reported `busy` and exited `0`. After the exact RELEASE frame, the holder
reported Completed and actually exited `0`. The copied executable's `lab-open`
then acquired canonical authority, read an absent journal and exited `0`.

The owning wrapper completed with exit `0`, no timeout, and all four children
terminal. It retained their actual handles through exit and EOF; the executable
copy remains in its owned build-target directory. Fixed bounded reports contain
no raw identities. This confirms the shared canonical location and independent
lease across those same-account processes and executable locations. The sequence
does not call the continuity candidate, publish a journal or certify exact
cross-process identities, other accounts, power transitions or recovery.

## Isolated retained-process recovery checks

The Windows `native_recovery_tests` fixture now adds two cases at the existing
verified-progress interruption boundary. A separate owned process survives forced
writer exit. The writer records that actor's actual PID, full creation time, image
and token SID/session/authentication LUID in the existing journal binding. These
values stay in owned temporary metadata and RAM, with fixed protocol markers only.
Machine, boot and logical rule/profile ownership remain explicit fixture domains.

The new recovery worker hydrates the original scalar baseline 7, exactly checks
the recorded native tuple and opens a lifetime guard while the actor lives. It
retains that same worker-local guard and isolated lease through caller shutdown
timeout. Reattachment leaves the physical target at 12 with zero target writes
and no baseline recapture. After the parent ends only its owned actor, positive
exit from that **same guard's poll** permits fixture-owner release, verified
restoration to 7 and durable release. An Unknown poll during the exit/query race
permits no write: the worker retains the same guard until its own positive signal
or the existing bounded deadline. It never reopens or substitutes the process.

In the second case the actor ends before guard acquisition. Actual failed native
verification remains Unknown; recovery retains the exact journal bytes, baseline
7 and physical target 12, without release, retirement or restoration. No parent
`Child::wait` boolean supplies lifetime evidence to the resolver.

Run the bounded fixture module with:

```powershell
cargo test --locked --features 'windows tuning-test-controls' --bin cpu-affinity-tool native_recovery_tests -- --test-threads=1
```

The seven tests include the original three forced boundaries, these two native
process cases, one pure resolver regression and the conditional child helper.
The helper's default no-op is not native evidence. This module uses only owned
isolated temporary scalar/metadata files and exact owned child handles. It does
not publish canonically, prove native boot/logon/game attachment, install a
shipping resolver or close the remaining S36 native scope gates. The existing CI
script's three required named recovery tests are unchanged.

## Disposable-VM canonical publication helper

The application test `canonical_publication_lab` is compiled only for Windows
with `tuning-test-controls` and is explicitly ignored. Its source and isolated
fixtures prepare a separate publication acceptance packet; canonical execution
has not been performed. Select a fresh disposable Windows VM snapshot before
running it. Do not run it against this PC's existing canonical metadata or recovery
journal. The selection marker below is an operator assertion, not hypervisor
attestation. A usable native TPM/UEFI machine source is also required; an
unsupported VM stops without a fixture identity fallback.

The helper requires actual native `Created` dispositions for both managed
directories and the lease, canonical authority and a first `Absent` read.
Unknown, present (including empty or corrupt), nonfresh or noncanonical input
cannot reach journal publication. Opening can create protected directory/lease
metadata before a later refusal; the helper retains that metadata and never
repairs or deletes it. A previous probe run in the same snapshot therefore
disqualifies that snapshot for this helper.

Only in the selected VM, from an elevated PowerShell at this reviewed repository
checkpoint, invoke the exact ignored test:

```powershell
$env:CPU_AFFINITY_PUBLICATION_LAB_SNAPSHOT = 'fresh-disposable-windows-snapshot'
$env:CPU_AFFINITY_PUBLICATION_LAB_CASE = 'success'
cargo test --locked --features 'windows tuning-test-controls' --bin cpu-affinity-tool app::features::tuning::platform::publication_lab_tests::canonical_publication_lab -- --ignored --exact --nocapture --test-threads=1
```

The finite case may instead be one of `fault-stage`, `fault-old-move`,
`fault-publish`, `fault-before-cleanup` or `fault-after-cleanup`. Choose one case
per fresh snapshot; prepare/reset the disposable VM outside this helper before
another case. There is no path override or cleanup command.

The worker creates two valid current-format **empty** `JournalDocument` records
(revisions 1 and 2) from the actual native machine and default coordinator
snapshot. There are no modules, owners, resources or setting writes. Full native
machine source/bytes are rechecked before each publication. The production store
syncs and verifies staging, then uses two verified handle-relative no-replace
renames for replacement. The read-only lab evidence checks fixed artifact
security, complete bytes and opaque native IDs, including the actual synced-stage
ID becoming the journal ID. It also requires Busy while the original lease lives.

Success compares the complete reopened record and native IDs. A selected injected
fault must retain the exact phase artifacts and poison the original writer.
Their bytes/IDs are verified **before** original-owner Drop. Incomplete artifact
cases then require the actual fresh constructor's refusal; they do not claim
post-Drop artifact reinspection. The after-cleanup fault may reopen the fully
verified new record. No automatic restoration or deletion follows either case.

The helper emits only fixed JSON fields `schema`, `result`, `worker_terminal` and
`verified`. Successful selected cases report `verified` or `fault_retained`,
`verified=true`, a terminal worker and process exit 0. Refusal, worker failure,
missing report or any timeout is unsuccessful. A ten-second timeout first reports
`pending` with `worker_terminal=false`; it retains and waits for the **same** worker
and lease, then remains unsuccessful even after late completion. Wait for actual
terminal completion; do not replace the worker or treat a timeout as release.
Raw machine data, journal bytes, IDs, paths and arbitrary native error text are
not emitted by the helper. Normal GUI composition and shipping write admission
remain unchanged; ordinary tests and CI leave this canonical test ignored.

## Existing-metadata host publication helper

The separate Windows+`tuning-test-controls` test
`host_canonical_publication_lab` is explicitly ignored. The user-authorized
same-account host lab passed success and all five faults with verified rollback
on 2026-10-08, as recorded below. The disposable-VM helper above is unchanged.

This route uses **existing-only canonical opening and final reopening** through
the same production Known Folder, security and lease pipeline. It never creates
missing directories or the lease. Both managed directories and lease must have
actual `Opened` provenance; journal, staging and previous must all be absent.
Present, corrupt, empty-present, unknown, unsafe or missing metadata refuses
before publication. There is no caller path override or fixture-identity fallback.

Only after source review and acceptance gates, select the exact ignored test in
an elevated shell at the reviewed checkpoint:

```powershell
$env:CPU_AFFINITY_PUBLICATION_HOST_LAB = 'restore-verified-absent-only'
$env:CPU_AFFINITY_PUBLICATION_HOST_LAB_CASE = 'success'
cargo test --locked --features 'windows tuning-test-controls' --bin cpu-affinity-tool app::features::tuning::platform::publication_lab_tests::host_canonical_publication_lab -- --ignored --exact --nocapture --test-threads=1
```

Start with success. Only after its actual terminal report/exit is accepted, run
the five finite cases `fault-stage`, `fault-old-move`, `fault-publish`,
`fault-before-cleanup` and `fault-after-cleanup` sequentially. Stop after any
failure, refusal or timeout; do not launch a replacement or attempt manual repair.
The host test writes only two valid empty machine-bound records, revisions 1/2,
using the existing production publication protocol and native machine re-query.
It applies no game settings or recovery modules.

A private capability in the actual worker-owned store retains each verified
newly created publication handle **before** the first write or sync, together
with its full bytes and confirmed name transitions. Snapshots and caller IDs
cannot mint this capability. Every controlled terminal outcome attempts its
single-use rollback. The complete set, names, IDs, bytes, security and original
root/lease authority are validated before any deletion, then revalidated for
each exact-handle delete. Original directories, lease and ACLs stay untouched.
After artifact handles close, the worker verifies the lease-only namespace,
drops the original store and reopens existing canonical metadata to prove Absent
and the same root/lease IDs. Poison from a fault remains sticky on the old store.

The host report starts on a fresh line and contains exactly `schema`, `result`,
`worker_terminal`, `verified` and `rollback_verified`. Accepted terminal results
are `verified_rolled_back` or `fault_rolled_back`, both proof booleans true and
process exit 0. Other finite results are `pending`, `refused`, `failed`,
`timed_out`, `worker_failed` and `rollback_unresolved`; none passes acceptance.
No raw identity, paths, journal bytes or arbitrary error text is reported.

Rollback claims cover the controlled terminal cases, **not power loss or forced
exit**. Failed verification before delivery of a safe newly created native Node
cannot provide a cleanup receipt. Unsealed or unproven partial bytes, substitution or unknown
entries cause nondestructive refusal. Unknown native completion retains the same
worker, request buffers, capability and lease; no cleanup is guessed. A ten-second
timeout reports pending and retains that worker until actual terminal completion;
late verified rollback still leaves the run unsuccessful.

## Accepted controlled host publication — 2026-10-08

The independently reviewed source checkpoint `7a7f9b4` was exercised on the
current Windows PC under explicitly authorized same-account elevation. The
test EXE SHA256 was
`1F01887924D20B5AFEA805C982BEF1AAE4223F03990C9B846682A89F22F75B30`
(16,805,376 bytes). Success ran first; the five fault cases then ran serially
under one elevated controller. Every next case required the previous actual
wrapper exit 0 and its strict accepted rollback report.

| Case | Run tag | Accepted result |
| --- | --- | --- |
| `success` | `fc584802bbc24256b810ba23c7cf178a` | `verified_rolled_back` |
| `fault-stage` | `cfc5d8399a6149438dbd58b1b690ef16` | `fault_rolled_back` |
| `fault-old-move` | `59cbc54c71ef4de7bd0caedda79f7823` | `fault_rolled_back` |
| `fault-publish` | `8d9659348e8b4686b75b56c8f97347aa` | `fault_rolled_back` |
| `fault-before-cleanup` | `08e78ee075964f9088240298cd0ff55d` | `fault_rolled_back` |
| `fault-after-cleanup` | `bc92bba7282548feaa5bb1163666d346` | `fault_rolled_back` |

All six reports contained `worker_terminal`, `verified` and
`rollback_verified` equal to true; all one-case wrappers and both original
launchers/controllers actually terminated with exit 0. Each same native worker
proved its exact artifact rollback, followed by an existing-only canonical
reopen with Absent and the original root/lease IDs. Fixed sanitized reports
remain under ignored `target/host-publication-lab`; the accepted EXE and bounded
manifest are retained under `target/host-publication-accepted/7a7f9b4`.

This accepts controlled canonical publication and rollback on the tested
same-account scope. It does not demonstrate canonical recovery of a setting,
forced host-process interruption, power-loss durability, boot/logon/game
attachment or another account. Original root/lease identities and ACLs were
preserved; gaming settings were unchanged. Shipping writer admission remains closed.

## Remaining acceptance gates

The probe supplies a concrete way to inspect canonical metadata and lease
contention. Successful candidate invocations alone do not certify canonical
publication; the separate six-case host packet above supplies its bounded
publication/rollback evidence. Filtered-token object access, full machine and
continuity equality across elevated accounts, power transitions and authoritative
game reattachment remain separate gates.
The comparator covers only the stated same-account serial processes, and the
moved-executable check covers only its stated lease/location sequence.
Those require their separate native matrix. No probe result alone completes the
recovery foundation or admits a gaming-setting writer.

See [the modular specification](modular-gaming-features-plan.md),
[the module contribution contract](../src/app/features/tuning/README.md) and
[the release smoke matrix](release-smoke-matrix.md).
