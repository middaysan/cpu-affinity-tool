# Tuning module contribution contract

The dormant platform supplies typed contracts, a catalog, a resource coordinator,
a bounded serial runner, a bounded recovery journal and typed recovery decisions.
The production catalog is empty and write admission is closed. The Windows
boundary supplies store/lease and identity primitives; canonical native admission
remains closed pending scope/lifetime validation. The two backend shapes and
cross-adapter simulators under `testing/` compile only with `cfg(test)`.
Isolated forced-process recovery is demonstrated at three durable boundaries.
Canonical/elevated/boot validation, authoritative game attachment, app-state
integration and user controls remain staged work. Passing isolated test objects
does not admit a writable module.

The canonical specification is
[`docs/modular-gaming-features-plan.md`](../../../../docs/modular-gaming-features-plan.md).
The Windows recommendations and exclusions are documented in
[`docs/windows11-gaming-module-research.md`](../../../../docs/windows11-gaming-module-research.md).

## Structure

- `mod.rs` is the public feature facade and future shipping catalog composition.
- `platform/contract.rs` contains validated IDs, targets, setting resources,
  capability and status values, exact canonical tokens, and typed plans.
- `platform/catalog.rs` contains `Module`, checked registration and preparation,
  and the opaque executor that retains its concrete module and typed state.
- `platform/codec.rs` is the bounded, versioned serialization boundary.
- `platform/coordinator.rs` owns atomic claims, equal-request sharing, conflicts,
  first baselines, step verification, compensation and last-owner restoration.
- `platform/runner.rs` owns bounded serial dispatch, generations, cancellation,
  terminal tickets, immutable snapshots and bounded shutdown.
- `platform/journal.rs` owns aggregate limits, deduplicated typed source envelopes,
  exact unknown input retention and durable intent/progress/ownership transitions.
- `platform/recovery.rs` distinguishes verified attachment, positive expiry and
  Unknown for each actual resource lifetime.
- `platform/writer.rs` creates worker-local storage authority, imports retained
  typed sources and maps bounded native errors. Its canonical factory is dormant.
- `testing/scalar.rs` demonstrates a scalar machine setting.
- `testing/user_setting.rs` demonstrates two native fields, exact absent/present
  values with original type and raw bytes, partial application, subset restoration,
  and a restart-required stored effect.
- `testing/value_setting.rs` uses the user backend through a separate typed
  adapter to exercise sharing and edits across modules with different snapshots.

The root `allow(dead_code)` is limited to this dormant staged foundation. Remove
or narrow it when F4 integrates the public facade; do not add per-module lint
exceptions to hide unused integrations.

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
observing or recapturing native values. Recovery uses exact confirmed/attempted
tokens, preserves external drift and rejects contradictory lifetime evidence.
Production native admission remains closed even for an absent journal; a Clean
store does not prove safe process/session provenance. Native scope, elevated
cross-user and power-transition evidence plus Linux integration checks remain
explicit gates. The nonshipping Windows recovery script verifies
three forced-process boundaries against isolated physical scalar targets and
requires each named test to exist before running the suite. These tests do not
certify canonical/elevated/boot scope or native gaming settings. There is no
immediate crash rollback guarantee.

Continuity identity stores all 32 nonce bytes with a fixed versioned producer,
domain and registry view; its shape is not native authenticity or a boot UUID.
Test-domain identities are distinct and unavailable in production. The unreleased
journal format remains version 1, while legacy bare-array continuity identities
are blocked with their exact raw bytes retained; there is no inferred upgrade.
The canonical continuity producer remains unavailable pending its native matrix.

The required-feature os_api `tuning-canonical-probe` example provides safe token
preflight and explicitly selected elevated metadata observation/contention. Default
preflight always skips canonical open. Lab commands may initialize missing
protected directories and lease metadata; they never publish journal data or
change gaming settings. No lab command is an implicit test or release step.
See [the probe guide](../../../../docs/tuning-native-probe.md) for exact commands,
worker timeout semantics and the remaining canonical native gates.
