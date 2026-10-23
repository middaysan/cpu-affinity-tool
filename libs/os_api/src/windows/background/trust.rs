//! Offline Authenticode/catalog verification on a pinned, non-reparse file.
use super::{native, unsupported};
use crate::background::FileIdentity;
use crate::gaming::GamingError;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    mem::size_of,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, HWND},
        Security::{
            Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, Catalog::*, CertGetNameStringW},
            WinTrust::*,
        },
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_READ, GETFINALPATHNAMEBYHANDLE_FLAGS, GetFileInformationByHandle,
            GetFinalPathNameByHandleW,
        },
    },
    core::{PCWSTR, w},
};

type Stamp = FileIdentity;
#[derive(Default)]
pub(super) struct TrustCache {
    entries: BTreeMap<Stamp, (Instant, bool)>,
}

fn stamp(handle: HANDLE) -> Result<Stamp, GamingError> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut info) }
        .map_err(|e| native("Background file identity", e))?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(unsupported("Reparse image refused"));
    }
    Ok(Stamp {
        volume: info.dwVolumeSerialNumber,
        index: u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow),
        size: u64::from(info.nFileSizeHigh) << 32 | u64::from(info.nFileSizeLow),
        modified: u64::from(info.ftLastWriteTime.dwHighDateTime) << 32
            | u64::from(info.ftLastWriteTime.dwLowDateTime),
    })
}

impl TrustCache {
    pub(super) fn verify(&mut self, path: &Path) -> Result<FileIdentity, GamingError> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
            .map_err(|_| unsupported("Cannot pin Windows component file"))?;
        let handle = HANDLE(file.as_raw_handle());
        let before = stamp(handle)?;
        let mut final_path = vec![0u16; 32768];
        let length = unsafe {
            GetFinalPathNameByHandleW(handle, &mut final_path, GETFINALPATHNAMEBYHANDLE_FLAGS(0))
        } as usize;
        if length == 0 || length >= final_path.len() {
            return Err(unsupported("Cannot verify final component path"));
        }
        let final_path = String::from_utf16(&final_path[..length])
            .map_err(|_| unsupported("Invalid final path"))?;
        if !final_path
            .strip_prefix(r"\\?\")
            .unwrap_or(&final_path)
            .eq_ignore_ascii_case(&path.to_string_lossy())
        {
            return Err(unsupported("Component resolved to another location"));
        }
        let now = Instant::now();
        self.entries
            .retain(|_, (at, _)| now.duration_since(*at) < Duration::from_secs(300));
        let valid = if let Some((_, valid)) = self.entries.get(&before) {
            *valid
        } else {
            let valid = verify_signatures(handle, path);
            if self.entries.len() >= 128 {
                self.entries.clear();
            }
            self.entries.insert(before, (now, valid));
            valid
        };
        if stamp(handle)? != before {
            return Err(GamingError::Changed);
        }
        if valid {
            Ok(before)
        } else {
            Err(unsupported(
                "Microsoft Windows signature could not be verified offline",
            ))
        }
    }
}

fn microsoft_signer(state: HANDLE) -> bool {
    let provider = unsafe { WTHelperProvDataFromStateData(state) };
    if provider.is_null() {
        return false;
    }
    let signer = unsafe { WTHelperGetProvSignerFromChain(provider, 0, false, 0) };
    if signer.is_null() {
        return false;
    }
    let signer = unsafe { &*signer };
    if signer.dwError != 0
        || signer.csCertChain == 0
        || signer.csCertChain > 64
        || signer.pasCertChain.is_null()
    {
        return false;
    }
    let leaf = unsafe { &*signer.pasCertChain };
    if leaf.pCert.is_null() || leaf.dwError != 0 {
        return false;
    }
    let mut name = [0u16; 256];
    let length = unsafe {
        CertGetNameStringW(
            leaf.pCert,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            None,
            Some(&mut name),
        )
    } as usize;
    if length <= 1 || length > name.len() {
        return false;
    }
    String::from_utf16(&name[..length - 1]).is_ok_and(|name| {
        matches!(
            name.as_str(),
            "Microsoft Windows" | "Microsoft Corporation" | "Microsoft Windows Publisher"
        )
    })
}

fn verify_data(data: &mut WINTRUST_DATA) -> bool {
    data.cbStruct = size_of::<WINTRUST_DATA>() as u32;
    data.dwUIChoice = WTD_UI_NONE;
    data.dwStateAction = WTD_STATEACTION_VERIFY;
    data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let status = unsafe { WinVerifyTrustEx(HWND(-1isize as _), &mut action, data) };
    let valid = status == 0 && microsoft_signer(data.hWVTStateData);
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe { WinVerifyTrustEx(HWND(-1isize as _), &mut action, data) };
    valid
}

struct CatalogAdmin(isize);
impl Drop for CatalogAdmin {
    fn drop(&mut self) {
        unsafe {
            let _ = CryptCATAdminReleaseContext(self.0, 0);
        }
    }
}
struct CatalogEntry {
    admin: isize,
    entry: isize,
}
impl Drop for CatalogEntry {
    fn drop(&mut self) {
        unsafe {
            let _ = CryptCATAdminReleaseCatalogContext(self.admin, self.entry, 0);
        }
    }
}

fn verify_signatures(handle: HANDLE, path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(path.as_ptr()),
        hFile: handle,
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        ..Default::default()
    };
    if verify_data(&mut data) {
        return true;
    }
    for algorithm in [w!("SHA256"), w!("SHA1")] {
        let mut admin = 0;
        if unsafe { CryptCATAdminAcquireContext2(&mut admin, None, algorithm, None, None) }.is_err()
        {
            continue;
        }
        let admin = CatalogAdmin(admin);
        let mut length = 0;
        if unsafe { CryptCATAdminCalcHashFromFileHandle2(admin.0, handle, &mut length, None, None) }
            .is_err()
            || length == 0
            || length > 128
        {
            continue;
        }
        let mut hash = vec![0u8; length as usize];
        if unsafe {
            CryptCATAdminCalcHashFromFileHandle2(
                admin.0,
                handle,
                &mut length,
                Some(hash.as_mut_ptr()),
                None,
            )
        }
        .is_err()
            || length as usize != hash.len()
        {
            continue;
        }
        let tag: String = hash.iter().map(|byte| format!("{byte:02X}")).collect();
        let tag: Vec<u16> = tag.encode_utf16().chain(Some(0)).collect();
        let mut previous = None;
        for _ in 0..8 {
            // Enumeration consumes the preceding context, matching Microsoft's
            // CodeSigning sample. Retain RAII only for the final current entry.
            let mut previous_handle = previous.take().map_or(0, |entry: CatalogEntry| {
                let handle = entry.entry;
                std::mem::forget(entry);
                handle
            });
            let entry = unsafe {
                CryptCATAdminEnumCatalogFromHash(
                    admin.0,
                    &hash,
                    None,
                    (previous_handle != 0).then_some(&mut previous_handle),
                )
            };
            if entry == 0 {
                break;
            }
            let entry = CatalogEntry {
                admin: admin.0,
                entry,
            };
            let mut info = CATALOG_INFO {
                cbStruct: size_of::<CATALOG_INFO>() as u32,
                ..Default::default()
            };
            if unsafe { CryptCATCatalogInfoFromContext(entry.entry, &mut info, 0) }.is_ok() {
                let mut catalog = WINTRUST_CATALOG_INFO {
                    cbStruct: size_of::<WINTRUST_CATALOG_INFO>() as u32,
                    pcwszCatalogFilePath: PCWSTR(info.wszCatalogFile.as_ptr()),
                    pcwszMemberTag: PCWSTR(tag.as_ptr()),
                    pcwszMemberFilePath: PCWSTR(path.as_ptr()),
                    hMemberFile: handle,
                    pbCalculatedFileHash: hash.as_mut_ptr(),
                    cbCalculatedFileHash: length,
                    hCatAdmin: admin.0,
                    ..Default::default()
                };
                let mut data = WINTRUST_DATA {
                    dwUnionChoice: WTD_CHOICE_CATALOG,
                    Anonymous: WINTRUST_DATA_0 {
                        pCatalog: &mut catalog,
                    },
                    ..Default::default()
                };
                if verify_data(&mut data) {
                    return true;
                }
            }
            previous = Some(entry);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "explicit read-only offline Windows signature/catalog acceptance"]
    fn native_offline_windows_component_and_unsigned_test_image_are_distinct() {
        let component = super::super::directory(false).unwrap().join("kernel32.dll");
        let mut cache = TrustCache::default();
        let result = cache.verify(&component);
        eprintln!("Offline Windows component verification: {result:?}");
        assert!(result.is_ok());
        assert!(cache.verify(&std::env::current_exe().unwrap()).is_err());
        assert!(cache.entries.len() <= 128);
    }
}
