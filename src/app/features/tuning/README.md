# Tuning module contribution contract

The dormant platform supplies typed contracts, a catalog, a resource coordinator,
a bounded serial runner, an independent configured-intent consumer, a bounded
recovery journal and scoped durable recovery. The production catalog is empty;
the dormant native factory supplies Unavailable configured input, so write
admission remains closed. The Windows boundary supplies store/lease and identity
primitives. The two backend shapes and
cross-adapter simulators under `testing/` compile only with `cfg(test)`.
Isolated forced-process recovery is demonstrated at three durable boundaries.
Controlled canonical publication/rollback and same-worker rehydration have
explicit same-account elevated lab evidence. Authoritative game/user/boot scope,
runtime app-state integration and user controls remain staged work. Schema v11
already stores optional bounded configuration without starting the runner.
The Windows OS boundary supplies a retained configuration-source API and sealed
receipts for exact loaded/published bytes. Storage derives nonserialized proof
from those same bounded v11 bytes; a draft cannot supply saved authority. Journal
FORMAT2 matches the independently expected original source before hydration or
owner matching; legacy or foreign evidence remains blocked and retained. Worker
source guards are scoped to bounded operations. The live configured producer is
still Unavailable; provenance supplies no game/logon/boot authority.
Positively qualified state absence is published only inside the retained initial
load call, with bounded defaults, no backup and checked no-replace against original
absence. The actual resulting base receipt is retained even when tuning is Off.
Later qualified saves use their original receipt, never a freshly adopted leaf;
uncertain or changed evidence blocks saves without ordinary fallback. An originally
unavailable source remains base-only even if a later save can open its directory.
The dormant saved-input builder accepts sealed proof and derives immutable tuples
from its exact bytes. Envelope v2 separates a same-owner old continuation from
the new requested value; explicit withdrawals alone supply Disabled. Drafts,
missing records and journal ownership do not supply intent. Dormant headless
transactions and matching recovery use this producer; application composition
and native mutation admission remain separate requirements.
Passing isolated test objects
does not admit a writable module.

The canonical specification is
[`docs/modular-gaming-features-plan.md`](../../../../docs/modular-gaming-features-plan.md).
The Windows recommendations and exclusions are documented in
[`docs/windows11-gaming-module-research.md`](../../../../docs/windows11-gaming-module-research.md).

## Structure

- `mod.rs` is the public feature facade and future shipping catalog composition.
- `config.rs` owns profile/association/allocation and exact saved-intent persistence
  shapes. An absent envelope is Off. Exact raw forward data remains read-only;
  malformed/oversized evidence refuses saves. Known structural data still requires
  its module codec/catalog and an independently qualified source before execution.
- `configured_source.rs` builds immutable input from saved sealed proof. Its
  ordinary saved-file fixtures supply explicit test origin and no native proof;
  production runner composition remains Unavailable.
- `platform/contract.rs` contains validated IDs, targets, setting resources,
  capability and status values, exact canonical tokens, and typed plans.
- `platform/catalog.rs` contains `Module`, checked registration and preparation,
  and the opaque executor that retains its concrete module and typed state.
- `platform/codec.rs` is the bounded, versioned serialization boundary.
- `platform/coordinator.rs` owns atomic claims, equal-request sharing, conflicts,
  first baselines, step verification, compensation and last-owner restoration.
- `platform/runner.rs` owns bounded serial dispatch, generations, cancellation,
  terminal tickets, immutable snapshots and bounded shutdown.
- `service.rs` owns dormant headless saved transactions over candidate state and
  logical identities, exact withdrawals and terminal acknowledgement bookkeeping.
- `platform/journal.rs` owns aggregate limits, deduplicated typed source envelopes,
  exact unknown input retention and durable intent/progress/ownership transitions.
- `platform/recovery.rs` distinguishes verified attachment, positive expiry and
  Unknown for each actual resource lifetime.
- `platform/configured_intent.rs` consumes immutable complete current intent
  independently of the journal; F4 supplies the saved-config/session producer.
- `platform/writer.rs` creates worker-local storage authority, seals native scope
  to canonical opening and maps bounded native errors. Its factory is dormant.
- `platform/writer_scope.rs` applies durable bootstrap recovery and checks exact
  configured and resource evidence throughout commands and publication.
- `testing/scalar.rs` demonstrates a scalar machine setting.
- `testing/user_setting.rs` demonstrates two native fields, exact absent/present
  values with original type and raw bytes, partial application, subset restoration,
  and a restart-required stored effect.
- `testing/value_setting.rs` uses the user backend through a separate typed
  adapter to exercise sharing and edits across modules with different snapshots.

The root `allow(dead_code)` is limited to this dormant staged foundation. Remove
or narrow it when F4 integrates the public facade; do not add per-module lint
exceptions to hide unused integrations.

Configuration uses a separately versioned envelope bounded to 4 MiB, depth 32
and 65536 nodes, including raw metadata. Known records cap profiles/modules per
profile at 64, associations at 256 and activation/withdrawal tuples together at
256 in v1. V2 caps activation/withdrawal/transition owner records together at 256;
both transition tuples count toward aggregate bytes/nodes. A transition retains
the same owner/profile/actor/target/mode and advances configuration generation.
Old continuation never authorizes old Acquire; only the new tuple does. Exact
active/new preferences and game associations are required as saved shape, without
establishing native game eligibility. Unsupported execution-relevant codecs make
the producer unavailable with records retained; unused preferences do not block it.
Allocation counters exceed every retained used ID/generation. Unknown
structural fields/versions preserve original JSON through unrelated state saves;
understood metadata remains validated before an Unknown result, and duplicate
JSON names are blocked before conversion can discard evidence. Whole-state reads
and serialized saves have a 16 MiB cap; output admission precedes every backup,
directory creation, stage write and publication. Raw tuning evidence is guarded
even when an unrelated base field fails decoding.
Module payload versions belong to the registered codecs, independently of the
platform envelope version. File parsing does not authenticate an account, source
directory, saved producer or journal origin. Receipt qualification and origin
binding are present; durable transactions and recovery composition remain later
F4 work, and production input stays Unavailable.

## Adding a module

1. Write a failing contract or adapter test before implementing behavior. Include
   unavailable and denied contexts, no-op preparation, original absence, partial
   native failure, external drift, desired-value updates, and exact restoration.
2. Add one bounded feature directory with concrete `Config`, `Observation`,
   `Snapshot`, and `Step` types. Keep `serde_json::Value` inside codecs only.
   Configuration defaults to unmanaged at the composition boundary.
3. Implement `Module` and its codecs. Declare an immutable `FeatureId`, nonzero
   config/snapshot versions, supported targets, OS-setting resource fields, and
   activation modes. Read-only descriptors have no mutation resources or modes.
   Restart-required settings support only manual activation in this contract.
   Each resource declares its lifetime requirement, defaulting to `Unsupported`.
   Use `PersistentMachine` only for a resource whose native scope and persistence
   are independent of caller user/logon context. A Machine target projection is
   an address, not proof of that lifetime. Current-user power settings need their
   own verified context; declaring a Machine field must not bypass that check.
   Implement the pure `snapshot_config_matches(snapshot, config)` hook: a decoded
   request must agree with its retained typed snapshot without a native read.
4. Return exact `ResourceClaim` keys and canonical desired tokens. A resource field
   describes the native setting independently of the feature ID. Tokens encode
   presence, native type, and value without lossy hashes; matching semantics,
   version, and bytes mean equal requests. `observation_states` uses the same
   representation for native observations and `baseline_states` projects the
   immutable captured originals. Never encode unrelated fields in a resource token.
5. Keep the first original baseline immutable. Preserve all known app-written
   partial/native states when preparing an update. Every step lists all resources
   it can write and exact canonical `after` values for those fields. The coordinator
   stages these values before admitting the call; a state merely mentioned in a
   snapshot or a future step is not evidence that it was written. A zero-step plan
   still retains its real baseline and claims.
6. Classify and restore the exact released subset. Preserve current fields outside
   that subset, even when they changed externally or another owner still manages
   them. Restore original absence by deleting only the original managed field;
   never substitute a presumed Windows default. Empty release subsets are no-ops.
   A subset restore must not add untouched external values to known app-written
   history in its returned recovery snapshot.
7. Native execution belongs behind `libs/os_api`. Identity constructors validate
   representation, not actual OS provenance. Native adapters must inspect the
   machine/user/logon/boot or process creation token and image path, then revalidate
   the target on the same handle used for the write. Never fall back to PID or
   executable-name matches. Capability checks short-circuit backend reads/writes.
8. Verify native state after each write. Return `Verification::Mismatch` when it
   differs; a successful syscall or completed step count does not prove success.
   Verification and canonical projection use the same typed observation, so a
   separate read cannot silently promote a different value to confirmed evidence.
   Stored restart-required state is distinct from an effective running setting.
   Preserve bounded failure detail and native error domain/code.
9. Add one explicit registration in the shipping catalog composition. Platform
   core, UI, and runner code must not import concrete modules or branch on IDs.

## Minimal type template

Use the two test adapters as compilable examples of the complete trait. A new
module starts with this shape; native instructions and observations stay concrete:

```rust,ignore
#[derive(Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config { /* bounded requested values */ }

struct Observation { /* observed native values and target evidence */ }

#[derive(Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    // Exact first original values, including absence and native types.
    // Desired values and known intermediate states needed for recovery.
}

enum Step { /* conditional native instructions */ }

// impl Module for Adapter {
//     type Config = Config;
//     type Observation = Observation;
//     type Snapshot = Snapshot;
//     type Step = Step;
//     ... all descriptor, codec, observe, plan, classify, execute and verify hooks
// }
```

## Codec and executor limits

Envelopes are limited to 256 KiB, payload depth 32, and 8192 values. Claims are
limited to 64 exact resource keys, plans to 256 native steps, canonical tokens to
4096 bytes, and owned error detail to 1024 bytes. IDs and nonzero versions validate
on deserialization as well as construction. Snapshot envelopes bind the exact
target, including boot/logon incarnation for session and process targets.

Unknown features or versions remain untouched input envelopes and cannot activate
or restore. A future persistence integration must preserve their payloads when
saving unrelated settings and retain old recovery codecs when modules are hidden
or disabled. Preparation validates snapshot claims and serialization before
exposing an executable operation. It does not persist a prewrite journal.

`PreparedOperation` retains its module adapter, target, concrete config/snapshot,
and concrete instructions. The runner can inspect exact claims, step resources,
native canonical observations, baseline equality, and bounded snapshot envelopes;
it cannot interchange another module's instructions. `same_request` and
`same_snapshot` are conveniences for the same adapter. Ownership sharing must
compare each overlapping resource's canonical token independently.

## Ownership and runner boundaries

The coordinator acquires complete claims before execution and retains original
baseline/restorer data independently of the current writer. Equal canonical
requests can share a field across adapters; a different request requires a sole
owner. Last-owner restoration uses the original adapter with private ledger-based
expected-current evidence, rather than recapturing an app-modified baseline or
decoding another module's snapshot with the wrong codec. External drift retains
unresolved evidence and blocks blind restoration.

Preparation's read preflight cannot eliminate another actor's later race. The
platform checks expected-current evidence again immediately before dispatch, and
native steps must conditionally check their current values and target on the same
handle used for the write. Only the serial platform orchestration can dispatch
prepared mutation hooks. Modules do not provide a permissive execution gate.

The runner accepts at most 64 queued commands and 256 generation owners; the
coordinator retains at most 256 owners and 4096 resources. Explicit retirement of
an inactive generation owner leaves a permanent, bounded interval tombstone;
capacity exhaustion rejects admission without dropping unresolved ownership.
Terminal results belong to caller-held tickets rather than an unbounded result
queue. Immutable snapshot envelopes are shared rather than cloned per resource.

An owner retains its activation binding separately from module sources and
resource targets. Explicit manual requests carry a profile and nonzero saved
configuration generation. Game requests additionally carry logical GroupId/RuleId,
Running or Foreground eligibility, exact logon/continuity identity and a bounded
set of verified process instances. Deserialization validates shape only. Unknown
bindings remain representable for retained legacy evidence and test callers;
they cannot establish native attachment authority. The runner rejects a configured
binding whose generation differs from its command generation before queue
acceptance, generation advancement or cancellation of older accepted intent.

Recovery requires the exact retained game binding plus a nonempty verified subset
of its recorded process instances. Resource lifetime remains independent of game
eligibility. Positive foreground loss releases only an exact foreground owner;
it does not retire a process or logon. Changing recorded membership or actor
requires the future explicit reconciliation API, rather than rebinding an owner
through an ordinary configuration update. Missing journal bindings deserialize
as Unknown; malformed bindings retain blocked raw evidence.

Cancellation or a newer generation suppresses activation, while compensation and
restoration continue. Shutdown timeout retains the same worker and ownership;
unresolved cleanup and worker panic have explicit results. Retiring Process or
Session resources requires an authoritative lifetime producer, not rule deletion,
read denial or an unchecked PID. The OS boundary provides a query/synchronize-only
process guard: exact creation/image/token identity and zero-timeout lifetime
observations use the same retained object. A signaled guard proves that object's
exit; query/open/wait failure remains Unknown. F4 wires authoritative game
bindings and execution snapshots; this guard grants no setting writes.

The worker records intent before each call, verified progress before advancing,
and ownership transitions even when they require no native write. The journal is
limited to 4 MiB with at most 8448 deduplicated sources. Publication failure
retains the previous and proposed evidence and closes mutation admission;
observation remains available. Unknown codecs or targets never trigger a clean
fallback, fresh baseline capture or eviction of unresolved state.

Only a Send factory crosses threads. It creates and drops the potentially !Send
store and native lease on the serial worker, around commands, panic handling and
shutdown publication. Timeout or caller Drop does not release a still-working
authority. Orderly shutdown has an explicit stamp and retains manual owners.
Restart effects survive empty ledgers and unrelated later observations/transitions
until a future explicit validated acknowledgement clears them.

Hydration decodes original, current-writer and owner sources independently, without
observing or recapturing native values. Scoped bootstrap then applies recovery
under durable controls, reobserves current values and requires a completed result
before readiness. Recovery uses exact confirmed/attempted tokens, preserves
external drift and rejects contradictory lifetime evidence.

Current configured input is an immutable complete snapshot of at most 256 owners
and 4 MiB including binding/target metadata, or Unavailable. Exact owner, binding,
config, target and mode must match. Missing intent, a changed generation or an
omitted owner is Unknown; release requires explicit disablement of the recorded
tuple. F4 must retain independent withdrawal intent until durable release is
verified. Numeric profile/owner identity also needs a qualified configuration
origin before other-account admission; the machine-wide journal cannot provide
that origin.

Native scope is privately constructed from actual machine observation and
canonical store opening. Only exact Manual bindings and declared PersistentMachine
resources have a represented native lifetime path; both original restorer and
current writer must satisfy it. User-context settings cannot enter by projecting
their address to Machine. Unsupported scope remains Unknown. Commands and
zero-write sharing/transitions require current evidence before preparation, after
durable intent before execution, and at each publication. Changed authority
retains original/proposed evidence rather than compensating blindly. Observe
remains available when mutation is blocked.

The dormant runner reserves configuration publication through one non-clonable
permit in the existing bounded FIFO. It settles prior work and releases only the
idle configuration leaf, retaining the same coordinator/store/lease and origin.
Save requires an exact worker acknowledgement; only a fresh verified abort before
publication may restore the old input. Started uncertainty, timeout and caller
Drop retain that worker context. A matching sealed saved revision and its producer
are installed together after current source and actual machine checks; rejected
handoffs retain saved evidence and the original journal. Application saves are
not yet composed through this seam, and fixture handoffs supply no native permit.
Headless transactions adopt candidate state and logical identities after successful
persistence. Optional bounded v2 acknowledgement metadata retains the pending tuple
until its own save succeeds. Pruning requires fresh retained-worker corroboration
against the current journal; deserialized acknowledgement fields supply no authority.

Production native admission remains closed even for an absent journal: the
default configured producer is Unavailable, and an explicit staged gate also
blocks native mutations after the private revision handoff. Matching update
recovery and saved transactions remain prerequisites. A Clean store is
insufficient authority.
Native scope, elevated
cross-user and power-transition evidence plus Linux integration checks remain
explicit gates. The nonshipping Windows recovery script verifies
three forced-process boundaries against isolated physical scalar targets and
requires each named test to exist before running the suite. These tests do not
certify canonical/elevated/boot scope or native gaming settings. There is no
immediate crash rollback guarantee.

Continuity identity stores all 32 nonce bytes with a fixed versioned producer,
domain and registry view; its shape is not native authenticity or a boot UUID.
Test-domain identities are distinct and unavailable in production. Journal
FORMAT2 binds the independently expected saved configuration origin; FORMAT1,
missing/foreign origin and legacy bare-array continuity identities are blocked
with their exact raw bytes retained; there is no inferred upgrade.
The canonical continuity producer remains unavailable pending its native matrix.

The Windows `canonical_marker` candidate is separately gated by
`tuning-test-controls` and is not composed into the application. Only explicitly
selected `lab-continuity-*` probe commands invoke it; existing commands do not.
Its private codec, registry policy and operation traces cover bounded decisions;
owned private-hive and current-token tests characterize the reusable native
primitives. The candidate API borrows the retained canonical store/lease and
revalidates actual retained token and exact machine context. It checks both
retained keys and the reopened fixed namespace before and after its bounded
volatility probe. No fixture or decoded nonce can supply that authority. Source
review and isolated tests do not complete the canonical, elevated or
power-transition acceptance gates; shipping recovery admission remains closed.

The required-feature os_api `tuning-canonical-probe` example provides safe token
preflight and explicitly selected elevated metadata observation/contention. Default
preflight always skips canonical open. Lab commands may initialize missing
protected directories and lease metadata; they never publish journal data or
change gaming settings. The separate continuity commands validate or initialize
fixed protected registry metadata and compare two observations on one retained
guard; Unknown and incomplete results remain unsuccessful. These commands do not
certify native acceptance merely by being compiled or fake-tested. No lab command
is an implicit test or release step.
See [the probe guide](../../../../docs/tuning-native-probe.md) for exact commands,
worker timeout semantics and the remaining canonical native gates.
