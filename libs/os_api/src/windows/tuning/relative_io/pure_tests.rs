use super::*;
use std::mem::offset_of;
use windows::Win32::Storage::FileSystem::FILE_ID_BOTH_DIR_INFO;

fn record(units: &[u16], next: u32, attributes: u32, id: i64) -> Vec<u8> {
    let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let mut bytes = vec![0; (header + units.len() * 2 + 7) & !7];
    let put32 = |bytes: &mut [u8], offset: usize, value: u32| {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    put32(
        &mut bytes,
        offset_of!(FILE_ID_BOTH_DIR_INFO, NextEntryOffset),
        next,
    );
    put32(
        &mut bytes,
        offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength),
        (units.len() * 2) as u32,
    );
    put32(
        &mut bytes,
        offset_of!(FILE_ID_BOTH_DIR_INFO, FileAttributes),
        attributes,
    );
    let id_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileId);
    bytes[id_offset..id_offset + 8].copy_from_slice(&id.to_le_bytes());
    for (index, unit) in units.iter().enumerate() {
        bytes[header + index * 2..header + index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    bytes
}
fn records(names: &[&str]) -> Vec<u8> {
    let mut result = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let units: Vec<_> = name.encode_utf16().collect();
        let mut bytes = record(&units, 0, index as u32, -(index as i64) - 1);
        if index + 1 < names.len() {
            let length = bytes.len() as u32;
            bytes[..4].copy_from_slice(&length.to_le_bytes());
        }
        result.extend(bytes);
    }
    result
}
fn malformed(bytes: &[u8]) {
    assert_eq!(
        parse_records(bytes, 0),
        Err(TuningNativeError::UnsafeStore("relative directory records"))
    );
}

#[test]
fn names_accept_exact_utf16_bound_without_path_semantics() {
    for value in [
        "a",
        "file.bin",
        "internal space",
        ".hidden",
        "é_日本語",
        "😀😀",
    ] {
        let name = RelativeName::new(value).unwrap();
        assert_eq!(name.units(), value.encode_utf16().collect::<Vec<_>>());
        assert_eq!(RelativeName::from_units(name.units()).unwrap(), name);
    }
    let ascii = "a".repeat(NAME_UNITS);
    assert_eq!(RelativeName::new(&ascii).unwrap().units().len(), NAME_UNITS);
    let non_bmp = "😀".repeat(NAME_UNITS / 2);
    assert_eq!(
        RelativeName::new(&non_bmp).unwrap().units().len(),
        NAME_UNITS
    );
}

#[test]
fn names_reject_traversal_ads_absolute_aliases_invalid_utf16_and_oversize() {
    for value in [
        "",
        ".",
        "..",
        "...",
        "a/b",
        "a\\b",
        "C:\\x",
        "\\x",
        "/x",
        "a:b",
        "a:stream:$DATA",
        "a\0b",
        "a\u{1}b",
        "a\u{1f}b",
        "a*",
        "a?",
        "a|",
        "a<",
        "a>",
        "a\"",
        "a.",
        "a ",
        "CON",
        "con.txt",
        "aux",
        "nul.any",
        "PrN",
        "COM1.txt",
        "lpt9",
        "COM¹",
        "LPT².txt",
    ] {
        assert_eq!(
            RelativeName::new(value),
            Err(TuningNativeError::UnsafeStore("relative name")),
            "invalid name class retained"
        );
    }
    for units in [
        vec![0xd800],
        vec![0xdc00],
        vec![0xd800, b'a' as u16],
        vec![b'a' as u16; NAME_UNITS + 1],
        vec![0],
        vec![],
    ] {
        assert_eq!(
            RelativeName::from_units(&units),
            Err(TuningNativeError::UnsafeStore("relative name"))
        );
    }
    assert!(RelativeName::new(&"😀".repeat(NAME_UNITS / 2 + 1)).is_err());
}

#[test]
fn completion_accepts_only_synchronous_actual_opened_or_created_for_requested_mode() {
    assert_eq!(
        completion(0, 0, 1, OpenMode::Existing),
        Ok(Disposition::Opened)
    );
    assert_eq!(
        completion(0, 0, 2, OpenMode::CreateNew),
        Ok(Disposition::Created)
    );
    assert_eq!(
        completion(0, 0, 1, OpenMode::OpenOrCreate),
        Ok(Disposition::Opened)
    );
    assert_eq!(
        completion(0, 0, 2, OpenMode::OpenOrCreate),
        Ok(Disposition::Created)
    );
    for (status, io, info, mode) in [
        (0x103, 0, 1, OpenMode::Existing),
        (0, 0x103, 1, OpenMode::Existing),
        (1, 0, 1, OpenMode::Existing),
        (0, 1, 1, OpenMode::Existing),
        (0xc0000034_u32 as i32, 0, 1, OpenMode::Existing),
        (0, 0xc0000034_u32 as i32, 1, OpenMode::Existing),
        (0, 0, 2, OpenMode::Existing),
        (0, 0, 1, OpenMode::CreateNew),
        (0, 0, 0, OpenMode::OpenOrCreate),
        (0, 0, 3, OpenMode::OpenOrCreate),
        (0, 0, 4, OpenMode::OpenOrCreate),
        (0, 0, 5, OpenMode::OpenOrCreate),
        (0, 0, usize::MAX, OpenMode::OpenOrCreate),
    ] {
        assert!(completion(status, io, info, mode).is_err());
    }
}

#[test]
fn enumeration_records_use_sdk_offsets_and_preserve_all_normal_names_and_facts() {
    let mut bytes = records(&[".", "..", "unrecognized.bin", "😀"]);
    bytes.resize(ENUM_BYTES, 0);
    let parsed = parse_records(&bytes, 0).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        parsed[0].name.units(),
        "unrecognized.bin".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(parsed[0].file_id, -3);
    assert_eq!(parsed[0].attributes, 2);
    assert_eq!(parsed[1].file_id, -4);
    assert_eq!(parsed[1].attributes, 3);
    assert_eq!(
        parsed[1].name.units(),
        "😀".encode_utf16().collect::<Vec<_>>()
    );
    assert!(
        parse_records(&records(&[".", ".."]), 32)
            .unwrap()
            .is_empty()
    );
    let maximum = "n".repeat(NAME_UNITS);
    assert_eq!(
        parse_records(&records(&[&maximum]), 0).unwrap()[0]
            .name
            .units()
            .len(),
        NAME_UNITS
    );
    let facts = parse_records(&record(&[b'x' as u16], 0, 0x8000_0400, i64::MIN), 0).unwrap();
    assert_eq!(facts[0].attributes, 0x8000_0400);
    assert_eq!(facts[0].file_id, i64::MIN);
}

#[test]
fn directory_parser_refuses_truncated_odd_malformed_nonprogress_and_unaligned_records() {
    let valid = records(&["a", "b"]);
    let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    for length in [0, 1, header - 1, header, header + 1] {
        malformed(&valid[..length]);
    }
    let name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength);
    for size in [0_u32, 1, 3, u32::MAX] {
        let mut bytes = valid.clone();
        bytes[name_offset..name_offset + 4].copy_from_slice(&size.to_le_bytes());
        malformed(&bytes);
    }
    for next in [
        1_u32,
        7,
        8,
        header as u32,
        u32::MAX,
        ((valid.len() + 7) & !7) as u32,
    ] {
        let mut bytes = valid.clone();
        bytes[..4].copy_from_slice(&next.to_le_bytes());
        malformed(&bytes);
    }
    let mut bytes = records(&["a"]);
    bytes[header..header + 2].copy_from_slice(&0xd800_u16.to_le_bytes());
    malformed(&bytes);
    let mut bytes = records(&["a"]);
    bytes[offset_of!(FILE_ID_BOTH_DIR_INFO, ShortNameLength)] = 25;
    malformed(&bytes);
    let mut bytes = records(&["a"]);
    bytes[offset_of!(FILE_ID_BOTH_DIR_INFO, ShortNameLength)] = 255;
    malformed(&bytes);
    let mut bytes = records(&["a"]);
    bytes[offset_of!(FILE_ID_BOTH_DIR_INFO, ShortNameLength)] = 1;
    malformed(&bytes);
    malformed(&vec![0; ENUM_BYTES]);
    malformed(&vec![0; ENUM_BYTES + 1]);
    malformed(&records(&["a/b"]));
    malformed(&records(&["a:stream"]));
    malformed(&records(&[&"a".repeat(NAME_UNITS + 1)]));
}

#[test]
fn directory_parser_and_loop_are_bounded_across_calls_not_per_batch() {
    let names: Vec<_> = (0..ENTRY_LIMIT).map(|i| format!("entry-{i}")).collect();
    let refs: Vec<_> = names.iter().map(String::as_str).collect();
    assert_eq!(
        parse_records(&records(&refs), 0).unwrap().len(),
        ENTRY_LIMIT
    );
    assert!(parse_records(&records(&refs), 1).is_err());
    assert!(parse_records(&records(&["last"]), ENTRY_LIMIT).is_err());
    assert!(parse_records(&records(&["last"]), usize::MAX).is_err());
    assert!(parse_records(&records(&vec!["."; ENTRY_LIMIT + 3]), 0).is_err());
    assert_eq!(enumeration_budget(0, 0), Ok(()));
    assert_eq!(enumeration_budget(CALL_LIMIT, ENTRY_LIMIT), Ok(()));
    for (calls, entries) in [
        (CALL_LIMIT + 1, 0),
        (0, ENTRY_LIMIT + 1),
        (usize::MAX, 0),
        (0, usize::MAX),
    ] {
        assert_eq!(
            enumeration_budget(calls, entries),
            Err(TuningNativeError::UnsafeStore(
                "relative enumeration bounds"
            ))
        );
    }
}

#[test]
fn fixture_shape_constructor_is_separate_and_cannot_select_an_arbitrary_path() {
    assert_eq!(
        RelativeName::fixture("fixture-owned_1").unwrap().units(),
        "fixture-owned_1".encode_utf16().collect::<Vec<_>>()
    );
    for value in [
        "owned",
        "fixture-../x",
        "fixture-C:x",
        "fixture-a.b",
        "fixture-é",
        "fixture-\\target",
    ] {
        assert!(RelativeName::fixture(value).is_err());
    }
}
