//! Explicit disposable-plan validation; never selects or creates a host power plan.
use super::*;
use os_api::{
    core_parking::{self as api, ParkingScope},
    gaming_cpu_power::{self as cpu, CpuPowerKey, CpuPowerSetting::*},
};

#[test]
#[ignore = "ROOT: requires the helper-owned disposable power plan and balanced AC mode"]
fn native_roundtrip_core_parking_scopes() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let user = checked(gaming::interactive_user())?;
    let expected = std::env::var("CPU_AFFINITY_PARKING_TEST_PLAN")
        .map_err(|_| "Disposable plan witness missing")?;
    let expected = checked(u128::from_str_radix(&expected.replace('-', ""), 16))?;
    let plan = checked(gaming::read_power_plan(&user))?;
    assert_eq!(plan, gaming::GuidValue(expected.to_be_bytes()));
    assert!(checked(gaming::power_schemes(&user))?
        .iter()
        .any(|s| s.id == plan && s.name.starts_with("CpuAffinityTool parking validation")));
    assert_eq!(
        checked(gaming::read_power_mode(&user, gaming::PowerSource::Ac))?,
        gaming::POWER_MODE_BALANCED
    );
    let dc = checked(gaming::read_power_mode(&user, gaming::PowerSource::Dc))?;
    let fields = [
        Frequency,
        Frequency1,
        Maximum,
        Maximum1,
        Minimum,
        Minimum1,
        EnergyPreference,
        EnergyPreference1,
        Boost,
        ParkingMinimum,
        ParkingMinimum1,
        ParkingMaximum,
        ParkingMaximum1,
    ];
    let snapshot = checked(api::inspect(&user))?;
    assert!(snapshot.requests(ParkingScope::All).is_ok());
    let scopes = vec![ParkingScope::All];
    for scope in scopes {
        let registry = tweaks::registry();
        let module = module(&registry, "core_parking");
        let target = checked(module.canonical_target(&user_target(&user)))?;
        let headless = checked(module.editor().tweak_config(&target))?;
        assert_eq!(headless.payload["scope"], json!("All"));
        let (_, _, options) = module.observe_tweak(&target, None);
        let mut options = options.ok_or("Native choices unavailable")?;
        options.payload["scope"] = checked(serde_json::to_value(scope))?;
        let prepared = checked(module.editor().prepare_tweak_options(&target, &options))?;
        assert_eq!(
            headless.payload["bindings"], prepared.payload["bindings"],
            "Headless Apply must use the same captured all-core keys"
        );
        let changes = checked(module.fields(&target, &prepared))?;
        assert!(!changes.is_empty());
        assert!(changes
            .iter()
            .all(|f| CpuPowerKey::from_wire_id(f.key.field.as_str())
                .unwrap()
                .scheme
                == plan));
        let originals = fields
            .into_iter()
            .map(|setting| {
                let key = CpuPowerKey {
                    scheme: plan,
                    setting,
                };
                checked(cpu::read_value(&user, key)).map(|value| (key, value))
            })
            .collect::<TestResult<Vec<_>>>()?;
        roundtrip(registry, module, target, prepared, || {
            for (key, value) in &originals {
                if !changes
                    .iter()
                    .any(|f| f.key.field.as_str() == key.wire_id())
                {
                    assert_eq!(
                        checked(cpu::read_value(&user, *key))?,
                        *value,
                        "Unselected field changed: {key:?}"
                    );
                }
            }
            assert_eq!(checked(gaming::read_power_plan(&user))?, plan);
            assert_eq!(
                checked(gaming::read_power_mode(&user, gaming::PowerSource::Dc))?,
                dc
            );
            Ok(())
        })?;
        for (key, value) in &originals {
            assert_eq!(checked(cpu::read_value(&user, *key))?, *value);
        }
        eprintln!("Parking {scope:?}: Apply/readback/disk reload/Undo PASS; {} changed fields; other AC fields and DC mode unchanged",changes.len());
    }
    // Exercise absent restoration on this disposable plan only, after normal cases.
    let scope = ParkingScope::All;
    let key = CpuPowerKey {
        scheme: plan,
        setting: if scope == ParkingScope::Performance {
            ParkingMinimum1
        } else {
            ParkingMinimum
        },
    };
    let previous = checked(cpu::read_value(&user, key))?;
    checked(cpu::write_value(&user, key, previous, None))?;
    assert_eq!(checked(cpu::read_value(&user, key))?, None);
    let registry = tweaks::registry();
    let module = module(&registry, "core_parking");
    let target = checked(module.canonical_target(&user_target(&user)))?;
    let (_, _, options) = module.observe_tweak(&target, None);
    let mut options = options.ok_or("Absent-override choices unavailable")?;
    options.payload["scope"] = checked(serde_json::to_value(scope))?;
    let prepared = checked(module.editor().prepare_tweak_options(&target, &options))?;
    assert!(checked(module.fields(&target, &prepared))?
        .iter()
        .any(|f| f.key.field.as_str() == key.wire_id()));
    roundtrip(registry, module, target, prepared, || Ok(()))?;
    assert_eq!(checked(cpu::read_value(&user, key))?, None);
    // Return even the test precondition before handing the plan back to its owner.
    checked(cpu::write_value(&user, key, None, previous))?;
    eprintln!("Parking absent original: exact absence after disk-reloaded Undo PASS");
    Ok(())
}
