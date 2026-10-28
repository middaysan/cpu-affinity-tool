use super::*;
use crate::{background::PlacementCpu, gaming_cpu_power::CpuPowerSetting};

fn cpu(id: u32, group: u16, core: u32, class: u8) -> PlacementCpu {
    PlacementCpu {
        id,
        group,
        logical: id as u8,
        physical_core: core,
        efficiency_class: class,
        numa: 0,
        cache: 0,
        allowed: true,
        reserved: false,
    }
}

#[test]
fn parking_classes_count_smt_and_preserve_groups_without_guessing_chiplets() {
    let classes = classes_from_topology(&[
        cpu(0, 0, 0, 1),
        cpu(1, 0, 0, 1),
        cpu(2, 0, 1, 0),
        cpu(3, 1, 0, 0),
    ])
    .unwrap();
    assert_eq!(
        classes
            .iter()
            .map(|c| (c.id, c.physical_cores, c.logical_processors))
            .collect::<Vec<_>>(),
        vec![(0, 2, 2), (1, 1, 2)]
    );
}

#[test]
fn parking_unknown_class_mapping_or_conflicting_topology_is_unavailable() {
    for cpus in [
        vec![],
        vec![cpu(0, 0, 0, 2)],
        vec![cpu(0, 0, 0, 1)],
        vec![cpu(0, 0, 0, 0), cpu(0, 0, 1, 1)],
        vec![cpu(0, 0, 0, 0), cpu(1, 0, 0, 1)],
    ] {
        assert!(classes_from_topology(&cpus).is_err(), "{cpus:?}");
    }
}

#[test]
fn performance_scope_changes_only_class_one_parking_fields() {
    let hybrid = classes_from_topology(&[cpu(0, 0, 0, 0), cpu(1, 0, 1, 1)]).unwrap();
    assert_eq!(
        settings_for_scope(&hybrid, ParkingScope::Performance).unwrap(),
        vec![
            CpuPowerSetting::ParkingMinimum1,
            CpuPowerSetting::ParkingMaximum1
        ]
    );
    assert_eq!(
        settings_for_scope(&hybrid, ParkingScope::All).unwrap(),
        vec![
            CpuPowerSetting::ParkingMinimum,
            CpuPowerSetting::ParkingMaximum,
            CpuPowerSetting::ParkingMinimum1,
            CpuPowerSetting::ParkingMaximum1
        ]
    );
    let homogeneous = classes_from_topology(&[cpu(0, 0, 0, 0)]).unwrap();
    assert!(settings_for_scope(&homogeneous, ParkingScope::Performance).is_err());
    assert_eq!(
        settings_for_scope(&homogeneous, ParkingScope::All)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn parking_snapshot_rejects_foreign_duplicate_and_missing_fields() {
    let scheme = crate::gaming::GuidValue([4; 16]);
    let mut snapshot = ParkingSnapshot {
        scheme,
        overlay: crate::gaming::GuidValue([0; 16]),
        classes: classes_from_topology(&[cpu(0, 0, 0, 0)]).unwrap(),
        values: vec![],
    };
    for setting in settings_for_scope(&snapshot.classes, ParkingScope::All).unwrap() {
        snapshot.values.push(ParkingValue {
            key: crate::gaming_cpu_power::CpuPowerKey { scheme, setting },
            effective: 25,
        });
    }
    assert_eq!(snapshot.requests(ParkingScope::All).unwrap().len(), 2);
    let mut other = snapshot.clone();
    other.values[0].key.setting = CpuPowerSetting::Frequency;
    assert!(other.requests(ParkingScope::All).is_err());
    let mut other = snapshot.clone();
    other.values.push(ParkingValue {
        key: crate::gaming_cpu_power::CpuPowerKey {
            scheme,
            setting: CpuPowerSetting::Frequency,
        },
        effective: 100,
    });
    assert!(other.requests(ParkingScope::All).is_err());
    let mut other = snapshot.clone();
    other.values.push(other.values[0].clone());
    assert!(other.requests(ParkingScope::All).is_err());
    let mut other = snapshot.clone();
    other.values[0].key.scheme = crate::gaming::GuidValue([5; 16]);
    assert!(other.requests(ParkingScope::All).is_err());
    snapshot.values.pop();
    assert!(snapshot.requests(ParkingScope::All).is_err());
}

#[test]
#[ignore = "explicit read-only native parking policy and topology probe"]
fn native_parking_discovery() {
    let user = crate::gaming::interactive_user().unwrap();
    let snapshot = inspect(&user).unwrap();
    eprintln!(
        "Parking discovery: {}",
        serde_json::to_string(&snapshot).unwrap()
    );
    for scope in [ParkingScope::All, ParkingScope::Performance] {
        eprintln!("{scope:?}: {:?}", snapshot.requests(scope));
    }
}
