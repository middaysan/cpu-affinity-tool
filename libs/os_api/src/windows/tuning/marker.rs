//! Nonshipping process-private registry characterization. No HKLM/HKCU access.
//!
//! A positive volatile probe is not a boot identifier. Canonical security,
//! cross-user and power-transition gates remain unavailable to production.

use super::{code_error, native_error, store::TuningStore, wide};
use crate::tuning::{TuningNativeError, TuningStoreAuthority};
use std::{ffi::OsStr, marker::PhantomData, path::Path, rc::Rc};
use windows::{
    Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
    Win32::System::Registry::{
        HKEY, KEY_ALL_ACCESS, REG_BINARY, REG_CREATE_KEY_DISPOSITION, REG_CREATED_NEW_KEY,
        REG_OPEN_CREATE_OPTIONS, REG_OPTION_NON_VOLATILE, REG_OPTION_VOLATILE, REG_PROCESS_APPKEY,
        REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegLoadAppKeyW, RegOpenKeyExW,
        RegQueryValueExW, RegSetValueExW,
    },
    core::PCWSTR,
};

const NONCE_BYTES: usize = 32;
const CHILD_MUST_BE_VOLATILE: u32 = 1021;
const MAX_PROBES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningVolatilityEvidence {
    PositiveVolatility,
    Unknown,
}
use TuningVolatilityEvidence as Evidence;

#[derive(Clone, PartialEq, Eq)]
pub struct TuningMarkerNonce([u8; NONCE_BYTES]);
impl TuningMarkerNonce {
    pub fn bytes(&self) -> &[u8; NONCE_BYTES] {
        &self.0
    }
}
impl std::fmt::Debug for TuningMarkerNonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningMarkerNonce { value: redacted }")
    }
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

// Root key closes before the pinned directory authority. All registry handles
// are relative to a REG_PROCESS_APPKEY hive; there is no global deletion path.
pub struct TuningVolatileMarkerFixture {
    key: Key,
    store: TuningStore,
    probes: usize,
    _worker_only: PhantomData<Rc<()>>,
}
impl TuningVolatileMarkerFixture {
    pub fn open_test_directory(root: &Path) -> Result<Self, TuningNativeError> {
        let mut store = TuningStore::open_test_directory(root, 128)?;
        if store.read()?.is_some() {
            return Err(TuningNativeError::UnsafeStore(
                "private fixture must be empty",
            ));
        }
        let path = root.join("private-marker-fixture.hive");
        let value = wide(path.as_os_str())?;
        let mut key = HKEY::default();
        let code = unsafe {
            RegLoadAppKeyW(
                PCWSTR(value.as_ptr()),
                &mut key,
                KEY_ALL_ACCESS.0,
                REG_PROCESS_APPKEY,
                None,
            )
        };
        if code.0 != 0 {
            return Err(code_error("load private fixture hive", code.0));
        }
        Ok(Self {
            key: Key(key),
            store,
            probes: 0,
            _worker_only: PhantomData,
        })
    }
    pub fn authority(&self) -> TuningStoreAuthority {
        self.store.authority()
    }
    pub fn initialize(&mut self) -> Result<TuningMarkerNonce, TuningNativeError> {
        self.store.validate_authority()?;
        let (key, disposition) = create(self.key.0, "marker-v1", REG_OPTION_VOLATILE)?;
        if disposition != REG_CREATED_NEW_KEY {
            return Err(TuningNativeError::UnsafeStore(
                "preexisting private marker is preserved",
            ));
        }
        let nonce = random_nonce()?;
        set(&key, REG_BINARY, &nonce)?;
        if self.evidence(&key, &nonce)? != Evidence::PositiveVolatility {
            return Err(TuningNativeError::Unsupported(
                "private volatility not proven",
            ));
        }
        Ok(TuningMarkerNonce(nonce))
    }
    pub fn observe(&mut self, expected: &TuningMarkerNonce) -> Result<Evidence, TuningNativeError> {
        self.store.validate_authority()?;
        let mut key = HKEY::default();
        let code = unsafe {
            RegOpenKeyExW(
                self.key.0,
                windows::core::w!("marker-v1"),
                None,
                KEY_ALL_ACCESS,
                &mut key,
            )
        };
        if code.0 != 0 {
            return Ok(Evidence::Unknown);
        }
        self.evidence(&Key(key), &expected.0)
    }
    #[cfg(test)]
    fn create(
        &mut self,
        name: &str,
        option: REG_OPEN_CREATE_OPTIONS,
    ) -> Result<(Key, REG_CREATE_KEY_DISPOSITION), TuningNativeError> {
        self.store.validate_authority()?;
        create(self.key.0, name, option)
    }
    fn evidence(
        &mut self,
        key: &Key,
        expected: &[u8; NONCE_BYTES],
    ) -> Result<Evidence, TuningNativeError> {
        self.store.validate_authority()?;
        if self.probes >= MAX_PROBES || !nonce_matches(key, expected) {
            return Ok(Evidence::Unknown);
        }
        self.probes += 1;
        let name = format!("volatility-probe-{}", hex(&random_nonce()?));
        let text = wide(OsStr::new(&name))?;
        let mut child = HKEY::default();
        let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
        let code = unsafe {
            RegCreateKeyExW(
                key.0,
                PCWSTR(text.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS,
                None,
                &mut child,
                Some(&mut disposition),
            )
        };
        if code.0 == 0 {
            // A successful or collided probe is Unknown. Any newly created
            // child belongs to this private fixture; retain it until unload.
            drop(Key(child));
        }
        self.store.validate_authority()?;
        Ok(if nonce_matches(key, expected) {
            probe_result(code.0, disposition)
        } else {
            Evidence::Unknown
        })
    }
}

#[cfg(test)]
type PrivateHive = TuningVolatileMarkerFixture;

fn create(
    parent: HKEY,
    name: &str,
    option: REG_OPEN_CREATE_OPTIONS,
) -> Result<(Key, REG_CREATE_KEY_DISPOSITION), TuningNativeError> {
    let text = wide(OsStr::new(name))?;
    let mut key = HKEY::default();
    let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
    let code = unsafe {
        RegCreateKeyExW(
            parent,
            PCWSTR(text.as_ptr()),
            None,
            PCWSTR::null(),
            option,
            KEY_ALL_ACCESS,
            None,
            &mut key,
            Some(&mut disposition),
        )
    };
    if code.0 != 0 {
        return Err(code_error("create private fixture key", code.0));
    }
    Ok((Key(key), disposition))
}

fn set(key: &Key, kind: REG_VALUE_TYPE, bytes: &[u8]) -> Result<(), TuningNativeError> {
    let name = wide(OsStr::new("nonce-v1"))?;
    let code = unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, kind, Some(bytes)) };
    if code.0 != 0 {
        return Err(code_error("set private fixture nonce", code.0));
    }
    Ok(())
}

fn probe_result(code: u32, _disposition: REG_CREATE_KEY_DISPOSITION) -> Evidence {
    if code == CHILD_MUST_BE_VOLATILE {
        Evidence::PositiveVolatility
    } else {
        Evidence::Unknown
    }
}

fn nonce_matches(key: &Key, expected: &[u8; NONCE_BYTES]) -> bool {
    if expected.iter().all(|value| *value == 0) {
        return false;
    }
    let name = windows::core::w!("nonce-v1");
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0;
    if unsafe { RegQueryValueExW(key.0, name, None, Some(&mut kind), None, Some(&mut size)) }.0 != 0
        || kind != REG_BINARY
        || size != NONCE_BYTES as u32
    {
        return false;
    }
    let mut bytes = [0; NONCE_BYTES];
    if unsafe {
        RegQueryValueExW(
            key.0,
            name,
            None,
            Some(&mut kind),
            Some(bytes.as_mut_ptr()),
            Some(&mut size),
        )
    }
    .0 != 0
        || kind != REG_BINARY
        || size != NONCE_BYTES as u32
    {
        return false;
    }
    bytes == *expected
}

fn random_nonce() -> Result<[u8; NONCE_BYTES], TuningNativeError> {
    let mut nonce = [0; NONCE_BYTES];
    unsafe { BCryptGenRandom(None, &mut nonce, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
        .ok()
        .map_err(|e| native_error("private fixture nonce generation", e))?;
    if nonce.iter().all(|value| *value == 0) {
        return Err(TuningNativeError::Unsupported("invalid random nonce"));
    }
    Ok(nonce)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::TuningTemporaryDirectory as TempRoot;
    use super::*;
    use std::fs;
    use windows::Win32::System::Registry::{REG_OPENED_EXISTING_KEY, REG_SZ};
    fn volatile(hive: &mut PrivateHive, name: &str) -> Key {
        let (key, disposition) = hive.create(name, REG_OPTION_VOLATILE).unwrap();
        assert_eq!(disposition, REG_CREATED_NEW_KEY);
        key
    }
    #[test]
    fn exact_nonce_and_positive_native_volatility_probe_are_required() {
        let root = TempRoot::new().unwrap();
        let mut hive = PrivateHive::open_test_directory(root.path()).unwrap();
        let key = volatile(&mut hive, "marker");
        let nonce = [0x91; NONCE_BYTES];
        set(&key, REG_BINARY, &nonce).unwrap();
        assert_eq!(
            hive.evidence(&key, &nonce).unwrap(),
            Evidence::PositiveVolatility
        );
        assert_eq!(
            hive.evidence(&key, &[0x92; NONCE_BYTES]).unwrap(),
            Evidence::Unknown
        );
    }
    #[test]
    fn requested_volatile_existing_nonvolatile_key_remains_unknown() {
        let root = TempRoot::new().unwrap();
        let mut hive = PrivateHive::open_test_directory(root.path()).unwrap();
        let (original, disposition) = hive.create("marker", REG_OPTION_NON_VOLATILE).unwrap();
        assert_eq!(disposition, REG_CREATED_NEW_KEY);
        let nonce = [0x31; NONCE_BYTES];
        set(&original, REG_BINARY, &nonce).unwrap();
        let (existing, disposition) = hive.create("marker", REG_OPTION_VOLATILE).unwrap();
        assert_eq!(disposition, REG_OPENED_EXISTING_KEY);
        assert_eq!(hive.evidence(&existing, &nonce).unwrap(), Evidence::Unknown);
    }
    #[test]
    fn missing_wrong_type_wrong_length_and_zero_nonce_remain_unknown() {
        let root = TempRoot::new().unwrap();
        let mut hive = PrivateHive::open_test_directory(root.path()).unwrap();
        let key = volatile(&mut hive, "marker");
        let nonce = [0x13; NONCE_BYTES];
        assert_eq!(hive.evidence(&key, &nonce).unwrap(), Evidence::Unknown);
        for (kind, bytes) in [
            (REG_SZ, nonce.to_vec()),
            (REG_BINARY, vec![1; 31]),
            (REG_BINARY, vec![1; 33]),
            (REG_BINARY, vec![0; 32]),
            (REG_BINARY, vec![1; 65536]),
        ] {
            set(&key, kind, &bytes).unwrap();
            assert_eq!(hive.evidence(&key, &nonce).unwrap(), Evidence::Unknown);
        }
        set(&key, REG_BINARY, &nonce).unwrap();
        assert_eq!(
            hive.evidence(&key, &nonce).unwrap(),
            Evidence::PositiveVolatility
        );
    }
    #[test]
    fn private_hive_unload_removes_volatile_but_preserves_nonvolatile_control() {
        let root = TempRoot::new().unwrap();
        let mut hive = PrivateHive::open_test_directory(root.path()).unwrap();
        let volatile = volatile(&mut hive, "volatile");
        let (stable, _) = hive.create("stable", REG_OPTION_NON_VOLATILE).unwrap();
        let nonce = [0x17; NONCE_BYTES];
        set(&volatile, REG_BINARY, &nonce).unwrap();
        set(&stable, REG_BINARY, &nonce).unwrap();
        assert_eq!(
            hive.evidence(&volatile, &nonce).unwrap(),
            Evidence::PositiveVolatility
        );
        drop(volatile);
        drop(stable);
        drop(hive);
        let path = wide(root.path().join("private-marker-fixture.hive").as_os_str()).unwrap();
        let mut key = HKEY::default();
        assert_eq!(
            unsafe {
                RegLoadAppKeyW(
                    PCWSTR(path.as_ptr()),
                    &mut key,
                    KEY_ALL_ACCESS.0,
                    REG_PROCESS_APPKEY,
                    None,
                )
            }
            .0,
            0
        );
        let root_key = Key(key);
        let mut child = HKEY::default();
        assert_eq!(
            unsafe {
                RegOpenKeyExW(
                    root_key.0,
                    windows::core::w!("volatile"),
                    None,
                    KEY_ALL_ACCESS,
                    &mut child,
                )
            }
            .0,
            2
        );
        assert_eq!(
            unsafe {
                RegOpenKeyExW(
                    root_key.0,
                    windows::core::w!("stable"),
                    None,
                    KEY_ALL_ACCESS,
                    &mut child,
                )
            }
            .0,
            0
        );
        let _stable = Key(child);
    }
    #[test]
    fn only_child_must_be_volatile_is_positive() {
        assert_eq!(
            probe_result(
                CHILD_MUST_BE_VOLATILE,
                REG_CREATE_KEY_DISPOSITION::default()
            ),
            Evidence::PositiveVolatility
        );
        for code in [0, 2, 5, 87, 1018, 1722, u32::MAX] {
            assert_eq!(probe_result(code, REG_CREATED_NEW_KEY), Evidence::Unknown);
            assert_eq!(
                probe_result(code, REG_OPENED_EXISTING_KEY),
                Evidence::Unknown
            );
        }
    }
    #[test]
    fn preexisting_private_hive_is_preserved_without_import_or_reinitialization() {
        let root = TempRoot::new().unwrap();
        let mut hive = PrivateHive::open_test_directory(root.path()).unwrap();
        let _nonce = hive.initialize().unwrap();
        drop(hive);
        let path = root.path().join("private-marker-fixture.hive");
        let before = fs::read(&path).unwrap();
        assert!(PrivateHive::open_test_directory(root.path()).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }
    #[test]
    fn generated_marker_is_exact_nonzero_private_and_never_reinitialized() {
        let root = TempRoot::new().unwrap();
        let mut hive = PrivateHive::open_test_directory(root.path()).unwrap();
        let nonce = hive.initialize().unwrap();
        assert!(nonce.0.iter().any(|byte| *byte != 0));
        assert_eq!(hive.observe(&nonce).unwrap(), Evidence::PositiveVolatility);
        assert!(!format!("{nonce:?}").contains(&format!("{:?}", nonce.0)));
        assert!(hive.initialize().is_err());
        assert_eq!(hive.observe(&nonce).unwrap(), Evidence::PositiveVolatility);
    }
    #[test]
    fn private_probe_work_is_bounded_without_renewing_unknown_evidence() {
        let root = TempRoot::new().unwrap();
        let mut hive = PrivateHive::open_test_directory(root.path()).unwrap();
        let key = volatile(&mut hive, "marker");
        let nonce = [0x7f; NONCE_BYTES];
        set(&key, REG_BINARY, &nonce).unwrap();
        for _ in 0..16 {
            assert_eq!(
                hive.evidence(&key, &nonce).unwrap(),
                Evidence::PositiveVolatility
            );
        }
        assert_eq!(hive.evidence(&key, &nonce).unwrap(), Evidence::Unknown);
    }
}
