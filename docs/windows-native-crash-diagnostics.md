# Windows Native Crash Diagnostics

Use this guide only when CPU Affinity Tool exits without a local crash report and a maintainer needs a native Windows crash dump to investigate an access violation or similar unhandled native failure.

This is a manual, opt-in support procedure. CPU Affinity Tool never enables Windows Error Reporting (WER) LocalDumps, never edits the registry, and never uploads a dump. Do not turn it on for routine use.

## Before enabling a dump

- Reproduce with the exact release `cpu-affinity-tool.exe` that failed.
- Keep the matching `cpu_affinity_tool.pdb` from the same release. A PDB with a different GUID or age cannot reliably symbolize that EXE.
- Choose a directory that the elevated application can write to, for example `C:\CpuAffinityToolDumps`.
- Treat every dump as sensitive: it can contain paths, command lines, in-memory text, and data from the application or related processes. Do not attach one publicly without reviewing it and getting maintainer guidance.

## Enable a small, bounded capture

1. Create the dump directory in Explorer or from an elevated PowerShell prompt:

   ```powershell
   New-Item -ItemType Directory -Force -Path C:\CpuAffinityToolDumps
   ```

2. Open Registry Editor as an administrator and create this key exactly:

   ```text
   HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\cpu-affinity-tool.exe
   ```

3. Add these values to that key:

   | Name | Type | Value | Purpose |
   | --- | --- | --- | --- |
   | `DumpFolder` | `REG_EXPAND_SZ` | `C:\CpuAffinityToolDumps` | Destination for captures |
   | `DumpType` | `REG_DWORD` | `1` | Mini dump; start here |
   | `DumpCount` | `REG_DWORD` | `3` | Retains at most three dumps |

4. Restart CPU Affinity Tool and reproduce the unexpected exit once. Do not repeatedly run the scenario with capture enabled.
5. If Windows writes a `.dmp` file, record its timestamp and the exact EXE version. Share only the requested, reviewed artifact through the support channel agreed with the maintainer.

`DumpType = 1` is intentionally the default in this guide. A full dump (`DumpType = 2`) can be much larger and more sensitive; use it only after an explicit maintainer request and after confirming adequate free disk space.

## Limits and interpretation

- A dump is not guaranteed. Forced termination, power loss, anti-cheat action, some security-product actions, and crashes outside WER's handling path can leave no dump.
- A dump identifies the process state at failure; it does not by itself prove that CPU affinity, monitoring, an overlay, a driver, or another application caused the failure.
- The app's **Activity** Event Log record is supplemental evidence. Its sanitized module, offset, and timestamp can help match a dump, but it is not a root-cause determination.

## Disable and remove the capture

After collecting the requested evidence, delete the `cpu-affinity-tool.exe` key under `LocalDumps` in Registry Editor. Then review the dump directory and delete or securely archive the files according to your support agreement. Removing the key restores the normal WER behavior for this executable.
