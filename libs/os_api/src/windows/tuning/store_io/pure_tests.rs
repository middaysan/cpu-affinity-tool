use super::*;

#[test]
fn exact_native_completion_is_required_before_inspecting_or_releasing_request() {
    assert_eq!(call_progress(0), NativeProgress::InspectCompletedOutput);
    for status in [0x103, 1, 0x101, i32::MAX] {
        assert_eq!(call_progress(status), NativeProgress::RetainSameWorker);
    }
    for status in [i32::MIN, 0xc0000034_u32 as i32, 0xc0000120_u32 as i32] {
        assert_eq!(call_progress(status), NativeProgress::TerminalFailure);
    }
    assert_eq!(io_progress(0), NativeProgress::InspectCompletedOutput);
    for status in [0x103, 1, i32::MIN, 0xc0000120_u32 as i32] {
        assert_eq!(io_progress(status), NativeProgress::RetainSameWorker);
    }
}

#[test]
fn production_component_accepts_complete_255_utf16_units_and_refuses_path_aliases() {
    for value in [
        "ProgramData",
        "CpuAffinityTool",
        "tuning",
        "journal-v1.json",
        "日本語",
    ] {
        assert_eq!(component(&value.encode_utf16().collect::<Vec<_>>()), Ok(()));
    }
    assert_eq!(component(&vec![b'x' as u16; COMPONENT_UNITS]), Ok(()));
    let mut wide = "😀"
        .repeat(COMPONENT_UNITS / 2)
        .encode_utf16()
        .collect::<Vec<_>>();
    wide.push(b'x' as u16);
    assert_eq!(component(&wide), Ok(()));
    for value in [
        "", ".", "..", "a\\b", "a/b", "a:stream", "C:\\a", "\\a", "a\0b", "a*", "a?", "a.", "a ",
        "CON", "con.bin", "LPT¹",
    ] {
        assert!(matches!(
            component(&value.encode_utf16().collect::<Vec<_>>()),
            Err(TuningNativeError::UnsafeStore("store relative component"))
        ));
    }
    for units in [
        vec![b'x' as u16; COMPONENT_UNITS + 1],
        vec![0xd800],
        vec![0xdc00],
    ] {
        assert_eq!(
            component(&units),
            Err(TuningNativeError::UnsafeStore("store relative component"))
        );
    }
}

#[test]
fn global_anchor_requires_equal_complete_guid_roots_and_real_root_identity() {
    let root = r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\"
        .encode_utf16()
        .collect::<Vec<_>>();
    assert_eq!(global_root_proof(&root, &root, true, true, true), Ok(()));
    for (id, local, acl) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        assert_eq!(
            global_root_proof(&root, &root, id, local, acl),
            Err(TuningNativeError::UnsafeStore("store global volume root"))
        );
    }
    let other = r"\\?\Volume{12345678-1234-1234-1234-123456789abd}\"
        .encode_utf16()
        .collect::<Vec<_>>();
    assert_eq!(
        global_root_proof(&root, &other, true, true, true),
        Err(TuningNativeError::UnsafeStore("store global volume root"))
    );
    for malformed in [
        r"C:\",
        r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\dir",
        r"\\?\Volume{12345678-1234-1234-1234-123456789abz}\",
        r"\\?\Volume{12345678-1234-1234-1234-123456789abc}",
    ] {
        let malformed = malformed.encode_utf16().collect::<Vec<_>>();
        assert_eq!(
            global_root_proof(&malformed, &malformed, true, true, true),
            Err(TuningNativeError::UnsafeStore("store global volume root"))
        );
    }
}
