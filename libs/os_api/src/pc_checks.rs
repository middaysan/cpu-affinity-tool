//! Bounded read-only facts for an explicit PC-check refresh.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PcCheckError {
    Unsupported,
    Native { operation: &'static str, code: i32 },
    InvalidData { source: &'static str },
    Limit { source: &'static str },
}

impl std::fmt::Display for PcCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("This observation requires Windows."),
            Self::Native { operation, code } => {
                write!(f, "{operation} failed (native code {code}).")
            }
            Self::InvalidData { source } => write!(f, "{source} did not provide valid data."),
            Self::Limit { source } => write!(f, "{source} exceeded the bounded read limit."),
        }
    }
}
impl std::error::Error for PcCheckError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitSnapshot {
    pub used_bytes: u64,
    pub limit_bytes: u64,
    /// System peak since boot, not a measured peak of the selected game.
    pub peak_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub physical_total_bytes: u64,
    pub physical_available_bytes: u64,
    pub commit: Result<CommitSnapshot, PcCheckError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RamConfiguration {
    /// Firmware-reported system-memory devices; this does not prove channel mode.
    pub reported_slots: u16,
    /// Unknown device sizes keep the occupied count unknown.
    pub populated_modules: Option<u16>,
    pub configured_speeds_mt_s: Vec<u32>,
    pub unknown_speed_modules: u16,
}

#[derive(Clone, Debug)]
pub struct PcSnapshot {
    pub memory: Result<MemorySnapshot, PcCheckError>,
    pub ram: Result<RamConfiguration, PcCheckError>,
    pub cpu_model: Option<String>,
    /// Adapter names from the bounded Windows display inventory. Driver health,
    /// driver versions and the game's actual renderer are not inferred from them.
    pub graphics_adapters: Result<Vec<String>, PcCheckError>,
}

pub fn snapshot() -> PcSnapshot {
    #[cfg(target_os = "windows")]
    {
        crate::windows::pc_checks::collect_snapshot()
    }
    #[cfg(not(target_os = "windows"))]
    {
        PcSnapshot {
            memory: Err(PcCheckError::Unsupported),
            ram: Err(PcCheckError::Unsupported),
            cpu_model: None,
            graphics_adapters: Err(PcCheckError::Unsupported),
        }
    }
}

#[cfg(any(target_os = "windows", test))]
pub(crate) const MAX_FIRMWARE_BYTES: usize = 256 * 1024;
#[cfg(any(target_os = "windows", test))]
const MAX_MEMORY_DEVICES: usize = 64;

#[cfg(any(target_os = "windows", test))]
pub(crate) fn parse_ram_firmware(raw: &[u8]) -> Result<RamConfiguration, PcCheckError> {
    let invalid = || PcCheckError::InvalidData {
        source: "SMBIOS RAM configuration",
    };
    let limit = || PcCheckError::Limit {
        source: "SMBIOS RAM configuration",
    };
    if raw.len() > MAX_FIRMWARE_BYTES {
        return Err(limit());
    }
    if raw.len() < 8 || (raw[1], raw[2]) < (2, 1) {
        return Err(invalid());
    }
    // RawSMBIOSData header from GetSystemFirmwareTable(RSMB); the declared
    // payload must match the completed bounded native read exactly.
    let declared = u32::from_le_bytes(raw[4..8].try_into().map_err(|_| invalid())?) as usize;
    if declared != raw.len() - 8 {
        return Err(invalid());
    }
    let version = (raw[1], raw[2]);
    let table = &raw[8..];
    let mut records = Vec::new();
    let mut cursor = 0;
    while cursor < table.len() {
        if records.len() >= 4096 {
            return Err(limit());
        }
        if table.len() - cursor < 4 {
            return Err(invalid());
        }
        let length = table[cursor + 1] as usize;
        if length < 4 || length > table.len() - cursor {
            return Err(invalid());
        }
        let formatted_end = cursor + length;
        let strings_length = table[formatted_end..]
            .windows(2)
            .position(|pair| pair == [0, 0])
            .ok_or_else(invalid)?
            + 2;
        let record = &table[cursor..formatted_end];
        records.push(record);
        cursor = formatted_end + strings_length;
        if record[0] == 127 {
            break;
        }
    }
    let system_arrays: Vec<u16> = records
        .iter()
        .filter(|record| record[0] == 16 && record.len() >= 7 && record[5] == 3)
        .map(|record| u16::from_le_bytes([record[2], record[3]]))
        .collect();
    if system_arrays.is_empty() || system_arrays.len() > MAX_MEMORY_DEVICES {
        return Err(invalid());
    }
    let mut ram = RamConfiguration {
        reported_slots: 0,
        populated_modules: Some(0),
        configured_speeds_mt_s: Vec::new(),
        unknown_speed_modules: 0,
    };
    let mut handles = std::collections::BTreeSet::new();
    for record in records {
        if record[0] != 17 {
            continue;
        }
        if record.len() < 14 {
            return Err(invalid());
        }
        let array = u16::from_le_bytes([record[4], record[5]]);
        if !system_arrays.contains(&array) {
            continue;
        }
        if ram.reported_slots as usize >= MAX_MEMORY_DEVICES {
            return Err(limit());
        }
        let handle = u16::from_le_bytes([record[2], record[3]]);
        if !handles.insert(handle) {
            return Err(invalid());
        }
        ram.reported_slots += 1;
        let size = u16::from_le_bytes([record[12], record[13]]);
        if size == 0 {
            continue;
        }
        // DSP0134 Type 17: FFFFh is unknown. 7FFFh requires Extended Size
        // (2.7+, offset 1Ch), whose high bit is reserved.
        let known_present = size != 0xffff
            && (size != 0x7fff
                || (version >= (2, 7) && record.len() >= 32 && {
                    let extended =
                        u32::from_le_bytes(record[28..32].try_into().map_err(|_| invalid())?);
                    extended != 0 && extended & 0x8000_0000 == 0
                }));
        if !known_present {
            ram.populated_modules = None;
            ram.unknown_speed_modules += 1;
            continue;
        }
        if let Some(count) = &mut ram.populated_modules {
            *count += 1;
        }
        // Configured Memory Speed (20h, 2.7+) is actual firmware-reported
        // configuration in MT/s. Speed (15h) is capability, never a fallback.
        let configured = if version >= (2, 7) && record.len() >= 34 {
            u16::from_le_bytes([record[32], record[33]])
        } else {
            0
        };
        let speed = match configured {
            0 => None,
            0xffff if version >= (3, 3) && record.len() >= 92 => {
                let extended =
                    u32::from_le_bytes(record[88..92].try_into().map_err(|_| invalid())?);
                (extended >= 0xffff && extended & 0x8000_0000 == 0).then_some(extended)
            }
            0xffff => None,
            value => Some(u32::from(value)),
        };
        if let Some(speed) = speed {
            ram.configured_speeds_mt_s.push(speed);
        } else {
            ram.unknown_speed_modules += 1;
        }
    }
    if ram.reported_slots == 0 {
        return Err(invalid());
    }
    ram.configured_speeds_mt_s.sort_unstable();
    ram.configured_speeds_mt_s.dedup();
    Ok(ram)
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn commit_from_pages(
    used: u64,
    limit: u64,
    peak: u64,
    page_size: u64,
) -> Result<CommitSnapshot, PcCheckError> {
    let invalid = || PcCheckError::InvalidData {
        source: "System commit counters",
    };
    if page_size == 0 || limit == 0 || used > limit || used > peak {
        return Err(invalid());
    }
    Ok(CommitSnapshot {
        used_bytes: used.checked_mul(page_size).ok_or_else(invalid)?,
        limit_bytes: limit.checked_mul(page_size).ok_or_else(invalid)?,
        peak_bytes: peak.checked_mul(page_size).ok_or_else(invalid)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_table(version: (u8, u8), records: &[Vec<u8>]) -> Vec<u8> {
        let table: Vec<u8> = records.iter().flatten().copied().collect();
        let mut raw = vec![0, version.0, version.1, 0];
        raw.extend_from_slice(&(table.len() as u32).to_le_bytes());
        raw.extend(table);
        raw
    }

    fn array(handle: u16, use_code: u8) -> Vec<u8> {
        let mut record = vec![0; 15];
        record[0] = 16;
        record[1] = 15;
        record[2..4].copy_from_slice(&handle.to_le_bytes());
        record[5] = use_code;
        record.extend([0, 0]);
        record
    }

    fn device(handle: u16, size: u16, configured: u16, length: u8) -> Vec<u8> {
        let mut record = vec![0; length as usize];
        record[0] = 17;
        record[1] = length;
        record[2..4].copy_from_slice(&handle.to_le_bytes());
        record[4..6].copy_from_slice(&0x100_u16.to_le_bytes());
        record[12..14].copy_from_slice(&size.to_le_bytes());
        if length >= 23 {
            record[21..23].copy_from_slice(&6400_u16.to_le_bytes());
        }
        if length >= 34 {
            record[32..34].copy_from_slice(&configured.to_le_bytes());
        }
        record.extend([0, 0]);
        record
    }

    #[test]
    fn pc_checks_ram_uses_configured_speed_and_excludes_empty_slots_and_video_memory() {
        let mut video = device(4, 8192, 16000, 34);
        video[4..6].copy_from_slice(&0x200_u16.to_le_bytes());
        let raw = raw_table(
            (3, 2),
            &[
                array(0x100, 3),
                array(0x200, 4),
                device(1, 16384, 4800, 34),
                device(2, 0, 0, 34),
                device(3, 16384, 4800, 34),
                video,
            ],
        );
        let ram = parse_ram_firmware(&raw).unwrap();
        assert_eq!(ram.reported_slots, 3);
        assert_eq!(ram.populated_modules, Some(2));
        assert_eq!(ram.configured_speeds_mt_s, vec![4800]);
        assert_eq!(ram.unknown_speed_modules, 0);
    }

    #[test]
    fn pc_checks_ram_never_substitutes_capable_speed_for_unknown_configuration() {
        for (version, length) in [((3, 2), 34), ((2, 6), 34), ((2, 7), 23)] {
            let raw = raw_table(version, &[array(0x100, 3), device(1, 8192, 0, length)]);
            let ram = parse_ram_firmware(&raw).unwrap();
            assert!(ram.configured_speeds_mt_s.is_empty());
            assert_eq!(ram.unknown_speed_modules, 1);
        }
    }

    #[test]
    fn pc_checks_ram_unknown_size_keeps_module_count_unknown() {
        let raw = raw_table((3, 2), &[array(0x100, 3), device(1, 0xffff, 4800, 34)]);
        let ram = parse_ram_firmware(&raw).unwrap();
        assert_eq!(ram.populated_modules, None);
        assert!(ram.configured_speeds_mt_s.is_empty());
    }

    #[test]
    fn pc_checks_ram_extended_speed_requires_version_and_valid_reserved_bit() {
        let mut memory = device(1, 8192, 0xffff, 92);
        memory[88..92].copy_from_slice(&70000_u32.to_le_bytes());
        let ram =
            parse_ram_firmware(&raw_table((3, 3), &[array(0x100, 3), memory.clone()])).unwrap();
        assert_eq!(ram.configured_speeds_mt_s, vec![70000]);
        for version in [(3, 2), (3, 3)] {
            memory[88..92].copy_from_slice(&0x8001_0000_u32.to_le_bytes());
            let ram = parse_ram_firmware(&raw_table(version, &[array(0x100, 3), memory.clone()]))
                .unwrap();
            assert!(ram.configured_speeds_mt_s.is_empty());
            assert_eq!(ram.unknown_speed_modules, 1);
        }
    }

    #[test]
    fn pc_checks_ram_rejects_malformed_truncated_and_unterminated_firmware() {
        let valid = raw_table((3, 2), &[array(0x100, 3), device(1, 8192, 4800, 34)]);
        for count in 0..valid.len() {
            assert!(parse_ram_firmware(&valid[..count]).is_err());
        }
        let mut unterminated = valid.clone();
        *unterminated.last_mut().unwrap() = 1;
        assert!(parse_ram_firmware(&unterminated).is_err());
        let mut short_record = valid;
        short_record[9] = 3;
        assert!(parse_ram_firmware(&short_record).is_err());
    }

    #[test]
    fn pc_checks_ram_enforces_byte_and_memory_device_limits() {
        assert!(matches!(
            parse_ram_firmware(&vec![0; MAX_FIRMWARE_BYTES + 1]),
            Err(PcCheckError::Limit { .. })
        ));
        let mut records = vec![array(0x100, 3)];
        records.extend((0..=MAX_MEMORY_DEVICES).map(|index| device(index as u16, 8192, 4800, 34)));
        assert!(matches!(
            parse_ram_firmware(&raw_table((3, 2), &records)),
            Err(PcCheckError::Limit { .. })
        ));
    }

    #[test]
    fn pc_checks_commit_conversion_rejects_impossible_counters_and_overflow() {
        let commit = commit_from_pages(2, 4, 3, 4096).unwrap();
        assert_eq!(commit.used_bytes, 8192);
        assert_eq!(commit.limit_bytes, 16384);
        assert_eq!(commit.peak_bytes, 12288);
        assert!(commit_from_pages(5, 4, 5, 4096).is_err());
        assert!(commit_from_pages(2, 4, 1, 4096).is_err());
        assert!(commit_from_pages(1, 4, 2, 0).is_err());
        assert!(commit_from_pages(1, u64::MAX, 2, 4096).is_err());
        // A peak can exceed today's limit after a page-file configuration change.
        assert!(commit_from_pages(2, 4, 8, 4096).is_ok());
    }
}
