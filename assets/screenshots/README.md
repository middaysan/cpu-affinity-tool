# Application screenshots

Captured on 2026-10-22 from the current Windows development UI at source commit `fc02c4d20c199edd1e89ebbda0182585e2aebeda`. The existing diagnostic executable shares that UI; its package version still says 1.6.3, but these are development screenshots, not screenshots of the published 1.6.3 release.

The main view remains at [`../screenshot.png`](../screenshot.png). The other images show:

- `cpu-history.png`: a selected logical CPU, real recorded history and its contributing process.
- `processes.png`: the same group's current Processes view.
- `history-export.png`: the graph's PNG/CSV export menu.
- `system-tweaks.png`: current Windows settings and explicit actions; no tweaks were applied for the capture.
- `mcp-connection.png`: the actual connection window, using an isolated instance and temporary port. No connection key is displayed or copied into an image.
- `main-light.png`: the same example groups in the light theme.

The isolated configuration uses demonstration groups and private copies of program files. `CpuAffinityReadmeLoad.exe` is a locally named Python interpreter running a bounded CPU workload. The graph data is measured, not drawn or injected. These images are not performance benchmarks. Original PNG pixels are retained without retouching or compositing.

No working user rules, Windows tweaks or published executable were changed. The demo processes, listener and temporary connection credentials were closed or removed after capture. Refresh this set when visible controls change.
