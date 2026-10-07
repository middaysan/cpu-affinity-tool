//! Actual fixed relative appkey setup; native capabilities are never fabricated.
use super::super::test_support::TuningTemporaryDirectory as TempRoot;
use super::{
    TuningContinuityCandidateMode as Mode, UnknownReason, capsule,
    driver::{Component, Counts, Disposition, DriverFailure},
    fixture_tests::{Fixture, raw_value, set, set_named},
    native_key::{self, FixtureMarkerAccess, NativeKey},
    primitives::{self, Key},
};
use crate::tuning::{TuningNativeError, TuningStore, TuningStoreAuthority};
use std::{ffi::OsString, fs};
use windows::Win32::System::Registry::{
    HKEY, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_BINARY, REG_DWORD, REG_LINK, REG_NONE,
    REG_OPTION_OPEN_LINK, REG_SZ, REG_VALUE_TYPE, RegDeleteKeyExW, RegOpenKeyExW, RegQueryValueExW,
};

fn fresh(fixture: &Fixture, access: FixtureMarkerAccess) -> NativeKey {
    let marker = native_key::create_fixture_marker(&fixture.hive, access).unwrap();
    assert_eq!(marker.component(), Component::Marker);
    assert_eq!(marker.disposition(), Disposition::New);
    marker
}
fn spent(result: Result<(), DriverFailure>) {
    assert_eq!(
        result,
        Err(DriverFailure::Unknown(UnknownReason::Initialization))
    );
}
fn delete_exact_marker(fixture: &Fixture) {
    // Only this fixed empty child of our retained process-private appkey.
    assert_eq!(
        unsafe {
            RegDeleteKeyExW(
                fixture.hive.raw(),
                windows::core::w!("ContinuityV1"),
                KEY_WOW64_64KEY.0,
                None,
            )
        }
        .0,
        0
    );
}

#[test]
fn native_new_marker_writes_exact_full_capsule_once_and_preserves_first_value() {
    let fixture = Fixture::new();
    let marker = fresh(&fixture, FixtureMarkerAccess::ReadWrite);
    assert_eq!(
        primitives::marker_counts(marker.key()).unwrap(),
        Counts {
            values: 0,
            subkeys: 0
        }
    );
    assert_eq!(raw_value(marker.key()), None);
    let mut nonce = [0x21; 32];
    nonce[31] = 0x92;
    let original = capsule::encode(nonce).unwrap();
    native_key::set_new_marker_once(&marker, &original).unwrap();
    assert_eq!(
        raw_value(marker.key()),
        Some((REG_BINARY, original.to_vec()))
    );
    assert_eq!(primitives::read_capsule(marker.key()).unwrap().nonce, nonce);
    nonce[31] ^= 1;
    spent(native_key::set_new_marker_once(
        &marker,
        &capsule::encode(nonce).unwrap(),
    ));
    assert_eq!(
        raw_value(marker.key()),
        Some((REG_BINARY, original.to_vec()))
    );
    assert_eq!(
        primitives::marker_counts(marker.key()).unwrap(),
        Counts {
            values: 1,
            subkeys: 0
        }
    );
    assert_eq!(
        format!("{marker:?}"),
        "NativeKey { native_handle: opaque, authority: private }"
    );
    drop(marker);
    fixture.close();
}

#[test]
fn native_malformed_capsules_issue_no_set_and_leave_first_valid_attempt_unused() {
    let fixture = Fixture::new();
    let marker = fresh(&fixture, FixtureMarkerAccess::ReadWrite);
    let valid = capsule::encode([0x32; 32]).unwrap();
    let mut invalid = vec![[0; 56]];
    for offset in [0, 8, 12, 16, 20] {
        let mut bytes = valid;
        bytes[offset] ^= 1;
        invalid.push(bytes);
    }
    let mut zero_nonce = valid;
    zero_nonce[24..].fill(0);
    invalid.push(zero_nonce);
    for bytes in invalid {
        assert_eq!(
            native_key::set_new_marker_once(&marker, &bytes),
            Err(DriverFailure::Unknown(UnknownReason::MalformedCapsule))
        );
        assert_eq!(raw_value(marker.key()), None);
        assert_eq!(
            primitives::marker_counts(marker.key()).unwrap(),
            Counts {
                values: 0,
                subkeys: 0
            }
        );
    }
    native_key::set_new_marker_once(&marker, &valid).unwrap();
    assert_eq!(raw_value(marker.key()), Some((REG_BINARY, valid.to_vec())));
    spent(native_key::set_new_marker_once(&marker, &valid));
    drop(marker);
    fixture.close();
}

#[test]
fn native_readonly_marker_sdk_denial_consumes_its_only_attempt() {
    let fixture = Fixture::new();
    let marker = fresh(&fixture, FixtureMarkerAccess::ReadOnly);
    let valid = capsule::encode([0x43; 32]).unwrap();
    assert!(matches!(
        native_key::set_new_marker_once(&marker, &valid),
        Err(DriverFailure::Native(TuningNativeError::Denied {
            code: 5,
            ..
        }))
    ));
    spent(native_key::set_new_marker_once(&marker, &valid));
    assert_eq!(raw_value(marker.key()), None);
    assert_eq!(
        primitives::marker_counts(marker.key()).unwrap(),
        Counts {
            values: 0,
            subkeys: 0
        }
    );
    drop(marker);
    fixture.close();
}

#[test]
fn native_deleted_marker_sdk_error_consumes_its_only_attempt_without_recreation() {
    let fixture = Fixture::new();
    let marker = fresh(&fixture, FixtureMarkerAccess::ReadWrite);
    delete_exact_marker(&fixture);
    let valid = capsule::encode([0x54; 32]).unwrap();
    assert!(matches!(
        native_key::set_new_marker_once(&marker, &valid),
        Err(DriverFailure::Native(TuningNativeError::Io {
            code: 1018,
            ..
        }))
    ));
    spent(native_key::set_new_marker_once(&marker, &valid));
    assert!(primitives::marker_counts(marker.key()).is_err());
    let mut reopened = HKEY::default();
    assert_eq!(
        unsafe {
            RegOpenKeyExW(
                fixture.hive.raw(),
                windows::core::w!("ContinuityV1"),
                Some(REG_OPTION_OPEN_LINK.0),
                KEY_SET_VALUE | KEY_WOW64_64KEY,
                &mut reopened,
            )
        }
        .0,
        2
    );
    // Failure never defines a returned owned handle, so nothing is closed here.
    drop(marker);
    fixture.close();
}

#[test]
fn native_existing_sdk_disposition_never_receives_a_write_capability() {
    // Keep the wrong capsule type, with a well-formed native UTF-16 terminator.
    let mut wrong_type = vec![0x77; 56];
    wrong_type[54..].fill(0);
    for original in [
        None,
        Some((REG_BINARY, capsule::encode([0x65; 32]).unwrap().to_vec())),
        Some((REG_BINARY, vec![1, 2, 3])),
        Some((REG_SZ, wrong_type)),
        Some((REG_BINARY, vec![0x88; 57])),
    ] {
        let fixture = Fixture::new();
        let created = fresh(&fixture, FixtureMarkerAccess::ReadWrite);
        if let Some((kind, bytes)) = &original {
            set(created.key(), *kind, bytes);
        }
        drop(created);
        let existing =
            native_key::create_fixture_marker(&fixture.hive, FixtureMarkerAccess::ReadWrite)
                .unwrap();
        assert_eq!(existing.component(), Component::Marker);
        assert_eq!(existing.disposition(), Disposition::Existing);
        assert_eq!(raw_value(existing.key()), original);
        spent(native_key::set_new_marker_once(
            &existing,
            &capsule::encode([0x99; 32]).unwrap(),
        ));
        assert_eq!(raw_value(existing.key()), original);
        drop(existing);
        fixture.close();
    }
}

fn link_header(key: &Key) -> Option<(REG_VALUE_TYPE, u32)> {
    // Header-only test oracle, including the deliberately oversized fixture.
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0;
    let code = unsafe {
        RegQueryValueExW(
            key.raw(),
            windows::core::w!("SymbolicLinkValue"),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    }
    .0;
    if code == 2 {
        return None;
    }
    assert_eq!(code, 0);
    Some((kind, size))
}

#[test]
fn native_missing_symbolic_link_value_is_clean_without_capsule_or_target_reads() {
    let fixture = Fixture::new();
    let marker = fresh(&fixture, FixtureMarkerAccess::ReadWrite);
    assert_eq!(link_header(marker.key()), None);
    native_key::refuse_symbolic_link_header(marker.key()).unwrap();
    assert_eq!(
        primitives::marker_counts(marker.key()).unwrap(),
        Counts {
            values: 0,
            subkeys: 0
        }
    );
    drop(marker);
    fixture.close();
}

#[test]
fn native_any_symbolic_link_header_presence_is_unknown_and_preserved() {
    for (kind, payload) in [
        (REG_NONE, vec![]),
        (REG_BINARY, vec![]),
        (REG_SZ, vec![1, 2, 3]),
        (REG_LINK, vec![0, 0]),
        (REG_DWORD, vec![1, 0, 0, 0]),
        (REG_BINARY, vec![0xaa; 65_537]),
    ] {
        let fixture = Fixture::new();
        let marker = fresh(&fixture, FixtureMarkerAccess::ReadWrite);
        let original = capsule::encode([0x76; 32]).unwrap();
        set(marker.key(), REG_BINARY, &original);
        set_named(
            marker.key(),
            windows::core::w!("SymbolicLinkValue"),
            kind,
            &payload,
        );
        let expected_header = Some((kind, payload.len() as u32));
        assert_eq!(link_header(marker.key()), expected_header);
        assert_eq!(
            native_key::refuse_symbolic_link_header(marker.key()),
            Err(DriverFailure::Unknown(UnknownReason::Context))
        );
        assert_eq!(link_header(marker.key()), expected_header);
        assert_eq!(
            raw_value(marker.key()),
            Some((REG_BINARY, original.to_vec()))
        );
        assert_eq!(
            primitives::marker_counts(marker.key()).unwrap(),
            Counts {
                values: 2,
                subkeys: 0
            }
        );
        drop(marker);
        fixture.close();
    }
}

#[test]
fn native_symbolic_link_query_denial_and_deleted_handle_stay_unknown() {
    let fixture = Fixture::new();
    let marker = fresh(&fixture, FixtureMarkerAccess::ReadWrite);
    let mut handle = HKEY::default();
    assert_eq!(
        unsafe {
            RegOpenKeyExW(
                fixture.hive.raw(),
                windows::core::w!("ContinuityV1"),
                Some(REG_OPTION_OPEN_LINK.0),
                KEY_SET_VALUE | KEY_WOW64_64KEY,
                &mut handle,
            )
        }
        .0,
        0
    );
    // SAFETY: the successful relative owned-appkey open returned exclusive HKEY ownership.
    let without_query = unsafe { Key::from_owned(handle) };
    assert_eq!(
        native_key::refuse_symbolic_link_header(&without_query),
        Err(DriverFailure::Unknown(UnknownReason::Context))
    );
    assert_eq!(link_header(marker.key()), None);
    drop(without_query);
    delete_exact_marker(&fixture);
    assert_eq!(
        native_key::refuse_symbolic_link_header(marker.key()),
        Err(DriverFailure::Unknown(UnknownReason::Context))
    );
    drop(marker);
    fixture.close();
}

fn managed_entries(root: &TempRoot) -> Vec<OsString> {
    let mut names: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}
fn isolated_factory_refusal(bytes: Option<&[u8]>) {
    let root = TempRoot::new().unwrap();
    let mut store = TuningStore::open_test_directory(root.path(), 128).unwrap();
    assert_eq!(store.authority(), TuningStoreAuthority::IsolatedTest);
    if let Some(bytes) = bytes {
        store.publish(bytes).unwrap();
    }
    let before = store.read().unwrap();
    assert_eq!(before.as_deref(), bytes);
    let entries = managed_entries(&root);
    for mode in [
        Mode::ObserveExisting,
        Mode::InitializeOnlyWhenCanonicalJournalAbsent,
    ] {
        assert_eq!(
            store.canonical_continuity_candidate(mode).unwrap_err(),
            TuningNativeError::UnsafeStore(
                "canonical continuity candidate requires canonical store"
            )
        );
        assert_eq!(store.authority(), TuningStoreAuthority::IsolatedTest);
        assert_eq!(store.read().unwrap(), before);
        assert_eq!(managed_entries(&root), entries);
        let journal = root.path().join("journal-v1.json");
        match bytes {
            Some(bytes) => assert_eq!(fs::read(journal).unwrap(), bytes),
            None => assert!(!journal.exists()),
        }
    }
    drop(store);
    root.cleanup().unwrap();
}

#[test]
fn native_public_factory_refuses_actual_isolated_absent_store_before_context() {
    isolated_factory_refusal(None);
}

#[test]
fn native_public_factory_refuses_actual_isolated_present_junk_without_changes() {
    isolated_factory_refusal(Some(b"not-json\0\xff retained interrupted evidence"));
}
