//! Shared primitive characterization in owned REG_PROCESS_APPKEY hives only.
//! Neither fixture observations nor successful probes construct native authority.
use super::super::{test_support::TuningTemporaryDirectory as TempRoot, wide};
use super::{
    capsule,
    driver::{CapsuleShape, Counts, Disposition, DriverResult, View},
    primitives::{self, Key, PrivateProbeName},
    registry_policy::{self, Scope},
    security_descriptor,
};
use std::{ffi::OsStr, mem::size_of};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
            TokenElevation, TokenIntegrityLevel,
        },
        System::Registry::{
            HKEY, KEY_ALL_ACCESS, KEY_WOW64_64KEY, REG_BINARY, REG_CREATE_KEY_DISPOSITION,
            REG_CREATED_NEW_KEY, REG_OPENED_EXISTING_KEY, REG_OPTION_NON_VOLATILE,
            REG_OPTION_OPEN_LINK, REG_OPTION_VOLATILE, REG_PROCESS_APPKEY, REG_SZ, REG_VALUE_TYPE,
            RegCreateKeyExW, RegDeleteKeyExW, RegLoadAppKeyW, RegOpenKeyExW, RegQueryValueExW,
            RegSetValueExW,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
    core::PCWSTR,
};

pub(super) struct Fixture {
    // Field order ensures the private hive closes before the owned directory.
    pub(super) hive: Key,
    directory: TempRoot,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let directory = TempRoot::new().unwrap();
        let hive = Self::load(&directory);
        Self { hive, directory }
    }
    fn load(directory: &TempRoot) -> Key {
        let path = wide(directory.path().join("capsule-fixture.hive").as_os_str()).unwrap();
        let mut handle = HKEY::default();
        let code = unsafe {
            RegLoadAppKeyW(
                PCWSTR(path.as_ptr()),
                &mut handle,
                // A process-private hive root has no global alternate-view scope.
                // Native64 remains explicit on each relative child operation.
                KEY_ALL_ACCESS.0,
                REG_PROCESS_APPKEY,
                None,
            )
        };
        assert_eq!(code.0, 0, "only an owned process-private hive is loaded");
        // SAFETY: this successful appkey open returned a new, exclusively owned HKEY.
        unsafe { Key::from_owned(handle) }
    }
    pub(super) fn close(self) {
        let Self { hive, directory } = self;
        drop(hive);
        directory.cleanup().unwrap();
    }
}
fn component(name: &str) -> Vec<u16> {
    assert!(!name.is_empty() && !name.contains(['\\', '/']) && !name.contains('\0'));
    wide(OsStr::new(name)).unwrap()
}
fn create(parent: &Key, name: &str, volatile: bool) -> (Key, REG_CREATE_KEY_DISPOSITION) {
    let name = component(name);
    let mut handle = HKEY::default();
    let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
    assert_eq!(
        unsafe {
            RegCreateKeyExW(
                parent.raw(),
                PCWSTR(name.as_ptr()),
                None,
                PCWSTR::null(),
                if volatile {
                    REG_OPTION_VOLATILE
                } else {
                    REG_OPTION_NON_VOLATILE
                },
                KEY_ALL_ACCESS | KEY_WOW64_64KEY,
                None,
                &mut handle,
                Some(&mut disposition),
            )
        }
        .0,
        0
    );
    // SAFETY: creation/open returned one exclusively owned child of our appkey.
    (unsafe { Key::from_owned(handle) }, disposition)
}
fn open(parent: &Key, name: &str) -> Result<Key, u32> {
    let name = component(name);
    let mut handle = HKEY::default();
    let code = unsafe {
        RegOpenKeyExW(
            parent.raw(),
            PCWSTR(name.as_ptr()),
            Some(REG_OPTION_OPEN_LINK.0),
            KEY_ALL_ACCESS | KEY_WOW64_64KEY,
            &mut handle,
        )
    };
    if code.0 != 0 {
        Err(code.0)
    } else {
        // SAFETY: successful relative appkey open returns a new owned handle.
        Ok(unsafe { Key::from_owned(handle) })
    }
}
pub(super) fn set_named(key: &Key, name: PCWSTR, kind: REG_VALUE_TYPE, bytes: &[u8]) {
    assert_eq!(
        unsafe { RegSetValueExW(key.raw(), name, None, kind, Some(bytes)) }.0,
        0
    );
}
pub(super) fn set(key: &Key, kind: REG_VALUE_TYPE, bytes: &[u8]) {
    set_named(key, windows::core::w!("identity-v1"), kind, bytes);
}
pub(super) fn raw_value(key: &Key) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    // Test-only preservation query over fixture-controlled bounded payloads.
    // The shared native capsule data path itself uses only a stack56 buffer.
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0;
    let code = unsafe {
        RegQueryValueExW(
            key.raw(),
            windows::core::w!("identity-v1"),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if code.0 == 2 {
        return None;
    }
    assert_eq!(code.0, 0);
    assert!(size <= 64 * 1024);
    let mut bytes = vec![0; size as usize];
    let expected_kind = kind;
    let expected_size = size;
    assert_eq!(
        unsafe {
            RegQueryValueExW(
                key.raw(),
                windows::core::w!("identity-v1"),
                None,
                Some(&mut kind),
                Some(bytes.as_mut_ptr()),
                Some(&mut size),
            )
        }
        .0,
        0
    );
    assert_eq!(kind, expected_kind);
    assert_eq!(size, expected_size);
    Some((kind, bytes))
}
fn encoded(nonce: [u8; 32]) -> [u8; 56] {
    capsule::encode(nonce).unwrap()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FixtureVolatility {
    PositiveVolatility,
    Unknown,
}
struct FixtureObservation {
    shape: CapsuleShape,
    volatility: FixtureVolatility,
}
impl std::fmt::Debug for FixtureObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FixtureObservation { shape_only: true, value: redacted }")
    }
}
fn characterize(key: &Key, name: &PrivateProbeName) -> DriverResult<FixtureObservation> {
    let shape = primitives::read_capsule(key)?;
    let counts = primitives::marker_counts(key)?;
    // A test-only observation controls which raw primitive is characterized;
    // the production operation driver owns its independent counts/latch policy.
    let volatility = if counts
        == (Counts {
            values: 1,
            subkeys: 0,
        }) {
        let outcome = primitives::probe_nonvolatile_child_once(key, View::Native64, name)?;
        let after = primitives::marker_counts(key)?;
        let readback = primitives::read_capsule(key)?;
        if outcome.native_code == 1021 && after == counts && readback == shape {
            FixtureVolatility::PositiveVolatility
        } else {
            FixtureVolatility::Unknown
        }
    } else {
        FixtureVolatility::Unknown
    };
    Ok(FixtureObservation { shape, volatility })
}

#[test]
fn fixture_shared_fixed56_read_preserves_all32_and_private_positive_volatility() {
    let fixture = Fixture::new();
    let (key, disposition) = create(&fixture.hive, "volatile", true);
    assert_eq!(disposition, REG_CREATED_NEW_KEY);
    let nonce = [0x51; 32];
    let bytes = encoded(nonce);
    set(&key, REG_BINARY, &bytes);
    assert_eq!(
        primitives::capsule_header(&key).unwrap(),
        Some(super::driver::ValueHeader { kind: 3, size: 56 })
    );
    let mut read = [0; 56];
    assert_eq!(
        primitives::capsule_data(&key, &mut read).unwrap(),
        super::driver::ValueHeader { kind: 3, size: 56 }
    );
    assert_eq!(read, bytes);
    let observation = characterize(&key, &PrivateProbeName::for_fixture([1; 32]).unwrap()).unwrap();
    assert_eq!(observation.shape.nonce, nonce);
    assert_eq!(
        observation.volatility,
        FixtureVolatility::PositiveVolatility
    );
    assert_eq!(
        primitives::marker_counts(&key).unwrap(),
        Counts {
            values: 1,
            subkeys: 0
        }
    );
    assert!(!format!("{observation:?}").contains(&format!("{nonce:?}")));
    let mut last_byte_changed = nonce;
    last_byte_changed[31] ^= 1;
    set(&key, REG_BINARY, &encoded(last_byte_changed));
    assert_ne!(primitives::read_capsule(&key).unwrap(), observation.shape);
    assert_eq!(
        raw_value(&key),
        Some((REG_BINARY, encoded(last_byte_changed).to_vec()))
    );
    drop(key);
    fixture.close();
}

#[test]
fn fixture_existing_nonvolatile_request_stays_unknown_and_retains_probe_child() {
    let fixture = Fixture::new();
    let (original, disposition) = create(&fixture.hive, "existing", false);
    assert_eq!(disposition, REG_CREATED_NEW_KEY);
    let bytes = encoded([0x61; 32]);
    set(&original, REG_BINARY, &bytes);
    let (existing, disposition) = create(&fixture.hive, "existing", true);
    assert_eq!(disposition, REG_OPENED_EXISTING_KEY);
    let name = PrivateProbeName::for_fixture([2; 32]).unwrap();
    let result =
        primitives::probe_nonvolatile_child_once(&existing, View::Native64, &name).unwrap();
    assert_eq!(result.native_code, 0);
    assert_eq!(result.disposition, Some(Disposition::New));
    assert_eq!(
        primitives::marker_counts(&existing).unwrap(),
        Counts {
            values: 1,
            subkeys: 1
        }
    );
    let before = raw_value(&existing);
    // A new observer sees persistent metadata; it never grows another artifact.
    let observation =
        characterize(&existing, &PrivateProbeName::for_fixture([3; 32]).unwrap()).unwrap();
    assert_eq!(observation.volatility, FixtureVolatility::Unknown);
    assert_eq!(
        primitives::marker_counts(&existing).unwrap(),
        Counts {
            values: 1,
            subkeys: 1
        }
    );
    assert_eq!(raw_value(&existing), before);
    drop(existing);
    drop(original);
    fixture.close();
}

#[test]
fn fixture_collided_probe_returns_unknown_without_deleting_existing_child_or_value() {
    let fixture = Fixture::new();
    let (key, _) = create(&fixture.hive, "nonvolatile", false);
    set(&key, REG_BINARY, &encoded([0x71; 32]));
    let name = PrivateProbeName::for_fixture([4; 32]).unwrap();
    let text = String::from_utf16(&name.wide()[..70]).unwrap();
    let (existing_child, disposition) = create(&key, &text, false);
    assert_eq!(disposition, REG_CREATED_NEW_KEY);
    set(&existing_child, REG_BINARY, &[0x33; 12]);
    let before = raw_value(&existing_child);
    // Direct raw primitive collision characterization stays inside this owned hive.
    let result = primitives::probe_nonvolatile_child_once(&key, View::Native64, &name).unwrap();
    assert_eq!(result.native_code, 0);
    assert_eq!(result.disposition, Some(Disposition::Existing));
    assert_eq!(raw_value(&existing_child), before);
    assert_eq!(
        primitives::marker_counts(&key).unwrap(),
        Counts {
            values: 1,
            subkeys: 1
        }
    );
    drop(existing_child);
    drop(key);
    fixture.close();
}

#[test]
fn fixture_missing_wrong_type_partial_and_oversized_capsules_are_preserved() {
    let fixture = Fixture::new();
    let (key, _) = create(&fixture.hive, "unknown", true);
    assert_eq!(primitives::capsule_header(&key).unwrap(), None);
    assert!(primitives::read_capsule(&key).is_err());
    assert_eq!(raw_value(&key), None);
    let valid = encoded([0x21; 32]);
    let mut bad_header = valid;
    bad_header[0] ^= 1;
    let mut zero_nonce = valid;
    zero_nonce[24..].fill(0);
    for (kind, bytes) in [
        (REG_SZ, valid.to_vec()),
        (REG_BINARY, Vec::new()),
        (REG_BINARY, vec![1; 12]),
        (REG_BINARY, vec![1; 55]),
        (REG_BINARY, vec![1; 57]),
        (REG_BINARY, vec![1; 64 * 1024]),
        (REG_BINARY, bad_header.to_vec()),
        (REG_BINARY, zero_nonce.to_vec()),
    ] {
        set(&key, kind, &bytes);
        let before = raw_value(&key);
        assert!(primitives::read_capsule(&key).is_err());
        assert_eq!(raw_value(&key), before);
        assert_eq!(
            primitives::marker_counts(&key).unwrap(),
            Counts {
                values: 1,
                subkeys: 0
            }
        );
    }
    set(&key, REG_BINARY, &valid);
    assert_eq!(primitives::read_capsule(&key).unwrap().nonce, [0x21; 32]);
    drop(key);
    fixture.close();
}

#[test]
fn fixture_type_and_length_changes_between_native_header_and_fixed_data_are_unknown() {
    let fixture = Fixture::new();
    let (key, _) = create(&fixture.hive, "changed-query", true);
    let valid = encoded([0x31; 32]);
    for (kind, changed) in [
        (REG_SZ, valid.to_vec()),
        (REG_BINARY, vec![1; 55]),
        (REG_BINARY, vec![1; 57]),
    ] {
        set(&key, REG_BINARY, &valid);
        let header = primitives::capsule_header(&key).unwrap().unwrap();
        assert_eq!(header.kind, 3);
        assert_eq!(header.size, 56);
        set(&key, kind, &changed);
        let mut buffer = [0; 56];
        assert_ne!(primitives::capsule_data(&key, &mut buffer), Ok(header));
        assert!(primitives::read_capsule(&key).is_err());
        assert_eq!(raw_value(&key), Some((kind, changed)));
        assert_eq!(
            primitives::marker_counts(&key).unwrap(),
            Counts {
                values: 1,
                subkeys: 0
            }
        );
    }
    drop(key);
    fixture.close();
}

#[test]
fn fixture_extra_values_or_subkeys_skip_probe_and_preserve_owned_metadata() {
    let fixture = Fixture::new();
    let (extra_value, _) = create(&fixture.hive, "extra-value", true);
    set(&extra_value, REG_BINARY, &encoded([0x41; 32]));
    set_named(
        &extra_value,
        windows::core::w!("fixture-extra"),
        REG_BINARY,
        &[1; 12],
    );
    assert_eq!(
        primitives::marker_counts(&extra_value).unwrap(),
        Counts {
            values: 2,
            subkeys: 0
        }
    );
    let before = raw_value(&extra_value);
    assert_eq!(
        characterize(
            &extra_value,
            &PrivateProbeName::for_fixture([5; 32]).unwrap()
        )
        .unwrap()
        .volatility,
        FixtureVolatility::Unknown
    );
    assert_eq!(
        primitives::marker_counts(&extra_value).unwrap(),
        Counts {
            values: 2,
            subkeys: 0
        }
    );
    assert_eq!(raw_value(&extra_value), before);
    let (extra_subkey, _) = create(&fixture.hive, "extra-subkey", true);
    set(&extra_subkey, REG_BINARY, &encoded([0x41; 32]));
    let (child, _) = create(&extra_subkey, "owned-child", true);
    let before = raw_value(&extra_subkey);
    assert_eq!(
        characterize(
            &extra_subkey,
            &PrivateProbeName::for_fixture([6; 32]).unwrap()
        )
        .unwrap()
        .volatility,
        FixtureVolatility::Unknown
    );
    assert_eq!(
        primitives::marker_counts(&extra_subkey).unwrap(),
        Counts {
            values: 1,
            subkeys: 1
        }
    );
    assert_eq!(raw_value(&extra_subkey), before);
    drop(child);
    drop(extra_subkey);
    drop(extra_value);
    fixture.close();
}

#[test]
fn fixture_deleted_owned_key_never_becomes_a_recreated_or_live_capsule() {
    let fixture = Fixture::new();
    let (deleted, _) = create(&fixture.hive, "deleted", true);
    set(&deleted, REG_BINARY, &encoded([0x11; 32]));
    assert!(primitives::read_capsule(&deleted).is_ok());
    // Delete only this empty, exact child relative to the retained private hive.
    assert_eq!(
        unsafe {
            RegDeleteKeyExW(
                fixture.hive.raw(),
                windows::core::w!("deleted"),
                KEY_WOW64_64KEY.0,
                None,
            )
        }
        .0,
        0
    );
    assert!(primitives::read_capsule(&deleted).is_err());
    assert!(primitives::marker_counts(&deleted).is_err());
    drop(deleted);
    assert!(matches!(open(&fixture.hive, "deleted"), Err(2)));
    fixture.close();
}

#[test]
fn fixture_appkey_unload_removes_only_volatile_scope_and_retains_control_capsule() {
    let fixture = Fixture::new();
    let (volatile, _) = create(&fixture.hive, "volatile-control", true);
    let (stable, _) = create(&fixture.hive, "nonvolatile-control", false);
    let bytes = encoded([0x81; 32]);
    set(&volatile, REG_BINARY, &bytes);
    set(&stable, REG_BINARY, &bytes);
    assert_eq!(
        primitives::read_capsule(&volatile).unwrap().nonce,
        [0x81; 32]
    );
    assert_eq!(primitives::read_capsule(&stable).unwrap().nonce, [0x81; 32]);
    drop(volatile);
    drop(stable);
    let Fixture { hive, directory } = fixture;
    drop(hive);
    let reloaded = Fixture {
        hive: Fixture::load(&directory),
        directory,
    };
    assert!(matches!(open(&reloaded.hive, "volatile-control"), Err(2)));
    let stable = open(&reloaded.hive, "nonvolatile-control").unwrap();
    assert_eq!(primitives::read_capsule(&stable).unwrap().nonce, [0x81; 32]);
    assert_eq!(raw_value(&stable), Some((REG_BINARY, bytes.to_vec())));
    drop(stable);
    reloaded.close();
}

#[derive(Debug)]
enum FixtureIntegrity {
    Low,
    Medium,
    High,
    System,
    Unknown,
}
struct TokenGuard(HANDLE);
impl Drop for TokenGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
fn measured_token_class() -> (bool, FixtureIntegrity) {
    // A bounded read of one retained actual token measures only classification.
    // It never constructs the canonical producer's machine/session authority.
    let mut handle = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) }.unwrap();
    let token = TokenGuard(handle);
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    }
    .unwrap();
    assert_eq!(returned as usize, size_of::<TOKEN_ELEVATION>());
    assert!(elevation.TokenIsElevated <= 1);
    let mut required = 0;
    assert!(
        unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, None, 0, &mut required) }
            .is_err()
    );
    let capacity = required as usize;
    assert!((size_of::<TOKEN_MANDATORY_LABEL>()..=64 * 1024).contains(&capacity));
    let mut aligned = vec![0usize; capacity.div_ceil(size_of::<usize>())];
    unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            Some(aligned.as_mut_ptr().cast()),
            required,
            &mut returned,
        )
    }
    .unwrap();
    let length = returned as usize;
    assert!(length >= size_of::<TOKEN_MANDATORY_LABEL>() && length <= capacity);
    // SAFETY: the aligned initialized buffer contains the checked native header.
    let label =
        unsafe { std::ptr::read_unaligned(aligned.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
    let start = (label.Label.Sid.0 as usize)
        .checked_sub(aligned.as_ptr() as usize)
        .unwrap();
    assert!(
        start >= size_of::<TOKEN_MANDATORY_LABEL>() && start.checked_add(12).unwrap() <= length
    );
    assert_ne!(label.Label.Attributes & 0x20, 0);
    // SAFETY: each inspected SID byte lies in the checked returned token buffer.
    let bytes = unsafe { std::slice::from_raw_parts(aligned.as_ptr().cast::<u8>(), length) };
    let sid = &bytes[start..start + 12];
    let integrity = if sid[..8] == [1, 1, 0, 0, 0, 0, 0, 16] {
        match u32::from_le_bytes(sid[8..12].try_into().unwrap()) {
            0x1000..0x2000 => FixtureIntegrity::Low,
            0x2000..0x3000 => FixtureIntegrity::Medium,
            0x3000..0x4000 => FixtureIntegrity::High,
            0x4000..0x5000 => FixtureIntegrity::System,
            _ => FixtureIntegrity::Unknown,
        }
    } else {
        FixtureIntegrity::Unknown
    };
    (elevation.TokenIsElevated == 1, integrity)
}

#[test]
fn fixture_default_appkey_registry_descriptor_is_shape_only_and_never_canonical() {
    let (elevated, integrity) = measured_token_class();
    println!(
        "fixture_token_class={}/{integrity:?}",
        if elevated { "elevated" } else { "standard" }
    );
    let fixture = Fixture::new();
    let (key, _) = create(&fixture.hive, "default-security", true);
    let native = primitives::security_descriptor_bytes(&key).unwrap();
    let bytes = native.bytes();
    assert!(registry_policy::descriptor_buffer_size(bytes.len() as u32).is_ok());
    assert!(!format!("{native:?}").contains("S-1-"));
    // Appkeys inherit their ordinary descriptor. Unsupported actual fields stay
    // Unknown; successfully parsed current-user facts must fail strict policy.
    // No fixture label, protected DACL, or canonical authority is presumed.
    match security_descriptor::parse_descriptor(bytes, Scope::Managed) {
        Ok(facts) => assert!(registry_policy::validate(&facts, Scope::Managed).is_err()),
        Err(_) => println!("fixture_descriptor_policy=Unknown"),
    }
    set(&key, REG_BINARY, &encoded([0x91; 32]));
    let observation = characterize(&key, &PrivateProbeName::for_fixture([7; 32]).unwrap()).unwrap();
    assert_eq!(observation.shape.nonce, [0x91; 32]);
    assert_eq!(
        observation.volatility,
        FixtureVolatility::PositiveVolatility
    );
    drop(key);
    fixture.close();
}

#[test]
fn fixture_probe_names_are_fixed_bounded_single_components_and_redacted() {
    assert!(PrivateProbeName::for_fixture([0; 32]).is_err());
    let random = PrivateProbeName::random().unwrap();
    let value = random.wide();
    assert_eq!(value.len(), 71);
    assert_eq!(value[70], 0);
    assert!(
        value[..70]
            .iter()
            .all(|byte| *byte != 0 && *byte != b'\\' as u16 && *byte != b'/' as u16)
    );
    assert_eq!(String::from_utf16(&value[..6]).unwrap(), "probe-");
    assert!(
        value[6..70]
            .iter()
            .all(|byte| char::from_u32(*byte as u32).unwrap().is_ascii_hexdigit())
    );
    assert!(!format!("{random:?}").contains(&String::from_utf16(&value[..70]).unwrap()));
}
