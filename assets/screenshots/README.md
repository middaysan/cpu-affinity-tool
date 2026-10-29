# Application screenshots

These images show the current Windows interface with synthetic rules and owned
demonstration processes. They were captured through the existing diagnostic MCP
renderer using package version **1.7.2**. Capture-time source was
`974de6fc23fdac8743e025125ed1bd8be04dfdb7`; the diagnostic EXE's SHA-256, UTC time,
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

No working user rules, published executable, real services or Windows settings
were changed. The app, workloads, listener, ticket and temporary keys were
cleaned up successfully. Refresh affected scenes alongside visible UI changes
using [the capture and publication workflow](../../docs/readme-screenshots.md).
