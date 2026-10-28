//! Bounded x64 Windows topology codecs and GLPI/CPU-Set evidence join.
use super::{MAX_CPU_SETS, PlacementCpu, PlacementSkip};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreRelation {
    pub masks: Vec<(u16, u64)>,
    pub efficiency_class: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetRecord {
    pub id: u32,
    pub group: u16,
    pub logical: u8,
    pub core_hint: u8,
    pub cache: u8,
    pub numa: u8,
    pub efficiency_class: u8,
    pub unavailable: bool,
}
fn invalid() -> PlacementSkip {
    PlacementSkip::InvalidTopology
}
fn records(bytes: &[u8]) -> Result<Vec<&[u8]>, PlacementSkip> {
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let mut remaining = bytes;
    let mut records = vec![];
    while !remaining.is_empty() {
        let length =
            u32::from_le_bytes(remaining.get(4..8).ok_or_else(invalid)?.try_into().unwrap())
                as usize;
        // CPU-Set records put Size first; their decoder supplies its own loop.
        if length < 8 || length > remaining.len() || records.len() == MAX_CPU_SETS {
            return Err(invalid());
        }
        records.push(&remaining[..length]);
        remaining = &remaining[length..];
    }
    Ok(records)
}
pub fn decode_cores(bytes: &[u8]) -> Result<Vec<CoreRelation>, PlacementSkip> {
    let mut result = vec![];
    for record in records(bytes)? {
        if record.len() < 32 || u32::from_le_bytes(record[..4].try_into().unwrap()) != 0 {
            return Err(invalid());
        }
        let count = u16::from_le_bytes(record[30..32].try_into().unwrap()) as usize;
        if count == 0 || count > 256 || record.len() < 32 + 16 * count {
            return Err(invalid());
        }
        let mut masks = vec![];
        for mask in record[32..32 + 16 * count].as_chunks::<16>().0 {
            let bits = u64::from_le_bytes(mask[..8].try_into().unwrap());
            if bits == 0 {
                return Err(invalid());
            }
            masks.push((u16::from_le_bytes(mask[8..10].try_into().unwrap()), bits));
        }
        result.push(CoreRelation {
            masks,
            efficiency_class: record[9],
        });
    }
    Ok(result)
}
pub fn decode_sets(bytes: &[u8]) -> Result<Vec<SetRecord>, PlacementSkip> {
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let mut remaining = bytes;
    let mut result = vec![];
    let mut count = 0;
    while !remaining.is_empty() {
        let length = u32::from_le_bytes(remaining.get(..4).ok_or_else(invalid)?.try_into().unwrap())
            as usize;
        if length < 8 || length > remaining.len() || count == MAX_CPU_SETS {
            return Err(invalid());
        }
        count += 1;
        let record = &remaining[..length];
        remaining = &remaining[length..];
        let kind = u32::from_le_bytes(record[4..8].try_into().unwrap());
        if kind != 0 {
            continue;
        }
        if record.len() < 32 {
            return Err(invalid());
        }
        let flags = record[19];
        result.push(SetRecord {
            id: u32::from_le_bytes(record[8..12].try_into().unwrap()),
            group: u16::from_le_bytes(record[12..14].try_into().unwrap()),
            logical: record[14],
            core_hint: record[15],
            cache: record[16],
            numa: record[17],
            efficiency_class: record[18],
            unavailable: (flags & 2 != 0 && flags & 4 == 0) || flags & 0xf8 != 0,
        });
    }
    if result.is_empty() {
        return Err(invalid());
    }
    Ok(result)
}
pub fn join(
    sets: &[SetRecord],
    cores: &[CoreRelation],
) -> Result<Vec<PlacementCpu>, PlacementSkip> {
    if sets.is_empty()
        || cores.is_empty()
        || sets.len() > MAX_CPU_SETS
        || cores.len() > MAX_CPU_SETS
    {
        return Err(invalid());
    }
    let mut logicals = BTreeMap::new();
    for (index, core) in cores.iter().enumerate() {
        // The current persisted physical-core key is group-relative. A core
        // spanning groups cannot be safely split under this contract.
        if core.masks.len() != 1 {
            return Err(invalid());
        }
        for &(group, mask) in &core.masks {
            if mask == 0 {
                return Err(invalid());
            }
            for logical in 0..64 {
                if mask & (1u64 << logical) != 0
                    && logicals
                        .insert(
                            (group, logical as u8),
                            (index as u32, core.efficiency_class),
                        )
                        .is_some()
                {
                    return Err(invalid());
                }
            }
        }
    }
    if logicals.len() != sets.len() {
        return Err(invalid());
    }
    let mut ids = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut hints = BTreeMap::new();
    let mut reverse = BTreeMap::new();
    let mut result = vec![];
    for set in sets {
        let &(core, class) = logicals
            .get(&(set.group, set.logical))
            .ok_or_else(invalid)?;
        if !ids.insert(set.id)
            || !seen.insert((set.group, set.logical))
            || set.efficiency_class != class
            || hints
                .insert((set.group, set.core_hint), core)
                .is_some_and(|old| old != core)
            || reverse
                .insert((set.group, core), set.core_hint)
                .is_some_and(|old| old != set.core_hint)
        {
            return Err(invalid());
        }
        result.push(PlacementCpu {
            id: set.id,
            group: set.group,
            logical: set.logical,
            physical_core: core,
            efficiency_class: class,
            numa: ((set.group as u32) << 16) | set.numa as u32,
            cache: ((set.group as u32) << 16) | set.cache as u32,
            allowed: !set.unavailable,
            reserved: set.unavailable,
        });
    }
    result.sort_by_key(|cpu| cpu.id);
    Ok(result)
}

#[cfg(test)]
mod tests;
