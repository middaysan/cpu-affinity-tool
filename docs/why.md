# Why This Tool Exists

CPU Affinity Tool exists for users who want repeatable CPU-affinity and priority rules without redoing the same manual setup every launch.

## What problem it targets

Some systems behave better when the foreground workload and the background workload do not compete for the same CPU cores.

Typical examples:

- a game plus a browser
- a game plus Discord and launchers
- a game plus recording or streaming tools
- heavy work software running beside other background applications

## Where it can help

This tool is most useful when:

- the active workload is CPU-bound
- background apps are actually consuming meaningful CPU time
- the system has hybrid cores or segmented CPU topology
- you want the same launch layout every time

## Where it will not magically help

This tool is not a guaranteed FPS booster.

It will not reliably help when:

- the bottleneck is the GPU
- background load is already light
- the application ignores or quickly overrides affinity settings
- the Windows scheduler already handles the workload well enough

## Why not just use Task Manager

Task Manager is fine for one-off experiments, but it is inconvenient when you want:

- saved rules
- a repeatable launch flow
- automatic correction after launch

## Why this project stays narrow

The project is intentionally focused on a smaller problem:

- affinity and priority rules
- repeatable launch behavior
- simple Windows-first workflow

It does not try to become a general-purpose system optimizer or a catch-all gaming performance suite.
