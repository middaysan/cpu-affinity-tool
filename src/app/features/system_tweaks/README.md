# System Tweaks

A module is a statically compiled Rust adapter. Register it once in `tweaks/mod.rs`; the UI and controller discover it through the registry. Concrete configuration decoding, the editor and native setting semantics stay in the module. Unsafe Windows code stays in `libs/os_api`.

The module platform is currently enabled only for the Windows binary. Linux omits Modules and rule-profile controls, registers no modules and performs no module observation, activation, recovery or periodic polling. Its existing queue handles only ordinary state/rule persistence and retains the raw tuning field unchanged, including unknown data. `platforms/windows.rs` composes the Windows catalog; `platforms/linux.rs` is an inactive extension point for a possible future implementation. Linux native tweaks are not implemented.

## Compatibility

Each adapter supplies `ModuleEditor::platform_support()`. The map separates Windows 10 and 11 and requires an actual read-only OS build, independent of compatibility manifests. Unsupported cards are grey, explain the requirement in help, and cannot be applied or observed. Recovery/Undo retains its existing conditional native verification.

| Tweak | Windows 10 | Windows 11 |
| --- | --- | --- |
| Performance on AC power | Unavailable | Available, subject to native API support |
| Core parking | Unavailable | All cores; detected class-only control is visibly unavailable |
| Game Mode | 1703+ | Available |
| Background clips | Available | Available |
| Windowed games | Unavailable | 22H2+ |
| Screen refresh rate | Available, subject to compatible modes | Available, subject to compatible modes |
| Mouse acceleration / accessibility hotkeys | Available | Available |
| Game GPU | 1803+, subject to GPU support | Subject to GPU support |

Legacy process QoS requires Windows 10 1709+; the retained timer-policy experiment requires Windows 11. This table describes compatibility, not measured FPS or latency. Windows 10 native UI/apply/restore is not certified by Windows 11 tests. Stored state continues to use the existing `tuning` field and stable module IDs.

## Small interface

`Module` supplies a descriptor, typed configuration validator/tweak metadata, available targets, read-only observation, desired field changes, exact field reads and conditional writes. Optional actions retain refresh, bounded CSV import and fixed Windows Settings pages for compatibility. The compact System Tweaks presenter reads cached snapshots; native calls run on the existing application worker. Legacy editor fixtures remain test-only.

The current UI uses direct Apply/Undo, without visible profiles or triggers. One horizontal segmented switch stays visible above scrolling content: System (the default), GPU / network and Check PC. Full descriptions appear in hover help. Section selection is transient and dispatches no native request; Check PC reads only after its explicit action. Global errors, Undo and display confirmation remain available across sections. Hidden manual intents reuse the schema-v11 profile/target persistence. Previous configurations remain readable and restorable; classic affinity/priority stays in Affinity groups. GPU preference qualifies a selected saved EXE before dispatch. Already-suitable values create no owner/baseline. Display changes carry a durable unconfirmed intent and 15-second keep/undo timer; failed withdrawal retains evidence. Hardware guides and explicit cached PC checks make no native writes.

Full card capabilities refresh on the existing worker only while the page is
visible and uncovered. Hidden snapshots retain health and operation feedback;
owned-field inspection, session reconciliation, confirmation expiry and shutdown
recovery continue. Compact adapters may share one native observation and
readiness result through `Module::observe_tweak`; display refresh uses one mode
inventory instead of repeated driver tests. Apply and Undo retain independent
native validation and never use a UI snapshot as mutation authority.

Core parking has a separate manual adapter and reuses the legacy native CPU AC
field boundary, so equal/conflicting legacy requests retain the same resource
identity. One combined observation publishes bounded immutable option data with
the card. The GUI never queries native topology; Apply rechecks plan/mode and
class identity before persisting exact parking-only keys. Class 0's shared
minimum is not presented as an E-only switch. Separate-class activation is
rejected at the configuration boundary, not just disabled in the GUI: actual
class-1 policy readback did not establish functional unparking on the test host.
Uncertain class mappings and
missing policy fields remain unavailable. Recovery uses captured keys rather
than rediscovering a new active plan. The original absent/explicit distinction
and conditional restoration are unchanged.

Session observations run only for enabled non-manual associations or retained
Session owners, once per second and after accepted requests. Manual-only and
undone configurations perform no session enumeration or repeated saved-state
verification on queue timeouts. Inspection, confirmation expiry and shutdown
keep their independent safety checks.

After proven session expiry, compact intent is withdrawn and an unowned card
discovers its current target. At the shared 64-profile limit, only disabled
hidden intents without associations or controller evidence can be reclaimed;
legacy profiles, pending restoration and monotonic IDs remain intact. An
interrupted intent publication must not pin an unowned card to an old session.
The transient retired editor is absent. Tests wait for asynchronous saves through
explicit fixture helpers, preserving production queue-acceptance semantics.

A resource is an exact target plus a stable owned-field ID. Equal requests share the first baseline. A newly unowned field already matching the requested value creates no owner or baseline, including within a partially changed tweak; existing owners still share their first original. Conflicting requests fail before writing. Last-owner restoration compares the current owned field with the app-written token and preserves other fields. A module must never turn a read error into a default, infer success from its desired value or recapture an uncertain baseline.

Persistent account settings use `Account { sid }`; GPU preferences use the account SID and executable path. Their module-owned `canonical_target` codec maps legacy logon-bearing targets to the same resource during recovery and acquisition, preserving the first baseline across Windows sign-ins. Ambiguous canonical keys block recovery without publishing or discarding the old evidence. Saved legacy manual targets remain readable; comparisons use the same codec. Parsing an account SID grants no native authority: each operation verifies the live caller/Explorer/UAC identity, requires the same account and pins its exact current logon until readback completes. Mouse, accessibility and display changes remain session-scoped. Their `target_has_expired` hook accepts only a read-only native proof that the exact previous logon no longer exists, with a verified same-account current desktop before and after the query. Reconciliation then retires that module target and its shared ownership without reading or writing the new session. Publication failure retains the original evidence. A different logon alone, a still-present old session or a denied/failed query remains blocked and can be retried with Undo; it never authorizes a cross-session write.

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
