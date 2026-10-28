# Built-in application process names

New Windows GUI imports recognize20application families and fill the existing **Tracked process names** list. You can remove names in Application rule. Removed names are not added again when the rule is opened, started or reloaded.

| Application entry | Default tracking |
| --- | --- |
| Steam client `steam.exe` | Its exact main executable and `steamwebhelper.exe` in the selected installation's supported CEF directories. Steam games keep their separate AppID rules. |
| Discord Stable `Discord.exe` in `Discord/app-<version>` | All matching instances of that exact executable. Chromium workers can share the same name. A direct versioned path is not silently changed after an update. |
| Discord Stable shortcut through root `Discord.exe` or `Update.exe --processStart Discord.exe` | Real `Discord.exe` instances in that installation's immediate version directories. The root stub/updater is only a launcher. |

The catalog uses finite layouts. Other channels, shared runtimes and services have no blanket defaults. AUMID entries retain their existing package tracking; the installed-app picker's file-based entries use the same new-import defaults as Add file and file drop.

## Tracking and controls

Generated helper names remain tied to the recognized installation and the current interactive logon. Exact PID/creation identity is rechecked before publication. Another installation's helper, a game under `steamapps`, or an unrelated process with the same basename is not automatically included. A detached generated helper cannot keep the Steam client running. The existing optional descendant setting remains an explicit user choice.

CPU, private resident RAM and scheduling use the same tracked membership. Overlapping rules can show the same process in each application, while a group's RAM and tracked CPU union count it once. The existing collector and sampling cadence are unchanged. Whole-machine process CPU, per-CPU scheduler load and system load retain their different scopes and intervals.

Discord's root launcher or standard updater is launched as an ordinary desktop process without the application's affinity or priority. Discovery binds the actual application processes. A bounded five-minute launch intent permits initial scheduling/readback with Process monitor Off, including late members; it does not become continuous enforcement. Launch acknowledgement alone is not proof that the client started.

Focus first tries actual windows belonging to verified application members. If a Steam/Discord client has hidden its window, the program requests normal client activation through the selected executable or Discord's qualified launcher. This does not apply scheduling settings. The log distinguishes a focused window from an activation request; Windows and the client still control foreground behavior.

## Saved settings and MCP

Existing rules are not expanded. To use defaults on an existing application, add a new entry or keep managing its process list manually. Explicit manually added names outside generated provenance retain their existing matching behavior.

Schema 13 stores a small optional `builtin_process_binding` with the catalog ID, definition version and generated-name provenance. The visible list determines which generated names are enabled. Removing/re-adding a generated name retains its installation scope. Unknown or incompatible definitions stay inert and preserved. An incompatible launch target does not convert generated names into global matches. New catalog releases do not silently add names to saved rules.

Older configurations load without eager rewriting; a successful explicit save upgrades their schema. Older binaries cannot safely interpret new provenance and must not be used to downgrade a schema-13 state. Unexpected binding evidence in an older schema is retained behind a save refusal.

MCP `create_rule` and `resolve_installed_app` remain explicit and receive no GUI defaults. `update_rule` replaces the visible list exactly and retains the existing internal provenance; an empty list does not refill defaults. The metadata in `list_rules` is read-only and is not an editable input field.

`launch_saved` / `restart_saved` for these built-in entries return ordinary `activation_requested` acknowledgements, without a fabricated launcher PID or Run ID. Elevated activation and inherited output capture are unavailable for this route. Use the tracked roster and exact process APIs after discovery. Existing unbound EXE rules retain their managed-launch contract; `launch_program` remains available for an independent invocation.

## Validation limits

Deterministic regressions cover import boundaries, removal/restart, old-schema preservation, foreign installations/logons, PID reuse, helper retirement, Discord version directories, coalescing, Focus, one-shot scheduling, MCP replacement and CPU/RAM unions.

`scripts/test-windows-known-apps.ps1` prepares bounded owned desktop clients for the ignored native acceptance test. It exercises the real launch, discovery, scheduling, window and telemetry boundaries, including ordinary root-stub and legacy-updater rights, CPU19 when available, hidden-window activation, a foreign helper and overlapping rules. The fixture product strings test classification, not cryptographic publisher trust. This probe needs an interactive Windows desktop and elevation for ETW; it is separate from ordinary CI.

Owned clients do not certify every current or future Steam/Discord build. Real-app acceptance should cover the installed shortcut and layout, updates, tray/minimized behavior and exact telemetry/scheduling readback. No hooks, process injection, private application API or additional sampler is introduced.

## Additional18profiles

The data table is `src/app/features/rules/known_apps/catalog.rs`. Every row defines finite main/launcher/helper paths and PE company evidence; filenames or display names alone do not enable a profile. Only files actually present inside the selected canonical installation are inserted on new import. Known names retain scope if manually enabled later. Numeric version components are bounded; there is no arbitrary recursive wildcard or folder scan during monitoring.

| Application | Main / optional ordinary launcher | Conditional app-owned helpers |
| --- | --- | --- |
| Epic Games Launcher | Portal/Binaries/Win64 or Win32/EpicGamesLauncher.exe | EpicWebHelper.exe in the listed Engine/Portal binary paths |
| Battle.net | Battle.net.exe; Battle.net Launcher.exe | Battle.net Helper.exe, including numeric Battle.net version folders; excludes ProgramData Agent |
| EA app | EADesktop.exe; EALauncher.exe | EACefSubProcess.exe; excludes EA background/local-host services |
| Ubisoft Connect | upc.exe / UbisoftConnect.exe; Uplay / UbisoftGameLauncher | UbisoftWebCore.exe in the client root |
| Riot Client | RiotClientServices.exe | UX/RiotClientUx.exe / RiotClientUxRender.exe; no League/VALORANT/Vanguard |
| GOG GALAXY | GalaxyClient.exe | GalaxyClient Helper.exe in that installation; no Galaxy service or game |
| Rockstar Games Launcher | Launcher.exe; LauncherPatcher.exe | Main client only; external Social Club and library service excluded |
| OBS Studio | bin/64bit/obs64.exe or bin/32bit/obs32.exe | Matching finite obs-plugins browser-page paths |
| Logitech G HUB | lghub.exe / system_tray/lghub_system_tray.exe | lghub_agent.exe; updater excluded |
| SteelSeries GG | SteelSeriesGG.exe | SteelSeriesGGClient.exe / SteelSeriesGGEZClient.exe; audio/driver modules excluded |
| Razer Synapse | RazerAppEngine.exe | Main-image instances only; no guessed shared engine/services |
| New Microsoft Teams | ms-teams.exe | Main-image instances only; external shared WebView2 excluded |
| Zoom Workplace | bin/Zoom.exe, including numeric bin_ folders | CptHost, CptControl, zWebview2Agent and ZoomHybridConf within that installation; services/updaters excluded |
| New Outlook | olk.exe | Main-image instances only; external shared WebView2 excluded |
| Adobe Creative Cloud | ACC/Creative Cloud.exe | Listed local CEF/Creative Cloud helper paths; shared Adobe IPC/sync/update processes excluded |
| Visual Studio | Common7/IDE/devenv.exe | Listed local ServiceHub Roslyn paths; debugged apps, terminals and shared build workers excluded |
| NVIDIA App | CEF/NVIDIA app.exe | Listed local NVIDIA Overlay paths; containers/driver services excluded |
| AMD Software | RadeonSoftware.exe | Local QtWebEngineProcess.exe; driver/capture/PresentMon tools excluded |

Existing Steam/Discord bindings keep version1 and their stored names. Schema13 is unchanged. EA/Battle.net/Ubisoft/Rockstar launcher aliases reuse ordinary handoff and the existing real-main one-shot readback; intermediate launchers are excluded from membership. MCP uses the same route. App/group CPU/RAM still consume the existing admitted identity set; no sampler, dependency or settings panel is added.

### Evidence and limitations

Vendor references establish process roles/names where published, not universal current install paths: [Epic](https://www.epicgames.com/help/c-32735058/c-Trending_0/kod-pomilki-4004-pid-chas-pridbannya-cherez-platizhnu-platformu-epic-a17546390), [Rockstar launcher](https://support.rockstargames.com/articles/4AuSUO4f7o2ba96rfALqDl/game-update-in-the-rockstar-games-launcher-stops-completely-and-returns-an-error), [Logitech](https://support.logi.com/hc/en-in/articles/360036179173-G-HUB-freezes-while-loading-and-logo-animation-loops), [SteelSeries](https://support.steelseries.com/hc/en-us/articles/360060998272-Is-GG-not-opening), [Razer](https://mysupport.razer.com/app/answers/detail/a_id/1620/kw/macro%20modules), [Zoom](https://support.zoom.com/hc/en/article?id=zm_kb&sysparm_article=KB0058758), [Teams](https://learn.microsoft.com/microsoftteams/troubleshoot/teams-administration/include-exclude-teams-from-antivirus-dlp), [shared WebView2](https://learn.microsoft.com/en-us/deployoffice/webview2-install), [Adobe](https://helpx.adobe.com/x-productkb/global/adobe-background-processes.html). The local AMD UI/Qt filenames and metadata were read directly without activation. Other finite helper variants are conditional candidates: a matching app-owned file must exist before insertion; absence does not add a speculative name.

Automated cases cover each of18data rows, foreign installations, excluded components, version directories, launcher handoff, removals and retained provenance. A separate opt-in test builds owned PE/layout fixtures without executing them to check native import classification; it does not certify all20vendor installations or launch/update/tray behavior. Unlisted layouts receive no guessed helper. AUMID entries keep existing package tracking rather than receiving EXE defaults. Shared WebView2 attribution and cross-product services require different ownership evidence and are outside this simple catalog.

For the additional18profiles, Focus restores an existing eligible window only. It does not invoke the EXE again if no window is found: some applications, including Visual Studio, support multiple independent instances. This avoids creating duplicates; hidden-to-tray activation outside Steam/Discord is not claimed.


## A process listed in more than one app

A repeated name is a hint, not proof that the same process is shared: two installations can contain different `helper.exe` files. The rule editor lists other apps using that name. If the same running process really belongs to rules requesting different settings, the editor shows its controlling app and **Take control here**. Save Changes commits that choice; Cancel discards it. No process names are removed from either app.

An earlier saved rule keeps control when a new rule overlaps it. Conflicts already present before this policy require a choice of controlling app; the main row offers **Resolve**. Rules requesting equal settings share one writer without a warning. The selected app supplies affinity and priority for that executable only while it actually tracks the process. If it no longer claims an instance, another eligible rule can control it. Removing the selected rule clears its preference.

CPU and RAM remain visible in every app that tracks the instance. Group RAM and tracked CPU unions still count shared instances once within their respective scope; app rows should not be added together to estimate a unique total. Changing the controller does not move an application between groups or enable Process monitor. While automatic monitoring is Off, the choice governs subsequent explicit launches/Fix and existing bounded launch readback.


Different instances of one executable do not imply a shared-process conflict. Ownership notices keep the exact overlapping rule sets separate. When two independent sets need different controllers, choosing one does not erase the other set's saved preference: the latest eligible owner is used for each instance. Process lists and telemetry membership remain unchanged.
