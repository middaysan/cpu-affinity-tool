# System Tweaks

A module is a statically compiled Rust adapter. Register it once in `tweaks/mod.rs`; the UI and controller discover it through the registry. Concrete configuration decoding, the editor and native setting semantics stay in the module. Unsafe Windows code stays in `libs/os_api`.

The module platform is currently enabled only for the Windows binary. Linux omits Modules and rule-profile controls, registers no modules and performs no module observation, activation, recovery or periodic polling. Its existing queue handles only ordinary state/rule persistence and retains the raw tuning field unchanged, including unknown data. `platforms/windows.rs` composes the Windows catalog; `platforms/linux.rs` is an inactive extension point for a possible future implementation. Linux native tweaks are not implemented.

## Compatibility

Each adapter supplies `ModuleEditor::platform_support()`. The map separates Windows 10 and 11 and requires an actual read-only OS build, independent of compatibility manifests. Unsupported cards are grey, explain the requirement in help, and cannot be applied or observed. Recovery/Undo retains its existing conditional native verification.

| Tweak | Windows 10 | Windows 11 |
| --- | --- | --- |
| Performance on AC power | Unavailable | Available, subject to native API support |
| Game Mode | 1703+ | Available |
| Background clips | Available | Available |
| Windowed games | Unavailable | 22H2+ |
| Screen refresh rate | Available, subject to compatible modes | Available, subject to compatible modes |
| Mouse acceleration / accessibility hotkeys | Available | Available |
| Game GPU | 1803+, subject to GPU support | Subject to GPU support |

Legacy process QoS requires Windows 10 1709+; the retained timer-policy experiment requires Windows 11. This table describes compatibility, not measured FPS or latency. Windows 10 native UI/apply/restore is not certified by Windows 11 tests. Stored state continues to use the existing `tuning` field and stable module IDs.

## Small interface

`Module` supplies a descriptor, typed configuration validator/tweak metadata, available targets, read-only observation, desired field changes, exact field reads and conditional writes. Optional actions retain refresh, bounded CSV import and fixed Windows Settings pages for compatibility. The compact System Tweaks presenter reads cached snapshots; native calls run on the existing application worker. Legacy editor fixtures remain test-only.

The current UI uses direct Apply/Undo, without visible profiles or triggers. One horizontal segmented switch stays visible above scrolling content: System (the default), GPU / network, Check PC and Restore. Full descriptions appear in hover help. Section selection is transient and dispatches no native request; Check PC reads only after its explicit action. Global errors, Undo and display confirmation remain available across sections. Hidden manual intents reuse the schema-v11 profile/target persistence. Previous configurations remain readable and restorable; classic affinity/priority stays in Affinity groups. GPU preference qualifies a selected saved EXE before dispatch. Already-suitable values create no owner/baseline. Display changes carry a durable unconfirmed intent and 15-second keep/undo timer; failed withdrawal retains evidence. Hardware guides and explicit cached PC checks make no native writes.

A resource is an exact target plus a stable owned-field ID. Equal requests share the first baseline. Conflicting requests fail before writing. Last-owner restoration compares the current owned field with the app-written token and preserves other fields. A module must never turn a read error into a default, infer success from its desired value or recapture an uncertain baseline.

Persistent account settings use `Account { sid }`; GPU preferences use the account SID and executable path. Their module-owned `canonical_target` codec maps legacy logon-bearing targets to the same resource during recovery and acquisition, preserving the first baseline across Windows sign-ins. Ambiguous canonical keys block recovery without publishing or discarding the old evidence. Saved legacy manual targets remain readable; comparisons use the same codec. Parsing an account SID grants no native authority: each operation verifies the live caller/Explorer/UAC identity, requires the same account and pins its exact current logon until readback completes. Mouse, accessibility and display changes remain session-scoped; unresolved evidence for a previous session stays blocked rather than being applied to another session.

Errors and restoration summaries belong to the latest operation. A successful retry removes the previous transient error; current controller faults and recovery evidence remain visible. Partial Undo reports partial restoration, and several restored fields retain the strongest restart requirement. A newer Apply or refused request clears the old success message.

## Files

- `config.rs`: profiles, logical rule associations and bounded versioned configuration. Unknown extensions remain exact and read-only; old lifecycle evidence remains blocked.
- `platform/registry.rs`: the module map and checked registration.
- `platform/controller.rs`: field ownership, first baselines, verification, conflicts and restoration.
- `persistence.rs`: one bounded recovery document, atomic publication and the independent writer lease.
- `platform/codec.rs` and `contract.rs`: bounded serialization and small shared value types.
- `service.rs`: saved candidates and logical identities; failed saves retain the previous saved state.
- `session_reducer.rs`: one bounded background request queue, existing execution observations and immutable snapshots.
- `presentation.rs` and the System Tweaks presenter: module-owned preset metadata, compact Apply/Undo, help and honest status.
- `pc_checks.rs`: pure summaries and hardware guidance over one explicit bounded native observation.
- `tweaks/<name>/mod.rs`: one independent managed adapter per folder. Shared bounded helpers stay in `tweaks/common.rs` and `preference_common.rs`.
- `tweaks/diagnostics/`: read-only inventories, CSV/A-B and native Settings guidance.

## Contribution checks

Write behavior tests before implementation. Test common sharing/failure/recovery guarantees in the controller; test only native/configuration differences in an adapter. Configuration construction and registry registration perform no native operations. Process writes validate the exact creation token and image on the same retained handle. Process completion requires a positive observation of the same instance; errors remain Unknown.

In-memory setting fixtures supply deterministic desktop availability, including explicit unsupported cases, so common ownership/recovery tests also run on Windows Server CI hosts. Native adapters retain actual OS-build admission; fixtures do not establish host compatibility.

Persistent settings save compact durable original/current/attempted records before the first write, retain an independent writer lease and reconcile unfinished records against independently saved intent at startup. Unavailable saved state, conflicting evidence and legacy recovery artifacts block native admission. CPU Sets/priority coordinate with the existing affinity/priority writers before their first relevant write.

Manual activation stays in effect after closing the application and retains its recovery baseline; explicit disablement restores it. Quit releases Session owners only with verified mutation authority. An exact positively exited process target can retire without a write to its vanished resource.

A module is ready only after an actual apply → read → disable → verify original round trip. A fixture is not native acceptance. Native uncertainty preserves recovery evidence and reports a concrete error.

See [the product plan](../../../../docs/modular-gaming-features-plan.md) and [Windows research](../../../../docs/windows11-gaming-module-research.md). The former general continuity/receipt/laboratory engine was removed; do not rebuild it as a module prerequisite.
