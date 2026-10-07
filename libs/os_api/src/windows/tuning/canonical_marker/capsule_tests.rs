use super::capsule::*;

#[test]
fn capsule_full32_mapping_roundtrips_without_authority() {
    let nonce = [0x91; NONCE_BYTES];
    let encoded = encode(nonce).expect("valid private shape must encode");
    assert_eq!(encoded.len(), 56);
    assert_eq!(&encoded[..8], b"CATCONT1");
    for offset in [8, 12, 16, 20] {
        assert_eq!(&encoded[offset..offset + 4], &1u32.to_le_bytes());
    }
    assert_eq!(&encoded[24..], &nonce);
    let shape = decode(BINARY_TYPE, &encoded).unwrap();
    assert_eq!(
        (shape.producer, shape.version, shape.domain, shape.view),
        (1, 1, 1, 1)
    );
    assert_eq!(shape.nonce, nonce);
    let mut other = nonce;
    other[31] ^= 1;
    assert_ne!(shape, decode(BINARY_TYPE, &encode(other).unwrap()).unwrap());
    assert!(format!("{shape:?}").contains("redacted"));
    assert!(!format!("{shape:?}").contains("145"));
    assert!(encode([0; NONCE_BYTES]).is_err());
    for offset in [0, 8, 12, 16, 20, 24] {
        let mut malformed = encoded;
        if offset == 24 {
            malformed[24..].fill(0);
        } else {
            malformed[offset] ^= 1;
        }
        assert!(decode(BINARY_TYPE, &malformed).is_err());
    }
    for length in [0, 16, 32, 55, 57, 65536] {
        assert!(decode(BINARY_TYPE, &vec![1; length]).is_err());
    }
    assert!(decode(1, &encoded).is_err());
    // Existing private nonce-v1 is32 bytes; no fixture-to-canonical promotion.
    assert!(decode(BINARY_TYPE, &nonce).is_err());
}
#[test]
fn capsule_query_header_bounds_before_data_read() {
    assert_eq!(fixed_read_size(BINARY_TYPE, 56), Ok(CAPSULE_BYTES));
    for kind in [0, 1, 4, u32::MAX] {
        assert!(fixed_read_size(kind, 56).is_err());
    }
    for size in [0, 32, 55, 57, 65536, u32::MAX] {
        assert!(fixed_read_size(BINARY_TYPE, size).is_err());
    }
}
