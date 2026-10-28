# Refreshing README screenshots

README images are captured from the real Windows UI with synthetic rules and
owned demonstration processes. Graph values come from the normal CPU collector;
they are not drawn or injected. The capture mode reuses the existing live MCP
fixture, production HTTP configuration and diagnostic renderer screenshots.

## Capture, review, publish

From an elevated Windows PowerShell on a desktop, with Python 3.11 or newer:

```powershell
./scripts/test-windows-live-mcp.ps1 -CaptureReadme
```

The command prints a `target/lm-*/evidence/screenshots` directory. Inspect the
individual PNGs before publishing. Require readable text, the intended screen,
real demo activity, no accidental tooltip, no clipped controls and no personal
data. System Tweaks is read-only; never press Apply or Undo for documentation.
The MCP scene never copies or displays its private key.

```powershell
./scripts/publish-readme-screenshots.ps1 -CaptureDirectory <printed-directory>
python scripts/check-readme-screenshots.py
```

Both PowerShell commands accept `-Python <interpreter>`. Capture also supports
`-SkipBuild -ExePath <diagnostic-exe>` when deliberately reusing an identified
artifact. It refuses an administrator shipping manifest rather than silently
using a production profile.

Publication first checks all eight native PNGs, fixed asset names, byte hashes,
dimensions, the normalized scenario-contract hash and successful cleanup. It copies only
the PNG whitelist and sanitized `capture.json`. Logs, state, session tickets and
credentials remain outside assets. Failed or partial captures cannot replace the
README set. Publication does not launch the app or need administrator rights.

## Fixed scenes

The single scene/hero definition is
[`scripts/fixtures/readme-screenshots.json`](../scripts/fixtures/readme-screenshots.json).
The runner uses real controls and bounded waits to capture:

| Asset | Screen and precondition |
| --- | --- |
| `main-dark.png` | Two demo groups, active application/helper and background workload, current CPU/RAM, dark theme |
| `topology.png` | Edit the existing demo group; actual host topology and selected logical CPUs |
| `cpu-history.png` | Tracked group, selected busy logical CPU, at least 18 completed measured intervals |
| `processes.png` | The same group's populated, sortable process table |
| `history-export.png` | The actual graph's PNG/CSV context menu, without saving a user-selected file |
| `system-tweaks.png` | Read current Windows settings; no owned changes or Apply |
| `mcp-connection.png` | Actual isolated localhost connection UI; credentials are not shown or copied |
| `main-light.png` | The same groups in the light theme |

The lead [`assets/screenshot.png`](../assets/screenshot.png) is generated from
**Affinity groups / CPU selection / CPU history**. Native application pixels are
copied unscaled and uncropped. Labels, 32-pixel gutters and contrasting vertical
separators are placed outside the app frames. Keep the three columns and their
order; retain individual full-size images for readers who want details.

## When UI changes

Review screenshot impact in every change that alters visible layout, labels,
navigation, controls, themes, metrics or graph/table presentation. Refresh the
affected scenes in the same PR; a shared header, font, theme or layout change
usually requires the full set and a rebuilt lead image. Keep captions and links
consistent. Do not keep an old image merely because its path still exists.

`capture.json` records capture-time source/artifact identity, UI input hashes,
scale, scene/frame hashes and cleanup. The checker reports changed UI input files
for review; it does not treat a comment-only refactor as a pixel regression.
Explain a reviewed nonvisual change in the PR instead of claiming a new capture.
Never rewrite capture provenance to make an old image appear current.

Before opening a PR, inspect both the full-size lead image and its approximate
GitHub-page width. Include the capture source/artifact, refreshed scene list,
visual review and cleanup result in the PR. Screenshots are documentation, not
performance benchmarks or shipping/vendor/real-service certification.
