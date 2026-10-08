# Adding a gaming module

A module is a statically compiled Rust adapter. Register it once in `modules/mod.rs`; the UI and controller discover it through the registry. Concrete configuration decoding, the editor and native setting semantics stay in the module. Unsafe Windows code stays in `libs/os_api`.

## Small interface

`Module` supplies a descriptor, editor, available targets, read-only observation, desired field changes, exact field reads and conditional writes. Optional actions cover refresh, bounded CSV import and fixed Windows Settings pages. Editors render cached data; native calls run on the existing application worker.

A resource is an exact target plus a stable owned-field ID. Equal requests share the first baseline. Conflicting requests fail before writing. Last-owner restoration compares the current owned field with the app-written token and preserves other fields. A module must never turn a read error into a default, infer success from its desired value or recapture an uncertain baseline.

## Files

- `config.rs`: profiles, logical rule associations and bounded versioned configuration. Unknown extensions remain exact and read-only; old lifecycle evidence remains blocked.
- `platform/registry.rs`: the module map and checked registration.
- `platform/controller.rs`: field ownership, first baselines, verification, conflicts and restoration.
- `persistence.rs`: one bounded recovery document, atomic publication and the independent writer lease.
- `platform/codec.rs` and `contract.rs`: bounded serialization and small shared value types.
- `service.rs`: saved candidates and logical identities; failed saves retain the previous saved state.
- `session_reducer.rs`: one bounded background request queue, existing execution observations and immutable snapshots.
- `presentation.rs` and the Modules presenter: switches, module-owned editors/actions, triggers and honest status.
- `modules/managed/`: native managed settings.
- `modules/diagnostics/`: read-only inventories, CSV/A-B and native Settings guidance.

## Contribution checks

Write behavior tests before implementation. Test common sharing/failure/recovery guarantees in the controller; test only native/configuration differences in an adapter. Configuration construction and registry registration perform no native operations. Process writes validate the exact creation token and image on the same retained handle. Process completion requires a positive observation of the same instance; errors remain Unknown.

Persistent settings save compact durable original/current/attempted records before the first write, retain an independent writer lease and reconcile unfinished records against independently saved intent at startup. Unavailable saved state, conflicting evidence and legacy recovery artifacts block native admission. CPU Sets/priority coordinate with the existing affinity/priority writers before their first relevant write.

Manual activation stays in effect after closing the application and retains its recovery baseline; explicit disablement restores it. Quit releases Session owners only with verified mutation authority. An exact positively exited process target can retire without a write to its vanished resource.

A module is ready only after an actual apply → read → disable → verify original round trip. A fixture is not native acceptance. Native uncertainty preserves recovery evidence and reports a concrete error.

See [the product plan](../../../../docs/modular-gaming-features-plan.md) and [Windows research](../../../../docs/windows11-gaming-module-research.md). The former general continuity/receipt/laboratory engine was removed; do not rebuild it as a module prerequisite.
