//! Current-token reads and malformed native shapes only; no native authority cast.
use super::{
    UnknownReason,
    driver::DriverFailure,
    native_token::{self, Integrity, RetainedCurrentToken, TokenStamp},
};
use std::mem::size_of;
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, LUID},
    Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TOKEN_STATISTICS, TokenElevation, TokenIntegrityLevel, TokenPrimary,
        TokenStatistics,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

fn sid(authority: u64, subauthorities: &[u32]) -> Vec<u8> {
    assert!(authority <= 0xffff_ffff_ffff && subauthorities.len() <= 15);
    let mut bytes = vec![1, subauthorities.len() as u8];
    bytes.extend_from_slice(&authority.to_be_bytes()[2..]);
    for value in subauthorities {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
fn token_variable_buffers_are_bounded_before_allocation_and_never_resize() {
    for required in [16, 24, 65_536] {
        assert_eq!(
            native_token::variable_buffer_size(required, 16).unwrap(),
            required as usize
        );
        assert_eq!(
            native_token::returned_buffer_size(required as usize, required, 16).unwrap(),
            required as usize
        );
    }
    for required in [0, 15, 65_537, u32::MAX] {
        assert!(native_token::variable_buffer_size(required, 16).is_err());
    }
    assert!(native_token::variable_buffer_size(65_536, usize::MAX).is_err());
    for (capacity, returned, header) in [
        (24, 20, 16),
        (24, 28, 16),
        (24, 0, 16),
        (15, 15, 16),
        (65_537, 65_537, 16),
        (24, 24, usize::MAX),
        (usize::MAX, u32::MAX, 16),
    ] {
        assert!(native_token::returned_buffer_size(capacity, returned, header).is_err());
    }
}

#[test]
fn token_sid_pointer_requires_full_aligned_header_excluded_bounded_sid() {
    let exact = sid(0xabcd_ef01_2345, &[u32::MAX; 15]);
    assert_eq!(exact.len(), 68);
    let mut bytes = vec![0; 16];
    bytes.extend_from_slice(&exact);
    bytes.extend_from_slice(&[0xa5; 8]);
    assert_eq!(
        native_token::bounded_sid_bytes(&bytes, 16, 0x1000, 0x1010).unwrap(),
        exact
    );
    for (header, base, pointer) in [
        (16, 0x1000, 0x0fff),
        (16, 0x1000, 0x100c),
        (16, 0x1000, 0x1011),
        (16, 0x1001, 0x1011),
        (16, 0x1000, 0x105c),
        (16, 0x1000, usize::MAX),
        (usize::MAX, 0x1000, 0x1010),
        (16, usize::MAX - 31, usize::MAX - 15),
    ] {
        assert!(native_token::bounded_sid_bytes(&bytes, header, base, pointer).is_err());
    }
    for length in [0, 15, 16, 23, 83] {
        assert!(native_token::bounded_sid_bytes(&bytes[..length], 16, 0x1000, 0x1010).is_err());
    }
    for (revision, count) in [(0, 15), (2, 15), (1, 0), (1, 16), (1, 255)] {
        let mut malformed = bytes.clone();
        malformed[16] = revision;
        malformed[17] = count;
        assert!(native_token::bounded_sid_bytes(&malformed, 16, 0x1000, 0x1010).is_err());
    }
}

#[test]
fn token_sid_text_preserves_all48_authority_bits_and_all15_subauthorities() {
    assert_eq!(
        native_token::sid_text(&sid(5, &[21, 0, u32::MAX])).unwrap(),
        "S-1-5-21-0-4294967295"
    );
    assert_eq!(
        native_token::sid_text(&sid(u32::MAX as u64, &[1])).unwrap(),
        "S-1-4294967295-1"
    );
    assert_eq!(
        native_token::sid_text(&sid(0x0001_0000_0000, &[1])).unwrap(),
        "S-1-0x000100000000-1"
    );
    assert_eq!(
        native_token::sid_text(&sid(0xabcd_ef01_2345, &[7])).unwrap(),
        "S-1-0xabcdef012345-7"
    );
    let full = sid(0xffff_ffff_ffff, &[u32::MAX; 15]);
    let expected = format!("S-1-0xffffffffffff{}", "-4294967295".repeat(15));
    assert_eq!(native_token::sid_text(&full).unwrap(), expected);
    for bytes in [vec![], vec![1; 7], sid(5, &[])] {
        assert!(native_token::sid_text(&bytes).is_err());
    }
    let valid = sid(5, &[21, 7]);
    let mut suffix = valid.clone();
    suffix.push(0);
    assert!(native_token::sid_text(&suffix).is_err());
    assert!(native_token::sid_text(&valid[..valid.len() - 1]).is_err());
    for (revision, count) in [(0, 2), (2, 2), (1, 16), (1, 255)] {
        let mut malformed = valid.clone();
        malformed[0] = revision;
        malformed[1] = count;
        assert!(native_token::sid_text(&malformed).is_err());
    }
}

#[test]
fn token_fixed_native_fields_and_elevation_require_exact_outputs() {
    for size in [4, 8, 56] {
        native_token::fixed_field_size(size, size as u32).unwrap();
        for returned in [0, (size - 1) as u32, (size + 1) as u32, u32::MAX] {
            assert!(native_token::fixed_field_size(size, returned).is_err());
        }
    }
    assert!(native_token::fixed_field_size(usize::MAX, u32::MAX).is_err());
    assert!(!native_token::elevation_value(0).unwrap());
    assert!(native_token::elevation_value(1).unwrap());
    for value in [2, 0x8000_0000, u32::MAX] {
        assert!(native_token::elevation_value(value).is_err());
    }
}

#[test]
fn token_authentication_luid_keeps_both_words_and_signed_high_bits() {
    for (low, high, expected) in [
        (1, 0, 1),
        (0, 1, 0x0000_0001_0000_0000),
        (0x7654_3210, 0x1234_5678, 0x1234_5678_7654_3210),
        (1, i32::MIN, 0x8000_0000_0000_0001),
        (u32::MAX, -1, u64::MAX),
    ] {
        assert_eq!(
            native_token::authentication_id(low, high).unwrap(),
            expected
        );
    }
    assert!(native_token::authentication_id(0, 0).is_err());
}

#[test]
fn token_integrity_requires_effective_exact_attributes_and_preserves_unknown_rids() {
    for (rid, class) in [
        (0x1000, Integrity::Low),
        (0x2000, Integrity::Medium),
        (0x2100, Integrity::Medium),
        (0x3000, Integrity::High),
        (0x4000, Integrity::System),
    ] {
        let facts = native_token::integrity_facts(&sid(16, &[rid]), 0x60).unwrap();
        assert_eq!(facts.class, class);
        assert_eq!(facts.rid, rid);
        assert_eq!(facts.attributes, 0x60);
    }
    for rid in [
        0,
        0x1001,
        0x1fff,
        0x2001,
        0x2101,
        0x2fff,
        0x3001,
        0x3fff,
        0x4100,
        u32::MAX,
    ] {
        let facts = native_token::integrity_facts(&sid(16, &[rid]), 0x60).unwrap();
        assert_eq!(facts.class, Integrity::Unknown);
        assert_eq!(facts.rid, rid);
        assert_eq!(facts.attributes, 0x60);
    }
    for attributes in [0, 0x20, 0x40, 0x10, 0x70, 0x61, 0x8000_0060, u32::MAX] {
        assert!(native_token::integrity_facts(&sid(16, &[0x3000]), attributes).is_err());
        assert!(native_token::integrity_facts(&sid(16, &[0x4000]), attributes).is_err());
    }
    for malformed in [
        sid(5, &[0x3000]),
        sid(16, &[]),
        sid(16, &[0x3000, 1]),
        sid(0x1000_0000_0010, &[0x3000]),
    ] {
        assert!(native_token::integrity_facts(&malformed, 0x60).is_err());
    }
    let mut malformed = sid(16, &[0x3000]);
    malformed[0] = 2;
    assert!(native_token::integrity_facts(&malformed, 0x60).is_err());
    let valid = sid(16, &[0x3000]);
    assert!(native_token::integrity_facts(&valid[..11], 0x60).is_err());
    let mut trailing = valid;
    trailing.push(0);
    assert!(native_token::integrity_facts(&trailing, 0x60).is_err());
}

#[test]
fn token_coherence_rejects_replacement_mutation_zero_or_nonprimary_stamps() {
    let original = TokenStamp {
        token_id: 0x1234_5678_7654_3210,
        modified_id: 0x8765_4321_1234_5678,
        authentication_id: 0xfedc_ba98_7654_3210,
        primary: true,
    };
    native_token::token_coherence(original.token_id, original, original, original).unwrap();
    let provenance_only = TokenStamp {
        authentication_id: 7,
        ..original
    };
    native_token::token_coherence(original.token_id, original, original, provenance_only).unwrap();
    for bit in [1, 1u64 << 32] {
        for field in 0..3 {
            let mut changed = original;
            match field {
                0 => changed.token_id ^= bit,
                1 => changed.modified_id ^= bit,
                _ => changed.authentication_id ^= bit,
            }
            assert!(
                native_token::token_coherence(original.token_id, original, changed, original)
                    .is_err()
            );
            assert!(
                native_token::token_coherence(original.token_id, changed, original, original)
                    .is_err()
            );
            if field < 2 {
                assert!(
                    native_token::token_coherence(original.token_id, original, original, changed)
                        .is_err()
                );
            }
        }
        assert!(
            native_token::token_coherence(original.token_id ^ bit, original, original, original)
                .is_err()
        );
    }
    assert!(native_token::token_coherence(0, original, original, original).is_err());
    for field in 0..4 {
        let mut malformed = original;
        match field {
            0 => malformed.token_id = 0,
            1 => malformed.modified_id = 0,
            2 => malformed.authentication_id = 0,
            _ => malformed.primary = false,
        }
        assert!(
            native_token::token_coherence(original.token_id, malformed, malformed, original)
                .is_err()
        );
        assert!(
            native_token::token_coherence(original.token_id, original, malformed, original)
                .is_err()
        );
        if field != 2 {
            assert!(
                native_token::token_coherence(original.token_id, original, original, malformed)
                    .is_err()
            );
        }
    }
    let redacted = format!("{original:?}");
    assert!(redacted.contains("redacted"));
    for value in [
        original.token_id,
        original.modified_id,
        original.authentication_id,
    ] {
        assert!(!redacted.contains(&value.to_string()));
        assert!(!redacted.contains(&format!("{value:x}")));
    }
}

struct OracleToken(HANDLE);
impl Drop for OracleToken {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
fn fixed_oracle<T: Default>(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> T {
    let mut value = T::default();
    let mut returned = 0;
    unsafe {
        GetTokenInformation(
            token,
            class,
            Some((&mut value as *mut T).cast()),
            size_of::<T>() as u32,
            &mut returned,
        )
    }
    .unwrap();
    assert_eq!(returned as usize, size_of::<T>());
    value
}
fn luid_bits(value: LUID) -> u64 {
    ((value.HighPart as u32 as u64) << 32) | value.LowPart as u64
}
struct OracleFacts {
    elevated: bool,
    class: Integrity,
    rid: u32,
    attributes: u32,
    token_id: u64,
    modified_id: u64,
    authentication_id: u64,
}
fn actual_token_oracle() -> OracleFacts {
    // This independent handle is read-only and never becomes a producer guard.
    let mut handle = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) }.unwrap();
    let token = OracleToken(handle);
    let before: TOKEN_STATISTICS = fixed_oracle(token.0, TokenStatistics);
    assert_eq!(before.TokenType, TokenPrimary);
    let elevation: TOKEN_ELEVATION = fixed_oracle(token.0, TokenElevation);
    assert!(elevation.TokenIsElevated <= 1);
    let mut required = 0;
    assert!(
        unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, None, 0, &mut required) }
            .is_err()
    );
    let capacity = required as usize;
    assert!((size_of::<TOKEN_MANDATORY_LABEL>()..=65_536).contains(&capacity));
    let mut aligned = vec![0usize; capacity.div_ceil(size_of::<usize>())];
    let mut returned = 0;
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
    assert_eq!(returned, required);
    // SAFETY: the initialized aligned buffer contains the checked native header.
    let label =
        unsafe { std::ptr::read_unaligned(aligned.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
    let base = aligned.as_ptr() as usize;
    let pointer = label.Label.Sid.0 as usize;
    assert_eq!(pointer % 4, 0);
    let offset = pointer.checked_sub(base).unwrap();
    assert!(
        offset >= size_of::<TOKEN_MANDATORY_LABEL>() && offset.checked_add(12).unwrap() <= capacity
    );
    // SAFETY: all inspected bytes lie within the checked returned native buffer.
    let bytes = unsafe { std::slice::from_raw_parts(aligned.as_ptr().cast::<u8>(), capacity) };
    let mandatory_sid = &bytes[offset..offset + 12];
    assert_eq!(&mandatory_sid[..8], &[1, 1, 0, 0, 0, 0, 0, 16]);
    let rid = u32::from_le_bytes(mandatory_sid[8..12].try_into().unwrap());
    let class = match rid {
        0x1000 => Integrity::Low,
        0x2000 | 0x2100 => Integrity::Medium,
        0x3000 => Integrity::High,
        0x4000 => Integrity::System,
        _ => Integrity::Unknown,
    };
    let after: TOKEN_STATISTICS = fixed_oracle(token.0, TokenStatistics);
    assert_eq!(after.TokenType, TokenPrimary);
    assert!(
        before.TokenId == after.TokenId,
        "oracle token identity changed"
    );
    assert!(
        before.ModifiedId == after.ModifiedId,
        "oracle token mutation changed"
    );
    assert!(
        before.AuthenticationId == after.AuthenticationId,
        "oracle authentication identity changed"
    );
    OracleFacts {
        elevated: elevation.TokenIsElevated == 1,
        class,
        rid,
        attributes: label.Label.Attributes,
        token_id: luid_bits(before.TokenId),
        modified_id: luid_bits(before.ModifiedId),
        authentication_id: luid_bits(before.AuthenticationId),
    }
}

#[cfg(test)]
pub(super) fn actual_integrity_attributes_supported() -> bool {
    actual_token_oracle().attributes == 0x60
}

#[test]
fn token_actual_retained_primary_snapshot_matches_independent_readonly_oracles() {
    let independent_identity = crate::windows::OS::tuning_current_token_identity().unwrap();
    let oracle = actual_token_oracle();
    println!(
        "actual_token_snapshot_policy={}",
        if oracle.attributes == 0x60 {
            "SupportedAttributes"
        } else {
            "UnknownAttributes"
        }
    );
    let token = RetainedCurrentToken::open_current().unwrap();
    // Actual attributes are measured, never assumed from the token's class.
    // Unsupported effective-label facts remain Unknown under the strict policy.
    if oracle.attributes != 0x60 {
        assert!(matches!(
            token.snapshot(),
            Err(DriverFailure::Unknown(UnknownReason::Context))
        ));
        return;
    }
    let first = token.snapshot().unwrap();
    assert_eq!(first.identity(), &independent_identity);
    assert!(
        first.identity().authentication_id() == oracle.authentication_id,
        "snapshot authentication identity differs"
    );
    assert_eq!(first.elevated(), oracle.elevated);
    assert_eq!(first.integrity(), oracle.class);
    assert_eq!(first.integrity_rid(), oracle.rid);
    assert_eq!(first.integrity_attributes(), oracle.attributes);
    assert!(
        first.token_id() == oracle.token_id,
        "snapshot token identity differs"
    );
    assert!(
        first.modified_id() == oracle.modified_id,
        "snapshot token mutation differs"
    );
    assert_eq!(token.snapshot().unwrap(), first);
    assert_eq!(
        format!("{token:?}"),
        "RetainedCurrentToken { native_handle: opaque }"
    );
    assert_eq!(
        format!("{first:?}"),
        "TokenSnapshot { provenance: redacted }"
    );
    println!(
        "actual_token_class={}/{:?}",
        if first.elevated() {
            "elevated"
        } else {
            "standard"
        },
        first.integrity()
    );
}
