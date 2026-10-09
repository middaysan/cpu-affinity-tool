//! Readonly cards. Observation and actions run on the application worker; editors
//! render only cached immutable text and never call native inspection APIs.
mod measurements;

use crate::app::features::tuning::{
    platform::{
        codec::{EnvelopeKind, JsonCodec, ModuleEnvelope, VersionedCodec},
        contract::*,
        registry::{FieldChange, Module, ModuleObservation, Registry},
    },
    presentation::{ModuleAction, ModuleEditor, ModuleSection},
};
use os_api::gaming_diagnostics::{self as native, DiagnosticError, SettingsPage};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
enum Kind {
    Measurements,
    Display,
    Latency,
    Audio,
    Irq,
    Usb,
    GameMode,
    Capture,
    Windowed,
    Gpu,
}
impl Kind {
    const ALL: [Self; 10] = [
        Self::Measurements,
        Self::Display,
        Self::Latency,
        Self::Audio,
        Self::Irq,
        Self::Usb,
        Self::GameMode,
        Self::Capture,
        Self::Windowed,
        Self::Gpu,
    ];
    fn id(self) -> &'static str {
        match self {
            Self::Measurements => "measurements",
            Self::Display => "display_audit",
            Self::Latency => "latency_advisor",
            Self::Audio => "audio_diagnostics",
            Self::Irq => "irq_diagnostics",
            Self::Usb => "usb_topology",
            Self::GameMode => "game_mode",
            Self::Capture => "background_capture",
            Self::Windowed => "windowed_optimizations",
            Self::Gpu => "gpu_preference",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Measurements => "Compare game results",
            Self::Display => "Display information",
            Self::Latency => "Input delay guide",
            Self::Audio => "Audio information",
            Self::Irq => "Device interrupts",
            Self::Usb => "USB connections",
            Self::GameMode => "Game Mode",
            Self::Capture => "Background recording",
            Self::Windowed => "Windowed game optimizations",
            Self::Gpu => "Game graphics card",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Measurements => "Import two benchmark results to compare game smoothness.",
            Self::Display => "Check screen resolution, refresh rate and graphics cards.",
            Self::Latency => "Review options that may affect input delay.",
            Self::Audio => "View active sound devices and apps using audio.",
            Self::Irq => "Inspect device interrupt settings without changing them.",
            Self::Usb => "See which USB devices share a controller.",
            Self::GameMode => "Review Windows Game Mode settings.",
            Self::Capture => "Review background recording in Windows.",
            Self::Windowed => "Review Windows options for games played in a window.",
            Self::Gpu => "Choose a game's graphics card in Windows Settings.",
        }
    }
    fn settings(self) -> Option<SettingsPage> {
        match self {
            Self::Measurements => None,
            Self::Display => Some(SettingsPage::Display),
            Self::Latency => Some(SettingsPage::GpuPreference),
            Self::Audio => Some(SettingsPage::Sound),
            Self::Irq | Self::Usb => Some(SettingsPage::Devices),
            Self::GameMode => Some(SettingsPage::GameMode),
            Self::Capture => Some(SettingsPage::BackgroundCapture),
            Self::Windowed => Some(SettingsPage::WindowedOptimizations),
            Self::Gpu => Some(SettingsPage::GpuPreference),
        }
    }
    fn guidance(self) -> &'static str {
        match self {
            Self::Measurements => "Use warmed-up repeated A/B or ABBA runs, one change and the same scene/build/settings. Frame times report throughput and pacing; they do not certify click-to-photon latency.",
            Self::Display => "Nominal desktop Hz is not measured game FPS. VRR/DRR, HDR, active game mode and exact fractional refresh are not established by this inventory; inspect native Display/Graphics Settings and the monitor menu.",
            Self::Latency => "Use game-integrated NVIDIA Reflex or AMD Anti-Lag 2 only in supported games. Driver queue controls, frame caps, VRR and V-Sync trade throughput, queuing and tearing; benchmark one change at a time. This app does not enable vendor controls. HAGS and core-parking changes remain native/advanced guidance. Timer resolution, QPC precision, ISR/DPC placement and mouse report rate are different mechanisms. A timer request in a tray app does not prove a game receives finer resolution or lower input delay; global timer/HPET/BCD tweaks are excluded.",
            Self::Audio => "A session PID identifies an audio client; a multiprocess session reports only its initial client PID. Neither establishes an endpoint-to-audiodg mapping. Game buffers, shared engine/APOs and device ISR/DPC are separate layers. Review buffer/effect controls in the game and vendor/native UI. This app does not record sound, pin audiodg, restart audio services or change global enhancements. Imported frame times do not diagnose audio glitches; correlate glitches with an external WPA trace.",
            Self::Irq => "Stored interrupt preferences are registry requests, not measured effective ISR/DPC placement. Assignment masks are processor-group relative; a driver may target DPCs separately. Correlate actual glitches/stutters with a WPA trace before changing device policy. MSI/IRQ mutation, device restart and automatic audio-core recipes remain deferred.",
            Self::Usb => "A host controller may serve mouse, audio and other devices. Parent chains identify shared controller scope; friendly names do not. This inventory does not measure firmware polling rate, USB bandwidth, ISR/DPC latency or physical port number. Selective-suspend changes and controller restart are outside this card.",
            Self::GameMode => "Review Game Mode in native Settings. This card does not infer its enabled state or write undocumented registry values. Availability depends on Windows build/user policy; any performance effect requires your own comparison.",
            Self::Capture => "Open Captures and review background recording (Record what happened). Change that choice independently of Game Bar, screenshots or other recording tools. This card does not infer recording activity or change capture policy.",
            Self::Windowed => "Native optimizations apply to compatible windowed/borderless DX10/11 games. Review Auto HDR interactions and per-app exceptions, then restart the game after a change. This card does not claim every DX12/Vulkan game benefits or infer the current switch.",
            Self::Gpu => "Set a per-app preferred GPU when multiple suitable adapters exist. A requested preference does not prove which adapter renders the game. Inspect the game's renderer and benchmark; native Settings preserves other preference fields. HAGS and vendor driver writers are not managed here.",
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadonlyConfig {}

struct DiagnosticModule {
    kind: Kind,
    codec: JsonCodec<ReadonlyConfig>,
    captures: Mutex<measurements::Captures>,
    observed: Mutex<Option<ModuleObservation>>,
}
impl DiagnosticModule {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            codec: JsonCodec::new(
                FeatureId::new(kind.id()).expect("static feature ID"),
                EnvelopeKind::Config,
                SchemaVersion::new(1).expect("version"),
            ),
            captures: Mutex::new(measurements::Captures::default()),
            observed: Mutex::new(None),
        }
    }
    fn observation(&self) -> Result<String, DiagnosticError> {
        match self.kind {
            Kind::Measurements => Ok(self.captures.lock().map_err(|_| DiagnosticError::Limit { operation: "measurement cache" })?.summary()),
            Kind::Display | Kind::Latency => display_summary(),
            Kind::Audio => audio_summary(),
            Kind::Irq => irq_summary(),
            Kind::Usb => usb_summary(),
            _ if cfg!(target_os = "windows") => Ok("Open Windows Settings to view or change this option. Its current value is shown there.".into()),
            _ => Err(DiagnosticError::Unsupported),
        }
    }
    fn collect(&self, target: &TargetIdentity) -> ModuleObservation {
        let result = if *target == TargetIdentity::Machine {
            self.observation()
        } else {
            Err(DiagnosticError::Unsupported)
        };
        let (capability, summary) = match result {
            Ok(summary) => (Capability::Available, summary),
            Err(error) => {
                let message = error.to_string();
                let capability = if matches!(
                    &error,
                    DiagnosticError::Native {
                        code: 5 | -2147024891,
                        ..
                    }
                ) {
                    Capability::Denied(message.clone())
                } else {
                    Capability::Unsupported(message.clone())
                };
                (capability, message)
            }
        };
        ModuleObservation {
            capability,
            summary: ObservationSummary {
                summary: format!("{summary}\n\n{}", self.kind.guidance()),
                effect: Effect::Immediate,
            },
            effect: Effect::Immediate,
        }
    }
    fn refresh(&self) -> Result<(), ModuleError> {
        let observation = self.collect(&TargetIdentity::Machine);
        let result = match &observation.capability {
            Capability::Available => Ok(()),
            Capability::Denied(message) => Err(ModuleError::new(ErrorKind::Denied, "refresh")
                .with_detail(message.clone())
                .expect("bounded native error")),
            Capability::Unsupported(message) => {
                Err(ModuleError::new(ErrorKind::Unsupported, "refresh")
                    .with_detail(message.clone())
                    .expect("bounded native error"))
            }
        };
        *self
            .observed
            .lock()
            .map_err(|_| ModuleError::new(ErrorKind::Read, "observation cache"))? =
            Some(observation);
        result
    }
}

pub(crate) fn register(registry: &mut Registry) -> Result<(), ModuleError> {
    for kind in Kind::ALL {
        registry.register(Arc::new(DiagnosticModule::new(kind)))?;
    }
    Ok(())
}

fn native_error(error: DiagnosticError, operation: &'static str) -> ModuleError {
    let kind = match &error {
        DiagnosticError::Unsupported => ErrorKind::Unsupported,
        DiagnosticError::Native { code, .. } if *code == 5 || *code == -2147024891 => {
            ErrorKind::Denied
        }
        _ => ErrorKind::Read,
    };
    let mut result = ModuleError::new(kind, operation);
    if let DiagnosticError::Native { code, .. } = &error {
        result = result.with_native(NativeError {
            domain: ResourceId::new("gaming_diagnostics").expect("static ID"),
            code: *code,
        });
    }
    result
        .with_detail(error.to_string())
        .expect("bounded diagnostic error")
}

impl Module for DiagnosticModule {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            id: self.feature(),
            config_version: SchemaVersion::new(1).expect("version"),
            title: self.kind.title(),
            description: self.kind.description(),
            access: ModuleAccess::ReadOnly,
            target_scopes: vec![TargetScope::Machine],
            activation_modes: Vec::new(),
            effect: Effect::Immediate,
        }
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        vec![TargetIdentity::Machine]
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        if *target != TargetIdentity::Machine {
            return self.collect(target);
        }
        if let Ok(cache) = self.observed.lock() {
            if let Some(observation) = &*cache {
                return observation.clone();
            }
        }
        let observation = self.collect(target);
        if let Ok(mut cache) = self.observed.lock() {
            *cache = Some(observation.clone());
        }
        observation
    }
    fn fields(
        &self,
        _: &TargetIdentity,
        _: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        Err(ModuleError::new(ErrorKind::Unsupported, "readonly fields"))
    }
    fn read(&self, _: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        Err(ModuleError::new(
            ErrorKind::Unsupported,
            "readonly field read",
        ))
    }
    fn write(&self, _: &ResourceKey, _: &[u8], _: &[u8]) -> Result<(), ModuleError> {
        Err(ModuleError::new(
            ErrorKind::Unsupported,
            "readonly field write",
        ))
    }
    fn action(&self, action: ModuleAction) -> Result<(), ModuleError> {
        match action {
            ModuleAction::Refresh => self.refresh(),
            ModuleAction::OpenSettings => native::open_settings(
                self.kind
                    .settings()
                    .ok_or_else(|| ModuleError::new(ErrorKind::Unsupported, "settings"))?,
            )
            .map_err(|error| native_error(error, "open settings")),
            ModuleAction::Import(path) if matches!(self.kind, Kind::Measurements) => {
                let capture = measurements::import(&path).map_err(|error| {
                    ModuleError::new(ErrorKind::InvalidConfig, "import CSV")
                        .with_detail(error.to_string())
                        .expect("bounded import error")
                })?;
                self.captures
                    .lock()
                    .map_err(|_| ModuleError::new(ErrorKind::Read, "measurement cache"))?
                    .accept(capture);
                self.refresh()
            }
            _ => Err(ModuleError::new(ErrorKind::Unsupported, "action")),
        }
    }
}

impl ModuleEditor for DiagnosticModule {
    fn section(&self) -> ModuleSection {
        match self.kind {
            Kind::GameMode | Kind::Capture | Kind::Windowed | Kind::Gpu => ModuleSection::Windows,
            _ => ModuleSection::Diagnostics,
        }
    }
    fn feature(&self) -> FeatureId {
        FeatureId::new(self.kind.id()).expect("static feature ID")
    }
    fn default_config(&self) -> ModuleEnvelope {
        self.codec
            .encode(&ReadonlyConfig::default(), None)
            .expect("static configuration")
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        self.codec.decode(config).is_ok()
    }
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        if !self.accepts_config(config) {
            return Err(());
        }
        ui.label(self.kind.guidance());
        Ok(false)
    }
    fn summary(&self, config: &ModuleEnvelope) -> Option<String> {
        self.accepts_config(config)
            .then(|| self.kind.description().into())
    }
    fn actions(&self, ui: &mut eframe::egui::Ui) -> Option<ModuleAction> {
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Refresh").clicked() {
                action = Some(ModuleAction::Refresh);
            }
            if self.kind.settings().is_some()
                && ui
                    .add_enabled(
                        cfg!(target_os = "windows"),
                        eframe::egui::Button::new("Open Windows Settings"),
                    )
                    .clicked()
            {
                action = Some(ModuleAction::OpenSettings);
            }
            if matches!(self.kind, Kind::Measurements) {
                let label = self
                    .captures
                    .lock()
                    .map(|captures| captures.next_label())
                    .unwrap_or("A");
                if ui.button(format!("Import results {label}")).clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Frame-time CSV", &["csv"])
                        .pick_file()
                    {
                        action = Some(ModuleAction::Import(path));
                    }
                }
            }
        });
        if matches!(self.kind, Kind::Measurements) {
            ui.collapsing("Import format and limits", |ui| {
                ui.label("UTF-8 normalized benchmark CSV, exact header below; repeat the same metadata on each frame row. Supports quoted fields. Export/convert one process and swap chain from your benchmark/PresentMon version. frame_type is rendered or generated; frame_time_ms must be finite, >0 and ≤60,000. Maximum 4 MiB, 100,000 rows and 256 bytes per field.");
                ui.add(eframe::egui::Label::new(measurements::CSV_HEADER).wrap());
                ui.label("Example row: PresentMon,2.4,game.exe,0x1,ComposedFlip,rendered,export,scene-a,16.67");
                ui.label("Imports alternate A/B; a failed import preserves both previous captures. Compatible metadata is declared by the CSV and cannot verify the scene, temperature or game settings. Captures live in memory for this app session.");
            });
        }
        action
    }
}

fn display_summary() -> Result<String, DiagnosticError> {
    let displays = native::display_inventory()?;
    if displays.is_empty() {
        return Ok("No active desktop display was reported.".into());
    }
    Ok(displays
        .iter()
        .map(|display| {
            format!(
                "{}{} · {} · {} × {} · {}\nAdapter: {}",
                display.device_name,
                if display.primary { " (primary)" } else { "" },
                display
                    .monitor_name
                    .as_deref()
                    .unwrap_or("monitor name unavailable"),
                display.width,
                display.height,
                display
                    .nominal_refresh_hz
                    .map(|hz| format!("{hz} Hz nominal"))
                    .unwrap_or_else(|| "refresh unavailable".into()),
                display.adapter_name
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn audio_summary() -> Result<String, DiagnosticError> {
    let endpoints = native::audio_inventory()?;
    let mut lines = vec![format!(
        "{} active audio endpoints. Session inventory is a point-in-time observation.",
        endpoints.len()
    )];
    for endpoint in &endpoints {
        lines.push(format!(
            "{} · {:?}{}\nEndpoint: {}\nPnP endpoint identity: {}",
            endpoint.name.as_deref().unwrap_or("name unavailable"),
            endpoint.direction,
            if endpoint.default_multimedia {
                " (multimedia default)"
            } else {
                ""
            },
            endpoint.id,
            endpoint.pnp_instance_id.as_deref().unwrap_or("unavailable")
        ));
        for session in &endpoint.sessions {
            let owner = match session.client {
                native::AudioClient::SingleProcess(pid) => format!("client PID {pid}"),
                native::AudioClient::MultipleProcesses { initial_pid } => {
                    format!("multiprocess session; initial client PID {initial_pid}")
                }
                native::AudioClient::SystemSounds => "system sounds session".into(),
                native::AudioClient::Unavailable => "client identity unavailable".into(),
            };
            lines.push(format!("  {:?}: {owner}", session.state));
        }
        if let Some(error) = &endpoint.session_error {
            lines.push(format!("  Sessions unavailable: {error}"));
        }
    }
    match native::pnp_inventory() {
        Ok(inventory) => {
            lines.push("Present audio device/driver inventory (separate from session PID):".into());
            for device in inventory
                .devices
                .iter()
                .filter(|device| device.class.eq_ignore_ascii_case("MEDIA"))
            {
                lines.push(format!(
                    "{} · service {} · transport {}\n{}",
                    device.name,
                    available(&device.service),
                    available(&device.enumerator),
                    device.instance_id
                ));
            }
        }
        Err(error) => lines.push(format!("Audio PnP/driver inventory unavailable: {error}")),
    }
    Ok(lines.join("\n"))
}

fn available(value: &str) -> &str {
    if value.is_empty() {
        "unavailable"
    } else {
        value
    }
}
fn raw_policy(value: &native::StoredInterruptValue) -> String {
    match value {
        native::StoredInterruptValue::Absent => "not stored".into(),
        native::StoredInterruptValue::Unavailable { code } => format!("unavailable ({code})"),
        native::StoredInterruptValue::Present { value_type, bytes } => format!(
            "type {value_type}, bytes {}",
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    }
}
fn irq_summary() -> Result<String, DiagnosticError> {
    let inventory = native::pnp_inventory()?;
    Ok(irq_inventory_summary(&inventory))
}
fn irq_inventory_summary(inventory: &native::PnpInventory) -> String {
    let mut lines = vec![format!(
        "{} present PnP devices. Processor groups: {}.",
        inventory.devices.len(),
        inventory
            .processor_groups
            .iter()
            .map(|group| format!("{}: {} logical CPUs", group.group, group.logical_processors))
            .collect::<Vec<_>>()
            .join(", ")
    )];
    for device in inventory.devices.iter().take(256) {
        let policy = inventory
            .interrupt_policies
            .iter()
            .find(|policy| policy.instance_id == device.instance_id);
        lines.push(format!("{} [{}] · driver service {}\n{}\nParent: {}\nAffinity: {} · assignment: {} · priority: {} · MSI supported preference: {}",
            device.name, available(&device.class), available(&device.service), device.instance_id, device.parent_id.as_deref().unwrap_or("unavailable/root"),
            policy.map(|policy| raw_policy(&policy.affinity_policy)).unwrap_or_else(|| "not stored".into()),
            policy.map(|policy| raw_policy(&policy.assignment_set)).unwrap_or_else(|| "not stored".into()),
            policy.map(|policy| raw_policy(&policy.priority)).unwrap_or_else(|| "not stored".into()),
            policy.map(|policy| raw_policy(&policy.msi_supported)).unwrap_or_else(|| "not stored".into())));
    }
    if inventory.devices.len() > 256 {
        lines.push(format!(
            "{} additional devices omitted from this bounded summary.",
            inventory.devices.len() - 256
        ));
    }
    lines.push("Read failures stay visible. No trace was collected; actual ISR/DPC durations and effective CPU placement are unavailable.".into());
    lines.join("\n\n")
}
fn usb_summary() -> Result<String, DiagnosticError> {
    let inventory = native::pnp_inventory()?;
    Ok(usb_inventory_summary(&inventory))
}
fn usb_inventory_summary(inventory: &native::PnpInventory) -> String {
    let mut lines = vec![format!("{} present devices inspected. USB descendants are linked by Configuration Manager parent identities.", inventory.devices.len())];
    let matching: Vec<_> = inventory
        .devices
        .iter()
        .filter_map(|device| {
            let controller = native::usb_controller(device, &inventory.devices);
            (controller.is_some()
                || device.enumerator.eq_ignore_ascii_case("USB")
                || device.class.eq_ignore_ascii_case("USB"))
            .then_some((device, controller))
        })
        .collect();
    for (device, controller) in matching.iter().take(256) {
        lines.push(format!(
            "{} [{}] · {}\n{}\nParent: {}\nHost controller: {}\nLocation: {}",
            device.name,
            available(&device.class),
            available(&device.service),
            device.instance_id,
            device.parent_id.as_deref().unwrap_or("unavailable/root"),
            controller
                .map(|controller| format!("{} ({})", controller.name, controller.instance_id))
                .unwrap_or_else(|| "not established from present parent chain".into()),
            device.location.as_deref().unwrap_or("unavailable")
        ));
    }
    if matching.len() > 256 {
        lines.push(format!(
            "{} additional matching devices omitted from this bounded summary.",
            matching.len() - 256
        ));
    }
    lines.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_registry_has_all_independent_readonly_cards_and_no_write_paths() {
        let mut registry = Registry::default();
        register(&mut registry).unwrap();
        for id in [
            "measurements",
            "display_audit",
            "latency_advisor",
            "audio_diagnostics",
            "irq_diagnostics",
            "usb_topology",
            "game_mode",
            "background_capture",
            "windowed_optimizations",
            "gpu_preference",
        ] {
            let module = registry.get(&FeatureId::new(id).unwrap()).unwrap();
            assert_eq!(module.descriptor().access, ModuleAccess::ReadOnly);
            assert!(module.descriptor().activation_modes.is_empty());
            let target = TargetIdentity::Machine;
            let key = ResourceKey {
                target: target.clone(),
                field: ResourceId::new("test").unwrap(),
            };
            assert_eq!(
                module
                    .fields(&target, &module.editor().default_config())
                    .unwrap_err()
                    .kind,
                ErrorKind::Unsupported
            );
            assert_eq!(module.read(&key).unwrap_err().kind, ErrorKind::Unsupported);
            assert_eq!(
                module.write(&key, &[], &[]).unwrap_err().kind,
                ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn diagnostics_readonly_codec_rejects_unknown_configuration() {
        let module = DiagnosticModule::new(Kind::Display);
        let mut config = module.default_config();
        assert!(module.accepts_config(&config));
        config.payload = serde_json::json!({"enable": true});
        assert!(!module.accepts_config(&config));
    }

    #[test]
    fn diagnostics_observations_use_retained_cache_between_refresh_actions() {
        let module = DiagnosticModule::new(Kind::Display);
        *module.observed.lock().unwrap() = Some(ModuleObservation {
            capability: Capability::Available,
            summary: ObservationSummary {
                summary: "retained native observation".into(),
                effect: Effect::Immediate,
            },
            effect: Effect::Immediate,
        });
        for _ in 0..3 {
            assert_eq!(
                module.observe(&TargetIdentity::Machine).summary.summary,
                "retained native observation"
            );
        }
    }

    #[test]
    fn diagnostics_failed_import_action_keeps_captures_and_published_observation() {
        let module = DiagnosticModule::new(Kind::Measurements);
        let path = std::env::temp_dir().join(format!(
            "cpu-affinity-diagnostics-invalid-{}.csv",
            std::process::id()
        ));
        let mut captures = module.captures.lock().unwrap();
        let row = "x,1,g,s,p,rendered,e,w,10\n";
        captures.accept(
            measurements::parse(format!("{}\n{row}", measurements::CSV_HEADER).as_bytes()).unwrap(),
        );
        captures.accept(
            measurements::parse(format!("{}\n{row}", measurements::CSV_HEADER).as_bytes()).unwrap(),
        );
        let summary = captures.summary();
        drop(captures);
        module.action(ModuleAction::Refresh).unwrap();
        let observed = module.observe(&TargetIdentity::Machine).summary.summary;
        std::fs::write(
            &path,
            format!("{}\nx,1,g,s,p,rendered,e,w,NaN\n", measurements::CSV_HEADER),
        )
        .unwrap();
        let result = module.action(ModuleAction::Import(path.clone()));
        std::fs::remove_file(path).unwrap();
        assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidConfig);
        assert_eq!(module.captures.lock().unwrap().summary(), summary);
        assert_eq!(module.captures.lock().unwrap().next_label(), "A");
        assert_eq!(
            module.observe(&TargetIdentity::Machine).summary.summary,
            observed
        );
    }

    fn inventory(count: usize) -> native::PnpInventory {
        let devices = (0..count)
            .map(|index| native::PnpDevice {
                instance_id: format!("USB\\DEVICE{index}"),
                parent_id: None,
                name: format!("Device {index}"),
                class: "USB".into(),
                service: "USBHUB3".into(),
                enumerator: "USB".into(),
                location: None,
            })
            .collect();
        native::PnpInventory {
            devices,
            processor_groups: vec![native::ProcessorGroup {
                group: 0,
                logical_processors: 16,
            }],
            interrupt_policies: vec![],
        }
    }

    #[test]
    fn diagnostics_inventory_summaries_are_bounded_and_do_not_hide_omission() {
        let inventory = inventory(257);
        let usb = usb_inventory_summary(&inventory);
        assert!(usb.contains("1 additional matching devices omitted"));
        assert!(!usb.contains("USB\\DEVICE256"));
        let irq = irq_inventory_summary(&inventory);
        assert!(irq.contains("1 additional devices omitted"));
        assert!(!irq.contains("USB\\DEVICE256"));
        assert!(irq.contains("not stored"));
        assert!(irq.contains("actual ISR/DPC"));
    }
}
