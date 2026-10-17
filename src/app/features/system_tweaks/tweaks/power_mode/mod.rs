use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
};
use os_api::gaming::{self, GuidValue, PowerSource};
use os_api::gaming_cpu_power::{
    self as cpu_power, CpuPowerKey, CpuPowerOptions, CpuPowerRequest, CpuPowerSetting,
};
use serde::{Deserialize, Serialize};
const ID: &str = "power_mode";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    Efficiency,
    #[default]
    Balanced,
    Performance,
}

#[cfg(test)]
mod cpu_options_tests {
    use super::*;

    #[test]
    fn basic_power_preset_changes_only_the_ac_mode() {
        let target = TargetIdentity::Account {
            sid: "S-1-5-21-1".into(),
        };
        let prepared = PowerMode.tweak_config(&target).unwrap();
        let config = configuration(&prepared).unwrap();
        assert!(
            config.cpu.is_none(),
            "New applies must not request CPU policy changes"
        );
        let fields = PowerMode.fields(&target, &prepared).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key.field.as_str(), "user_power_mode_ac");
        assert_eq!(fields[0].desired, gaming::POWER_MODE_PERFORMANCE.0);
    }

    #[test]
    fn basic_power_preset_has_no_cpu_options_but_accepts_previous_restore_data() {
        let module = PowerMode;
        let previous = envelope(
            ID,
            &Config {
                cpu: Some(CpuOptions {
                    maximum_mhz: 4500,
                    ..Default::default()
                }),
                ..ac_config()
            },
        );
        assert!(
            module.accepts_config(&previous),
            "Previous originals must remain restorable"
        );
        assert!(module.tweak_options(None).is_none());
        assert!(module.tweak_options(Some(&previous)).is_none());
        let target = TargetIdentity::Account {
            sid: "S-1-5-21-1".into(),
        };
        assert_eq!(
            module
                .prepare_tweak_options(&target, &previous)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidConfig
        );
    }

    #[test]
    fn cpu_policy_readback_does_not_claim_measured_clocks() {
        assert_eq!(
            PowerMode.tweak_verified_label(),
            "Windows power preference verified"
        );
    }

    #[test]
    fn retired_cpu_editor_preserves_saved_limits_without_rendering_controls() {
        let module = PowerMode;
        for original in [0, 500, 4500, 60000] {
            let mut config = envelope(
                ID,
                &Config {
                    cpu: Some(CpuOptions {
                        maximum_mhz: original,
                        ..Default::default()
                    }),
                    ..ac_config()
                },
            );
            let mut output = eframe::egui::Context::default().run_ui(
                eframe::egui::RawInput {
                    screen_rect: Some(eframe::egui::Rect::from_min_size(
                        eframe::egui::Pos2::ZERO,
                        eframe::egui::vec2(400.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| module.show_tweak_options(ui, &mut config),
            );
            output.textures_delta.clear();
            let cpu = configuration(&config).unwrap().cpu.unwrap();
            assert_eq!(
                cpu.maximum_mhz, original,
                "Retained recovery values must never be clamped"
            );
        }
    }
    #[test]
    fn retired_hybrid_configuration_remains_readable_without_controls() {
        let module = PowerMode;
        let config = ModuleEnvelope {
            payload: serde_json::json!({"ac":"Performance","dc":"Balanced","ac_only":true,
            "cpu":{"maximum_mhz":4500,"efficient_mhz":2500,"keep_high":true,"allow_parking":false,"requests":[]}}),
            ..module.default_config()
        };
        assert!(module.accepts_config(&config));
        assert!(module.tweak_options(Some(&config)).is_none());
        let mut draft = config.clone();
        let before = configuration(&draft).unwrap().cpu.unwrap();
        assert_eq!(before.maximum_mhz, 4500);
        assert_eq!(before.efficient_mhz, Some(2500));
        let mut output = eframe::egui::Context::default().run_ui(
            eframe::egui::RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(
                    eframe::egui::Pos2::ZERO,
                    eframe::egui::vec2(400.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                ui.set_width(330.0);
                module.show_tweak_options(ui, &mut draft);
            },
        );
        output.textures_delta.clear();
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                eframe::egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!labels.contains("P-cores") && !labels.contains("E-cores"));
        assert!(!labels.contains("Allow core parking"));
        assert!(module.accepts_config(&draft));
        let old = ModuleEnvelope {
            payload: serde_json::json!({"ac":"Performance","dc":"Balanced","ac_only":true,
            "cpu":{"maximum_mhz":60000,"keep_high":false,"requests":[]}}),
            ..module.default_config()
        };
        assert!(
            module.accepts_config(&old),
            "Old saved/owned values must remain restorable"
        );
    }

    #[test]
    fn cpu_options_are_ac_only_and_validate_frequency_steps() {
        let module = PowerMode;
        let config = |mhz, high| ModuleEnvelope {
            payload: serde_json::json!({"ac":"Performance", "dc":"Balanced", "ac_only":true,
                "cpu":{"maximum_mhz":mhz,"keep_high":high,"requests":[]}}),
            ..module.default_config()
        };
        for mhz in [0, 100, 4500, 64000] {
            assert!(
                module.accepts_config(&config(mhz, true)),
                "Requested MHz {mhz}"
            );
        }
        for mhz in [1, 99, 4501, 64100] {
            assert!(!module.accepts_config(&config(mhz, false)));
        }
        let mut battery = config(4500, false);
        battery.payload["ac_only"] = serde_json::json!(false);
        assert!(
            !module.accepts_config(&battery),
            "CPU options never manage battery settings"
        );
        assert!(
            module.accepts_config(&module.default_config()),
            "Legacy configuration remains readable"
        );
    }
    #[test]
    fn cpu_baseline_codec_preserves_absence_and_nonstep_original() {
        for value in [None, Some(0), Some(5401), Some(u32::MAX)] {
            assert_eq!(nullable_value(&value_bytes(value)).unwrap(), value);
        }
        for bytes in [vec![], vec![0, 1], vec![1, 2, 3], vec![2]] {
            assert!(nullable_value(&bytes).is_err());
        }
    }
    #[test]
    fn cpu_saved_bindings_survive_reload_without_retargeting_or_dc_fields() {
        let module = PowerMode;
        let request = CpuPowerRequest {
            key: CpuPowerKey {
                scheme: GuidValue(0x381b4222_f694_41f0_9685_ff5bb260df2eu128.to_be_bytes()),
                setting: CpuPowerSetting::Frequency,
            },
            value: 4500,
        };
        let config = envelope(
            ID,
            &Config {
                cpu: Some(CpuOptions {
                    maximum_mhz: 4500,
                    keep_high: false,
                    requests: vec![request.clone()],
                    ..Default::default()
                }),
                ..ac_config()
            },
        );
        let reloaded = ModuleEnvelope::from_bytes(&config.to_bytes().unwrap()).unwrap();
        let target = TargetIdentity::Account {
            sid: "S-1-5-21-1".into(),
        };
        let fields = module.fields(&target, &reloaded).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[1].key.field.as_str(), request.key.wire_id());
        assert_eq!(nullable_value(&fields[1].desired).unwrap(), Some(4500));
        assert!(fields
            .iter()
            .all(|f| f.key.field.as_str() != "user_power_mode_dc"));
        assert!(module.tweak_options(Some(&reloaded)).is_none());
        assert!(module.tweak_summary(&reloaded).unwrap().contains("Undo"));
        assert_eq!(
            configuration(&reloaded)
                .unwrap()
                .cpu
                .unwrap()
                .requests
                .len(),
            1
        );
    }
    #[test]
    fn cpu_options_reject_duplicate_foreign_and_mismatched_bindings() {
        let mut cpu = CpuOptions {
            maximum_mhz: 4500,
            keep_high: false,
            requests: vec![],
            ..Default::default()
        };
        let request = CpuPowerRequest {
            key: CpuPowerKey {
                scheme: GuidValue(0x381b4222_f694_41f0_9685_ff5bb260df2eu128.to_be_bytes()),
                setting: CpuPowerSetting::Frequency,
            },
            value: 4500,
        };
        cpu.requests = vec![request.clone()];
        assert!(cpu.valid());
        cpu.requests.push(request.clone());
        assert!(!cpu.valid());
        cpu.requests = vec![CpuPowerRequest {
            value: 4600,
            ..request
        }];
        assert!(!cpu.valid());
        cpu.requests[0].key.setting = CpuPowerSetting::Minimum;
        cpu.requests[0].value = 100;
        assert!(!cpu.valid());
        cpu.keep_high = true;
        assert!(cpu.valid());
        cpu.requests[0].key.scheme = GuidValue([0; 16]);
        assert!(!cpu.valid());
    }
}
impl Mode {
    fn guid(self) -> GuidValue {
        match self {
            Self::Efficiency => gaming::POWER_MODE_EFFICIENCY,
            Self::Balanced => gaming::POWER_MODE_BALANCED,
            Self::Performance => gaming::POWER_MODE_PERFORMANCE,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Efficiency => "Best power efficiency",
            Self::Balanced => "Balanced",
            Self::Performance => "Best performance",
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    ac: Mode,
    dc: Mode,
    #[serde(default)]
    ac_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cpu: Option<CpuOptions>,
}
const MAX_FREQUENCY_MHZ: u32 = 64_000;
const FREQUENCY_STEP_MHZ: u32 = 100;
const MAX_CPU_REQUESTS: usize = 26;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CpuOptions {
    maximum_mhz: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    efficient_mhz: Option<u32>,
    keep_high: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allow_parking: Option<bool>,
    #[serde(default)]
    requests: Vec<CpuPowerRequest>,
}
impl CpuOptions {
    fn native_options(&self) -> CpuPowerOptions {
        CpuPowerOptions {
            performance_mhz: self.maximum_mhz,
            efficient_mhz: self.efficient_mhz,
            keep_high: self.keep_high,
            allow_parking: self.allow_parking,
        }
    }
    fn valid_frequency(mhz: u32) -> bool {
        mhz == 0
            || ((FREQUENCY_STEP_MHZ..=MAX_FREQUENCY_MHZ).contains(&mhz)
                && mhz.is_multiple_of(FREQUENCY_STEP_MHZ))
    }
    fn valid(&self) -> bool {
        if !Self::valid_frequency(self.maximum_mhz)
            || self
                .efficient_mhz
                .is_some_and(|mhz| !Self::valid_frequency(mhz))
        {
            return false;
        }
        if self.requests.len() > MAX_CPU_REQUESTS {
            return false;
        }
        let mut keys = std::collections::BTreeSet::new();
        // Saved bindings may address only these AC fields with values matching the UI intent.
        self.requests.iter().all(|request| {
            keys.insert(request.key.wire_id())
                && CpuPowerKey::from_wire_id(&request.key.wire_id()).is_ok()
                && self.request_matches(request)
        })
    }
    fn request_matches(&self, request: &CpuPowerRequest) -> bool {
        match request.key.setting {
            CpuPowerSetting::Frequency => {
                request.value == self.efficient_mhz.unwrap_or(self.maximum_mhz)
            }
            CpuPowerSetting::Frequency1 => request.value == self.maximum_mhz,
            CpuPowerSetting::Maximum | CpuPowerSetting::Maximum1 => request.value == 100,
            CpuPowerSetting::Minimum | CpuPowerSetting::Minimum1 => {
                if self.keep_high {
                    request.value == 100
                } else {
                    self.allow_parking.is_some() && request.value == 0
                }
            }
            CpuPowerSetting::EnergyPreference | CpuPowerSetting::EnergyPreference1 => {
                request.value == 0
            }
            CpuPowerSetting::Boost => request.value == 1,
            CpuPowerSetting::ParkingMinimum | CpuPowerSetting::ParkingMinimum1 => self
                .allow_parking
                .is_some_and(|allow| request.value == if allow { 0 } else { 100 }),
            CpuPowerSetting::ParkingMaximum | CpuPowerSetting::ParkingMaximum1 => {
                self.allow_parking.is_some() && request.value == 100
            }
        }
    }
}
fn configuration(value: &ModuleEnvelope) -> Result<Config, ModuleError> {
    let config: Config = decode(ID, value)?;
    if let Some(cpu) = &config.cpu {
        // Legacy profiles retain their old AC/DC behavior; CPU controls are strictly AC-only.
        if !config.ac_only || config.ac != Mode::Performance || !cpu.valid() {
            return Err(error(ErrorKind::InvalidConfig, "AC processor settings"));
        }
    }
    Ok(config)
}
fn ac_config() -> Config {
    Config {
        ac: Mode::Performance,
        dc: Mode::Balanced,
        ac_only: true,
        cpu: None,
    }
}
fn prepare(target: &TargetIdentity, mut config: Config) -> Result<ModuleEnvelope, ModuleError> {
    if let Some(cpu) = &mut config.cpu {
        // Capture exact plan/overlay keys once, before saving and before the first write.
        cpu.requests = cpu_power::prepare_requests(&account_user(target)?, &cpu.native_options())
            .map_err(|e| native("prepare AC processor settings", e))?;
    }
    Ok(envelope(ID, &config))
}
fn nullable_value(bytes: &[u8]) -> Result<Option<u32>, ModuleError> {
    match bytes {
        [0] => Ok(None),
        [1, a, b, c, d] => Ok(Some(u32::from_le_bytes([*a, *b, *c, *d]))),
        _ => Err(error(ErrorKind::InvalidConfig, "CPU AC value")),
    }
}
fn value_bytes(value: Option<u32>) -> Vec<u8> {
    match value {
        None => vec![0],
        Some(value) => {
            let mut bytes = vec![1];
            bytes.extend(value.to_le_bytes());
            bytes
        }
    }
}
#[derive(Default)]
pub(crate) struct PowerMode;
fn source(key: &ResourceKey) -> Result<PowerSource, ModuleError> {
    if key.field == resource("user_power_mode_ac") {
        Ok(PowerSource::Ac)
    } else if key.field == resource("user_power_mode_dc") {
        Ok(PowerSource::Dc)
    } else {
        Err(error(ErrorKind::Unsupported, "power mode field"))
    }
}
fn name(guid: GuidValue) -> String {
    [Mode::Efficiency, Mode::Balanced, Mode::Performance]
        .into_iter()
        .find(|mode| mode.guid() == guid)
        .map(|m| m.label().into())
        .unwrap_or_else(|| format!("Unknown mode {:032x}", u128::from_be_bytes(guid.0)))
}
impl ModuleEditor for PowerMode {
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::WINDOWS11
    }
    fn tweak(&self) -> Option<crate::app::features::system_tweaks::presentation::TweakSpec> {
        use crate::app::features::system_tweaks::presentation::*;
        Some(TweakSpec {order:0, section:TweakSection::Games, title:"Performance on AC power",scope:"This PC · plugged in only",apply_label:"Apply",help:"Ask Windows to prefer performance while plugged in. Windows and your hardware decide the resulting clocks; this is not a frequency lock or a guaranteed FPS increase. Power use and heat may increase. Battery preferences stay unchanged. Undo restores the previous power-mode preference, including after restarting the app. If older CPU settings are still applied, use Undo to restore them before applying this simpler tweak."})
    }
    fn tweak_config(&self, target: &TargetIdentity) -> Result<ModuleEnvelope, ModuleError> {
        prepare(target, ac_config())
    }
    fn tweak_ready(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        self.tweak_ready_for_config(target, None)
    }
    fn tweak_ready_for_config(
        &self,
        target: &TargetIdentity,
        saved: Option<&ModuleEnvelope>,
    ) -> Result<bool, ModuleError> {
        let user = account_user(target)?;
        let config = saved
            .map(configuration)
            .transpose()?
            .unwrap_or_else(ac_config);
        if gaming::read_power_mode(&user, PowerSource::Ac).map_err(|e| native("read", e))?
            != config.ac.guid()
        {
            return Ok(false);
        }
        let Some(cpu) = config.cpu else {
            return Ok(true);
        };
        cpu_power::prepare_requests(&user, &cpu.native_options())
            .map(|requests| requests.is_empty())
            .map_err(|e| native("read AC processor settings", e))
    }
    fn prepare_tweak_options(
        &self,
        _target: &TargetIdentity,
        _value: &ModuleEnvelope,
    ) -> Result<ModuleEnvelope, ModuleError> {
        Err(error(
            ErrorKind::InvalidConfig,
            "CPU controls removed; restore previous settings with Undo",
        ))
    }
    fn tweak_summary(&self, value: &ModuleEnvelope) -> Option<String> {
        // Older bindings remain intact for exact restoration, without exposing new controls.
        configuration(value)
            .ok()?
            .cpu
            .map(|_| "Previous CPU settings · use Undo to restore".into())
    }
    fn tweak_verified_label(&self) -> &'static str {
        // Power APIs confirm policy values, not the CPU's physical operating frequency.
        "Windows power preference verified"
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        fields
            .iter()
            .find(|(id, _)| id.as_str() == "user_power_mode_ac")
            .and_then(|(_, bytes)| guid(bytes).ok())
            .map(name)
    }
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        envelope(ID, &Config::default())
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        configuration(config).is_ok()
    }
    #[cfg(test)]
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        let mut value: Config = decode(ID, config).map_err(|_| ())?;
        let before = (value.ac, value.dc);
        for (id, title, mode) in [
            ("power_mode_ac", "Plugged in", &mut value.ac),
            ("power_mode_dc", "On battery", &mut value.dc),
        ] {
            ui.horizontal(|ui| {
                ui.label(title);
                eframe::egui::ComboBox::from_id_salt(id)
                    .selected_text(mode.label())
                    .show_ui(ui, |ui| {
                        for candidate in [Mode::Efficiency, Mode::Balanced, Mode::Performance] {
                            ui.selectable_value(mode, candidate, candidate.label());
                        }
                    });
            });
        }
        ui.small(
            "Choose separately for plugged-in and battery use. Windows may adjust the active mode.",
        );
        let changed = before != (value.ac, value.dc);
        if changed {
            *config = envelope(ID, &value);
        }
        Ok(changed)
    }
}
impl Module for PowerMode {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Power mode",
            "Balance power use and performance for plugged-in and battery use.",
            TargetScope::User,
            all_modes(),
        )
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        account_targets()
    }
    fn canonical_target(&self, target: &TargetIdentity) -> Result<TargetIdentity, ModuleError> {
        canonical_account_target(target)
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        observation((|| {
            let user = observed_account_user(target)?;
            let ac =
                gaming::read_power_mode(&user, PowerSource::Ac).map_err(|e| native("read", e))?;
            let dc =
                gaming::read_power_mode(&user, PowerSource::Dc).map_err(|e| native("read", e))?;
            Ok(format!(
                "{} on AC power\nBattery: {}. The active mode may differ.",
                name(ac),
                name(dc)
            ))
        })())
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        account_user(target)?;
        let config = configuration(config)?;
        let mut fields = vec![field(
            target,
            "user_power_mode_ac",
            config.ac.guid().0.to_vec(),
        )];
        if !config.ac_only {
            fields.push(field(
                target,
                "user_power_mode_dc",
                config.dc.guid().0.to_vec(),
            ));
        }
        if let Some(cpu) = config.cpu {
            fields.extend(cpu.requests.into_iter().map(|request| {
                field(
                    target,
                    &request.key.wire_id(),
                    value_bytes(Some(request.value)),
                )
            }));
        }
        Ok(fields)
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        if key.field.as_str().starts_with("cpu_ac_") {
            let cpu_key = CpuPowerKey::from_wire_id(key.field.as_str())
                .map_err(|e| native("CPU field", e))?;
            return cpu_power::read_value(&account_user(&key.target)?, cpu_key)
                .map(value_bytes)
                .map_err(|e| native("read", e));
        }
        gaming::read_power_mode(&account_user(&key.target)?, source(key)?)
            .map(|v| v.0.to_vec())
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        if key.field.as_str().starts_with("cpu_ac_") {
            let cpu_key = CpuPowerKey::from_wire_id(key.field.as_str())
                .map_err(|e| native("CPU field", e))?;
            return cpu_power::write_value(
                &account_user(&key.target)?,
                cpu_key,
                nullable_value(expected)?,
                nullable_value(desired)?,
            )
            .map_err(|e| native("write", e));
        }
        gaming::write_power_mode(
            &account_user(&key.target)?,
            source(key)?,
            guid(expected)?,
            guid(desired)?,
        )
        .map_err(|e| native("write", e))
    }
    fn confirm_applied(&self, key: &ResourceKey, expected: &[u8]) -> Result<(), ModuleError> {
        if !key.field.as_str().starts_with("cpu_ac_") {
            return Ok(());
        }
        let cpu_key =
            CpuPowerKey::from_wire_id(key.field.as_str()).map_err(|e| native("CPU field", e))?;
        cpu_power::confirm_value(
            &account_user(&key.target)?,
            cpu_key,
            nullable_value(expected)?,
        )
        .map_err(|e| native("confirm CPU power setting", e))
    }
}
