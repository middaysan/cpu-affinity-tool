//! Production metadata store IO decisions and bounded record parser, shared by
//! its owned handle-relative native adapter. No descendant pathname fallback.
use crate::tuning::TuningNativeError;
pub(super) mod native;
use windows::{
    Wdk::Storage::FileSystem::{
        FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
        FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NTCREATEFILE_CREATE_DISPOSITION,
        NTCREATEFILE_CREATE_OPTIONS,
    },
    Win32::{
        Foundation::{OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE},
        Storage::FileSystem::{
            DELETE, FILE_ACCESS_RIGHTS, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY,
            FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, FILE_TRAVERSE, READ_CONTROL, SYNCHRONIZE,
        },
    },
};

pub(super) const COMPONENT_UNITS: usize = 255;
pub(super) type Result<T> = std::result::Result<T, TuningNativeError>;
pub(super) const ENUM_BYTES: usize = 64 * 1024;
pub(super) const ENTRY_LIMIT: usize = 32;
pub(super) const CALL_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Directory,
    File,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OpenMode {
    Existing,
    CreateNew,
    OpenOrCreate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sharing {
    Retained,
    Renameable,
    ExclusiveLease,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disposition {
    Opened,
    Created,
}
pub(super) struct OpenPlan {
    pub desired: FILE_ACCESS_RIGHTS,
    pub share: FILE_SHARE_MODE,
    pub disposition: NTCREATEFILE_CREATE_DISPOSITION,
    pub options: NTCREATEFILE_CREATE_OPTIONS,
    pub attributes: u32,
}
pub(super) fn open_plan(
    kind: Kind,
    mode: OpenMode,
    sharing: Sharing,
    read_only: bool,
) -> Result<OpenPlan> {
    let desired = match kind {
        Kind::Directory => {
            FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE
        }
        Kind::File => {
            FILE_GENERIC_READ
                | SYNCHRONIZE
                | if read_only {
                    FILE_ACCESS_RIGHTS(0)
                } else {
                    FILE_GENERIC_WRITE | DELETE
                }
        }
    };
    Ok(OpenPlan {
        desired,
        share: match sharing {
            Sharing::Retained => FILE_SHARE_READ | FILE_SHARE_WRITE,
            Sharing::Renameable => FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            Sharing::ExclusiveLease => FILE_SHARE_MODE(0),
        },
        disposition: match mode {
            OpenMode::Existing => FILE_OPEN,
            OpenMode::CreateNew => FILE_CREATE,
            OpenMode::OpenOrCreate => FILE_OPEN_IF,
        },
        options: FILE_SYNCHRONOUS_IO_NONALERT
            | FILE_OPEN_REPARSE_POINT
            | if kind == Kind::Directory {
                FILE_DIRECTORY_FILE
            } else {
                FILE_NON_DIRECTORY_FILE
            },
        attributes: (OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE).0,
    })
}
pub(super) fn disposition(information: usize, mode: OpenMode) -> Result<Disposition> {
    match (information, mode) {
        (1, OpenMode::Existing | OpenMode::OpenOrCreate) => Ok(Disposition::Opened),
        (2, OpenMode::CreateNew | OpenMode::OpenOrCreate) => Ok(Disposition::Created),
        _ => Err(TuningNativeError::UnsafeStore("store create disposition")),
    }
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub name: Vec<u16>,
    pub file_id: i64,
    pub attributes: u32,
}
pub(super) fn parse_records(buffer: &[u8], already_seen: usize) -> Result<Vec<Entry>> {
    use std::mem::offset_of;
    use windows::Win32::Storage::FileSystem::FILE_ID_BOTH_DIR_INFO;
    let invalid = || TuningNativeError::UnsafeStore("store directory records");
    let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    if buffer.len() < header || buffer.len() > ENUM_BYTES || already_seen > ENTRY_LIMIT {
        return Err(invalid());
    }
    let mut result = Vec::new();
    let mut cursor = 0_usize;
    for _ in 0..ENTRY_LIMIT + 2 {
        let start = cursor.checked_add(header).ok_or_else(invalid)?;
        if !cursor.is_multiple_of(8) || start > buffer.len() {
            return Err(invalid());
        }
        let field32 = |offset: usize| {
            u32::from_le_bytes(
                buffer[cursor + offset..cursor + offset + 4]
                    .try_into()
                    .expect("bounded SDK record field"),
            )
        };
        let next = field32(offset_of!(FILE_ID_BOTH_DIR_INFO, NextEntryOffset)) as usize;
        let length = field32(offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength)) as usize;
        let short = buffer[cursor + offset_of!(FILE_ID_BOTH_DIR_INFO, ShortNameLength)];
        if length == 0
            || !length.is_multiple_of(2)
            || length > COMPONENT_UNITS * 2
            || short > 24
            || !short.is_multiple_of(2)
        {
            return Err(invalid());
        }
        let end = start.checked_add(length).ok_or_else(invalid)?;
        if end > buffer.len() {
            return Err(invalid());
        }
        let name: Vec<_> = buffer[start..end]
            .chunks_exact(2)
            .map(|part| u16::from_le_bytes([part[0], part[1]]))
            .collect();
        if name != [46] && name != [46, 46] {
            component(&name).map_err(|_| invalid())?;
            let count = already_seen
                .checked_add(result.len())
                .and_then(|n| n.checked_add(1))
                .ok_or_else(invalid)?;
            if count > ENTRY_LIMIT {
                return Err(invalid());
            }
            let offset = cursor + offset_of!(FILE_ID_BOTH_DIR_INFO, FileId);
            result.push(Entry {
                name,
                file_id: i64::from_le_bytes(
                    buffer[offset..offset + 8]
                        .try_into()
                        .expect("bounded SDK record ID"),
                ),
                attributes: field32(offset_of!(FILE_ID_BOTH_DIR_INFO, FileAttributes)),
            });
        }
        if next == 0 {
            return Ok(result);
        }
        let following = cursor.checked_add(next).ok_or_else(invalid)?;
        if !next.is_multiple_of(8) || following < end || following >= buffer.len() {
            return Err(invalid());
        }
        cursor = following;
    }
    Err(invalid())
}

/// An ambiguous native result is never reported as terminal. This exact worker
/// retains the complete native request and all its outer authority frames until
/// process exit. Wakeups do not admit completion, cancellation or replacement.
pub(super) fn retain_nonterminal<T, F: FnOnce()>(request: Box<T>, entered: F) -> ! {
    let _owned_request = request;
    entered();
    loop {
        std::thread::park();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeProgress {
    InspectCompletedOutput,
    TerminalFailure,
    RetainSameWorker,
}

/// Called before any native output read. Negative native failure output is
/// undefined; any positive ambiguous/pending result retains its entire worker.
pub(super) fn call_progress(status: i32) -> NativeProgress {
    if status == 0 {
        NativeProgress::InspectCompletedOutput
    } else if status < 0 {
        NativeProgress::TerminalFailure
    } else {
        NativeProgress::RetainSameWorker
    }
}
/// Only an exact completed zero IOSB status can release native-used allocations.
pub(super) fn io_progress(status: i32) -> NativeProgress {
    if status == 0 {
        NativeProgress::InspectCompletedOutput
    } else {
        NativeProgress::RetainSameWorker
    }
}
pub(super) fn component(units: &[u16]) -> Result<()> {
    let invalid = || TuningNativeError::UnsafeStore("store relative component");
    if units.is_empty() || units.len() > COMPONENT_UNITS {
        return Err(invalid());
    }
    let text = String::from_utf16(units).map_err(|_| invalid())?;
    if text == "."
        || text == ".."
        || text.ends_with('.')
        || text.ends_with(' ')
        || text.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '|' | '<' | '>' | '"')
        })
    {
        return Err(invalid());
    }
    let stem = text
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if reserved {
        return Err(invalid());
    }
    Ok(())
}
/// Complete VOLUME_NAME_GUID final paths, not a prefix/serial-only proof. No
/// suffix can admit a substituted directory as the actual local volume root.
pub(super) fn global_root_proof(
    global: &[u16],
    local: &[u16],
    same_id: bool,
    fixed_local: bool,
    ntfs_persistent_acl: bool,
) -> Result<()> {
    let invalid = || TuningNativeError::UnsafeStore("store global volume root");
    if global != local || !same_id || !fixed_local || !ntfs_persistent_acl {
        return Err(invalid());
    }
    let text = String::from_utf16(global).map_err(|_| invalid())?;
    let guid = text
        .strip_prefix(r"\\?\Volume{")
        .and_then(|suffix| suffix.strip_suffix("}\\"))
        .ok_or_else(invalid)?;
    if guid.len() != 36
        || !guid.bytes().enumerate().all(|(index, b)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
#[path = "store_io/native_contract_tests.rs"]
mod native_contract_tests;
#[cfg(test)]
#[path = "store_io/pure_tests.rs"]
mod pure_tests;
#[cfg(test)]
#[path = "store_io/retention_tests.rs"]
mod retention_tests;
