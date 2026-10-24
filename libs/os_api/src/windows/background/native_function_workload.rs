//! Bounded test-only work protocol; it grants no production role authority.
use std::io::{self, BufRead};

pub(super) const MAX_REQUESTS: u32 = 24;
pub(super) const MAX_UNITS_PER_REQUEST: u32 = 1_000_000;
pub(super) const MAX_LINE_BYTES: usize = 128;
pub(super) const PREFIX: &str = "cpu-affinity-work-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WorkRequest {
    pub sequence: u32,
    pub units: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WorkReport {
    pub sequence: u32,
    pub total: u64,
    pub checksum: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Progress {
    pub sequence: u32,
    pub total: u64,
}

impl Progress {
    pub fn accept(&mut self, request: WorkRequest, report: WorkReport) -> Result<(), &'static str> {
        if !(1..=MAX_REQUESTS).contains(&request.sequence)
            || !(1..=MAX_UNITS_PER_REQUEST).contains(&request.units)
            || request.sequence != self.sequence + 1
        {
            return Err("Invalid, replayed or out-of-order work request");
        }
        let total = self.total + u64::from(request.units);
        if report.sequence != request.sequence
            || report.total != total
            || report.checksum != expected_checksum(total)
        {
            return Err("Child did not return the exact computed work result");
        }
        self.sequence = request.sequence;
        self.total = total;
        Ok(())
    }
}

fn fields(line: &str) -> Result<Vec<&str>, &'static str> {
    if line.len() > MAX_LINE_BYTES
        || !line.ends_with('\n')
        || !line[..line.len() - 1]
            .bytes()
            .all(|byte| (32..=126).contains(&byte))
    {
        return Err("Invalid or oversized protocol line");
    }
    Ok(line[..line.len() - 1].split(' ').collect())
}

fn decimal(value: &str) -> Result<u64, &'static str> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Expected bounded unsigned decimal");
    }
    value.parse().map_err(|_| "Unsigned decimal overflow")
}

pub(super) fn parse_request(line: &str) -> Result<Option<WorkRequest>, &'static str> {
    let fields = fields(line)?;
    if fields == ["quit"] {
        return Ok(None);
    }
    if fields.len() != 3 || fields[0] != "work" {
        return Err("Unknown work command");
    }
    let sequence = decimal(fields[1])?;
    let units = decimal(fields[2])?;
    if !(1..=u64::from(MAX_REQUESTS)).contains(&sequence)
        || !(1..=u64::from(MAX_UNITS_PER_REQUEST)).contains(&units)
    {
        return Err("Work command exceeds finite request/unit bounds");
    }
    Ok(Some(WorkRequest {
        sequence: sequence as u32,
        units: units as u32,
    }))
}

pub(super) fn parse_report(line: &str) -> Result<WorkReport, &'static str> {
    let fields = fields(line)?;
    if fields.len() != 4 || fields[0] != PREFIX {
        return Err("Invalid child work report");
    }
    let sequence = decimal(fields[1])?;
    let total = decimal(fields[2])?;
    let checksum = decimal(fields[3])?;
    if !(1..=u64::from(MAX_REQUESTS)).contains(&sequence)
        || !(1..=u64::from(MAX_REQUESTS) * u64::from(MAX_UNITS_PER_REQUEST)).contains(&total)
    {
        return Err("Child report exceeds finite request/unit bounds");
    }
    Ok(WorkReport {
        sequence: sequence as u32,
        total,
        checksum,
    })
}

fn expected_checksum(units: u64) -> u64 {
    let units = u128::from(units);
    (units * (units + 1) * (2 * units + 1) / 6) as u64
}

pub(super) fn read_bounded_line(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = Vec::new();
    let size = std::io::Read::take(&mut *reader, (MAX_LINE_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if size == 0 {
        return Ok(None);
    }
    if size > MAX_LINE_BYTES || line.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unterminated or oversized child line",
        ));
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Non-UTF-8 child line"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_progress_rejects_a_heartbeat_without_the_computed_result() {
        let mut progress = Progress::default();
        let request = WorkRequest {
            sequence: 1,
            units: 3,
        };
        let heartbeat = WorkReport {
            sequence: 1,
            total: 3,
            checksum: 3,
        };
        assert!(progress.accept(request, heartbeat).is_err());
        assert_eq!(progress, Progress::default());
        progress
            .accept(
                request,
                WorkReport {
                    checksum: 14,
                    ..heartbeat
                },
            )
            .unwrap();
        assert_eq!(
            progress,
            Progress {
                sequence: 1,
                total: 3
            }
        );
    }

    #[test]
    fn work_progress_refuses_replay_skips_and_wrong_totals_without_advancing() {
        let mut progress = Progress::default();
        let first = WorkRequest {
            sequence: 1,
            units: 3,
        };
        let first_report = WorkReport {
            sequence: 1,
            total: 3,
            checksum: 14,
        };
        progress.accept(first, first_report).unwrap();
        for (request, report) in [
            (first, first_report),
            (
                WorkRequest {
                    sequence: 3,
                    units: 2,
                },
                WorkReport {
                    sequence: 3,
                    total: 5,
                    checksum: 55,
                },
            ),
            (
                WorkRequest {
                    sequence: 2,
                    units: 2,
                },
                WorkReport {
                    sequence: 2,
                    total: 6,
                    checksum: 91,
                },
            ),
            (
                WorkRequest {
                    sequence: 2,
                    units: 2,
                },
                WorkReport {
                    sequence: 1,
                    total: 5,
                    checksum: 55,
                },
            ),
        ] {
            assert!(progress.accept(request, report).is_err());
            assert_eq!(
                progress,
                Progress {
                    sequence: 1,
                    total: 3
                }
            );
        }
        progress
            .accept(
                WorkRequest {
                    sequence: 2,
                    units: 2,
                },
                WorkReport {
                    sequence: 2,
                    total: 5,
                    checksum: 55,
                },
            )
            .unwrap();
        assert_eq!(progress.total, 5);
    }

    #[test]
    fn work_commands_and_reports_have_strict_finite_bounds() {
        assert_eq!(
            parse_request("work 1 3\n"),
            Ok(Some(WorkRequest {
                sequence: 1,
                units: 3
            }))
        );
        assert_eq!(parse_request("quit\n"), Ok(None));
        for line in [
            "work 0 3\n",
            "work 25 3\n",
            "work 1 0\n",
            "work 1 1000001\n",
            "work 1 -3\n",
            "work 1 3 extra\n",
            "work 1 3\nextra",
            "work 1 3",
            "work 1 ３\n",
            "work 1 3\r\n",
        ] {
            assert!(parse_request(line).is_err(), "{line:?}");
        }
        assert_eq!(
            parse_report("cpu-affinity-work-v1 1 3 14\n"),
            Ok(WorkReport {
                sequence: 1,
                total: 3,
                checksum: 14
            })
        );
        for line in [
            "cpu-affinity-work-v1 0 3 14\n",
            "cpu-affinity-work-v1 25 3 14\n",
            "cpu-affinity-work-v1 1 0 0\n",
            "cpu-affinity-work-v1 1 24000001 0\n",
            "cpu-affinity-work-v1 1 3 14 extra\n",
            "cpu-affinity-work-v1 1 3 14",
            "cpu-affinity-work-v1 1 3 -14\n",
        ] {
            assert!(parse_report(line).is_err(), "{line:?}");
        }
        assert!(parse_request(&"x".repeat(MAX_LINE_BYTES + 1)).is_err());
        assert!(parse_report(&format!("{PREFIX} {}\n", "0".repeat(MAX_LINE_BYTES))).is_err());
        assert_eq!(MAX_REQUESTS, 24);
        assert_eq!(MAX_UNITS_PER_REQUEST, 1_000_000);
    }

    #[test]
    fn independent_checksum_matches_fixed_mathematical_vectors() {
        for (units, checksum) in [
            (0, 0),
            (1, 1),
            (3, 14),
            (5, 55),
            (1_000_000, 333_333_833_333_500_000),
        ] {
            assert_eq!(expected_checksum(units), checksum);
        }
    }

    #[test]
    fn bounded_reader_refuses_partial_and_oversized_lines() {
        let mut reader = io::Cursor::new(b"work 1 3\nquit\n");
        assert_eq!(
            read_bounded_line(&mut reader).unwrap().as_deref(),
            Some("work 1 3\n")
        );
        assert_eq!(
            read_bounded_line(&mut reader).unwrap().as_deref(),
            Some("quit\n")
        );
        assert_eq!(read_bounded_line(&mut reader).unwrap(), None);
        for bytes in [
            b"work 1 3".to_vec(),
            vec![b'x'; MAX_LINE_BYTES + 1],
            vec![0xff, b'\n'],
        ] {
            assert_eq!(
                read_bounded_line(&mut io::Cursor::new(bytes))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
}
