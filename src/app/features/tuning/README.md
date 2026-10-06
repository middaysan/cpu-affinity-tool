# Tuning module contribution contract

F1 supplies a dormant, platform-independent contract and catalog. The production
catalog is empty. The two native-state simulators under `testing/` compile only
with `cfg(test)` and perform no OS changes. This foundation does not yet provide
ownership, cross-process coordination, crash recovery, app-state integration, or
user controls. Those stages must pass before a writable module can ship.

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
- `testing/scalar.rs` demonstrates a scalar machine setting.
- `testing/user_setting.rs` demonstrates two native fields, exact absent/present
  values with original type and raw bytes, partial application, subset restoration,
  and a restart-required stored effect.

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
   representation for native observations. Never encode unrelated fields in a
   resource token.
5. Keep the first original baseline immutable. Preserve all known app-written
   partial/native states when preparing an update. Every step lists all resources
   it can write. A zero-step plan still retains its real baseline and claims.
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

## Boundaries left for F2 and F3

The coordinator must acquire complete resource claims before execution, serialize
transactions, journal before each native write, verify results, and preserve the
first restoration adapter/baseline through shared-owner changes. Preparation's
read preflight cannot eliminate another actor's later race; native steps must
check their expected current value and target again and fail explicitly.

When a later owner using another module adapter edits a shared field, F2 must
retain the original adapter and use exact authorized-current-state evidence from
the resource ledger. The F1 classification hook recognizes its own typed history;
it does not yet authorize another adapter's new written value. Extend that seam
with a failing cross-adapter ownership test rather than recapturing a baseline or
decoding another module's snapshot with the wrong codec.

No timer, watchdog, lock protocol, write-ahead journal, repair loop, recovery UI, or
native rollback guarantee is implemented here. Windows native smoke validation
and Linux CI remain gates for later adapter/integration stages; the deterministic
F1 tests validate the shared contract on the current Windows host.
