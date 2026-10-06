# Tuning module contribution contract

The dormant platform supplies typed contracts, a catalog, an in-memory resource
coordinator and a bounded serial runner. The production catalog is empty and
runner write admission is closed. The two backend shapes and cross-adapter
simulators under `testing/` compile only with `cfg(test)` and perform no OS changes.
Durable journaling, cross-process coordination, crash recovery, authoritative
native identity/lifetime producers, app-state integration and user controls remain
later work. Those stages must pass before a writable module can ship.

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

Cancellation or a newer generation suppresses activation, while compensation and
restoration continue. Shutdown timeout retains the same worker and ownership;
unresolved cleanup and worker panic have explicit results. Retiring Process or
Session resources requires an authoritative lifetime producer, not rule deletion,
read denial or an unchecked PID. F4 supplies that producer.

The worker exposes prewrite/progress seams for F3. They do not yet persist a
write-ahead journal or acquire a native lease: production admission rejects all
writes. F3 must durably record intent before every native call and preserve the
same worker-owned lease through uncertain completion. No native rollback or
forced-termination recovery guarantee is implemented here. Windows native smoke
validation and Linux CI remain gates for later adapter/integration stages.
