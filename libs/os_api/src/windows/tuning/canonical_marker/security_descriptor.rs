//! Pure bounded self-relative descriptor parser; no Windows calls or authority.
use super::{
    UnknownReason,
    registry_policy::{self, AceFacts, DescriptorFacts, Scope, SidClass},
};

const SELF_RELATIVE: u16 = 0x8000;
const DACL_PRESENT: u16 = 4;
const SACL_PRESENT: u16 = 16;
const DACL_PROTECTED: u16 = 0x1000;

pub(super) fn parse_descriptor(
    bytes: &[u8],
    _scope: Scope,
) -> Result<DescriptorFacts, UnknownReason> {
    registry_policy::descriptor_buffer_size(
        bytes
            .len()
            .try_into()
            .map_err(|_| UnknownReason::QueryBound)?,
    )?;
    if bytes[0] != 1 || bytes[1] != 0 {
        return Err(UnknownReason::UnsafeSecurity);
    }
    let control = word(bytes, 2)?;
    if control & SELF_RELATIVE == 0 {
        return Err(UnknownReason::UnsafeSecurity);
    }
    let owner = relative_sid(bytes, dword(bytes, 4)? as usize)?;
    let group = dword(bytes, 8)? as usize;
    if group != 0 {
        relative_sid(bytes, group)?;
    }
    let dacl_offset = dword(bytes, 16)? as usize;
    if control & DACL_PRESENT == 0 || dacl_offset == 0 {
        return Err(UnknownReason::UnsafeSecurity);
    }
    let dacl = parse_acl(bytes, dacl_offset, false)?;
    let sacl_offset = dword(bytes, 12)? as usize;
    let labels = if control & SACL_PRESENT != 0 && sacl_offset != 0 {
        parse_acl(bytes, sacl_offset, true)?
    } else {
        if sacl_offset != 0 {
            return Err(UnknownReason::UnsafeSecurity);
        }
        Vec::new()
    };
    Ok(DescriptorFacts {
        owner,
        dacl_present: true,
        dacl_protected: control & DACL_PROTECTED != 0,
        dacl,
        labels,
    })
}

fn word(bytes: &[u8], offset: usize) -> Result<u16, UnknownReason> {
    let end = offset.checked_add(2).ok_or(UnknownReason::QueryBound)?;
    let field = bytes.get(offset..end).ok_or(UnknownReason::QueryBound)?;
    Ok(u16::from_le_bytes(
        field.try_into().expect("checked two-byte field"),
    ))
}
fn dword(bytes: &[u8], offset: usize) -> Result<u32, UnknownReason> {
    let end = offset.checked_add(4).ok_or(UnknownReason::QueryBound)?;
    let field = bytes.get(offset..end).ok_or(UnknownReason::QueryBound)?;
    Ok(u32::from_le_bytes(
        field.try_into().expect("checked four-byte field"),
    ))
}
fn aligned(offset: usize) -> Result<(), UnknownReason> {
    if offset.is_multiple_of(4) {
        Ok(())
    } else {
        Err(UnknownReason::QueryBound)
    }
}
fn relative_sid(bytes: &[u8], offset: usize) -> Result<SidClass, UnknownReason> {
    aligned(offset)?;
    registry_policy::relative_range(bytes.len(), offset, 8)?;
    let (class, length) = parse_sid(&bytes[offset..])?;
    registry_policy::relative_range(bytes.len(), offset, length)?;
    Ok(class)
}
fn parse_sid(bytes: &[u8]) -> Result<(SidClass, usize), UnknownReason> {
    let header = bytes.get(..8).ok_or(UnknownReason::QueryBound)?;
    if header[0] != 1 {
        return Err(UnknownReason::UnsafeSecurity);
    }
    let length = registry_policy::sid_read_size(header[1], bytes.len())?;
    let sid = &bytes[..length];
    let class = if exact_sid(sid, 5, &[18]) {
        SidClass::System
    } else if exact_sid(sid, 5, &[32, 544]) {
        SidClass::Administrators
    } else if exact_sid(
        sid,
        5,
        &[
            80, 956008885, 3418522649, 1831038044, 1853292631, 2271478464,
        ],
    ) {
        SidClass::TrustedInstaller
    } else if exact_sid(sid, 16, &[12288]) {
        SidClass::HighIntegrity
    } else if exact_sid(sid, 3, &[0]) {
        SidClass::CreatorOwner
    } else {
        SidClass::Other
    };
    Ok((class, length))
}
fn exact_sid(bytes: &[u8], authority: u8, subauthorities: &[u32]) -> bool {
    bytes[1] as usize == subauthorities.len()
        && bytes[2..8] == [0, 0, 0, 0, 0, authority]
        && bytes[8..]
            .chunks_exact(4)
            .zip(subauthorities)
            .all(|(bytes, expected)| {
                u32::from_le_bytes(bytes.try_into().expect("fixed RID")) == *expected
            })
}
fn parse_acl(bytes: &[u8], offset: usize, labels: bool) -> Result<Vec<AceFacts>, UnknownReason> {
    aligned(offset)?;
    let header_range = registry_policy::relative_range(bytes.len(), offset, 8)?;
    let header = &bytes[header_range];
    if !matches!(header[0], 2 | 4) || header[1] != 0 || word(header, 6)? != 0 {
        return Err(UnknownReason::UnsafeSecurity);
    }
    let length = word(header, 2)? as usize;
    let count = word(header, 4)? as usize;
    aligned(length)?;
    registry_policy::acl_walk_bounds(length as u32, count as u32)?;
    let range = registry_policy::relative_range(bytes.len(), offset, length)?;
    let acl = &bytes[range];
    // Count and total length are bounded before this one private fact allocation.
    let mut entries = Vec::with_capacity(count);
    let mut cursor = 8usize;
    for _ in 0..count {
        let head_end = cursor.checked_add(4).ok_or(UnknownReason::QueryBound)?;
        let head = acl.get(cursor..head_end).ok_or(UnknownReason::QueryBound)?;
        let kind = head[0];
        if (labels && kind != 17) || (!labels && !matches!(kind, 0 | 1)) {
            return Err(UnknownReason::UnsafeSecurity);
        }
        let ace_length = word(head, 2)? as usize;
        aligned(ace_length)?;
        if ace_length < 16 {
            return Err(UnknownReason::QueryBound);
        }
        let end = cursor
            .checked_add(ace_length)
            .ok_or(UnknownReason::QueryBound)?;
        let ace = acl.get(cursor..end).ok_or(UnknownReason::QueryBound)?;
        let (sid, sid_length) = parse_sid(&ace[8..])?;
        // Unknown appended application data is not an ordinary accepted ACE.
        if sid_length != ace.len() - 8 {
            return Err(UnknownReason::UnsafeSecurity);
        }
        entries.push(AceFacts {
            kind,
            flags: head[1],
            mask: dword(ace, 4)?,
            sid,
        });
        cursor = end;
    }
    Ok(entries)
}
