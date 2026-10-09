# Native tuning validation

The earlier canonical continuity/configuration-receipt and handle-transfer lab engine was removed during the four-stage simplification. Its commands and examples no longer exist. Historical source and reports are available in Git at `e275b2661a64676933e04eda391190c05c3c3af4`; those experiments are not acceptance evidence for the new gaming modules.

Existing ProgramData recovery evidence is inspected read-only. A journal, staging/previous artifact or an inspection error blocks native setting admission; the application does not delete, repair or recapture it. The old metadata directories/lease are not a gaming-setting change and are not automatically removed.

## Current read-only evidence

On 2026-10-08 the opt-in `gaming_native_readonly_inventory_reports_actual_display_audio_and_pnp` test passed on the current Windows desktop: 2 displays, 3 audio endpoints, 248 present devices and 1 processor group. It used real GDI, audio session and PnP APIs and made no setting changes. Counts are host observations, not hardware requirements.

Run explicitly:

```powershell
cargo test --locked --manifest-path libs/os_api/Cargo.toml gaming_native_readonly_inventory -- --ignored --nocapture
```

Pure API tests cover independent QoS/timer mask merging, hotkey-only merging, CPU Set IDs/empty originals, priority limits, exact raw interrupt values and real parent relationships. The process read characterization accepts the current instance and rejects a changed creation token or image path. These tests do not prove native apply/restore.

## Managed-module readiness

On 2026-10-08 all eight managed adapters passed actual apply → read result → disk reload → disable → verify original on the current Windows host. Power Mode AC and DC were tested separately; an additional owned-child test confirmed positive same-instance expiry and durable retirement without a dead-PID read/write. Production admission now requires verified independent saved configuration, the recovery lease and resolved native/recovery evidence.

The first accessibility run correctly failed to establish evidence because the selected hotkeys were already disabled. A test-only precondition then prepared one activation bit on a disabled accessibility feature through the same durable controller, ran the production adapter's full round trip, and restored the precondition. Final full mouse/accessibility structures matched the original; enabled features, timing, sound and unrelated flags were preserved. The preparation adapter is not registered in application code.

The eight adapters use actual SPI, power and process APIs. Process tests own hidden disposable children and kill/wait them. Every round trip has independent conditional rollback and retains its isolated evidence; a no-op, fixture or unavailable API is not accepted as native proof.

| Adapter | Required actual round trip |
| --- | --- |
| Power Plan | Select another existing scheme when available; verify and restore the original GUID. No scheme creation/deletion. |
| Power Mode | Verify supported API/source; independently preserve and restore AC/DC override GUIDs. |
| Mouse acceleration | Verify acceleration tuple and restoration while pointer speed remains unchanged. |
| Accessibility hotkeys | Preserve enabled accessibility functions, timing/other flags; verify owned shortcut fields only. |
| Process QoS | Use a disposable owned child with exact token/image; preserve timer/unknown policy bits. |
| CPU Sets | Use real topology IDs/groups and owned child; restore exact original empty/list selection. |
| Priority | Use owned child; verify original before the first relevant classic/module write and exact restoration. |
| Timer-policy experiment | Use owned child; verify only the documented process policy bit and preserve QoS. No claim of measured FPS/input-latency improvement. |

Recovery tests exercise the common mechanism with owned temporary settings: failure before write, physical write without confirmed progress, confirmed progress, shared ownership, conflicts, partial multi-field application and fresh-process disk reload. A forced child termination is allowed only with independent exact target rollback. No account/boot/VM/IPC laboratory matrix is required.

The three required forced-exit cases passed through `scripts/test-windows-tuning-recovery.ps1 -Offline`. Common tests also verify unavailable/corrupt/staged recovery, different saved-state sources, exact owner activation, publication failure, external drift/Unknown, valid empty projections, first-save initialization and guarded Session shutdown. Native callbacks do not hold GUI-facing locks.

Run actual module tests explicitly and serially; they change settings and verify rollback:

```powershell
cargo test --locked --features windows,tuning-test-controls --bin cpu-affinity-tool native_roundtrip_ -- --ignored --test-threads=1 --nocapture
```

Fresh Linux test/Clippy/release evidence is unavailable on this PC because no Linux runner is installed. Interactive Windows Modules/UAC smoke remains a manual check; rendered egui/state-level tests and actual adapters cover the automated boundary. These open checks are not native acceptance failures and are not treated as complete goal sign-off.

For the current Modules screen, select or create a profile, choose a category and use **Settings** to choose the value and **When** trigger. **Enabled** includes the setting in the profile; **Save changes** commits the draft. Connect an existing game under **Games using this profile** for automatic settings. Manual settings expose **Apply now** and stay applied until turned off. Confirm running/foreground transitions and restoration when disabling. **Windows settings** opens native Settings; **Check your PC** provides observations/imports without managed configuration. Technical inventories and error causes remain accessible under **Details** and **Error details**.

Testing on the current Windows host is authorized with verified rollback. Reboot/sleep/hibernation, account creation, protective ACL changes and push/merge/release are outside this goal. Successful application of a setting proves that API path and restoration, not a universal gaming-performance benefit; compare representative A/B captures separately.
