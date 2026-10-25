# System Tweaks

System Tweaks provides individual Windows settings with recorded Undo values. Open **System Tweaks** from the main navigation, read a setting's help, and apply it only when it suits your PC. Opening the page changes nothing. CPU affinity and process priority remain in **Affinity groups**.

The page has three sections: **System**, **GPU / network**, and **Check PC**. There is no automatic tuning bundle. A changed Windows preference does not establish a higher frame rate or lower latency; compare the same workload before and after a change.

## Settings you can apply

The System section contains the game and comfort settings below. **Game GPU** is in GPU / network. Availability depends on Windows, the hardware and the selected target; the app shows unavailable settings and failures.

| Setting | What its action does |
| --- | --- |
| **Performance on AC power** | Asks Windows to prefer performance while plugged in. Battery preferences stay unchanged. This does not lock CPU frequency; power use and heat may increase. |
| **Core parking** | Opens a scope chooser, then disables parking in the captured AC plan/mode policy for all cores. Separate classes are detected but shown as unavailable. Undo restores exact prior values. |
| **Game Mode** | Enables the stored Windows Game Mode preference. Restart the game after changing it. |
| **Background clips** | Turns off the **Record what happened** background buffer. Game Bar, screenshots and manual recording remain available. |
| **Windowed games** | Enables Windows optimizations for compatible DX10/11 windowed and borderless games. Restart the game. Auto HDR, VRR and per-app exceptions are preserved. |
| **Screen refresh rate** | Tests and temporarily applies the highest reported compatible refresh rate at the current resolution. Other screens are preserved. Confirm within **15 seconds**, or the app attempts to restore the previous mode. |
| **Game GPU** | Requests the high-performance GPU for one selected saved executable. Restart the game and check which GPU it actually uses. |
| **Shift and NumLock popups** | Turns off activation shortcuts for inactive Sticky, Filter and Toggle Keys. Features already in use and their shortcuts are preserved. |
| **Mouse acceleration** | Turns off Windows pointer acceleration. Games using raw mouse input usually ignore this setting. |

Comfort settings are preference choices, without an FPS gain. Screen refresh rate is the desktop's nominal Hz, not measured game FPS.

### Core parking

On Windows 11, choose **Core parking → Choose…**, select **All cores** and press **Apply**. Nothing changes when opening the card or chooser. Counts come from the current physical-core/SMT topology; CPU model names are not a lookup table. When Windows reports a separate performance-core class, the chooser shows its count with a disabled option and an explanation.

Separate-class activation is unavailable in this build, including through MCP. On the tested Windows 11 system, `CPMINCORES1=100` was stored and read back correctly, but Windows subsequently parked those cores again. This also occurred on disposable Balanced/Atlas plans with heterogeneous policy variants. The all-core minimum kept all 20 logical CPUs unparked. These are host-specific functional observations, not a claim that every Windows/CPU combination behaves the same. The app does not advertise an ineffective class-only control or alter extra scheduler policies to force it.

The base `CPMINCORES` setting is a shared minimum, so there is no separate **E-cores only**, chiplet, socket or arbitrary CPU-number control. Other policy values can already keep additional cores available. Unsupported or uncertain mappings remain unavailable instead of guessing a control. The selected minimum and, when needed, maximum unparked percentages are set to 100; frequency, boost, EPP, affinity and DC/battery settings are unchanged.

The setting captures exact plan and AC-mode keys when applied. It does not follow a later plan/mode change or continuously enforce policy. Undo restores the captured values, including an originally absent override, and preserves external changes. Use Undo before choosing another scope. **Stored parking policy verified** confirms settings readback, not permanent physical activity, disabled C-states or a measured performance gain. Power use and heat may increase.

References: [Microsoft CPMinCores](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/options-for-core-parking-cpmincores), [processor classes and power settings](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/configure-processor-power-management-options).

The development branch also contains a session-only background-load preview in each **Application rule**, with one default-Off checkbox and a fixed priority/EcoQoS policy. There is no Advanced/core-placement UI. It follows that saved EXE instead of creating persistent Manual ownership. It has no separate System Tweaks card. System-role mutation is currently not admitted pending controlled VM/function validation. See [background-workload controls, ownership and validation limits](background-workload.md).

## Undo and persistence

Before applying a setting, the app reads and records the original value. **Manual changes remain after closing or restarting the app.** Use the setting's Undo action, or **Undo my changes** to review restoring the changes managed by this tool.

Undo restores a field only when its current value still matches the tool's verified change. Later changes by Windows or another program are preserved, and skipped or uncertain restoration is shown. Some restored preferences need a game or Windows restart; follow the result shown in the app. Older applied settings can remain available for recovery and Undo even when they are no longer offered as new tweaks.

## Guides and checks

**Memory integrity** opens Windows Security's Core isolation page. The app does not read or change this security setting, record its original value, or manage its Undo. Make and restore any choice in Windows itself; disabling it reduces kernel protection and does not guarantee a performance gain.

**GPU / network** also provides hardware and network guidance. These guides do not change adapter properties, restart devices or apply driver settings.

In **Check PC**, choose **Check PC** to request read-only hardware information. Missing or uncertain details remain marked as such. For example, reported memory speed and occupied slots do not establish channel mode or an active XMP/EXPO profile. The check does not change BIOS settings, install drivers, or measure game FPS and input latency.

For agent access to supported System Tweaks operations, see the [MCP guide](mcp.md).
