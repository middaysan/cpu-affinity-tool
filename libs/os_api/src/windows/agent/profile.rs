use std::{os::windows::ffi::OsStrExt, path::Path};

/// Names authority by the frozen storage location, without lossy Unicode conversion.
pub fn profile_key(directory: &Path) -> Result<String, String> {
    use windows::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};
    if !directory.is_absolute() {
        return Err("A storage profile needs an absolute data directory".into());
    }
    let bytes: Vec<u8> = directory
        .as_os_str()
        .encode_wide()
        .flat_map(|unit| {
            let unit = match unit {
                65..=90 => unit + 32,
                47 => 92,
                _ => unit,
            };
            unit.to_le_bytes()
        })
        .collect();
    let mut hash = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, &bytes, &mut hash) }
        .ok()
        .map_err(|e| format!("Cannot identify storage profile: {e}"))?;
    Ok(hash.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::Path};
    #[test]
    fn storage_profiles_have_stable_distinct_lossless_keys() {
        let key = super::profile_key(Path::new(r"E:\Tools\First")).unwrap();
        assert_eq!(key.len(), 64);
        assert_eq!(
            key,
            super::profile_key(Path::new(r"e:\tools\first")).unwrap()
        );
        assert_ne!(
            key,
            super::profile_key(Path::new(r"E:\Tools\Other")).unwrap()
        );
        let a = OsString::from_wide(&[b'C' as u16, 58, 92, 0xd800]);
        let b = OsString::from_wide(&[b'C' as u16, 58, 92, 0xd801]);
        assert_ne!(
            super::profile_key(Path::new(&a)).unwrap(),
            super::profile_key(Path::new(&b)).unwrap()
        );
        assert!(super::profile_key(Path::new("relative")).is_err());
    }
}
