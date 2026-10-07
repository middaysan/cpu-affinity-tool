use super::UnknownReason;

pub(super) const CAPSULE_BYTES: usize = 56;
pub(super) const NONCE_BYTES: usize = 32;
pub(super) const BINARY_TYPE: u32 = 3;

/// Private decoded shape; a codec never issues canonical native observations.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct DecodedCapsule {
    pub producer: u32,
    pub version: u32,
    pub domain: u32,
    pub view: u32,
    pub nonce: [u8; NONCE_BYTES],
}
impl std::fmt::Debug for DecodedCapsule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DecodedCapsule { shape_only: true, value: redacted }")
    }
}
pub(super) fn encode(nonce: [u8; NONCE_BYTES]) -> Result<[u8; CAPSULE_BYTES], UnknownReason> {
    if nonce == [0; NONCE_BYTES] {
        return Err(UnknownReason::MalformedCapsule);
    }
    let mut bytes = [0; CAPSULE_BYTES];
    bytes[..8].copy_from_slice(b"CATCONT1");
    for offset in [8, 12, 16, 20] {
        bytes[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
    }
    bytes[24..].copy_from_slice(&nonce);
    Ok(bytes)
}
pub(super) fn decode(kind: u32, bytes: &[u8]) -> Result<DecodedCapsule, UnknownReason> {
    if bytes.len() != CAPSULE_BYTES || kind != BINARY_TYPE || &bytes[..8] != b"CATCONT1" {
        return Err(UnknownReason::MalformedCapsule);
    }
    let field = |offset: usize| {
        u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("fixed capsule field"),
        )
    };
    let data = DecodedCapsule {
        producer: field(8),
        version: field(12),
        domain: field(16),
        view: field(20),
        nonce: bytes[24..].try_into().expect("fixed capsule nonce"),
    };
    if data.producer != 1
        || data.version != 1
        || data.domain != 1
        || data.view != 1
        || data.nonce == [0; NONCE_BYTES]
    {
        return Err(UnknownReason::MalformedCapsule);
    }
    Ok(data)
}
/// A successful header admits only a stack56-byte data query, never a Vec size.
pub(super) fn fixed_read_size(kind: u32, size: u32) -> Result<usize, UnknownReason> {
    if kind == BINARY_TYPE && size == CAPSULE_BYTES as u32 {
        Ok(CAPSULE_BYTES)
    } else {
        Err(UnknownReason::QueryBound)
    }
}
