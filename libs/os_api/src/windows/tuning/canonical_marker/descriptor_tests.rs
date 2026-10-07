use super::{registry_policy::*, security_descriptor::parse_descriptor};

const SELF_RELATIVE: u16 = 0x8000;
const DACL_PRESENT: u16 = 4;
const SACL_PRESENT: u16 = 16;
const DACL_PROTECTED: u16 = 0x1000;

fn sid(authority: u8, subauthorities: &[u32]) -> Vec<u8> {
    let mut bytes = vec![1, subauthorities.len() as u8, 0, 0, 0, 0, 0, authority];
    for subauthority in subauthorities {
        bytes.extend(subauthority.to_le_bytes());
    }
    bytes
}
fn system() -> Vec<u8> {
    sid(5, &[18])
}
fn administrators() -> Vec<u8> {
    sid(5, &[32, 544])
}
fn installer() -> Vec<u8> {
    sid(
        5,
        &[
            80, 956008885, 3418522649, 1831038044, 1853292631, 2271478464,
        ],
    )
}
fn high() -> Vec<u8> {
    sid(16, &[12288])
}
fn ace(kind: u8, mask: u32, trustee: &[u8]) -> Vec<u8> {
    let mut bytes = vec![kind, 0];
    bytes.extend(((8 + trustee.len()) as u16).to_le_bytes());
    bytes.extend(mask.to_le_bytes());
    bytes.extend(trustee);
    bytes
}
fn acl(entries: &[Vec<u8>]) -> Vec<u8> {
    let length = 8 + entries.iter().map(Vec::len).sum::<usize>();
    assert!(length <= u16::MAX as usize);
    let mut bytes = vec![2, 0];
    bytes.extend((length as u16).to_le_bytes());
    bytes.extend((entries.len() as u16).to_le_bytes());
    bytes.extend([0, 0]);
    for entry in entries {
        bytes.extend(entry);
    }
    bytes
}
fn descriptor(
    owner: &[u8],
    dacl: &[Vec<u8>],
    labels: Option<&[Vec<u8>]>,
    protected: bool,
) -> Vec<u8> {
    let mut bytes = vec![0; 20];
    bytes[0] = 1;
    let mut control = SELF_RELATIVE | DACL_PRESENT;
    if protected {
        control |= DACL_PROTECTED;
    }
    if labels.is_some() {
        control |= SACL_PRESENT;
    }
    bytes[2..4].copy_from_slice(&control.to_le_bytes());
    bytes[4..8].copy_from_slice(&20u32.to_le_bytes());
    bytes.extend(owner);
    let offset = bytes.len() as u32;
    bytes[16..20].copy_from_slice(&offset.to_le_bytes());
    bytes.extend(acl(dacl));
    if let Some(labels) = labels {
        let offset = bytes.len() as u32;
        bytes[12..16].copy_from_slice(&offset.to_le_bytes());
        bytes.extend(acl(labels));
    }
    bytes
}
fn managed() -> Vec<u8> {
    descriptor(
        &administrators(),
        &[
            ace(0, KEY_ALL, &system()),
            ace(0, KEY_ALL, &administrators()),
        ],
        Some(&[ace(17, 1, &high())]),
        true,
    )
}
fn dacl_offset(bytes: &[u8]) -> usize {
    u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize
}
fn sacl_offset(bytes: &[u8]) -> usize {
    u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize
}

#[test]
fn descriptor_bytes_parse_managed_and_software_ancestor_without_authority() {
    let bytes = managed();
    let facts = parse_descriptor(&bytes, Scope::Managed).expect("valid bounded native shape");
    assert_eq!(facts.owner, SidClass::Administrators);
    assert!(facts.dacl_present && facts.dacl_protected);
    assert_eq!(facts.dacl.len(), 2);
    assert_eq!(facts.labels.len(), 1);
    assert_eq!(facts.labels[0].sid, SidClass::HighIntegrity);
    assert_eq!(validate(&facts, Scope::Managed), Ok(()));
    let ancestor = descriptor(
        &installer(),
        &[
            ace(0, KEY_ALL, &system()),
            ace(0, KEY_ALL, &administrators()),
            ace(0, KEY_READ, &sid(5, &[21, 1, 2, 3, 1001])),
        ],
        None,
        false,
    );
    let facts = parse_descriptor(&ancestor, Scope::Ancestor).unwrap();
    assert_eq!(facts.owner, SidClass::TrustedInstaller);
    assert!(!facts.dacl_protected);
    assert!(facts.labels.is_empty());
    assert_eq!(validate(&facts, Scope::Ancestor), Ok(()));
    assert!(validate(&facts, Scope::Managed).is_err());
}
#[test]
fn descriptor_header_offsets_and_sid_revision_fail_before_payload_walk() {
    let valid = managed();
    assert!(parse_descriptor(&valid, Scope::Managed).is_ok());
    for mutation in 0..10 {
        let mut bad = valid.clone();
        match mutation {
            0 => bad[0] = 2,
            1 => bad[1] = 1,
            2 => bad[2..4].copy_from_slice(&DACL_PRESENT.to_le_bytes()),
            3 => bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes()),
            4 => bad[4..8].copy_from_slice(&19u32.to_le_bytes()),
            5 => bad[4..8].copy_from_slice(&0u32.to_le_bytes()),
            6 => bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes()),
            7 => bad[12..16].copy_from_slice(&u32::MAX.to_le_bytes()),
            8 => bad[16..20].copy_from_slice(&u32::MAX.to_le_bytes()),
            _ => bad[20] = 2,
        }
        assert!(
            parse_descriptor(&bad, Scope::Managed).is_err(),
            "mutation {mutation}"
        );
    }
    for length in [0, 19] {
        assert!(parse_descriptor(&valid[..length], Scope::Managed).is_err());
    }
    let mut huge = valid;
    huge.resize(65537, 0);
    assert!(parse_descriptor(&huge, Scope::Managed).is_err());
}
#[test]
fn descriptor_acl_and_ace_lengths_are_bounded_before_sid_read() {
    let valid = managed();
    assert!(parse_descriptor(&valid, Scope::Managed).is_ok());
    let dacl = dacl_offset(&valid);
    let first_ace = dacl + 8;
    let sid_start = first_ace + 8;
    for mutation in 0..9 {
        let mut bad = valid.clone();
        match mutation {
            0 => bad[dacl] = 1,
            1 => bad[dacl + 2..dacl + 4].copy_from_slice(&7u16.to_le_bytes()),
            2 => bad[dacl + 2..dacl + 4].copy_from_slice(&u16::MAX.to_le_bytes()),
            3 => bad[dacl + 4..dacl + 6].copy_from_slice(&257u16.to_le_bytes()),
            4 => bad[first_ace + 2..first_ace + 4].copy_from_slice(&0u16.to_le_bytes()),
            5 => bad[first_ace + 2..first_ace + 4].copy_from_slice(&15u16.to_le_bytes()),
            6 => bad[first_ace + 2..first_ace + 4].copy_from_slice(&u16::MAX.to_le_bytes()),
            7 => bad[sid_start + 1] = 16,
            _ => bad[first_ace] = 9,
        }
        assert!(
            parse_descriptor(&bad, Scope::Managed).is_err(),
            "mutation {mutation}"
        );
    }
    let mut bad_label = valid.clone();
    bad_label[sacl_offset(&valid) + 8 + 8] = 2;
    assert!(parse_descriptor(&bad_label, Scope::Managed).is_err());
    let mut misaligned = valid.clone();
    misaligned[16..20].copy_from_slice(&((dacl + 1) as u32).to_le_bytes());
    assert!(parse_descriptor(&misaligned, Scope::Managed).is_err());
}
#[test]
fn descriptor_exact_caps_and_other_sid_are_shape_only() {
    let mut exact = managed();
    exact.resize(MAX_DESCRIPTOR_BYTES, 0);
    assert!(parse_descriptor(&exact, Scope::Managed).is_ok());
    let mut entries = vec![ace(0, KEY_ALL, &system()); 255];
    entries.push(ace(0, KEY_ALL, &administrators()));
    let exact = descriptor(
        &administrators(),
        &entries,
        Some(&[ace(17, 1, &high())]),
        true,
    );
    let facts = parse_descriptor(&exact, Scope::Managed).unwrap();
    assert_eq!(facts.dacl.len(), 256);
    assert!(validate(&facts, Scope::Managed).is_ok());
    entries.push(ace(0, KEY_READ, &system()));
    let too_many = descriptor(
        &administrators(),
        &entries,
        Some(&[ace(17, 1, &high())]),
        true,
    );
    assert!(parse_descriptor(&too_many, Scope::Managed).is_err());
    let long_sid = sid(5, &[21; 15]);
    assert_eq!(long_sid.len(), MAX_SID_BYTES);
    let other = descriptor(&long_sid, &[ace(0, KEY_ALL, &system())], None, false);
    let facts = parse_descriptor(&other, Scope::Ancestor).unwrap();
    assert_eq!(facts.owner, SidClass::Other);
    assert!(validate(&facts, Scope::Ancestor).is_err());
    let invalid_sid = sid(5, &[21; 16]);
    let other = descriptor(&invalid_sid, &[ace(0, KEY_ALL, &system())], None, false);
    assert!(parse_descriptor(&other, Scope::Ancestor).is_err());
}

#[test]
fn descriptor_sid_classes_require_exact_authority_count_and_every_subauthority() {
    for (owner, expected) in [
        (system(), SidClass::System),
        (administrators(), SidClass::Administrators),
        (installer(), SidClass::TrustedInstaller),
        (high(), SidClass::HighIntegrity),
    ] {
        let bytes = descriptor(&owner, &[ace(0, KEY_ALL, &system())], None, false);
        let facts = parse_descriptor(&bytes, Scope::Ancestor).unwrap();
        assert_eq!(facts.owner, expected);
    }
    let mut different_authority = system();
    different_authority[2] = 1;
    let aliases = [
        sid(5, &[18, 0]),
        sid(5, &[19]),
        sid(5, &[32, 544, 0]),
        sid(5, &[32, 545]),
        sid(
            5,
            &[
                80, 956008885, 3418522649, 1831038044, 1853292631, 2271478464, 0,
            ],
        ),
        sid(
            5,
            &[
                80, 956008885, 3418522649, 1831038044, 1853292631, 2271478465,
            ],
        ),
        sid(16, &[12288, 0]),
        sid(16, &[12289]),
        different_authority,
    ];
    for alias in aliases {
        let bytes = descriptor(&alias, &[ace(0, KEY_ALL, &system())], None, false);
        let facts = parse_descriptor(&bytes, Scope::Ancestor).unwrap();
        assert_eq!(facts.owner, SidClass::Other);
        assert!(validate(&facts, Scope::Ancestor).is_err());
        let bytes = descriptor(
            &administrators(),
            &[
                ace(0, KEY_ALL, &system()),
                ace(0, KEY_ALL, &administrators()),
            ],
            Some(&[ace(17, 1, &alias)]),
            true,
        );
        let facts = parse_descriptor(&bytes, Scope::Managed).unwrap();
        assert_eq!(facts.labels[0].sid, SidClass::Other);
        assert!(validate(&facts, Scope::Managed).is_err());
    }
}

#[test]
fn descriptor_raw_ace_kind_mask_and_flags_survive_into_security_policy() {
    let valid = managed();
    assert!(
        validate(
            &parse_descriptor(&valid, Scope::Managed).unwrap(),
            Scope::Managed
        )
        .is_ok()
    );
    let first_ace = dacl_offset(&valid) + 8;
    let label_ace = sacl_offset(&valid) + 8;

    let mut deny = valid.clone();
    deny[first_ace] = 1;
    let facts = parse_descriptor(&deny, Scope::Managed).unwrap();
    assert_eq!(facts.dacl[0].kind, 1);
    assert_eq!(facts.dacl[0].mask, KEY_ALL);
    assert!(validate(&facts, Scope::Managed).is_err());

    for mask in [KEY_READ, 0x0010_0000] {
        let mut read_only = valid.clone();
        read_only[first_ace + 4..first_ace + 8].copy_from_slice(&mask.to_le_bytes());
        let facts = parse_descriptor(&read_only, Scope::Managed).unwrap();
        assert_eq!(facts.dacl[0].mask, mask);
        assert!(validate(&facts, Scope::Managed).is_err());
    }
    let mut ineffective_grant = valid.clone();
    ineffective_grant[first_ace + 1] = 8;
    let facts = parse_descriptor(&ineffective_grant, Scope::Managed).unwrap();
    assert_eq!(facts.dacl[0].flags, 8);
    assert!(validate(&facts, Scope::Managed).is_err());

    let mut ineffective_label = valid.clone();
    ineffective_label[label_ace + 1] = 8;
    let facts = parse_descriptor(&ineffective_label, Scope::Managed).unwrap();
    assert_eq!(facts.labels[0].kind, 17);
    assert_eq!(facts.labels[0].mask, 1);
    assert_eq!(facts.labels[0].flags, 8);
    assert!(validate(&facts, Scope::Managed).is_err());
}

fn creator_owner_ancestor(trustee: &[u8], flags: u8) -> Vec<u8> {
    let mut placeholder = ace(0, KEY_ALL, trustee);
    placeholder[1] = flags;
    descriptor(
        &administrators(),
        &[
            placeholder,
            ace(0, KEY_ALL, &system()),
            ace(0, KEY_ALL, &administrators()),
        ],
        None,
        false,
    )
}

#[test]
fn creator_owner_observed_container_placeholder_is_ancestor_only() {
    let bytes = creator_owner_ancestor(&sid(3, &[0]), 2);
    let facts = parse_descriptor(&bytes, Scope::Ancestor).unwrap();
    assert_eq!(facts.dacl[0].sid, SidClass::CreatorOwner);
    assert_eq!(facts.dacl[0].flags, 2);
    assert_eq!(facts.dacl[0].mask, KEY_ALL);
    assert_eq!(validate(&facts, Scope::Ancestor), Ok(()));

    let mut managed_facts = facts;
    managed_facts.dacl_protected = true;
    managed_facts.labels = parse_descriptor(&managed(), Scope::Managed).unwrap().labels;
    assert!(validate(&managed_facts, Scope::Managed).is_err());
}

#[test]
fn creator_owner_aliases_and_ordinary_mutation_are_not_placeholders() {
    let mut different_authority = sid(3, &[0]);
    different_authority[2] = 1;
    for trustee in [
        sid(3, &[1]),
        sid(3, &[4]),
        sid(3, &[0, 0]),
        sid(5, &[0]),
        sid(5, &[21, 1, 2, 3, 1001]),
        different_authority,
    ] {
        let facts =
            parse_descriptor(&creator_owner_ancestor(&trustee, 2), Scope::Ancestor).unwrap();
        assert_eq!(facts.dacl[0].sid, SidClass::Other);
        assert!(validate(&facts, Scope::Ancestor).is_err());
    }
}

#[test]
fn creator_owner_does_not_trust_an_owner_or_supply_managed_full_grants() {
    let bytes = descriptor(&sid(3, &[0]), &[ace(0, KEY_ALL, &system())], None, false);
    let facts = parse_descriptor(&bytes, Scope::Ancestor).unwrap();
    assert!(validate(&facts, Scope::Ancestor).is_err());
    for missing in [system(), administrators()] {
        let bytes = descriptor(
            &administrators(),
            &[ace(0, KEY_ALL, &sid(3, &[0])), ace(0, KEY_ALL, &missing)],
            Some(&[ace(17, 1, &high())]),
            true,
        );
        let facts = parse_descriptor(&bytes, Scope::Managed).unwrap();
        assert!(validate(&facts, Scope::Managed).is_err());
    }
}

#[test]
fn creator_owner_exception_requires_exact_observed_ace_shape() {
    for flags in [0, 1, 4, 8, 10, 16, 18, 0x20, 0xff] {
        let bytes = creator_owner_ancestor(&sid(3, &[0]), flags);
        let facts = parse_descriptor(&bytes, Scope::Ancestor).unwrap();
        assert!(validate(&facts, Scope::Ancestor).is_err(), "flags {flags}");
    }
    let valid = creator_owner_ancestor(&sid(3, &[0]), 2);
    let first = dacl_offset(&valid) + 8;
    for kind in [1, 5, 9] {
        let mut bytes = valid.clone();
        bytes[first] = kind;
        assert!(
            parse_descriptor(&bytes, Scope::Ancestor)
                .and_then(|facts| validate(&facts, Scope::Ancestor))
                .is_err()
        );
    }
    for mask in [KEY_READ, KEY_WRITE, 0x1000_0000, 0x0010_0000, u32::MAX] {
        let mut bytes = valid.clone();
        bytes[first + 4..first + 8].copy_from_slice(&mask.to_le_bytes());
        let facts = parse_descriptor(&bytes, Scope::Ancestor).unwrap();
        assert!(validate(&facts, Scope::Ancestor).is_err(), "mask {mask}");
    }
}
