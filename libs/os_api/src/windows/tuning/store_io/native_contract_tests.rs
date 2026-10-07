use super::*;
use std::mem::offset_of;
use windows::Win32::Storage::FileSystem::FILE_ID_BOTH_DIR_INFO;

#[test]
fn native_open_plan_preserves_relative_nonoverwrite_and_sharing_contract() {
    for kind in [Kind::Directory, Kind::File] {
        for mode in [
            OpenMode::Existing,
            OpenMode::CreateNew,
            OpenMode::OpenOrCreate,
        ] {
            for sharing in [
                Sharing::Retained,
                Sharing::Renameable,
                Sharing::ExclusiveLease,
            ] {
                let p = open_plan(kind, mode, sharing, false).unwrap();
                assert_eq!(p.attributes, (OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE).0);
                assert_eq!(
                    p.disposition,
                    match mode {
                        OpenMode::Existing => FILE_OPEN,
                        OpenMode::CreateNew => FILE_CREATE,
                        OpenMode::OpenOrCreate => FILE_OPEN_IF,
                    }
                );
                assert_eq!(
                    p.options,
                    FILE_SYNCHRONOUS_IO_NONALERT
                        | FILE_OPEN_REPARSE_POINT
                        | if kind == Kind::Directory {
                            FILE_DIRECTORY_FILE
                        } else {
                            FILE_NON_DIRECTORY_FILE
                        }
                );
                assert_eq!(
                    p.share,
                    match sharing {
                        Sharing::Retained => FILE_SHARE_READ | FILE_SHARE_WRITE,
                        Sharing::Renameable =>
                            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                        Sharing::ExclusiveLease => FILE_SHARE_MODE(0),
                    }
                );
                assert!(p.desired.contains(SYNCHRONIZE | READ_CONTROL));
                if kind == Kind::Directory {
                    assert!(p.desired.contains(FILE_LIST_DIRECTORY | FILE_TRAVERSE));
                    assert!(!p.desired.contains(DELETE));
                } else {
                    assert!(
                        p.desired
                            .contains(FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE)
                    );
                }
            }
        }
    }
    let read = open_plan(Kind::File, OpenMode::Existing, Sharing::Renameable, true).unwrap();
    assert!(!read.desired.contains(DELETE));
    assert!(!read.desired.contains(FILE_GENERIC_WRITE));
}
#[test]
fn native_disposition_uses_exact_iosb_information_for_requested_operation() {
    for (n, mode, expected) in [
        (1, OpenMode::Existing, Disposition::Opened),
        (1, OpenMode::OpenOrCreate, Disposition::Opened),
        (2, OpenMode::CreateNew, Disposition::Created),
        (2, OpenMode::OpenOrCreate, Disposition::Created),
    ] {
        assert_eq!(disposition(n, mode).unwrap(), expected);
    }
    for (n, mode) in [
        (1, OpenMode::CreateNew),
        (2, OpenMode::Existing),
        (0, OpenMode::Existing),
        (3, OpenMode::OpenOrCreate),
        (usize::MAX, OpenMode::CreateNew),
    ] {
        assert!(matches!(
            disposition(n, mode),
            Err(TuningNativeError::UnsafeStore(_))
        ));
    }
}
fn record(name: &[u16], id: i64) -> Vec<u8> {
    let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let mut b = vec![0; header + name.len() * 2];
    let len = offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength);
    b[len..len + 4].copy_from_slice(&(name.len() as u32 * 2).to_le_bytes());
    let i = offset_of!(FILE_ID_BOTH_DIR_INFO, FileId);
    b[i..i + 8].copy_from_slice(&id.to_le_bytes());
    for (dst, u) in b[header..].chunks_exact_mut(2).zip(name) {
        dst.copy_from_slice(&u.to_le_bytes());
    }
    b
}
#[test]
fn native_records_preserve_full_component_id_and_filter_only_dot_metadata() {
    let name: Vec<_> = "n".repeat(COMPONENT_UNITS).encode_utf16().collect();
    let e = parse_records(&record(&name, 0x123456789abcdef), 0).unwrap();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].name, name);
    assert_eq!(e[0].file_id, 0x123456789abcdef);
    for dot in [vec![46], vec![46, 46]] {
        assert!(parse_records(&record(&dot, 1), 0).unwrap().is_empty());
    }
    assert_eq!(
        parse_records(&record(&[110], 1), ENTRY_LIMIT - 1)
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        parse_records(&record(&[110], 1), ENTRY_LIMIT),
        Err(TuningNativeError::UnsafeStore(_))
    ));
}
#[test]
fn native_records_refuse_malformed_capacity_offsets_lengths_and_utf16() {
    let mut cases = vec![
        vec![],
        vec![0; offset_of!(FILE_ID_BOTH_DIR_INFO, FileName)],
        vec![0; ENUM_BYTES + 1],
        record(&[0xd800], 1),
        record(&[110; COMPONENT_UNITS + 1], 1),
    ];
    let base = record(&[110], 1);
    for next in [1_u32, 8, u32::MAX] {
        let mut b = base.clone();
        b[..4].copy_from_slice(&next.to_le_bytes());
        cases.push(b);
    }
    for len in [0_u32, 1, u32::MAX] {
        let mut b = base.clone();
        let at = offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength);
        b[at..at + 4].copy_from_slice(&len.to_le_bytes());
        cases.push(b);
    }
    let mut short = base.clone();
    short[offset_of!(FILE_ID_BOTH_DIR_INFO, ShortNameLength)] = 128;
    cases.push(short);
    let mut truncated = base;
    truncated.pop();
    cases.push(truncated);
    for b in cases {
        assert!(
            matches!(parse_records(&b, 0), Err(TuningNativeError::UnsafeStore(_))),
            "malformed native record refused"
        );
    }
}
