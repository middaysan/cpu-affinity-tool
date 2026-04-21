# Comparison

This document compares CPU Affinity Tool with common alternatives for CPU-affinity workflows on Windows.

## Quick summary

CPU Affinity Tool is a focused Windows utility for saved affinity and priority rules.

It is useful when you want:

- repeatable launch rules
- a small GUI focused on CPU affinity and priority
- monitoring that can re-apply settings after launch

It is not trying to be a full replacement for every Windows process automation tool.

## Comparison table

| Tool | Saved launch rules | Monitoring / re-apply | Convenience | Complexity | Cost / availability | Scope |
| --- | --- | --- | --- | --- | --- | --- |
| CPU Affinity Tool | Yes | Yes | High for the supported workflow | Low | Open source, self-hosted binary | Narrow, focused affinity and priority workflow |
| Task Manager | No | No | Medium for one-off changes | Low | Included with Windows | Manual one-shot process tweaks |
| Process Lasso | Yes | Yes | High | Medium | Third-party product | Broad Windows process automation and policy tooling |
| PowerShell / CLI methods | Script-dependent | Script-dependent | Low without custom scripts | High | Built-in or free tooling | Fully manual or scripted workflows |

## Notes by alternative

### Task Manager

Task Manager is useful for quick one-off experiments.

It is not a good fit when you want:

- repeatable launch behavior
- saved rules
- automatic correction after launch

### Process Lasso

Process Lasso covers a broader Windows process-management space and has a more mature automation surface.

CPU Affinity Tool is narrower:

- fewer moving parts
- focused UI
- open-source codebase
- easier to audit for the specific affinity workflow it supports

### PowerShell and command-line approaches

Manual scripts can be precise, but they require more setup and maintenance.

They are a good fit if you:

- prefer scripting over GUI tools
- want full manual control
- already manage your system through automation

They are a poor fit if you want a ready-to-use saved-launch workflow.
