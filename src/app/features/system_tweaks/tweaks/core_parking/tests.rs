use super::*;
use os_api::{
    core_parking::{ParkingClass, ParkingValue},
    gaming::GuidValue,
    gaming_cpu_power::{CpuPowerKey, CpuPowerSetting},
};

fn snapshot() -> ParkingSnapshot {
    let scheme = GuidValue(0x381b4222_f694_41f0_9685_ff5bb260df2eu128.to_be_bytes());
    ParkingSnapshot {
        scheme,
        overlay: GuidValue([0; 16]),
        classes: vec![
            ParkingClass {
                id: 0,
                physical_cores: 8,
                logical_processors: 8,
            },
            ParkingClass {
                id: 1,
                physical_cores: 6,
                logical_processors: 12,
            },
        ],
        values: [
            CpuPowerSetting::ParkingMinimum,
            CpuPowerSetting::ParkingMaximum,
            CpuPowerSetting::ParkingMinimum1,
            CpuPowerSetting::ParkingMaximum1,
        ]
        .into_iter()
        .map(|setting| ParkingValue {
            key: CpuPowerKey { scheme, setting },
            effective: 25,
        })
        .collect(),
    }
}
fn prepared(scope: ParkingScope) -> ModuleEnvelope {
    let snapshot = snapshot();
    envelope(
        ID,
        &Config {
            scope,
            bindings: Some(snapshot.requests(scope).unwrap()),
            snapshot: Some(snapshot),
        },
    )
}

#[test]
fn parking_module_is_manual_off_by_default_and_requires_prepared_keys() {
    let module = CoreParking;
    assert_eq!(
        module.descriptor().activation_modes,
        vec![ActivationMode::Manual]
    );
    assert!(module.accepts_config(&module.default_config()));
    let target = TargetIdentity::Account {
        sid: "S-1-5-21-1".into(),
    };
    assert!(module.fields(&target, &module.default_config()).is_err());
    let registry = crate::app::features::system_tweaks::tweaks::registry();
    if crate::app::features::system_tweaks::MODULES_ENABLED {
        assert!(registry.get(&feature(ID)).is_some());
    }
}

#[test]
fn parking_performance_config_never_includes_other_class_or_frequency_fields() {
    let module = CoreParking;
    let target = TargetIdentity::Account {
        sid: "S-1-5-21-1".into(),
    };
    let separate = prepared(ParkingScope::Performance);
    assert!(
        !module.accepts_config(&separate),
        "Unqualified class policy must not become a new intent"
    );
    assert!(module.fields(&target, &separate).is_err());
    assert!(module.prepare_tweak_options(&target, &separate).is_err());
    let config = prepared(ParkingScope::All);
    let fields = module.fields(&target, &config).unwrap();
    assert_eq!(fields.len(), 4);
    for field in fields {
        let key = CpuPowerKey::from_wire_id(field.key.field.as_str()).unwrap();
        assert!(matches!(
            key.setting,
            CpuPowerSetting::ParkingMinimum
                | CpuPowerSetting::ParkingMaximum
                | CpuPowerSetting::ParkingMinimum1
                | CpuPowerSetting::ParkingMaximum1
        ));
        assert_eq!(nullable_value(&field.desired).unwrap(), Some(100));
    }
    for path in ["value", "key"] {
        let mut malformed = config.clone();
        if path == "value" {
            malformed.payload["bindings"][0]["value"] = 50.into();
        } else {
            malformed.payload["bindings"][0]["key"]["setting"] = "Frequency".into();
        }
        assert!(!module.accepts_config(&malformed));
    }
    let mut foreign = config.clone();
    foreign.payload["unexpected"] = true.into();
    assert!(!module.accepts_config(&foreign));
    let reloaded = ModuleEnvelope::from_bytes(&config.to_bytes().unwrap()).unwrap();
    assert_eq!(
        module.fields(&target, &reloaded).unwrap()[0].key,
        module.fields(&target, &config).unwrap()[0].key
    );
}

#[test]
fn parking_chooser_shows_only_confirmed_scopes_and_stays_compact() {
    let mut config = prepared(ParkingScope::All);
    let mut output = eframe::egui::Context::default().run_ui(
        eframe::egui::RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(
                eframe::egui::Pos2::ZERO,
                eframe::egui::vec2(360.0, 500.0),
            )),
            ..Default::default()
        },
        |ui| CoreParking.show_tweak_options(ui, &mut config),
    );
    output.textures_delta.clear();
    let text = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            eframe::egui::Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("All cores") && text.contains("Performance cores"),
        "{text}"
    );
    assert!(
        text.contains("6 cores") && text.contains("12 threads"),
        "{text}"
    );
    assert!(
        !text.contains("E-cores only") && !text.contains("MHz"),
        "{text}"
    );
}

#[test]
fn parking_prepare_refuses_plan_or_topology_changes_instead_of_retargeting() {
    let source = configuration(&prepared(ParkingScope::All)).unwrap();
    let mut changed = snapshot();
    changed.scheme = GuidValue([8; 16]);
    assert!(bind(source.clone(), changed).is_err());
    let mut changed = snapshot();
    changed.classes[0].physical_cores += 1;
    assert!(bind(source.clone(), changed).is_err());
    assert!(bind(source, snapshot()).is_ok());
}

#[test]
fn parking_absent_original_codec_is_exact_and_rejects_invalid_tokens() {
    for value in [None, Some(0), Some(37), Some(100)] {
        assert_eq!(nullable_value(&value_bytes(value)).unwrap(), value);
    }
    for bytes in [vec![], vec![0, 0], vec![1, 100], vec![1, 101, 0, 0, 0]] {
        assert!(nullable_value(&bytes).is_err());
    }
}
