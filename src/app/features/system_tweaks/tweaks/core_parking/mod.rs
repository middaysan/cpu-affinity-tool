//! Manual parking policy, with exact per-plan recovery owned by the tuning controller.
use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::{ModuleEditor, TweakSection, TweakSpec},
};
use os_api::{
    core_parking::{self as api, ParkingScope, ParkingSnapshot},
    gaming_cpu_power::{self as native_api, CpuPowerKey, CpuPowerRequest, CpuPowerSetting},
};
use serde::{Deserialize, Serialize};

const ID: &str = "core_parking";
const SEPARATE_CLASS_UNAVAILABLE:&str="Separate class control is unavailable in this build: Windows did not keep this class unparked during validation.";
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    scope: ParkingScope,
    snapshot: Option<ParkingSnapshot>,
    bindings: Option<Vec<CpuPowerRequest>>,
}
fn configuration(value: &ModuleEnvelope) -> Result<Config, ModuleError> {
    let config: Config = decode(ID, value)?;
    if config.scope != ParkingScope::All {
        return Err(error(ErrorKind::Unsupported, SEPARATE_CLASS_UNAVAILABLE));
    }
    match &config.snapshot {
        None if config.bindings.is_none() && config.scope == ParkingScope::All => {}
        Some(snapshot) => {
            let expected = snapshot
                .requests(config.scope)
                .map_err(|e| native("parking scope", e))?;
            if config
                .bindings
                .as_ref()
                .is_some_and(|bindings| *bindings != expected)
            {
                return Err(error(ErrorKind::InvalidConfig, "parking bindings"));
            }
        }
        _ => return Err(error(ErrorKind::InvalidConfig, "parking configuration")),
    }
    Ok(config)
}
fn scope_label(scope: ParkingScope) -> &'static str {
    match scope {
        ParkingScope::All => "All cores",
        ParkingScope::Performance => "Performance cores",
    }
}
fn bind(draft: Config, snapshot: ParkingSnapshot) -> Result<ModuleEnvelope, ModuleError> {
    if draft.scope != ParkingScope::All {
        return Err(error(ErrorKind::Unsupported, SEPARATE_CLASS_UNAVAILABLE));
    }
    let before = draft
        .snapshot
        .ok_or_else(|| error(ErrorKind::InvalidConfig, "parking choices unavailable"))?;
    if before.scheme != snapshot.scheme
        || before.overlay != snapshot.overlay
        || before.classes != snapshot.classes
    {
        return Err(error(
            ErrorKind::ExternalChange,
            "Power plan or CPU topology changed; reopen the chooser",
        ));
    }
    let bindings = snapshot
        .requests(draft.scope)
        .map_err(|e| native("prepare parking scope", e))?;
    Ok(envelope(
        ID,
        &Config {
            scope: draft.scope,
            snapshot: Some(snapshot),
            bindings: Some(bindings),
        },
    ))
}
fn nullable_value(bytes: &[u8]) -> Result<Option<u32>, ModuleError> {
    match bytes {
        [0] => Ok(None),
        [1, a, b, c, d] => {
            let value = u32::from_le_bytes([*a, *b, *c, *d]);
            if value <= 100 {
                Ok(Some(value))
            } else {
                Err(error(ErrorKind::InvalidConfig, "parking value"))
            }
        }
        _ => Err(error(ErrorKind::InvalidConfig, "parking value")),
    }
}
fn value_bytes(value: Option<u32>) -> Vec<u8> {
    match value {
        None => vec![0],
        Some(value) => [vec![1], value.to_le_bytes().to_vec()].concat(),
    }
}
fn parking_key(key: &ResourceKey) -> Result<CpuPowerKey, ModuleError> {
    let key =
        CpuPowerKey::from_wire_id(key.field.as_str()).map_err(|e| native("parking field", e))?;
    if !matches!(
        key.setting,
        CpuPowerSetting::ParkingMinimum
            | CpuPowerSetting::ParkingMaximum
            | CpuPowerSetting::ParkingMinimum1
            | CpuPowerSetting::ParkingMaximum1
    ) {
        return Err(error(ErrorKind::InvalidConfig, "parking field"));
    }
    Ok(key)
}

pub(crate) struct CoreParking;
impl ModuleEditor for CoreParking {
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::WINDOWS11
    }
    fn tweak(&self) -> Option<TweakSpec> {
        Some(TweakSpec{order:10,section:TweakSection::Games,title:"Core parking",scope:"Current power plan and AC mode · plugged in only",apply_label:"Choose…",help:"Keep all cores available to Windows while plugged in. Choose All cores, then Apply. A detected separate performance-core class is shown but cannot be applied in this build: native validation found that Windows stored its policy but subsequently parked those cores again. This is not a claim about every Windows/CPU combination. Only parking minimum/maximum policy values change; frequency, boost, affinity and battery settings stay unchanged. Windows provides no arbitrary chiplet or CPU-number switch. This verifies stored policy, not permanent physical activity or a guaranteed FPS improvement. Power use and heat may increase. The captured plan and mode remain the targets after restart; changing plans does not copy the tweak. Undo restores exact previous values, including absent overrides."})
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
    fn tweak_config(&self, target: &TargetIdentity) -> Result<ModuleEnvelope, ModuleError> {
        let snapshot =
            api::inspect(&account_user(target)?).map_err(|e| native("prepare parking", e))?;
        bind(
            Config {
                scope: ParkingScope::All,
                snapshot: Some(snapshot.clone()),
                bindings: None,
            },
            snapshot,
        )
    }
    fn tweak_ready(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        self.tweak_ready_for_config(target, None)
    }
    fn prepare_tweak_options(
        &self,
        target: &TargetIdentity,
        value: &ModuleEnvelope,
    ) -> Result<ModuleEnvelope, ModuleError> {
        let draft = configuration(value)?;
        let snapshot =
            api::inspect(&account_user(target)?).map_err(|e| native("prepare parking", e))?;
        bind(draft, snapshot)
    }
    fn tweak_ready_for_config(
        &self,
        target: &TargetIdentity,
        saved: Option<&ModuleEnvelope>,
    ) -> Result<bool, ModuleError> {
        let scope = saved
            .map(configuration)
            .transpose()?
            .map_or(ParkingScope::All, |c| c.scope);
        api::inspect(&account_user(target)?)
            .and_then(|snapshot| snapshot.requests(scope))
            .map(|r| r.is_empty())
            .map_err(|e| native("read parking policy", e))
    }
    fn show_tweak_options(&self, ui: &mut eframe::egui::Ui, value: &mut ModuleEnvelope) {
        let Ok(mut config) = configuration(value) else {
            return;
        };
        let Some(snapshot) = &config.snapshot else {
            return;
        };
        ui.label("Keep cores available while plugged in:");
        for scope in [ParkingScope::All, ParkingScope::Performance] {
            if snapshot.requests(scope).is_err() {
                continue;
            }
            let classes: Vec<_> = snapshot
                .classes
                .iter()
                .filter(|c| scope == ParkingScope::All || c.id == 1)
                .collect();
            let cores: u32 = classes.iter().map(|c| c.physical_cores).sum();
            let threads: u32 = classes.iter().map(|c| c.logical_processors).sum();
            let response = ui
                .add_enabled_ui(scope == ParkingScope::All, |ui| {
                    ui.radio_value(
                        &mut config.scope,
                        scope,
                        format!("{} · {cores} cores / {threads} threads", scope_label(scope)),
                    )
                })
                .inner;
            crate::debug_mcp_control!(format!("tweaks.parking.scope.{scope:?}"), response);
        }
        if snapshot.classes.len() > 1 {
            ui.small(SEPARATE_CLASS_UNAVAILABLE);
        }
        ui.small("Plugged in only. Battery settings stay unchanged.");
        ui.small("May increase power use and heat. Undo restores your previous settings.");
        config.bindings = None;
        *value = envelope(ID, &config);
    }
    fn tweak_summary(&self, value: &ModuleEnvelope) -> Option<String> {
        configuration(value)
            .ok()
            .map(|c| format!("{} · saved AC parking policy", scope_label(c.scope)))
    }
    fn tweak_verified_label(&self) -> &'static str {
        "Stored parking policy verified"
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        Some(format!(
            "Previous parking policy · {} stored fields",
            fields.len()
        ))
    }
    #[cfg(test)]
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        self.show_tweak_options(ui, config);
        Ok(false)
    }
}

type CardRead = (
    ModuleObservation,
    Result<bool, ModuleError>,
    Option<ModuleEnvelope>,
);
fn read_card(target: &TargetIdentity, saved: Option<&ModuleEnvelope>) -> CardRead {
    let result: Result<_, ModuleError> = (|| {
        let scope = saved
            .map(configuration)
            .transpose()?
            .map_or(ParkingScope::All, |c| c.scope);
        let snapshot = api::inspect(&observed_account_user(target)?)
            .map_err(|e| native("read parking policy", e))?;
        let ready = snapshot
            .requests(scope)
            .map_err(|e| native("read parking scope", e))?
            .is_empty();
        let count: u32 = snapshot
            .classes
            .iter()
            .filter(|c| scope == ParkingScope::All || c.id == 1)
            .map(|c| c.physical_cores)
            .sum();
        let summary = format!(
            "{}: {}\n{count} physical cores · AC policy only",
            scope_label(scope),
            if ready {
                "parking disabled"
            } else {
                "Windows-managed parking"
            }
        );
        Ok((
            summary,
            ready,
            envelope(
                ID,
                &Config {
                    scope,
                    snapshot: Some(snapshot),
                    bindings: None,
                },
            ),
        ))
    })();
    match result {
        Ok((summary, ready, options)) => (observation(Ok(summary)), Ok(ready), Some(options)),
        Err(e) => (observation(Err(e.clone())), Err(e), None),
    }
}
impl Module for CoreParking {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Core parking",
            "Keep a supported CPU class available using the captured AC parking policy.",
            TargetScope::User,
            vec![ActivationMode::Manual],
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
        read_card(target, None).0
    }
    fn observe_tweak(&self, target: &TargetIdentity, saved: Option<&ModuleEnvelope>) -> CardRead {
        read_card(target, saved)
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        value: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        let target = canonical_account_target(target)?;
        let config = configuration(value)?;
        let bindings = config
            .bindings
            .ok_or_else(|| error(ErrorKind::InvalidConfig, "Unprepared parking configuration"))?;
        Ok(bindings
            .into_iter()
            .map(|r| field(&target, &r.key.wire_id(), value_bytes(Some(r.value))))
            .collect())
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        native_api::read_value(&account_user(&key.target)?, parking_key(key)?)
            .map(value_bytes)
            .map_err(|e| native("read parking", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        native_api::write_value(
            &account_user(&key.target)?,
            parking_key(key)?,
            nullable_value(expected)?,
            nullable_value(desired)?,
        )
        .map_err(|e| native("write parking", e))
    }
    fn confirm_applied(&self, key: &ResourceKey, expected: &[u8]) -> Result<(), ModuleError> {
        native_api::confirm_value(
            &account_user(&key.target)?,
            parking_key(key)?,
            nullable_value(expected)?,
        )
        .map_err(|e| native("confirm parking", e))
    }
}

#[cfg(test)]
mod tests;
