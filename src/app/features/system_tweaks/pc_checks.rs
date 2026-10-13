//! Pure compact summaries and help for explicit, read-only PC checks.

use os_api::pc_checks::{PcSnapshot, RamConfiguration};

const MEMORY_HELP: &str = "This is a read-only snapshot of available physical RAM and system committed memory. Available RAM includes reusable standby pages, not only empty pages. Commit is allocated memory backed by RAM or page files; it is different from resident RAM. The peak is the system peak since boot, not the selected game's peak. One snapshot does not diagnose a leak, paging-related stutter or an FPS problem.\n\n1. Reproduce the same game scene and refresh while the problem occurs.\n2. In Task Manager, compare Memory and Committed; inspect which applications grow.\n3. If memory pressure correlates with stutter, save work and close an application you do not need. Keep the page file available.\n\nThis check does not trim working sets, purge caches, close applications or change the page file.";
const DOWNLOADS_HELP: &str = "Download activity is not measured by this check. It does not capture traffic or inspect accounts.\n\n1. During the problem, sort Task Manager's Processes list by Network and also check Disk. Resource Monitor can show the same activity in more detail.\n2. Check game launchers, Windows Update and cloud-sync applications. Pause a specific download in its own application only if it coincides with the problem.\n3. Repeat the same scene and resume the download afterwards.\n\nA background window can still be playing audio, recording or downloading. This app does not disable services, updates or networking.";
const RAM_HELP: &str = "Configured speed and occupied memory devices come from the PC's SMBIOS firmware when it provides valid data. The report uses Configured Memory Speed in MT/s; a module's advertised maximum speed is not substituted. Firmware data can be incomplete or wrong. Firmware slots can also represent soldered memory. Two occupied devices do not prove dual-channel wiring. Channel mode, timings and the active XMP/EXPO profile remain unknown.\n\n1. Compare speed and occupied slots with Task Manager > Performance > Memory.\n2. Check the motherboard's recommended slot placement and a supported vendor tool.\n3. XMP/EXPO is memory overclocking: check model compatibility and stability separately.\n\nThis app does not read or modify BIOS profiles, timings or voltages. Faster memory does not guarantee a proportional FPS increase.";
const BOOST_HELP: &str = "CPU/GPU clocks, temperature and power/current/thermal limits are not measured under game load. Idle clocks do not establish throttling.\n\n1. Repeat the problematic scene with the same graphics settings.\n2. Use a supported hardware/vendor tool to observe effective clocks, temperatures and any reported limit flags under that load.\n3. Match the measurements in time with the stutter. If a limit is not reported, keep it unknown.\n\nThis check does not poll sensors, raise power limits, change PBO, voltages or cooling. A clock reading by itself does not establish the cause of low FPS.";
const DRIVERS_HELP: &str = "CPU and display-adapter names are inventory only. Installed driver versions, driver health, chipset setup and the game's actual GPU/core routing are unverified.\n\n1. Compare chipset and graphics drivers with the hardware vendor's instructions for your model and Windows version. A new version alone does not prove a fix.\n2. On a mixed-CCD Ryzen X3D model, check the vendor's game-recognition setup and actual game scheduling under load. Single-CCD 7800X3D / 9800X3D models do not need a check for a second CCD without V-Cache.\n3. Preserve the vendor's required Game Mode, game-recognition and parking behavior instead of applying a blanket disable recipe.\n\nAffinity and priority remain in CPU Groups. This check does not install drivers, change core routing or create another game list.";
const GAME_HELP: &str = "The game's graphics, frame limiter, launch arguments, overlays and recording settings are not inspected.\n\n1. Match resolution, graphics quality and upscaling before comparing FPS. Check the game's limiter, V-Sync and display mode, plus any driver or external cap.\n2. Check active recording and overlays, including their browser components. Change one setting at a time and remember the original value.\n3. If supported by the game, try its latency setting and evaluate responsiveness separately from FPS. Frame generation does not provide an equal gain in responsiveness.\n4. Compare the same scene, versions and settings after shader-cache warm-up with several repeated measurements.\n\nThis check does not parse game files, disable recording or close applications. Affinity and priority remain in CPU Groups.";

pub(crate) fn collect() -> Vec<PcCheck> {
    report(&os_api::pc_checks::snapshot())
}

#[derive(Clone, Debug)]
pub(crate) struct PcCheck {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: String,
    pub help: &'static str,
}

fn report(snapshot: &PcSnapshot) -> Vec<PcCheck> {
    let memory = match &snapshot.memory {
        Ok(memory) => {
            let physical = format!(
                "{:.1} / {:.1} GiB available",
                gib(memory.physical_available_bytes),
                gib(memory.physical_total_bytes)
            );
            let commit = match &memory.commit {
                Ok(commit) => format!(
                    "Commit {:.1} / {:.1} GiB · boot peak {:.1} GiB",
                    gib(commit.used_bytes),
                    gib(commit.limit_bytes),
                    gib(commit.peak_bytes)
                ),
                Err(_) => "Commit unavailable".into(),
            };
            format!("{physical}\n{commit}")
        }
        Err(error) => format!("Unavailable · {error}"),
    };
    let ram = match &snapshot.ram {
        Ok(ram) => ram_summary(ram),
        Err(error) => format!("Unavailable · {error}\nChannels and XMP/EXPO unknown"),
    };
    let cpu = snapshot
        .cpu_model
        .as_deref()
        .map(compact_name)
        .unwrap_or_else(|| "CPU unknown".into());
    let graphics = match &snapshot.graphics_adapters {
        Ok(names) if !names.is_empty() => {
            let mut visible = names
                .iter()
                .take(3)
                .map(|name| compact_name(name))
                .collect::<Vec<_>>()
                .join(" / ");
            if names.len() > 3 {
                visible.push_str(" / more adapters");
            }
            visible
        }
        _ => "Display adapters unknown".into(),
    };
    vec![
        check("memory", "Is there enough memory?", memory, MEMORY_HELP),
        check(
            "downloads",
            "Background downloads",
            "Not measured · check Network during stutter".into(),
            DOWNLOADS_HELP,
        ),
        check("ram", "RAM configuration", ram, RAM_HELP),
        check(
            "boost",
            "CPU / GPU clocks and limits",
            "Not measured under load · temperatures and limits unknown".into(),
            BOOST_HELP,
        ),
        check(
            "drivers",
            "Drivers and X3D cores",
            format!("{cpu}\n{graphics}\nDrivers and routing unverified"),
            DRIVERS_HELP,
        ),
        check(
            "gameconfig",
            "Game graphics, limiter and recording",
            "Not inspected · check the game's settings".into(),
            GAME_HELP,
        ),
    ]
}

fn ram_summary(ram: &RamConfiguration) -> String {
    let modules = match ram.populated_modules {
        Some(count) => format!("{count} / {} firmware slots", ram.reported_slots),
        None => format!(
            "{} firmware slots · occupied count unknown",
            ram.reported_slots
        ),
    };
    let speed = if ram.configured_speeds_mt_s.is_empty() {
        "Configured speed unknown".into()
    } else {
        let values = ram
            .configured_speeds_mt_s
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" / ");
        let partial = if ram.unknown_speed_modules > 0 {
            format!(" · {} module speed unknown", ram.unknown_speed_modules)
        } else {
            String::new()
        };
        format!("{values} MT/s{partial}")
    };
    format!("{modules} · {speed}\nChannels and XMP/EXPO unknown")
}

pub(crate) fn hardware_guidance() -> Vec<PcCheck> {
    vec![
        check("latency", "Latency in supported games", "Reflex / Anti-Lag 2 / XeLL · manual guidance".into(),
            "Check the supported game's graphics menu for NVIDIA Reflex, AMD Anti-Lag 2 or Intel XeLL. These require game/hardware support. Anti-Lag 2 is an in-game feature; ordinary Anti-Lag is a separate game-profile control in AMD Software. If Reflex is available, use the game's integration. Driver low-latency support also depends on the API and driver; do not assume DX12 is always excluded.\n\nRemember the original setting, compare the same scene and restore it in the same place if it does not help. Reduced latency need not raise FPS. For G-SYNC/FreeSync, frame caps, V-Sync and frame generation, follow your game/vendor's supported combination and measure responsiveness separately. This guide does not enable vendor controls."),
        check("shader", "Shader preparation", "Check support in the vendor app or game store".into(),
            "Supported shader preparation can reduce waits or compilation stutter; it does not guarantee a general FPS increase. NVIDIA App may offer Auto Shader Compilation for supported known DirectX 12 shaders after a driver update. Microsoft Advanced Shader Delivery delivers prepared shaders for supported hardware, drivers, Windows builds and games through supported storefronts. Check the installed game for 'Precompiled shaders installed'; a GPU name alone does not prove eligibility. Vendor rollout announcements do not prove that your installed game supports the feature.\n\nFollow the vendor's current instructions and check the game after an update. This app does not clear or rebuild shader caches. Prepared or delivered cache content has no exact undo, so do not routinely delete useful caches."),
        check("mux", "Laptop GPU output", "MUX / Advanced Optimus · model-dependent guidance".into(),
            "Some laptops support direct display output from the discrete GPU through a MUX or NVIDIA Advanced Optimus. Check your laptop vendor's utility or the supported NVIDIA display controls; the option depends on the model and wiring. A mode can require a restart and consume more battery power. Remember the original mode and restore it in the same utility.\n\nWindows per-game GPU preference does not switch a MUX and does not prove which GPU actually renders the game. Verify the game's renderer and compare the same scene. This app does not switch laptop display routing."),
        check("ethernet", "Ethernet interrupt moderation", "Optional adapter test · game effect unmeasured".into(),
            "On a supported wired adapter, Interrupt Moderation combines interrupts. Disabling it can trade lower small-packet delay for higher CPU overhead; it does not promise a lower internet ping. Use Device Manager > Network adapters > your wired adapter > Properties > Advanced only when the vendor exposes the documented property.\n\nRecord the exact original value, compare the same workload, and restore that value if it does not help. A property change or restoration can briefly interrupt the connection. Do not change an adapter during a remote session you depend on.\n\nThis app does not write network properties, restart devices, capture traffic or apply a bundle of RSS, checksum offload, EEE and other changes. Wi-Fi and unsupported adapters are outside this guide."),
    ]
}

fn check(id: &'static str, title: &'static str, summary: String, help: &'static str) -> PcCheck {
    PcCheck {
        id,
        title,
        summary,
        help,
    }
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1_u64 << 30) as f64
}

fn compact_name(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use os_api::pc_checks::{CommitSnapshot, MemorySnapshot, PcCheckError};

    fn unknown() -> PcSnapshot {
        PcSnapshot {
            memory: Err(PcCheckError::Unsupported),
            ram: Err(PcCheckError::Unsupported),
            cpu_model: None,
            graphics_adapters: Err(PcCheckError::Unsupported),
        }
    }

    #[test]
    fn pc_checks_always_reports_six_topics_and_does_not_claim_unknown_health() {
        let checks = report(&unknown());
        assert_eq!(
            checks.iter().map(|check| check.id).collect::<Vec<_>>(),
            [
                "memory",
                "downloads",
                "ram",
                "boost",
                "drivers",
                "gameconfig"
            ]
        );
        for check in &checks {
            assert!(!check.summary.is_empty());
            assert!(!check.help.is_empty());
        }
        assert!(checks[0].summary.contains("Unavailable"));
        assert!(checks[1].summary.contains("Not measured"));
        assert!(checks[3].summary.contains("Not measured under load"));
        assert!(checks[4].summary.contains("unverified"));
        assert!(checks[5].summary.contains("Not inspected"));
    }

    #[test]
    fn pc_checks_current_memory_is_distinct_from_commit_and_game_peak() {
        let mut snapshot = unknown();
        snapshot.memory = Ok(MemorySnapshot {
            physical_total_bytes: 32 << 30,
            physical_available_bytes: 8 << 30,
            commit: Ok(CommitSnapshot {
                used_bytes: 12 << 30,
                limit_bytes: 48 << 30,
                peak_bytes: 20 << 30,
            }),
        });
        let checks = report(&snapshot);
        assert!(checks[0].summary.contains("8.0 / 32.0 GiB available"));
        assert!(checks[0].summary.contains("Commit 12.0 / 48.0 GiB"));
        assert!(checks[0].help.contains("since boot"));
        assert!(checks[0].help.contains("does not diagnose"));
    }

    #[test]
    fn pc_checks_partial_memory_read_keeps_commit_unknown() {
        let mut snapshot = unknown();
        snapshot.memory = Ok(MemorySnapshot {
            physical_total_bytes: 16 << 30,
            physical_available_bytes: 4 << 30,
            commit: Err(PcCheckError::Native {
                operation: "GetPerformanceInfo",
                code: 5,
            }),
        });
        let summary = &report(&snapshot)[0].summary;
        assert!(summary.contains("4.0 / 16.0 GiB available"));
        assert!(summary.contains("Commit unavailable"));
    }

    #[test]
    fn pc_checks_ram_facts_never_prove_channels_or_overclock_profile() {
        let ram = RamConfiguration {
            reported_slots: 4,
            populated_modules: Some(2),
            configured_speeds_mt_s: vec![4800],
            unknown_speed_modules: 0,
        };
        let summary = ram_summary(&ram);
        assert!(summary.contains("2 / 4 firmware slots"));
        assert!(summary.contains("4800 MT/s"));
        assert!(summary.contains("Channels and XMP/EXPO unknown"));
        let unknown = ram_summary(&RamConfiguration {
            populated_modules: None,
            configured_speeds_mt_s: vec![],
            unknown_speed_modules: 1,
            ..ram
        });
        assert!(unknown.contains("occupied count unknown"));
        assert!(unknown.contains("Configured speed unknown"));
    }

    #[test]
    fn pc_checks_mixed_ram_speeds_preserve_partial_unknown_configuration() {
        let summary = ram_summary(&RamConfiguration {
            reported_slots: 4,
            populated_modules: Some(3),
            configured_speeds_mt_s: vec![4800, 5200],
            unknown_speed_modules: 1,
        });
        assert!(summary.contains("4800 / 5200 MT/s"));
        assert!(summary.contains("1 module speed unknown"));
    }

    #[test]
    fn pc_checks_hardware_names_do_not_verify_drivers_or_x3d_routing() {
        let mut snapshot = unknown();
        snapshot.cpu_model = Some("AMD Ryzen 9 9950X3D".into());
        snapshot.graphics_adapters = Ok(vec!["NVIDIA GeForce RTX 4090".into()]);
        let checks = report(&snapshot);
        assert!(checks[4].summary.contains("9950X3D"));
        assert!(checks[4].summary.contains("RTX 4090"));
        assert!(checks[4].summary.contains("unverified"));
        assert!(checks[4].help.contains("mixed-CCD"));
        assert!(checks[4].help.contains("7800X3D / 9800X3D"));
    }

    #[test]
    fn pc_checks_hardware_guidance_keeps_latency_shader_mux_and_ethernet_limits() {
        let guides = hardware_guidance();
        assert_eq!(
            guides.iter().map(|guide| guide.id).collect::<Vec<_>>(),
            ["latency", "shader", "mux", "ethernet"]
        );
        assert!(guides[0].help.contains("Anti-Lag 2"));
        assert!(guides[0].help.contains("AMD Software"));
        assert!(guides[1].help.contains("Precompiled shaders installed"));
        assert!(guides[1].help.contains("does not clear"));
        assert!(guides[2].help.contains("does not switch a MUX"));
        assert!(guides[3].help.contains("CPU overhead"));
        assert!(guides[3].help.contains("does not write"));
    }
}
