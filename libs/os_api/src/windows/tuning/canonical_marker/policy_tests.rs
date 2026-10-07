use super::registry_policy::*;

fn trusted() -> DescriptorFacts {
    DescriptorFacts {
        owner: SidClass::Administrators,
        dacl_present: true,
        dacl_protected: true,
        dacl: vec![
            AceFacts {
                kind: 0,
                flags: 0,
                mask: KEY_ALL,
                sid: SidClass::System,
            },
            AceFacts {
                kind: 0,
                flags: 0,
                mask: KEY_ALL,
                sid: SidClass::Administrators,
            },
        ],
        labels: vec![AceFacts {
            kind: 17,
            flags: 0,
            mask: 1,
            sid: SidClass::HighIntegrity,
        }],
    }
}
#[test]
fn registry_policy_requires_trusted_owner_grants_and_high_label() {
    assert_eq!(validate(&trusted(), Scope::Managed), Ok(()));
    for alteration in 0..10 {
        let mut facts = trusted();
        match alteration {
            0 => facts.owner = SidClass::Other,
            1 => facts.owner = SidClass::TrustedInstaller,
            2 => facts.dacl_present = false,
            3 => facts.dacl_protected = false,
            4 => facts.dacl.clear(),
            5 => facts.dacl[0].mask = KEY_READ,
            6 => facts.dacl[1].mask = KEY_READ,
            7 => facts.labels.clear(),
            8 => facts.labels[0].flags = 8,
            _ => facts.labels[0].sid = SidClass::Other,
        }
        assert!(
            validate(&facts, Scope::Managed).is_err(),
            "alteration {alteration}"
        );
    }
    for mask in [
        0x2, 0x4, 0x20, 0x10000, 0x40000, 0x80000, 0x40000000, 0x10000000,
    ] {
        let mut facts = trusted();
        facts.dacl.push(AceFacts {
            kind: 0,
            flags: 0,
            mask,
            sid: SidClass::Other,
        });
        assert!(validate(&facts, Scope::Managed).is_err());
        assert!(validate(&facts, Scope::Ancestor).is_err());
    }
    for kind in [2, 5, 9, u8::MAX] {
        let mut facts = trusted();
        facts.dacl[0].kind = kind;
        assert!(validate(&facts, Scope::Managed).is_err());
    }
    // TrustedInstaller/read-only users are acceptable only on the ancestor.
    let mut ancestor = trusted();
    ancestor.owner = SidClass::TrustedInstaller;
    ancestor.dacl_protected = false;
    ancestor.labels.clear();
    ancestor.dacl.push(AceFacts {
        kind: 0,
        flags: 0,
        mask: 0x80000000,
        sid: SidClass::Other,
    });
    assert_eq!(validate(&ancestor, Scope::Ancestor), Ok(()));
    assert!(validate(&ancestor, Scope::Managed).is_err());
    assert_ne!(DANGEROUS_PARENT & KEY_WRITE, 0);
}
#[test]
fn registry_query_caps_precede_allocation_and_ace_walk() {
    assert_eq!(descriptor_buffer_size(20), Ok(RELATIVE_HEADER_BYTES));
    assert_eq!(descriptor_buffer_size(65536), Ok(MAX_DESCRIPTOR_BYTES));
    for length in [0, 19, 65537, u32::MAX] {
        assert!(descriptor_buffer_size(length).is_err());
    }
    assert_eq!(acl_walk_bounds(8, 0), Ok(()));
    assert_eq!(acl_walk_bounds(65536, MAX_ACES), Ok(()));
    for (bytes, count) in [(7, 0), (65537, 0), (8, 257), (u32::MAX, u32::MAX)] {
        assert!(acl_walk_bounds(bytes, count).is_err());
    }
    assert!(acl_walk_bounds(8, 1).is_err());
    assert_eq!(sid_read_size(15, 68), Ok(MAX_SID_BYTES));
    assert!(sid_read_size(16, 72).is_err());
    assert!(sid_read_size(15, 67).is_err());
    assert_eq!(relative_range(100, 20, 80), Ok(20..100));
    for (total, offset, size) in [(100, 19, 1), (100, 20, 81), (100, usize::MAX, 2)] {
        assert!(relative_range(total, offset, size).is_err());
    }
    assert!(relative_range(65537, 20, 16).is_err());
    assert!(relative_range(100, 20, 0).is_err());
}

#[test]
fn software_ancestor_rejects_untrusted_owner_and_mutation_without_managed_label_rules() {
    let mut ancestor = trusted();
    ancestor.owner = SidClass::TrustedInstaller;
    ancestor.dacl_protected = false;
    ancestor.labels.clear();
    ancestor.dacl.push(AceFacts {
        kind: 0,
        flags: 0,
        mask: KEY_READ,
        sid: SidClass::Other,
    });
    assert_eq!(validate(&ancestor, Scope::Ancestor), Ok(()));
    assert!(validate(&ancestor, Scope::Managed).is_err());
    for mask in [
        0x2, 0x4, 0x20, 0x10000, 0x40000, 0x80000, 0x40000000, 0x10000000,
    ] {
        let mut unsafe_parent = ancestor.clone();
        unsafe_parent.dacl.last_mut().unwrap().mask = mask;
        assert!(validate(&unsafe_parent, Scope::Ancestor).is_err());
    }
    for owner in [SidClass::Other, SidClass::HighIntegrity] {
        let mut unsafe_parent = ancestor.clone();
        unsafe_parent.owner = owner;
        assert!(validate(&unsafe_parent, Scope::Ancestor).is_err());
    }
    let mut missing_dacl = ancestor.clone();
    missing_dacl.dacl_present = false;
    assert!(validate(&missing_dacl, Scope::Ancestor).is_err());
    let mut complex = ancestor.clone();
    complex.dacl[0].kind = 9;
    assert!(validate(&complex, Scope::Ancestor).is_err());
}

#[test]
fn registry_denies_and_generic_masks_do_not_fabricate_effective_full_grants() {
    let mut generic = trusted();
    generic.dacl[0].mask = 0x10000000;
    generic.dacl[1].mask = 0x10000000;
    assert_eq!(validate(&generic, Scope::Managed), Ok(()));
    for sid in [SidClass::System, SidClass::Administrators, SidClass::Other] {
        let mut denied = trusted();
        denied.dacl.insert(
            0,
            AceFacts {
                kind: 1,
                flags: 0,
                mask: KEY_ALL,
                sid,
            },
        );
        assert!(validate(&denied, Scope::Managed).is_err());
        assert!(validate(&denied, Scope::Ancestor).is_err());
    }
    for mask in [0x00100000, 0x02000000, u32::MAX] {
        let mut unknown = trusted();
        unknown.dacl[0].mask = mask;
        assert!(validate(&unknown, Scope::Managed).is_err());
    }
    let mut inherited_only = trusted();
    inherited_only.dacl[0].flags = 8;
    assert!(validate(&inherited_only, Scope::Managed).is_err());
    let mut extra_label = trusted();
    extra_label.labels.push(extra_label.labels[0].clone());
    assert!(validate(&extra_label, Scope::Managed).is_err());
    let mut bad_label = trusted();
    bad_label.labels[0].mask = 0;
    assert!(validate(&bad_label, Scope::Managed).is_err());
}
