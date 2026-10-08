//! Normalized benchmark CSV: one positive frame time per row, with repeated capture
//! metadata. This explicit schema avoids confusing optional PresentMon metrics.
use std::{fs::File, io::Read, path::Path};

pub(super) const CSV_HEADER: &str = "source,source_version,process,swap_chain,presentation_mode,frame_type,capture_method,scenario,frame_time_ms";
pub(super) const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ROWS: usize = 100_000;
const MAX_FIELD_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CaptureMetadata {
    source: String,
    source_version: String,
    process: String,
    swap_chain: String,
    presentation_mode: String,
    frame_type: String,
    capture_method: String,
    scenario: String,
}

#[derive(Clone, Debug)]
pub(super) struct FrameStats {
    frames: usize,
    total_ms: f64,
    average_fps: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    variance_ms2: f64,
    frames_over_50_ms: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Capture {
    metadata: CaptureMetadata,
    stats: FrameStats,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ImportError {
    Io,
    TooLarge,
    TooManyRows,
    InvalidCsv,
    InvalidSchema,
    InvalidMetadata,
    InvalidTiming,
    MixedCapture,
    Empty,
}
impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Io => "Cannot read the selected CSV file.",
            Self::TooLarge => "CSV exceeds the 4 MiB limit.",
            Self::TooManyRows => "CSV exceeds 100,000 frames.",
            Self::InvalidCsv => "Invalid UTF-8 CSV, quoting, or a field longer than 256 bytes.",
            Self::InvalidSchema => "CSV must use the documented normalized frame-time header.",
            Self::InvalidMetadata => "Capture metadata must be nonempty; frame_type must be rendered or generated.",
            Self::InvalidTiming => "Frame times must be finite, greater than zero, at most 60,000 ms, and produce finite FPS.",
            Self::MixedCapture => "One CSV must contain one process, swap chain and consistent capture metadata.",
            Self::Empty => "CSV contains no frames.",
        })
    }
}

pub(super) fn import(path: &Path) -> Result<Capture, ImportError> {
    let file = File::open(path).map_err(|_| ImportError::Io)?;
    if !file.metadata().map_err(|_| ImportError::Io)?.is_file() {
        return Err(ImportError::Io);
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ImportError::Io)?;
    parse(&bytes)
}

/// Minimal RFC4180 field handling, including quoted commas and escaped quotes.
/// Metadata cannot contain newlines/control characters; a row has exactly 9 fields.
fn row(line: &str) -> Result<Vec<String>, ImportError> {
    let mut result = Vec::with_capacity(9);
    let mut chars = line.chars().peekable();
    loop {
        let mut field = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            let mut closed = false;
            while let Some(c) = chars.next() {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    field.push(c);
                }
                if field.len() > MAX_FIELD_BYTES {
                    return Err(ImportError::InvalidCsv);
                }
            }
            if !closed || chars.peek().is_some_and(|c| *c != ',') {
                return Err(ImportError::InvalidCsv);
            }
        } else {
            while chars.peek().is_some_and(|c| *c != ',') {
                let c = chars.next().ok_or(ImportError::InvalidCsv)?;
                if c == '"' {
                    return Err(ImportError::InvalidCsv);
                }
                field.push(c);
                if field.len() > MAX_FIELD_BYTES {
                    return Err(ImportError::InvalidCsv);
                }
            }
        }
        if field.len() > MAX_FIELD_BYTES || field.chars().any(char::is_control) {
            return Err(ImportError::InvalidCsv);
        }
        result.push(field);
        if result.len() > 9 {
            return Err(ImportError::InvalidSchema);
        }
        if chars.next().is_none() {
            break;
        }
    }
    if result.len() != 9 {
        return Err(ImportError::InvalidSchema);
    }
    Ok(result)
}

pub(super) fn parse(bytes: &[u8]) -> Result<Capture, ImportError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ImportError::InvalidCsv)?;
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next() != Some(CSV_HEADER) {
        return Err(ImportError::InvalidSchema);
    }
    let mut metadata = None;
    let mut times = Vec::new();
    for line in lines {
        if times.len() >= MAX_ROWS {
            return Err(ImportError::TooManyRows);
        }
        let fields = row(line)?;
        if fields[..8].iter().any(|value| value.trim().is_empty())
            || !matches!(fields[5].as_str(), "rendered" | "generated")
        {
            return Err(ImportError::InvalidMetadata);
        }
        let current = CaptureMetadata {
            source: fields[0].clone(),
            source_version: fields[1].clone(),
            process: fields[2].clone(),
            swap_chain: fields[3].clone(),
            presentation_mode: fields[4].clone(),
            frame_type: fields[5].clone(),
            capture_method: fields[6].clone(),
            scenario: fields[7].clone(),
        };
        if metadata
            .as_ref()
            .is_some_and(|previous| *previous != current)
        {
            return Err(ImportError::MixedCapture);
        }
        metadata.get_or_insert(current);
        let timing = fields[8]
            .parse::<f64>()
            .map_err(|_| ImportError::InvalidTiming)?;
        if !timing.is_finite() || timing <= 0.0 || timing > 60_000.0 {
            return Err(ImportError::InvalidTiming);
        }
        times.push(timing);
    }
    let metadata = metadata.ok_or(ImportError::Empty)?;
    let frames = times.len();
    let total_ms: f64 = times.iter().sum();
    let average_fps = 1000.0 * frames as f64 / total_ms;
    if !average_fps.is_finite() {
        return Err(ImportError::InvalidTiming);
    }
    let mean = total_ms / frames as f64;
    let variance_ms2 = times.iter().map(|time| (*time - mean).powi(2)).sum::<f64>() / frames as f64;
    let frames_over_50_ms = times.iter().filter(|time| **time > 50.0).count();
    times.sort_by(f64::total_cmp);
    let percentile = |percent: usize| times[(frames * percent).div_ceil(100).saturating_sub(1)];
    Ok(Capture {
        metadata,
        stats: FrameStats {
            frames,
            total_ms,
            average_fps,
            p50_ms: percentile(50),
            p95_ms: percentile(95),
            p99_ms: percentile(99),
            variance_ms2,
            frames_over_50_ms,
        },
    })
}

pub(super) struct Comparison {
    fps_change_percent: f64,
    p99_change_percent: f64,
}

pub(super) fn compare(a: &Capture, b: &Capture) -> Result<Comparison, &'static str> {
    if a.metadata != b.metadata {
        return Err("A/B metadata differs. Match source/version, process, swap chain, presentation mode, frame type, capture method and scenario.");
    }
    if (a.stats.total_ms - b.stats.total_ms).abs() / a.stats.total_ms.max(b.stats.total_ms) > 0.10 {
        return Err("A/B durations differ by more than 10%. Compare similarly long captures.");
    }
    Ok(Comparison {
        fps_change_percent: (b.stats.average_fps / a.stats.average_fps - 1.0) * 100.0,
        p99_change_percent: (b.stats.p99_ms / a.stats.p99_ms - 1.0) * 100.0,
    })
}

#[derive(Default)]
pub(super) struct Captures {
    a: Option<Capture>,
    b: Option<Capture>,
    next_b: bool,
}
impl Captures {
    pub(super) fn next_label(&self) -> &'static str {
        if self.next_b {
            "B"
        } else {
            "A"
        }
    }
    pub(super) fn accept(&mut self, capture: Capture) {
        if self.next_b {
            self.b = Some(capture);
        } else {
            self.a = Some(capture);
        }
        self.next_b = !self.next_b;
    }
    pub(super) fn summary(&self) -> String {
        let mut lines = Vec::new();
        for (label, capture) in [("A", &self.a), ("B", &self.b)] {
            if let Some(capture) = capture {
                let stats = &capture.stats;
                lines.push(format!("{label}: {} {} · {} · {} · {} frames ({}) · {:.2} s\nAverage {:.2} FPS · frame time P50/P95/P99 {:.2}/{:.2}/{:.2} ms · variance {:.2} ms² · >50 ms: {}",
                    capture.metadata.source, capture.metadata.source_version, capture.metadata.process, capture.metadata.scenario,
                    stats.frames, capture.metadata.frame_type, stats.total_ms / 1000.0, stats.average_fps,
                    stats.p50_ms, stats.p95_ms, stats.p99_ms, stats.variance_ms2, stats.frames_over_50_ms));
            }
        }
        if let (Some(a), Some(b)) = (&self.a, &self.b) {
            lines.push(match compare(a, b) {
                Ok(change) => format!("B versus A: average FPS {:+.2}% · P99 frame time {:+.2}% (lower is faster). Capture labels and duration are compatible; repeat warmed-up A/B or ABBA runs before drawing a conclusion.", change.fps_change_percent, change.p99_change_percent),
                Err(reason) => format!("A/B comparison unavailable: {reason}"),
            });
        }
        if lines.is_empty() {
            lines.push("Import capture A, then B. No benchmark data loaded.".into());
        }
        lines.push("FPS = 1000 × frame count / sum of frame times. Percentiles use nearest rank; variance is population variance. No input-latency metric is present. Compare the same scene/build/settings with similar power/thermal conditions and account for capture overhead.".into());
        lines.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv(times: &str) -> String {
        let mut result = format!("{CSV_HEADER}\n");
        for timing in times.split(',') {
            result.push_str(&format!(
                "PresentMon,2.4,game.exe,0x1,ComposedFlip,rendered,export,scene-a,{timing}\n"
            ));
        }
        result
    }

    #[test]
    fn diagnostics_csv_reports_actual_fps_and_nearest_rank_percentiles() {
        let run = parse(csv("10,20,30,40").as_bytes()).unwrap();
        assert_eq!(run.stats.frames, 4);
        assert_eq!(run.stats.total_ms, 100.0);
        assert_eq!(run.stats.average_fps, 40.0);
        assert_eq!(run.stats.p50_ms, 20.0);
        assert_eq!(run.stats.p95_ms, 40.0);
        assert_eq!(run.stats.p99_ms, 40.0);
        assert_eq!(run.stats.variance_ms2, 125.0);
        assert_eq!(run.stats.frames_over_50_ms, 0);
    }

    #[test]
    fn diagnostics_csv_rejects_missing_invalid_nonfinite_or_nonpositive_timings() {
        for timing in ["", "abc", "NaN", "inf", "-1", "0", "60001"] {
            assert!(parse(csv(timing).as_bytes()).is_err(), "accepted {timing}");
        }
        assert!(parse(CSV_HEADER.as_bytes()).is_err());
        assert!(parse(b"frame_time_ms\n12\n").is_err());
    }

    #[test]
    fn diagnostics_csv_rejects_finite_inputs_that_make_reported_fps_nonfinite() {
        assert!(parse(csv("1e-320").as_bytes()).is_err());
    }

    #[test]
    fn diagnostics_csv_rejects_mixed_runs_and_bounds_file_rows_and_fields() {
        let mixed = csv("10,10").replacen("scene-a,10", "scene-b,10", 1);
        assert!(parse(mixed.as_bytes()).is_err());
        assert!(parse(&vec![b'a'; MAX_FILE_BYTES + 1]).is_err());
        let row = "x,1,g,s,p,rendered,e,w,1\n";
        let too_many = format!("{CSV_HEADER}\n{}", row.repeat(MAX_ROWS + 1));
        assert!(parse(too_many.as_bytes()).is_err());
        let long = csv("10").replace("scene-a", &"a".repeat(MAX_FIELD_BYTES + 1));
        assert!(parse(long.as_bytes()).is_err());
    }

    #[test]
    fn diagnostics_csv_accepts_quoted_metadata_and_crlf_without_loss() {
        let input = csv("10,20")
            .replace("scene-a", "\"scene, \"\"A\"\"\"")
            .replace('\n', "\r\n");
        let run = parse(input.as_bytes()).unwrap();
        assert_eq!(run.metadata.scenario, "scene, \"A\"");
        assert!(parse(csv("10").replace("scene-a", "\"unclosed").as_bytes()).is_err());
    }

    #[test]
    fn diagnostics_ab_requires_exact_capture_metadata_and_similar_duration() {
        let a = parse(csv("10,10,10,10").as_bytes()).unwrap();
        let b = parse(csv("8,8,8,8,8").as_bytes()).unwrap();
        let comparison = compare(&a, &b).unwrap();
        assert_eq!(comparison.fps_change_percent, 25.0);
        assert!((comparison.p99_change_percent + 20.0).abs() < 1e-9);
        let generated = parse(
            csv("10,10,10,10")
                .replace("rendered", "generated")
                .as_bytes(),
        )
        .unwrap();
        assert!(compare(&a, &generated).is_err());
        let short = parse(csv("10").as_bytes()).unwrap();
        assert!(compare(&a, &short).is_err());
    }

    #[test]
    fn diagnostics_measurement_failed_import_keeps_both_previous_runs() {
        let mut captures = Captures::default();
        captures.accept(parse(csv("10,10").as_bytes()).unwrap());
        captures.accept(parse(csv("20").as_bytes()).unwrap());
        let previous = captures.summary();
        assert!(parse(csv("NaN").as_bytes()).is_err());
        assert_eq!(captures.summary(), previous);
        assert_eq!(captures.next_label(), "A");
        captures.accept(parse(csv("5,5,5,5").as_bytes()).unwrap());
        assert_eq!(captures.next_label(), "B");
    }
}
