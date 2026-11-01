# Application screenshots

These images show the current Windows interface with synthetic rules and owned
demonstration processes. They were captured through the existing diagnostic MCP
renderer using package version **1.7.2**. Capture-time source was
`5bd9ab9beabb581bdf1723f92c3f0d7f07e45f3e`; the diagnostic EXE's SHA-256, UTC time,
scale, scene/frame identities and original PNG hashes are in
[`capture.json`](capture.json). This is documentation capture, not certification
of a published executable or a performance benchmark.

The lead [`../screenshot.png`](../screenshot.png) combines **Affinity groups / CPU
selection / CPU history** in that order. All three original frames are copied
unscaled and uncropped; exterior labels, gutters and vertical separators make
the boundaries visible. The composition is 2196 x 1305 pixels. Individual native
frames are 700 x 1225 pixels at 1.75 pixels per UI point and remain unchanged.

| Original | View |
| --- | --- |
| [`main-dark.png`](main-dark.png) | Two groups, current per-app/group CPU and RAM, dark theme |
| [`topology.png`](topology.png) | Group editor with actual host topology and selected CPUs |
| [`cpu-history.png`](cpu-history.png) | Selected logical CPU, measured history and contributors |
| [`processes.png`](processes.png) | The same group's populated Processes table |
| [`history-export.png`](history-export.png) | The graph's real PNG/CSV context menu |
| [`system-tweaks.png`](system-tweaks.png) | Current Windows settings, without applying a change |
| [`mcp-connection.png`](mcp-connection.png) | Isolated localhost connection; no key is shown or copied |
| [`main-light.png`](main-light.png) | The same groups in the light theme |

`DemoApp.exe`, `DemoHelper.exe` and `BackgroundWorker.exe` are private copies of
the repository's bounded test workload. CPU activity is measured by the normal
collector, not injected or redrawn. CPU selections and current Windows settings
reflect the capture host and are not universal recommendations.

Application and group meters count work on the group's selected logical CPUs,
whose combined capacity is 100%. The matching group/application values in the
demo share the same measured interval and exact process union. Per-CPU graphs
retain their own 100% scale; the header meter covers the whole processor.

No working user rules, published executable, real services or Windows settings
were changed. The app, workloads, listener, ticket and temporary keys were
cleaned up successfully. Refresh affected scenes alongside visible UI changes
using [the capture and publication workflow](../../docs/readme-screenshots.md).

All eight scenes were refreshed on 2026-11-01 after the CPU-capacity change.
The original full-size frames and the lead image at README display width were
visually reviewed. **Limited access / Retry** and Steam detection warnings were
tested separately; these demonstration scenes show normal process access.

A later diagnostic-only correction distinguishes an unavailable denial-count
read from zero and improves live-test cleanup reporting. It changes no visible
controls, layout or demonstrated metric; the capture provenance remains original.
