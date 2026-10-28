use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
    core::PCWSTR,
};

fn product_strings(path: &Path) -> Option<(String, String, String)> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None) };
    if size == 0 || size > 1024 * 1024 {
        return None;
    }
    let mut data = vec![0u32; (size as usize).div_ceil(4)];
    unsafe { GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr().cast()) }
        .ok()?;
    let query = |key: &str| -> Option<(*mut c_void, usize)> {
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        let mut ptr = std::ptr::null_mut();
        let mut count = 0;
        unsafe {
            VerQueryValueW(
                data.as_ptr().cast(),
                PCWSTR(key.as_ptr()),
                &mut ptr,
                &mut count,
            )
        }
        .as_bool()
        .then_some((ptr, count as usize))
    };
    let mut translations = Vec::new();
    if let Some((ptr, count)) = query("\\VarFileInfo\\Translation")
        && !ptr.is_null()
        && count <= 64
        && count.is_multiple_of(4)
    {
        for offset in (0..count).step_by(4) {
            let base = ptr.cast::<u8>();
            translations.push(unsafe {
                (
                    std::ptr::read_unaligned(base.add(offset).cast::<u16>()),
                    std::ptr::read_unaligned(base.add(offset + 2).cast::<u16>()),
                )
            });
        }
    }
    translations.push((0x0409, 0x04b0));
    for (lang, page) in translations {
        let field = |name: &str| -> Option<String> {
            let (ptr, count) = query(&format!("\\StringFileInfo\\{lang:04x}{page:04x}\\{name}"))?;
            if ptr.is_null() || count == 0 || count > 512 {
                return None;
            }
            let text = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), count) };
            Some(
                String::from_utf16_lossy(text)
                    .trim_end_matches('\0')
                    .trim()
                    .into(),
            )
        };
        if let (Some(product), Some(company), Some(original)) = (
            field("ProductName"),
            field("CompanyName"),
            field("OriginalFilename"),
        ) {
            return Some((product, company, original));
        }
    }
    None
}

fn is_product(path: &Path, discord: bool) -> bool {
    product_strings(path).is_some_and(|(product, company, original)| {
        if discord {
            product.eq_ignore_ascii_case("Discord")
                && company.to_ascii_lowercase().starts_with("discord")
                && original.eq_ignore_ascii_case("Discord.exe")
        } else {
            product.eq_ignore_ascii_case("Steam")
                && company.eq_ignore_ascii_case("Valve Corporation")
                && original.eq_ignore_ascii_case("steam.exe")
        }
    })
}

pub fn qualify_catalog_image(path: &Path, vendors: &[&str]) -> bool {
    !vendors.is_empty()
        && vendors.len() <= 8
        && product_strings(path).is_some_and(|(product, company, _)| {
            let company = company.to_ascii_lowercase();
            !product.is_empty()
                && vendors.iter().any(|vendor| {
                    !vendor.is_empty()
                        && vendor.len() <= 80
                        && company.contains(&vendor.to_ascii_lowercase())
                })
        })
}

pub fn qualify_installation(path: &Path, discord_launcher: bool) -> bool {
    if !discord_launcher {
        return path.file_name().is_some_and(|name| {
            (name.eq_ignore_ascii_case("steam.exe") && is_product(path, false))
                || (name.eq_ignore_ascii_case("Discord.exe") && is_product(path, true))
        });
    }
    if !path.is_file()
        || !path.file_name().is_some_and(|name| {
            name.eq_ignore_ascii_case("Update.exe")
                || (name.eq_ignore_ascii_case("Discord.exe") && is_product(path, true))
        })
    {
        return false;
    }
    let Some(root) = path.parent() else {
        return false;
    };
    if !root
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Discord"))
    {
        return false;
    }
    let Ok(children) = std::fs::read_dir(root) else {
        return false;
    };
    let mut found = false;
    for (index, child) in children.enumerate() {
        if index >= 128 {
            return false;
        }
        let Ok(child) = child else {
            return false;
        };
        if child
            .file_name()
            .to_str()
            .is_some_and(crate::app_identity::is_version_directory)
            && crate::app_identity::canonical_scope(root, &child.path().join("Discord.exe"))
                .is_some_and(|(root, image)| image.starts_with(root))
            && is_product(&child.path().join("Discord.exe"), true)
        {
            found = true;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "explicit read-only installed Steam product/layout acceptance"]
    fn s107_installed_steam_product_is_recognized() {
        assert!(super::qualify_installation(
            std::path::Path::new(r"C:\Program Files (x86)\Steam\steam.exe"),
            false
        ));
    }
}
