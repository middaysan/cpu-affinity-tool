# Application screenshots

The lead image at [`../screenshot.png`](../screenshot.png) was supplied by the project author on 2026-10-22. It combines the Affinity groups and Create affinity group views and is retained exactly as supplied.

The six individual images in this directory were captured on 2026-10-22 from the Windows UI during 1.7.0 development. `main-light.png` was refreshed at source commit `9dba66e75f121df9fcc3a1e238789c6566733faa` using the diagnostic build with package version 1.7.0, including the approved **Group CPU load** caption. The other five images were captured at `fc02c4d20c199edd1e89ebbda0182585e2aebeda`; that earlier diagnostic build still reported package version 1.6.3. It contains development features and does not represent the published 1.6.3 release.

The individual images show:

- `cpu-history.png`: a selected logical CPU, real recorded history and its contributing process.
- `processes.png`: the same group's current Processes view.
- `history-export.png`: the graph's PNG/CSV export menu.
- `system-tweaks.png`: current Windows settings and explicit actions; no tweaks were applied for the capture.
- `mcp-connection.png`: the actual connection window, using an isolated instance and temporary port. No connection key is displayed or copied into an image.
- `main-light.png`: the same example groups in the light theme.

The isolated configuration for the individual captures uses demonstration groups and private copies of program files. `CpuAffinityReadmeLoad.exe` is a locally named Python interpreter running a bounded CPU workload. The graph data is measured, not drawn or injected. These images are not performance benchmarks. The six individual captures retain original PNG pixels without retouching or compositing; the lead image is the author-provided composition described above.

No working user rules, Windows tweaks or published executable were changed. The demo processes, listener and temporary connection credentials were closed or removed after capture. Refresh this set when visible controls change.
