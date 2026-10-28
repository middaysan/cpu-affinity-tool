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
| C1 | Real durable receipts; groups/rules/additional names; incomplete replacement and stale revision refused; reorder/move; stable IDs and unchanged saved values after restart |
| U1 | Real schema-13 file, no eager write on notice, actual read-only-file failure and retry, schema-14 readback, group/list retention and restart, both themes |
| E1, interactive opt-in | Ordinary managed launch, independent exact-handle affinity/priority readback, bounded output/EOF, wrong creation-token refusal, actual Focus acknowledgement |
| T1 | Two owned workloads, extra process, identical overlapping rule sets in two groups; three fresh coherent CPU/RAM union samples; helper after primary exit; graph opening and Off/On |
| T2 | Original system history, bounded UTF-8 CSV cursor download and parsing, retained immutable export after Off, foreign history epoch refused |

The opt-in E1 requires a real same-logon Explorer broker; CI does not pretend to
certify that interactive path. Windows may refuse foreground activation; the
acknowledgement is recorded, not represented as a foreground guarantee. Real
Steam/Discord/game acceptance, DPI/tray/login, PNG save dialogs and shipping
bootstrap remain in the [release smoke matrix](release-smoke-matrix.md).

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
