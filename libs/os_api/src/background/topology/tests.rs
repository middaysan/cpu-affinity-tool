use super::*;

fn set(id: u32, group: u16, logical: u8, core: u8, class: u8) -> SetRecord {
    SetRecord {
        id,
        group,
        logical,
        core_hint: core,
        cache: 0,
        numa: 0,
        efficiency_class: class,
        unavailable: false,
    }
}
#[test]
fn joined_topology_keeps_physical_siblings_and_namespaces_processor_groups() {
    let sets = vec![
        set(10, 0, 0, 0, 8),
        set(11, 0, 1, 0, 8),
        set(20, 1, 0, 0, 0),
    ];
    let cores = vec![
        CoreRelation {
            masks: vec![(0, 3)],
            efficiency_class: 8,
        },
        CoreRelation {
            masks: vec![(1, 1)],
            efficiency_class: 0,
        },
    ];
    let joined = join(&sets, &cores).unwrap();
    assert_eq!(joined[0].physical_core, joined[1].physical_core);
    assert_ne!(joined[0].physical_core, joined[2].physical_core);
    assert_ne!(joined[0].cache, joined[2].cache);
    assert_ne!(joined[0].numa, joined[2].numa);
    assert_eq!(super::super::efficient_pool(&joined).unwrap(), vec![20]);
}
#[test]
fn joined_topology_refuses_split_missing_duplicate_or_disagreeing_core_evidence() {
    let core = CoreRelation {
        masks: vec![(0, 3)],
        efficiency_class: 0,
    };
    assert!(join(&[set(10, 0, 0, 0, 0)], std::slice::from_ref(&core)).is_err());
    assert!(
        join(
            &[set(10, 0, 0, 0, 0), set(11, 0, 1, 1, 0)],
            std::slice::from_ref(&core)
        )
        .is_err()
    );
    assert!(
        join(
            &[set(10, 0, 0, 0, 0), set(11, 0, 1, 0, 8)],
            std::slice::from_ref(&core)
        )
        .is_err()
    );
    assert!(
        join(
            &[set(10, 0, 0, 0, 0), set(11, 0, 1, 0, 0)],
            &[core.clone(), core]
        )
        .is_err()
    );
}
#[test]
fn glpi_core_decoder_bounds_variable_masks_and_rejects_truncated_input() {
    let mut data = vec![0u8; 48];
    data[4..8].copy_from_slice(&48u32.to_le_bytes());
    data[9] = 8;
    data[30..32].copy_from_slice(&1u16.to_le_bytes());
    data[32..40].copy_from_slice(&3u64.to_le_bytes());
    data[40..42].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        decode_cores(&data).unwrap(),
        vec![CoreRelation {
            masks: vec![(2, 3)],
            efficiency_class: 8
        }]
    );
    for length in 0..48 {
        assert!(decode_cores(&data[..length]).is_err());
    }
    data[30..32].copy_from_slice(&2u16.to_le_bytes());
    assert!(decode_cores(&data).is_err());
}
#[test]
fn cpu_set_decoder_distinguishes_parking_and_foreign_allocation() {
    let mut data = vec![0u8; 32];
    data[0..4].copy_from_slice(&32u32.to_le_bytes());
    data[8..12].copy_from_slice(&100u32.to_le_bytes());
    data[15] = 2;
    data[16] = 3;
    data[17] = 4;
    data[18] = 5;
    data[19] = 1;
    let parked = decode_sets(&data).unwrap();
    assert!(!parked[0].unavailable);
    assert_eq!(
        (parked[0].cache, parked[0].numa, parked[0].efficiency_class),
        (3, 4, 5)
    );
    data[19] = 2;
    assert!(decode_sets(&data).unwrap()[0].unavailable);
    data[19] = 6;
    assert!(!decode_sets(&data).unwrap()[0].unavailable);
    data[19] = 8;
    assert!(decode_sets(&data).unwrap()[0].unavailable);
    assert!(decode_sets(&data[..31]).is_err());
}
