# Testing without another test platform

The fast Rust tests and built-app MCP scenarios answer different questions. Keep
deterministic tests for identity, ownership, stale generations, failed saves,
conditional Undo, CPU/RAM unions, unknown data and shutdown. A successful GUI poll
cannot prove that a forbidden write never occurred between polls.

## Build cost

The application's test package uses optimization level 1 and limited debug info.
Dependencies retain the existing dev settings and cache compatibility. Debug
assertions, overflow behavior, normal diagnostic builds and release builds are
unchanged. OS-library tests keep their own existing profile.

Measure before/after on the same source and machine:

```powershell
./scripts/measure-test-build.ps1 -Repeats 2 -Offline
```

This alternates AB/BA, cleans only the application's compiled test/dev artifacts,
reuses dependency artifacts equally, and records build and test-command time
separately under `target/test-build-timing-*/`. It can remove an existing debug
EXE; close that development instance first. It is a root-package rebuild
measurement, not a cold CI, GPU-performance or release-build benchmark. Compare
CI setup, cache and build steps separately rather than treating test execution as
the entire job. Cargo's [profile overrides](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides)
describe the package-specific settings.

On the Windows development host (Rust 1.98.1, cached dependencies), two alternating
root rebuilds on the same 995-test source measured:

| | Previous opt3/full debug | Test-only opt1/limited debug |
| --- | ---: | ---: |
| Root test build, seconds | 69.12 / 64.76 | 41.31 / 42.43 |
| Mean build, seconds | 66.94 | 41.87 |
| Test harness, seconds | 2.52 / 2.53 | 2.54 / 2.52 |

That is approximately 37% less root-test compilation time, with unchanged test
execution time. Cargo command overhead is recorded separately by the script
(one candidate command took 10.09 seconds despite a 2.54-second harness); these
numbers do not predict a cold hosted CI percentage. Before consolidation, the
same 997 tests also passed both profiles (2.55/2.53 seconds).

## Synthetic built-app acceptance

From an **elevated Windows PowerShell on a desktop**:

```powershell
python scripts/test_live_mcp.py
./scripts/test-windows-live-mcp.ps1
# Also require the ordinary Explorer launch broker and native child readback:
./scripts/test-windows-live-mcp.ps1 -InteractiveLaunch
```

The default pack exercises the compiled diagnostic app, its real rendering/input,
the production HTTP MCP server, application writer, disk persistence and CPU
worker. The existing standard-library debug adapter supplies settled UI reads and
input; it does not replace the HTTP business API. The runner has no extra Python
dependencies. A supplied `-Python` selects the interpreter; `-SkipBuild -ExePath`
can select an already built diagnostic artifact.

The separate `-CaptureReadme` mode reuses that fixture for fixed documentation
scenes and measured owned workloads. It enables monitoring only after creating
its synthetic rules; it does not run or replace the acceptance pack below.
See [README screenshots](readme-screenshots.md) for capture, visual review and
checked publication. CI validates committed assets without taking screenshots.

| Case | Evidence |
| --- | --- |
| R1 | Selected artifact hash, actual child PID, synthetic monitoring Off, orderly exit |
| C1 | Real durable receipts; groups/rules/additional names; incomplete replacement and stale revision refused; reorder/move; GUI group edit after HTTP update with visibility/peer-field and logical-ID retention; stable IDs and unchanged saved values after restart |
| U1 | Real schema-13 file, no eager write on notice, actual read-only-file failure and retry, schema-14 readback, group/list retention and restart, both themes |
| E1, interactive opt-in | Ordinary managed launch, independent exact-handle affinity/priority readback, bounded output/EOF, wrong creation-token refusal, actual Focus acknowledgement |
| T1 | Two owned workloads, extra process, identical overlapping rule sets in two groups; three fresh coherent CPU/RAM union samples; helper after primary exit; saved app/group metadata changes in an open graph without losing history/CPU selection; Off/On |
| T2 | Original system history, bounded UTF-8 CSV cursor download and parsing, retained immutable export after Off, foreign history epoch refused |
| S1 | Deliberately uninstalled synthetic Steam AppID, actionable warning in both themes, direct reason/chooser, no premature draft save, exact owned-process selection/measurement and unchanged helper list; never launches Steam |
| D1 | Three native scheduling access denials on an exact owned child, paused correction with readable app/group CPU/RAM, Limited access in both themes, explicit Retry/native affinity readback, and exact process-DACL restoration |

The opt-in E1 requires a real same-logon Explorer broker; CI does not pretend to
certify that interactive path. Windows may refuse foreground activation; the
acknowledgement is recorded, not represented as a foreground guarantee. Real
Steam/Discord/game acceptance, DPI/tray/login, PNG save dialogs and shipping
bootstrap remain in the [release smoke matrix](release-smoke-matrix.md).

D1 temporarily changes only an exact owned workload's process DACL. If enabled,
the runner and its retained diagnostic app temporarily disable their own debug
privilege so it cannot bypass that DACL. The original DACL and privilege flags
are restored before cleanup; no account privilege or Windows policy is changed.
The native access-denied preflight is mandatory, and readable metrics are checked
through the same app/group snapshots while correction is paused.
All C1/D1 workload basenames include the run's unique stamp: explicit helper-name
matching cannot admit another concurrent runner's owned processes.
S1 checks its retained child's liveness, refreshes the request-driven picker when needed
and scrolls overlapping list views before clicking the owned row; process count
and viewport position must not determine whether selection succeeds.

The optional `-ReadOnlyInstalledAumid 'Publisher.Package!App'` adds D2 in a separate
fresh profile when real-app observation is authorized. Enforcement stays Off;
it never launches that application or changes its scheduling. It requires an
already running app and checks automatic members belong to the current nonzero
Windows session. This does not qualify real services for mutation.

### Isolation and evidence

- Each run creates a short unique `target/lm-*/` directory, a synthetic sidecar,
  diagnostic marker and isolated protected MCP authority. It never imports user
  settings, autorun rules, tuning profiles or recovery journals.
- The app generates its own permanent key. Only its fresh diagnostic authority
  fixture receives a free port while the app is stopped, then the real Enable
  control starts HTTP access. Capabilities must identify the exact launched PID.
  This is diagnostic provisioning, not a production authentication bypass.
- Both MCP switches start Off. No services, tasks, power plans or real tuning
  modules are changed. Telemetry discovers only the synthetic configured names
  for its assertions. A bounded helper workload exits on its own stop marker or
  deadline; cleanup uses retained handles, never process basenames.
- Intermediate `evidence/result.json` records named cases, artifact identity,
  timings and cleanup. Credentials, raw inventories, state and arbitrary output
  are not included. Failure screenshots show only the synthetic app UI.
- A forced cleanup makes the run fail. Unsupported, omitted or blocked scenarios
  are not counted as successful coverage. Preserve the first failure; do not
  retry uncertain mutations with a new request key.

The shipping EXE has no diagnostic endpoint, input selectors or test-directory
override. Exact shipping acceptance needs a disposable account/VM and legitimate
Connection UI setup: changing `LOCALAPPDATA` does **not** isolate its protected
KnownFolder authority. EXE/PDB/manifest checks and nonshipping compilation fences
remain mandatory and independent of this suite.

## Focused consolidation

This change removes the test of the `Setting` fixture's hardcoded availability
mapping; the existing production-facing unsupported-action tests still prove
refusal before save/native write, and controller tests cover available actions.
The duplicated MCP connection-content assertions now run inside the existing
two-theme/two-viewport hierarchy test. The immutable `Capability` self-comparison
is removed; plain error text, capability handling and actual Details evidence
remain covered elsewhere.

No identity/ownership/CPU/RAM/Undo/persistence/revocation/shutdown tests are moved
to GUI polling. Build-contract parser tests remain while that parser exists.
Consolidation has no test-count target.

The CPU union scenario also verifies group-capacity units against the same original app history interval through HTTP, changes one/two-CPU masks, and excludes owned busy work outside a temporary mask. Graph windows are opened separately because an opaque modal hides the main list. Group fields are preserved and the synthetic mask is restored before subsequent cases. The denial case accepts measured zero CPU while its owned children are outside the selected group mask; readable telemetry must remain complete through refused writes.

Diagnostic denial counts are nullable when a nonblocking observation is busy. D1 requires at least three qualified pause observations and rejects any known reset; an unavailable read is never treated as zero. Completed fixtures are cleaned up before the next fixture starts. Cleanup failures retain the synthetic fixture label and bounded stage codes without paths, inventories or exception contents.

The first simplification slice removes only the retired test-only module editor,
its self-tests and unconstructed internal background-toolbar requests. Current
compact Apply/options/unknown-value tests, typed configuration validation,
controller/recovery/conditional Undo and background role admission checks remain.
The roster regression covers both launch-key transition and scoped runtime keys;
the graph regression uses actual rendered labels on successful and refused saves.
Theme selection reuses the gated writer fixture for no-op/invalid input, pending
publication and save failure rather than adding a separate persistence mock suite.
Normal README scenes retain their existing layout and copy: the graph change is
observable only after editing an already open source. Existing capture provenance
is retained; the live metadata case exercises that conditional update separately.

Group create/edit unification extends the existing gated writer fixture with
pending/success/refused GUI saves and retains field/identity checks. A stale GUI
projection must refuse its admitted request, retain the draft and newer external
group, then permit retry after adoption. The MCP-specific create limit stays
at its adapter. C1 uses the existing pointer input to edit a real group form after
an HTTP edit, checks unrelated fields/IDs, and verifies restart persistence.
GUI creation is covered by deterministic state fixtures; no text-input RPC or
new testing framework is introduced for this refactor.
