use super::*;

fn proof() -> TargetProof {
    TargetProof {
        catalog_version: 1,
        role: Role::DiagnosticsTelemetry,
        pid: 42,
        creation_token: 400,
        image_path: "C:\\Windows\\System32\\svchost.exe".into(),
        sid: "S-1-5-18".into(),
        session: 0,
        logon: 999,
        os_build: 26100,
        image: FileIdentity {
            volume: 1,
            index: 2,
            size: 3,
            modified: 4,
        },
        context: RoleEvidence::Service {
            name: "DiagTrack".into(),
            service_type: 0x10,
            arguments: "-k utcsvc -p".into(),
            dll_path: Some("C:\\Windows\\System32\\diagtrack.dll".into()),
            dll_identity: Some(FileIdentity {
                volume: 1,
                index: 3,
                size: 4,
                modified: 5,
            }),
        },
    }
}

#[test]
fn native_context_witness_is_bounded_versioned_and_binds_the_exact_process() {
    let proof = proof();
    let process = crate::gaming::ProcessIdentity {
        pid: 42,
        creation_token: 400,
        image_path: proof.image_path.clone(),
    };
    assert!(proof.matches(&process));
    let roundtrip: TargetProof =
        serde_json::from_slice(&serde_json::to_vec(&proof).unwrap()).unwrap();
    assert_eq!(roundtrip, proof);
    let mut changed = process.clone();
    changed.creation_token += 1;
    assert!(!proof.matches(&changed));
    let mut future = proof.clone();
    future.catalog_version += 1;
    assert!(!future.matches(&process));
    let mut corrupt = proof.clone();
    corrupt.image_path.push('\0');
    assert!(!corrupt.valid());
}

#[test]
fn changed_service_membership_or_file_identity_changes_the_retained_context() {
    let original = proof();
    let mut changed = original.clone();
    changed.image.index += 1;
    assert_ne!(changed, original);
    changed = original.clone();
    if let RoleEvidence::Service { name, .. } = &mut changed.context {
        *name = "RpcSs".into();
    }
    assert!(!changed.valid());
}

#[test]
fn priority_never_raises_an_existing_low_class_or_accepts_high_classes() {
    assert_eq!(priority_change(0x20), PriorityDecision::LowerToBelowNormal);
    for value in [0x40, 0x4000] {
        assert_eq!(priority_change(value), PriorityDecision::AlreadySuitable);
    }
    for value in [0x8000, 0x80, 0x100, 0, 0xdead] {
        assert_eq!(priority_change(value), PriorityDecision::Unexpected);
    }
}

#[test]
fn names_are_only_candidate_filters_and_never_authorize_a_generic_host() {
    assert_eq!(candidate_role("COMPATTELRUNNER.EXE"), Some(Role::Appraiser));
    assert_eq!(
        candidate_role("mscorsvw.exe"),
        Some(Role::FrameworkOptimizer)
    );
    for name in ["svchost.exe", "dllhost.exe", "taskhostw.exe", "unknown.exe"] {
        assert_eq!(candidate_role(name), None);
    }
    assert_eq!(service_role("diagtrack"), Some(Role::DiagnosticsTelemetry));
    assert_eq!(service_role("RpcSs"), None);
}

#[test]
fn hard_exclusions_cannot_be_overridden_by_a_catalog_role_or_opt_in() {
    for name in [
        "MsMpEng.exe",
        "audiodg.exe",
        "explorer.exe",
        "WUDFHost.exe",
        "lsass.exe",
        "dwm.exe",
    ] {
        assert!(excluded_image(name), "{name}");
    }
    for name in [
        "Audiosrv",
        "MMCSS",
        "GameInputSvc",
        "RpcSs",
        "Dhcp",
        "Schedule",
        "Winmgmt",
    ] {
        assert!(excluded_service(name), "{name}");
    }
}

#[test]
fn service_membership_requires_one_stable_running_expected_role() {
    let expected = ServiceMember {
        name: "DiagTrack".into(),
        pid: 42,
        running: true,
    };
    assert!(single_service_role(
        Role::DiagnosticsTelemetry,
        42,
        std::slice::from_ref(&expected)
    ));
    let mut shared = vec![expected.clone()];
    shared.push(ServiceMember {
        name: "RpcSs".into(),
        pid: 42,
        running: true,
    });
    assert!(!single_service_role(
        Role::DiagnosticsTelemetry,
        42,
        &shared
    ));
    assert!(!single_service_role(
        Role::DiagnosticsTelemetry,
        43,
        std::slice::from_ref(&expected)
    ));
    assert!(!single_service_role(
        Role::DiagnosticsTelemetry,
        42,
        &[ServiceMember {
            running: false,
            ..expected
        }]
    ));
    assert!(!single_service_role(Role::DiagnosticsTelemetry, 42, &[]));
}

fn cpu(id: u32, group: u16, logical: u8, core: u32, class: u8) -> PlacementCpu {
    PlacementCpu {
        id,
        group,
        logical,
        physical_core: core,
        efficiency_class: class,
        numa: 0,
        cache: 0,
        allowed: true,
        reserved: false,
    }
}

#[test]
fn efficient_pool_uses_class_and_whole_physical_cores_not_cpu_number_or_parking() {
    let topology = vec![
        cpu(70, 0, 0, 0, 0),
        cpu(71, 0, 1, 0, 0),
        cpu(72, 0, 2, 1, 8),
        cpu(73, 0, 3, 1, 8),
    ];
    assert_eq!(efficient_pool(&topology), Ok(vec![70, 71]));
    let homogeneous = vec![cpu(70, 0, 0, 0, 4), cpu(71, 0, 1, 0, 4)];
    assert_eq!(
        efficient_pool(&homogeneous),
        Err(PlacementSkip::Homogeneous)
    );
}

#[test]
fn pool_never_splits_siblings_or_selects_reserved_or_unavailable_sets() {
    let topology = vec![
        cpu(70, 0, 0, 0, 0),
        cpu(71, 0, 1, 0, 0),
        cpu(72, 0, 2, 1, 8),
    ];
    assert_eq!(
        validate_pool(&[70], &topology),
        Err(PlacementSkip::SplitPhysicalCore)
    );
    assert_eq!(validate_pool(&[70, 71], &topology), Ok(()));
    let mut denied = topology.clone();
    denied[1].reserved = true;
    assert_eq!(
        validate_pool(&[70, 71], &denied),
        Err(PlacementSkip::Unavailable)
    );
    assert_eq!(
        validate_pool(&[999], &topology),
        Err(PlacementSkip::UnknownSet)
    );
    assert_eq!(validate_pool(&[], &topology), Err(PlacementSkip::Empty));
}

#[test]
fn physical_core_identity_includes_processor_group_and_supports_more_than_64_cpus() {
    let topology: Vec<_> = (0..96)
        .map(|i| {
            cpu(
                1000 + i,
                (i / 64) as u16,
                (i % 64) as u8,
                (i % 64) / 2,
                if i < 64 { 8 } else { 0 },
            )
        })
        .collect();
    let selected: Vec<_> = (1064..1096).collect();
    assert_eq!(efficient_pool(&topology), Ok(selected.clone()));
    assert_eq!(validate_pool(&selected, &topology), Ok(()));
}

#[test]
fn topology_fingerprint_changes_on_identity_or_availability_but_not_record_order() {
    let topology = vec![cpu(1, 0, 0, 0, 0), cpu(2, 0, 1, 1, 8)];
    let mut reversed = topology.clone();
    reversed.reverse();
    assert_eq!(
        topology_fingerprint(&topology),
        topology_fingerprint(&reversed)
    );
    reversed[0].allowed = false;
    assert_ne!(
        topology_fingerprint(&topology),
        topology_fingerprint(&reversed)
    );
}

#[test]
fn placement_does_not_widen_a_preexisting_default_set_restriction() {
    assert_eq!(preserves_restriction(&[4, 5], &[]), Ok(()));
    assert_eq!(preserves_restriction(&[4], &[4, 5]), Ok(()));
    assert_eq!(
        preserves_restriction(&[4, 5], &[4]),
        Err(PlacementSkip::ExistingRestriction)
    );
}
#[test]
fn identity_verification_does_not_certify_system_role_mutation() {
    for role in [
        Role::Appraiser,
        Role::DeviceCensus,
        Role::FrameworkOptimizer,
        Role::DiagnosticsTelemetry,
        Role::SearchIndexer,
        Role::SearchProtocol,
        Role::SearchFilter,
    ] {
        assert!(!role.mutation_qualified());
        assert!(role.qualification_note().contains("not qualified"));
    }
}
