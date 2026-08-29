# Support

## Where to ask for help

Use the repository issue templates:

- **Bug report** for reproducible problems
- **Feature request** for proposed improvements

This repository does not currently use a separate support forum or public discussions area.

## Before opening an issue

Please check:

- [README.md](README.md)
- [docs/why.md](docs/why.md)
- [docs/comparison.md](docs/comparison.md)
- the latest release notes in `changelogs/`

## What to include for diagnostics

For technical problems, include as much of the following as possible:

- Windows version
- CPU model
- CPU Affinity Tool version
- whether the target was added with **Add file...** or **Add installed...**
- the exact target you launched
- clear reproduction steps
- expected behavior
- actual behavior
- screenshots or log output
- a reviewed crash report, if the **Crash reports** page contains one for the incident

The Windows build stores crash reports under the app's active data directory and never uploads them automatically. After the next completed startup scan, **Activity** keeps a summary of the newest validated crash report even if normal activity is cleared. Use the small report button in the app header, choose **Show in Explorer**, and review the full file before attaching it to a public issue. A report can contain local paths or other system details. If redaction is needed, make a copy and redact the copy instead of editing the app-managed original.

If you explicitly enabled the optional Windows Event Log diagnostic lookup, **Activity** can also show an `Unverified Windows Event Log record`. It is a local, read-only lookup of recent `Application Error` records for this executable, not an upload or a root-cause determination. The displayed Record ID, UTC time, exception code, and sanitized module basename are useful for a report, but do not attach raw Event Viewer exports or full event XML without reviewing them for local information. You can revoke the consent from Activity; revocation clears the visible record immediately. If saving the revoke fails, the app reports that it is session-only and keeps lookups disabled for the current run.

The absence of a crash report does not rule out a native crash, forced termination, out-of-memory termination, anti-cheat action, or another external stop. If the app disappeared without a report, also include:

- whether the process remained visible in Task Manager
- whether the tray icon remained available
- the relevant Reliability Monitor or Event Viewer entry
- the faulting module and exception code, if Windows recorded them

## Security issues

Do not report security issues in public GitHub issues.

See [SECURITY.md](SECURITY.md) for private reporting instructions.
