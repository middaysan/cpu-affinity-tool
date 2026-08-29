use std::ffi::OsString;
#[cfg(any(feature = "windows", test))]
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
#[cfg(any(feature = "windows", test))]
use std::io::Write;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
#[cfg(any(feature = "windows", test))]
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[cfg(any(feature = "windows", test))]
use std::sync::Arc;
#[cfg(any(feature = "windows", test))]
use std::thread::ThreadId;
use std::time::{Duration, Instant};
#[cfg(any(feature = "windows", test))]
use std::time::{SystemTime, UNIX_EPOCH};

pub const REPORT_DIRECTORY_NAME: &str = "crash-reports";
pub const REPORT_MAGIC: &str = "CPU-AFFINITY-TOOL-CRASH-REPORT\n";
pub const REPORT_COMPLETION_MARKER: &str = "--- END CRASH REPORT ---\n";
pub const REPORT_FORMAT_VERSION: u32 = 1;
pub const MAX_REPORT_BYTES: usize = 256 * 1024;
#[cfg(any(feature = "windows", test))]
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024;
pub const MAX_REASON_BYTES: usize = 512;
pub const MAX_APP_VERSION_BYTES: usize = 128;
#[cfg(any(feature = "windows", test))]
pub const MAX_NAME_ATTEMPTS: usize = 100;
pub const RETAIN_REPORTS: usize = 20;
pub const MAX_SCAN_ENTRIES: usize = 512;
#[cfg(any(feature = "windows", test))]
pub const MAX_UNPRUNED_REPORTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashReportKind {
    MainThreadPanic,
    NativeLoopError,
}

impl CrashReportKind {
    #[cfg(any(feature = "windows", test))]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MainThreadPanic => "main_thread_panic",
            Self::NativeLoopError => "native_loop_error",
        }
    }

    pub fn user_title(self) -> &'static str {
        match self {
            Self::MainThreadPanic => "Application panic",
            Self::NativeLoopError => "Application runtime failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "main_thread_panic" => Some(Self::MainThreadPanic),
            "native_loop_error" => Some(Self::NativeLoopError),
            _ => None,
        }
    }
}

#[cfg(any(feature = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPhase {
    PreparingRuntime,
    RunningUi,
    Closing,
}

#[cfg(any(feature = "windows", test))]
impl StartupPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreparingRuntime => "preparing_runtime",
            Self::RunningUi => "running_ui",
            Self::Closing => "closing",
        }
    }
}

#[cfg(any(feature = "windows", test))]
#[derive(Debug, Clone, Copy)]
pub struct ReportSource<'a> {
    pub file: &'a str,
    pub line: u32,
    pub column: u32,
}

#[cfg(any(feature = "windows", test))]
#[derive(Debug)]
pub struct ReportInput<'a> {
    pub kind: CrashReportKind,
    pub timestamp: SystemTime,
    pub process_id: u32,
    pub app_version: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
    pub phase: StartupPhase,
    pub payload: Option<&'a str>,
    pub source: Option<ReportSource<'a>>,
    pub backtrace: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReport {
    pub kind: CrashReportKind,
    pub timestamp_utc: String,
    pub app_version: String,
    pub reason: String,
}

#[cfg(any(feature = "windows", test))]
struct BoundedText {
    bytes: Vec<u8>,
    limit: usize,
}

#[cfg(any(feature = "windows", test))]
impl BoundedText {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(16 * 1024)),
            limit,
        }
    }

    fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.bytes.len())
    }

    fn push_str_lossless(&mut self, value: &str) -> bool {
        if value.len() > self.remaining() {
            return false;
        }
        self.bytes.extend_from_slice(value.as_bytes());
        true
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(any(feature = "windows", test))]
impl std::fmt::Write for BoundedText {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        if self.push_str_lossless(value) {
            return Ok(());
        }

        let mut accepted = self.remaining().min(value.len());
        while accepted > 0 && !value.is_char_boundary(accepted) {
            accepted -= 1;
        }
        self.bytes.extend_from_slice(&value.as_bytes()[..accepted]);
        Err(std::fmt::Error)
    }
}

#[cfg(any(feature = "windows", test))]
pub fn format_report(input: &ReportInput<'_>) -> Vec<u8> {
    let body_limit = MAX_REPORT_BYTES - REPORT_COMPLETION_MARKER.len();
    let mut output = BoundedText::new(body_limit);
    let timestamp = format_utc_timestamp(input.timestamp);

    let _ = write!(
        output,
        "{REPORT_MAGIC}format_version: {REPORT_FORMAT_VERSION}\n\
         event_kind: {}\n\
         timestamp_utc: {timestamp}\n\
         app_version: {}\n\
         operating_system: {}\n\
         architecture: {}\n\
         process_id: {}\n\
         startup_phase: {}\n",
        input.kind.as_str(),
        sanitized_inline(input.app_version, 128),
        sanitized_inline(input.os, 64),
        sanitized_inline(input.arch, 64),
        input.process_id,
        input.phase.as_str(),
    );

    let _ = output.push_str_lossless("source: ");
    match input.source {
        Some(source) => {
            let source_file = sanitized_inline(source.file, 2048);
            let _ = writeln!(output, "{source_file}:{}:{}", source.line, source.column);
        }
        None => {
            let _ = output.push_str_lossless("<unavailable>\n");
        }
    }

    let _ = output.push_str_lossless("payload:\n");
    let payload = input.payload.unwrap_or("<non-string panic payload>");
    append_normalized_section(
        &mut output,
        payload,
        MAX_PAYLOAD_BYTES,
        "[payload truncated]\n",
    );

    let _ = output.push_str_lossless("backtrace:\n");
    match input.backtrace {
        Some(backtrace) if !backtrace.is_empty() => {
            let remaining = output.remaining();
            append_normalized_section(&mut output, backtrace, remaining, "[backtrace truncated]\n");
        }
        _ => {
            let _ = output.push_str_lossless("<disabled or unavailable>\n");
        }
    }

    let mut bytes = output.into_bytes();
    if bytes.len() + REPORT_COMPLETION_MARKER.len() > MAX_REPORT_BYTES {
        bytes.truncate(MAX_REPORT_BYTES - REPORT_COMPLETION_MARKER.len());
        while std::str::from_utf8(&bytes).is_err() {
            bytes.pop();
        }
    }
    bytes.extend_from_slice(REPORT_COMPLETION_MARKER.as_bytes());
    bytes
}

#[cfg(any(feature = "windows", test))]
fn append_normalized_section(
    output: &mut BoundedText,
    value: &str,
    requested_limit: usize,
    truncation_marker: &str,
) {
    let section_limit = requested_limit.min(output.remaining());
    let mut normalized = String::with_capacity(value.len().min(section_limit));
    let mut chars = value.chars().peekable();

    while let Some(character) = chars.next() {
        let normalized_character = match character {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                '\n'
            }
            '\n' | '\t' => character,
            value if value.is_control() => '\u{fffd}',
            value => value,
        };

        let marker_reserve = truncation_marker.len().min(section_limit);
        let content_limit = section_limit.saturating_sub(marker_reserve);
        if normalized.len() + normalized_character.len_utf8() > content_limit {
            let _ = output.push_str_lossless(&normalized);
            let _ = output.push_str_lossless(truncation_marker);
            return;
        }
        normalized.push(normalized_character);
    }

    let _ = output.push_str_lossless(&normalized);
    if !normalized.ends_with('\n') {
        let _ = output.push_str_lossless("\n");
    }
}

fn sanitized_inline(value: &str, max_bytes: usize) -> String {
    let mut sanitized = String::with_capacity(value.len().min(max_bytes));
    for character in value.chars() {
        let character = if character.is_control() || matches!(character, '\r' | '\n' | '\t') {
            '\u{fffd}'
        } else {
            character
        };
        if sanitized.len() + character.len_utf8() > max_bytes {
            break;
        }
        sanitized.push(character);
    }
    sanitized
}

pub fn parse_report(bytes: &[u8]) -> Result<ParsedReport, String> {
    if bytes.len() > MAX_REPORT_BYTES {
        return Err("report exceeds the supported size".to_string());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "report is not valid UTF-8".to_string())?;
    if !text.starts_with(REPORT_MAGIC) || !text.ends_with(REPORT_COMPLETION_MARKER) {
        return Err("report is incomplete or has an unknown header".to_string());
    }

    let version = field(text, "format_version")
        .ok_or_else(|| "report format version is missing".to_string())?;
    if version != REPORT_FORMAT_VERSION.to_string() {
        return Err(format!("unsupported report format version: {version}"));
    }

    let kind = field(text, "event_kind")
        .and_then(CrashReportKind::parse)
        .ok_or_else(|| "report event kind is missing or unknown".to_string())?;
    let timestamp_utc = field(text, "timestamp_utc")
        .ok_or_else(|| "report timestamp is missing".to_string())?
        .to_string();
    let app_version = field(text, "app_version")
        .ok_or_else(|| "report application version is missing".to_string())?
        .to_string();
    let reason = text
        .split_once("payload:\n")
        .and_then(|(_, payload)| payload.lines().next())
        .unwrap_or("<unavailable>");

    Ok(ParsedReport {
        kind,
        timestamp_utc,
        app_version: sanitized_inline(&app_version, MAX_APP_VERSION_BYTES),
        reason: sanitized_inline(reason, MAX_REASON_BYTES),
    })
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}: ");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(str::trim)
}

#[cfg(any(feature = "windows", test))]
pub fn report_file_name(timestamp: SystemTime, process_id: u32, attempt: usize) -> Option<String> {
    if attempt >= MAX_NAME_ATTEMPTS {
        return None;
    }
    Some(format!(
        "crash-{}-p{process_id}-{attempt:02}.txt",
        format_utc_timestamp(timestamp)
    ))
}

#[cfg(any(feature = "windows", test))]
fn format_utc_timestamp(timestamp: SystemTime) -> String {
    const MAX_SUPPORTED_MILLIS: u64 = 253_402_300_799_999;
    let millis = timestamp
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(MAX_SUPPORTED_MILLIS as u128) as u64)
        .unwrap_or_default();
    let seconds = millis / 1_000;
    let milliseconds = millis % 1_000;
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    let (year, month, day) = civil_date_from_days(days as i64);

    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}.{milliseconds:03}Z")
}

#[cfg(any(feature = "windows", test))]
fn civil_date_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let adjusted = days_since_epoch + 719_468;
    let era = adjusted.div_euclid(146_097);
    let day_of_era = adjusted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    volume: u64,
    index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReportEntry {
    file_name: OsString,
    identity: FileIdentity,
    pub kind: CrashReportKind,
    pub timestamp_utc: String,
    pub app_version: String,
    pub reason: String,
    pub size_bytes: u64,
}

impl CrashReportEntry {
    pub fn path_in(&self, report_directory: &Path) -> PathBuf {
        report_directory.join(&self.file_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSnapshot {
    pub report_directory: PathBuf,
    root_identity: Option<FileIdentity>,
    pub reports: Vec<CrashReportEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    UnsafeRoot(String),
    ReadDirectory(String),
    ScanLimitExceeded,
    InvalidReport { file_name: String, reason: String },
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafeRoot(reason) => {
                write!(formatter, "unsafe crash report directory: {reason}")
            }
            Self::ReadDirectory(reason) => {
                write!(
                    formatter,
                    "could not read the crash report directory: {reason}"
                )
            }
            Self::ScanLimitExceeded => write!(
                formatter,
                "the crash report directory contains more than {MAX_SCAN_ENTRIES} entries"
            ),
            Self::InvalidReport { file_name, reason } => {
                write!(
                    formatter,
                    "could not validate crash report '{file_name}': {reason}"
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteError {
    NotFound,
    Unavailable,
    UnsafeRoot(String),
    IdentityChanged,
    Failed(String),
}

impl std::fmt::Display for DeleteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "the crash report no longer exists"),
            Self::Unavailable => {
                write!(
                    formatter,
                    "crash report actions are unavailable on this platform"
                )
            }
            Self::UnsafeRoot(reason) => {
                write!(formatter, "unsafe crash report directory: {reason}")
            }
            Self::IdentityChanged => write!(
                formatter,
                "the crash report changed after it was listed; refresh and try again"
            ),
            Self::Failed(reason) => {
                write!(formatter, "could not delete the crash report: {reason}")
            }
        }
    }
}

struct TrustedRoot {
    path: PathBuf,
    _handle: File,
    identity: FileIdentity,
}

impl TrustedRoot {
    fn open_existing(path: &Path) -> Result<Option<Self>, ScanError> {
        match open_directory_handle(path) {
            Ok(handle) => {
                validate_directory_handle(&handle)
                    .map_err(|error| ScanError::UnsafeRoot(error.to_string()))?;
                let identity = file_identity(&handle)
                    .map_err(|error| ScanError::UnsafeRoot(error.to_string()))?;
                Ok(Some(Self {
                    path: path.to_path_buf(),
                    _handle: handle,
                    identity,
                }))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ScanError::UnsafeRoot(error.to_string())),
        }
    }

    fn create_or_open(path: &Path) -> io::Result<Self> {
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let handle = open_directory_handle(path)?;
        validate_directory_handle(&handle)?;
        let identity = file_identity(&handle)?;
        Ok(Self {
            path: path.to_path_buf(),
            _handle: handle,
            identity,
        })
    }
}

#[cfg(any(feature = "windows", test))]
pub fn write_report(report_directory: &Path, input: &ReportInput<'_>) -> Result<PathBuf, String> {
    let root = TrustedRoot::create_or_open(report_directory).map_err(|error| {
        format!(
            "failed to prepare crash report directory '{}': {error}",
            report_directory.display()
        )
    })?;
    ensure_writer_capacity(&root)?;
    let report = format_report(input);

    for attempt in 0..MAX_NAME_ATTEMPTS {
        let file_name = report_file_name(input.timestamp, input.process_id, attempt)
            .ok_or_else(|| "crash report filename attempts were exhausted".to_string())?;
        let final_path = root.path.join(&file_name);
        let partial_path = root.path.join(format!("{file_name}.partial"));
        let mut file = match create_report_file(&partial_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve crash report '{}': {error}",
                    partial_path.display()
                ));
            }
        };

        if let Err(error) = validate_regular_file_handle(&file)
            .and_then(|_| file.write_all(&report))
            .and_then(|_| file.sync_all())
        {
            let _ = delete_open_file(&file, &partial_path);
            return Err(format!(
                "failed to write crash report '{}': {error}",
                partial_path.display()
            ));
        }

        match publish_open_file_no_replace(&file, &partial_path, &final_path) {
            Ok(()) => return Ok(final_path),
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    || final_path.try_exists().unwrap_or(false) =>
            {
                let _ = delete_open_file(&file, &partial_path);
            }
            Err(error) => {
                let _ = delete_open_file(&file, &partial_path);
                return Err(format!(
                    "failed to publish crash report '{}': {error}",
                    final_path.display()
                ));
            }
        }
    }

    Err(format!(
        "failed to reserve a unique crash report name after {MAX_NAME_ATTEMPTS} attempts"
    ))
}

#[cfg(any(feature = "windows", test))]
fn ensure_writer_capacity(root: &TrustedRoot) -> Result<(), String> {
    let entries = fs::read_dir(&root.path).map_err(|error| {
        format!(
            "failed to inspect crash report directory '{}': {error}",
            root.path.display()
        )
    })?;
    let mut managed_entries = 0usize;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_SCAN_ENTRIES {
            return Err(format!(
                "crash report writing is paused because the directory contains more than \
                 {MAX_SCAN_ENTRIES} entries"
            ));
        }
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect crash report directory '{}': {error}",
                root.path.display()
            )
        })?;
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_report_file_name(&file_name) || is_partial_report_file_name(&file_name) {
            managed_entries += 1;
            if managed_entries >= MAX_UNPRUNED_REPORTS {
                return Err(
                    "crash report writing is paused until the existing reports are reviewed"
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

pub fn scan_reports(report_directory: &Path) -> Result<ReportSnapshot, ScanError> {
    let Some(root) = TrustedRoot::open_existing(report_directory)? else {
        return Ok(ReportSnapshot {
            report_directory: report_directory.to_path_buf(),
            root_identity: None,
            reports: Vec::new(),
        });
    };

    let entries =
        fs::read_dir(&root.path).map_err(|error| ScanError::ReadDirectory(error.to_string()))?;
    let mut reports = Vec::new();

    for (index, entry) in entries.enumerate() {
        if index >= MAX_SCAN_ENTRIES {
            return Err(ScanError::ScanLimitExceeded);
        }
        let entry = entry.map_err(|error| ScanError::ReadDirectory(error.to_string()))?;
        let file_name = entry.file_name();
        let Some(file_name_text) = file_name.to_str() else {
            continue;
        };

        if is_partial_report_file_name(file_name_text) {
            return Err(ScanError::InvalidReport {
                file_name: file_name_text.to_string(),
                reason: "an incomplete .partial file is present".to_string(),
            });
        }
        let Some(file_timestamp) = report_file_timestamp(file_name_text) else {
            continue;
        };

        let path = root.path.join(&file_name);
        let mut file =
            open_report_file_for_read(&path).map_err(|error| ScanError::InvalidReport {
                file_name: file_name_text.to_string(),
                reason: error.to_string(),
            })?;
        validate_regular_file_handle(&file).map_err(|error| ScanError::InvalidReport {
            file_name: file_name_text.to_string(),
            reason: error.to_string(),
        })?;
        let identity = file_identity(&file).map_err(|error| ScanError::InvalidReport {
            file_name: file_name_text.to_string(),
            reason: error.to_string(),
        })?;
        let metadata = file.metadata().map_err(|error| ScanError::InvalidReport {
            file_name: file_name_text.to_string(),
            reason: error.to_string(),
        })?;
        if metadata.len() > MAX_REPORT_BYTES as u64 {
            return Err(ScanError::InvalidReport {
                file_name: file_name_text.to_string(),
                reason: "report exceeds the supported size".to_string(),
            });
        }

        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::by_ref(&mut file)
            .take((MAX_REPORT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| ScanError::InvalidReport {
                file_name: file_name_text.to_string(),
                reason: error.to_string(),
            })?;
        let parsed = parse_report(&bytes).map_err(|reason| ScanError::InvalidReport {
            file_name: file_name_text.to_string(),
            reason,
        })?;
        if parsed.timestamp_utc != file_timestamp {
            return Err(ScanError::InvalidReport {
                file_name: file_name_text.to_string(),
                reason: "filename and report timestamps do not match".to_string(),
            });
        }

        reports.push(CrashReportEntry {
            file_name,
            identity,
            kind: parsed.kind,
            timestamp_utc: parsed.timestamp_utc,
            app_version: parsed.app_version,
            reason: parsed.reason,
            size_bytes: metadata.len(),
        });
    }

    reports.sort_by(|left, right| {
        right
            .timestamp_utc
            .cmp(&left.timestamp_utc)
            .then_with(|| right.file_name.cmp(&left.file_name))
    });

    Ok(ReportSnapshot {
        report_directory: root.path,
        root_identity: Some(root.identity),
        reports,
    })
}

pub fn apply_retention(snapshot: &ReportSnapshot) -> Result<usize, DeleteError> {
    let mut deleted = 0;
    for report in snapshot.reports.iter().skip(RETAIN_REPORTS) {
        match delete_report(snapshot, report) {
            Ok(()) => deleted += 1,
            Err(DeleteError::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(deleted)
}

pub fn delete_report(
    snapshot: &ReportSnapshot,
    report: &CrashReportEntry,
) -> Result<(), DeleteError> {
    if !snapshot.reports.iter().any(|candidate| {
        candidate.file_name == report.file_name && candidate.identity == report.identity
    }) {
        return Err(DeleteError::IdentityChanged);
    }

    let root = TrustedRoot::open_existing(&snapshot.report_directory)
        .map_err(|error| DeleteError::UnsafeRoot(error.to_string()))?
        .ok_or(DeleteError::NotFound)?;
    if Some(root.identity) != snapshot.root_identity {
        return Err(DeleteError::UnsafeRoot(
            "the crash report directory changed after it was listed".to_string(),
        ));
    }

    let path = report.path_in(&root.path);
    let file = match open_report_file_for_delete(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(DeleteError::NotFound);
        }
        Err(error) => return Err(DeleteError::Failed(error.to_string())),
    };
    validate_regular_file_handle(&file).map_err(|error| DeleteError::Failed(error.to_string()))?;
    let current_identity =
        file_identity(&file).map_err(|error| DeleteError::Failed(error.to_string()))?;
    if current_identity != report.identity {
        return Err(DeleteError::IdentityChanged);
    }

    delete_open_file(&file, &path).map_err(|error| DeleteError::Failed(error.to_string()))
}

pub fn validated_report_path(
    snapshot: &ReportSnapshot,
    report: &CrashReportEntry,
) -> Result<PathBuf, DeleteError> {
    if !snapshot.reports.iter().any(|candidate| {
        candidate.file_name == report.file_name && candidate.identity == report.identity
    }) {
        return Err(DeleteError::IdentityChanged);
    }
    let root = TrustedRoot::open_existing(&snapshot.report_directory)
        .map_err(|error| DeleteError::UnsafeRoot(error.to_string()))?
        .ok_or(DeleteError::NotFound)?;
    if Some(root.identity) != snapshot.root_identity {
        return Err(DeleteError::UnsafeRoot(
            "the crash report directory changed after it was listed".to_string(),
        ));
    }

    let path = report.path_in(&root.path);
    let file = match open_report_file_for_read(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(DeleteError::NotFound);
        }
        Err(error) => return Err(DeleteError::Failed(error.to_string())),
    };
    validate_regular_file_handle(&file).map_err(|error| DeleteError::Failed(error.to_string()))?;
    if file_identity(&file).map_err(|error| DeleteError::Failed(error.to_string()))?
        != report.identity
    {
        return Err(DeleteError::IdentityChanged);
    }
    Ok(path)
}

pub fn prepare_report_directory(report_directory: &Path) -> Result<PathBuf, String> {
    TrustedRoot::create_or_open(report_directory)
        .map(|root| root.path)
        .map_err(|error| {
            format!(
                "failed to prepare crash report directory '{}': {error}",
                report_directory.display()
            )
        })
}

fn is_partial_report_file_name(file_name: &str) -> bool {
    file_name
        .strip_suffix(".partial")
        .is_some_and(is_report_file_name)
}

fn is_report_file_name(file_name: &str) -> bool {
    report_file_timestamp(file_name).is_some()
}

fn report_file_timestamp(file_name: &str) -> Option<&str> {
    let body = file_name
        .strip_prefix("crash-")
        .and_then(|name| name.strip_suffix(".txt"))?;
    let (timestamp, process_and_attempt) = body.split_once("-p")?;
    if timestamp.len() != 20 {
        return None;
    }
    let timestamp_bytes = timestamp.as_bytes();
    let timestamp_shape = timestamp_bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| match index {
            8 => *byte == b'T',
            15 => *byte == b'.',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        });
    if !timestamp_shape {
        return None;
    }

    let (process_id, attempt) = process_and_attempt.rsplit_once('-')?;
    let valid_suffix = !process_id.is_empty()
        && process_id.bytes().all(|byte| byte.is_ascii_digit())
        && attempt.len() == 2
        && attempt.bytes().all(|byte| byte.is_ascii_digit());
    valid_suffix.then_some(timestamp)
}

#[cfg(target_os = "windows")]
fn open_directory_handle(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
        .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
        .open(path)
}

#[cfg(not(target_os = "windows"))]
fn open_directory_handle(path: &Path) -> io::Result<File> {
    File::open(path)
}

fn validate_directory_handle(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_dir() {
        return Err(io::Error::other("path is not a directory"));
    }
    if metadata_is_reparse_or_symlink(&metadata) {
        return Err(io::Error::other(
            "directory is a symbolic link or reparse point",
        ));
    }
    Ok(())
}

fn validate_regular_file_handle(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("entry is not a regular file"));
    }
    if metadata_is_reparse_or_symlink(&metadata) {
        return Err(io::Error::other(
            "entry is a symbolic link or reparse point",
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn metadata_is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}

#[cfg(not(target_os = "windows"))]
fn metadata_is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(target_os = "windows")]
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), information.as_mut_ptr())
            .map_err(|_| io::Error::last_os_error())?;
        let information = information.assume_init();
        Ok(FileIdentity {
            volume: information.dwVolumeSerialNumber as u64,
            index: ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
        })
    }
}

#[cfg(unix)]
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(FileIdentity {
        volume: metadata.dev(),
        index: metadata.ino(),
    })
}

#[cfg(all(any(feature = "windows", test), target_os = "windows"))]
fn create_report_file(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    OpenOptions::new()
        .write(true)
        .access_mode((FILE_GENERIC_WRITE | DELETE).0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .create_new(true)
        .open(path)
}

#[cfg(all(any(feature = "windows", test), not(target_os = "windows")))]
fn create_report_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(target_os = "windows")]
fn open_report_file_for_read(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    OpenOptions::new()
        .read(true)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

#[cfg(not(target_os = "windows"))]
fn open_report_file_for_read(path: &Path) -> io::Result<File> {
    File::open(path)
}

#[cfg(target_os = "windows")]
fn open_report_file_for_delete(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    OpenOptions::new()
        .access_mode((FILE_GENERIC_READ | DELETE).0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

#[cfg(not(target_os = "windows"))]
fn open_report_file_for_delete(path: &Path) -> io::Result<File> {
    File::open(path)
}

#[cfg(target_os = "windows")]
fn file_rename_path(path: &Path) -> (Vec<u16>, u32) {
    use std::os::windows::ffi::OsStrExt;

    let mut wide_path = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let name_length = (wide_path.len() * size_of::<u16>()) as u32;
    wide_path.push(0);
    (wide_path, name_length)
}

#[cfg(all(any(feature = "windows", test), target_os = "windows"))]
fn publish_open_file_no_replace(
    file: &File,
    _source_path: &Path,
    final_path: &Path,
) -> io::Result<()> {
    use std::mem::{offset_of, size_of};
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileRenameInfo, SetFileInformationByHandle, FILE_RENAME_INFO,
    };

    let (final_name, final_name_length) = file_rename_path(final_path);
    let name_offset = offset_of!(FILE_RENAME_INFO, FileName);
    let byte_count = name_offset + final_name.len() * size_of::<u16>();
    let word_count = byte_count.div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; word_count];
    let information = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();

    unsafe {
        (*information).Anonymous.ReplaceIfExists = false;
        (*information).RootDirectory = HANDLE::default();
        (*information).FileNameLength = final_name_length;
        std::ptr::copy_nonoverlapping(
            final_name.as_ptr(),
            storage
                .as_mut_ptr()
                .cast::<u8>()
                .add(name_offset)
                .cast::<u16>(),
            final_name.len(),
        );
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileRenameInfo,
            information.cast(),
            byte_count as u32,
        )
        .map_err(|_| io::Error::last_os_error())
    }
}

#[cfg(all(any(feature = "windows", test), not(target_os = "windows")))]
fn publish_open_file_no_replace(
    _file: &File,
    source_path: &Path,
    final_path: &Path,
) -> io::Result<()> {
    fs::hard_link(source_path, final_path)?;
    fs::remove_file(source_path)
}

#[cfg(target_os = "windows")]
fn delete_open_file(file: &File, _path: &Path) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };

    let information = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            (&raw const information).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
        .map_err(|_| io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "windows"))]
fn delete_open_file(_file: &File, path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrashReportIndexState {
    Loading {
        last_complete: Option<ReportSnapshot>,
    },
    Ready {
        snapshot: ReportSnapshot,
    },
    Incomplete {
        last_complete: Option<ReportSnapshot>,
        reason: String,
    },
}

impl CrashReportIndexState {
    pub fn snapshot(&self) -> Option<&ReportSnapshot> {
        match self {
            Self::Loading { last_complete } | Self::Incomplete { last_complete, .. } => {
                last_complete.as_ref()
            }
            Self::Ready { snapshot } => Some(snapshot),
        }
    }

    fn last_complete(&self) -> Option<ReportSnapshot> {
        self.snapshot().cloned()
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub fn warning(&self) -> Option<&str> {
        match self {
            Self::Incomplete { reason, .. } => Some(reason),
            _ => None,
        }
    }

    pub fn indicator(&self) -> CrashReportIndicator {
        match self {
            Self::Loading {
                last_complete: None,
            } => CrashReportIndicator {
                count: None,
                stale: false,
                label: "Crash reports: loading".to_string(),
            },
            Self::Loading {
                last_complete: Some(snapshot),
            } => CrashReportIndicator {
                count: Some(snapshot.reports.len()),
                stale: true,
                label: format!(
                    "Saved crash reports: {} (refreshing)",
                    snapshot.reports.len()
                ),
            },
            Self::Ready { snapshot } => CrashReportIndicator {
                count: Some(snapshot.reports.len()),
                stale: false,
                label: format!("Saved crash reports: {}", snapshot.reports.len()),
            },
            Self::Incomplete {
                last_complete: Some(snapshot),
                ..
            } => CrashReportIndicator {
                count: Some(snapshot.reports.len()),
                stale: true,
                label: format!(
                    "Saved crash reports: {} (stale); some files could not be read",
                    snapshot.reports.len()
                ),
            },
            Self::Incomplete {
                last_complete: None,
                ..
            } => CrashReportIndicator {
                count: None,
                stale: true,
                label: "Crash report count unavailable; some files could not be read".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReportIndicator {
    pub count: Option<usize>,
    pub stale: bool,
    pub label: String,
}

pub struct CrashReportManager {
    report_directory: PathBuf,
    enabled: bool,
    state: CrashReportIndexState,
    worker: Option<Receiver<Result<ReportSnapshot, String>>>,
    worker_started: Option<Instant>,
    refresh_pending: bool,
}

impl CrashReportManager {
    pub fn new(report_directory: PathBuf) -> Self {
        let mut manager = Self {
            report_directory,
            enabled: true,
            state: CrashReportIndexState::Loading {
                last_complete: None,
            },
            worker: None,
            worker_started: None,
            refresh_pending: false,
        };
        manager.request_refresh();
        manager
    }

    #[cfg(test)]
    pub fn new_idle(report_directory: PathBuf) -> Self {
        Self {
            report_directory: report_directory.clone(),
            enabled: true,
            state: CrashReportIndexState::Ready {
                snapshot: ReportSnapshot {
                    report_directory,
                    root_identity: None,
                    reports: Vec::new(),
                },
            },
            worker: None,
            worker_started: None,
            refresh_pending: false,
        }
    }

    #[cfg(any(test, not(all(target_os = "windows", feature = "windows"))))]
    pub fn new_inactive(report_directory: PathBuf) -> Self {
        Self {
            report_directory: report_directory.clone(),
            enabled: false,
            state: CrashReportIndexState::Ready {
                snapshot: ReportSnapshot {
                    report_directory,
                    root_identity: None,
                    reports: Vec::new(),
                },
            },
            worker: None,
            worker_started: None,
            refresh_pending: false,
        }
    }

    pub fn report_directory(&self) -> &Path {
        &self.report_directory
    }

    pub fn state(&self) -> &CrashReportIndexState {
        &self.state
    }

    /// A compact Activity entry for the newest report from a complete,
    /// already-validated snapshot. The crash file remains the full support
    /// artifact and is not synchronously reread during startup.
    pub fn latest_activity_message(&self) -> Option<String> {
        let snapshot = self.state.snapshot()?;
        let report = snapshot.reports.first()?;
        Some(format!(
            "Previous crash report: {} at {} UTC (app v{}).\\nReason: {}\\nFull report: {}",
            report.kind.user_title(),
            report.timestamp_utc,
            report.app_version,
            report.reason,
            report.path_in(&snapshot.report_directory).display(),
        ))
    }

    pub fn request_refresh(&mut self) {
        if !self.enabled {
            return;
        }
        if self.worker.is_some() {
            self.refresh_pending = true;
            return;
        }
        self.start_worker();
    }

    pub fn poll(&mut self) -> bool {
        let Some(worker) = self.worker.as_ref() else {
            return false;
        };

        match worker.try_recv() {
            Ok(result) => {
                let last_complete = self.state.last_complete();
                self.worker = None;
                self.worker_started = None;
                self.state = match result {
                    Ok(snapshot) => CrashReportIndexState::Ready { snapshot },
                    Err(reason) => CrashReportIndexState::Incomplete {
                        last_complete,
                        reason,
                    },
                };
                if self.refresh_pending {
                    self.refresh_pending = false;
                    self.start_worker();
                }
                true
            }
            Err(TryRecvError::Disconnected) => {
                let last_complete = self.state.last_complete();
                self.worker = None;
                self.worker_started = None;
                self.state = CrashReportIndexState::Incomplete {
                    last_complete,
                    reason: "the crash report scan worker stopped unexpectedly".to_string(),
                };
                true
            }
            Err(TryRecvError::Empty) => {
                if self
                    .worker_started
                    .is_some_and(|started| started.elapsed() >= Duration::from_secs(2))
                    && !matches!(self.state, CrashReportIndexState::Incomplete { .. })
                {
                    let last_complete = self.state.last_complete();
                    self.state = CrashReportIndexState::Incomplete {
                        last_complete,
                        reason: "the crash report scan is taking longer than expected".to_string(),
                    };
                    return true;
                }
                false
            }
        }
    }

    #[cfg(test)]
    pub fn worker_is_active(&self) -> bool {
        self.worker.is_some()
    }

    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub fn worker_poll_interval(&self) -> Option<Duration> {
        self.worker.as_ref()?;
        Some(
            if self
                .worker_started
                .is_none_or(|started| started.elapsed() < Duration::from_secs(2))
            {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(2)
            },
        )
    }

    pub fn delete_one(&mut self, report: &CrashReportEntry) -> Result<(), DeleteError> {
        if !self.enabled {
            return Err(DeleteError::Unavailable);
        }
        let snapshot = self
            .state
            .snapshot()
            .cloned()
            .ok_or(DeleteError::NotFound)?;
        let result = delete_report(&snapshot, report);
        self.request_refresh();
        result
    }

    pub fn delete_saved_reports_from(
        &mut self,
        snapshot: &ReportSnapshot,
    ) -> Result<usize, DeleteError> {
        if !self.enabled {
            return Err(DeleteError::Unavailable);
        }
        let mut deleted = 0;
        for report in &snapshot.reports {
            match delete_report(snapshot, report) {
                Ok(()) => deleted += 1,
                Err(DeleteError::NotFound) => {}
                Err(error) => {
                    self.request_refresh();
                    return Err(error);
                }
            }
        }
        self.request_refresh();
        Ok(deleted)
    }

    fn start_worker(&mut self) {
        let (sender, receiver) = mpsc::channel();
        let report_directory = self.report_directory.clone();
        let last_complete = self.state.last_complete();
        self.state = CrashReportIndexState::Loading { last_complete };
        self.worker = Some(receiver);
        let now = Instant::now();
        self.worker_started = Some(now);

        let spawn_result = std::thread::Builder::new()
            .name("crash-report-scan".to_string())
            .spawn(move || {
                let result = scan_and_apply_retention(&report_directory);
                let _ = sender.send(result);
            });
        if let Err(error) = spawn_result {
            let last_complete = self.state.last_complete();
            self.worker = None;
            self.worker_started = None;
            self.state = CrashReportIndexState::Incomplete {
                last_complete,
                reason: format!("failed to start the crash report scan worker: {error}"),
            };
        }
    }
}

fn scan_and_apply_retention(report_directory: &Path) -> Result<ReportSnapshot, String> {
    let snapshot = scan_reports(report_directory).map_err(|error| error.to_string())?;
    if snapshot.reports.len() <= RETAIN_REPORTS {
        return Ok(snapshot);
    }
    apply_retention(&snapshot).map_err(|error| error.to_string())?;
    scan_reports(report_directory).map_err(|error| error.to_string())
}

#[cfg(any(feature = "windows", test))]
pub struct CrashReportContext {
    report_directory: PathBuf,
    main_thread_id: ThreadId,
    phase: AtomicU8,
}

#[cfg(any(feature = "windows", test))]
impl CrashReportContext {
    pub fn new(report_directory: PathBuf) -> Self {
        Self {
            report_directory,
            main_thread_id: std::thread::current().id(),
            phase: AtomicU8::new(StartupPhase::PreparingRuntime as u8),
        }
    }

    pub fn set_phase(&self, phase: StartupPhase) {
        self.phase.store(phase as u8, Ordering::Release);
    }

    fn phase(&self) -> StartupPhase {
        match self.phase.load(Ordering::Acquire) {
            value if value == StartupPhase::RunningUi as u8 => StartupPhase::RunningUi,
            value if value == StartupPhase::Closing as u8 => StartupPhase::Closing,
            _ => StartupPhase::PreparingRuntime,
        }
    }

    fn report(
        &self,
        kind: CrashReportKind,
        payload: Option<&str>,
        source: Option<ReportSource<'_>>,
        backtrace: Option<&str>,
    ) -> Result<PathBuf, String> {
        write_report(
            &self.report_directory,
            &ReportInput {
                kind,
                timestamp: SystemTime::now(),
                process_id: std::process::id(),
                app_version: env!("CARGO_PKG_VERSION"),
                os: std::env::consts::OS,
                arch: std::env::consts::ARCH,
                phase: self.phase(),
                payload,
                source,
                backtrace,
            },
        )
    }
}

#[cfg(any(feature = "windows", test))]
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
#[cfg(any(feature = "windows", test))]
static PANIC_WRITER_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(any(feature = "windows", test))]
thread_local! {
    static THIS_THREAD_IN_PANIC_HOOK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(feature = "windows", test))]
struct ThreadHookGuard;

#[cfg(any(feature = "windows", test))]
impl ThreadHookGuard {
    fn enter() -> Option<Self> {
        THIS_THREAD_IN_PANIC_HOOK.with(|active| {
            if active.replace(true) {
                None
            } else {
                Some(Self)
            }
        })
    }
}

#[cfg(any(feature = "windows", test))]
impl Drop for ThreadHookGuard {
    fn drop(&mut self) {
        THIS_THREAD_IN_PANIC_HOOK.with(|active| active.set(false));
    }
}

#[cfg(any(feature = "windows", test))]
struct PanicWriterGuard;

#[cfg(any(feature = "windows", test))]
impl PanicWriterGuard {
    fn enter() -> Option<Self> {
        PANIC_WRITER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}

#[cfg(any(feature = "windows", test))]
impl Drop for PanicWriterGuard {
    fn drop(&mut self) {
        PANIC_WRITER_ACTIVE.store(false, Ordering::Release);
    }
}

#[cfg(any(feature = "windows", test))]
pub fn install_panic_hook(context: Arc<CrashReportContext>) -> Result<(), &'static str> {
    if PANIC_HOOK_INSTALLED.swap(true, Ordering::AcqRel) {
        return Err("crash report panic hook is already installed");
    }

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        let Some(_thread_guard) = ThreadHookGuard::enter() else {
            return;
        };

        if std::thread::current().id() == context.main_thread_id {
            if let Some(_writer_guard) = PanicWriterGuard::enter() {
                let payload = panic_payload_text(information.payload());
                let location = information.location().map(|location| ReportSource {
                    file: location.file(),
                    line: location.line(),
                    column: location.column(),
                });
                let backtrace = std::backtrace::Backtrace::capture();
                let backtrace_text = (backtrace.status()
                    == std::backtrace::BacktraceStatus::Captured)
                    .then(|| display_bounded(&backtrace, MAX_REPORT_BYTES / 2));
                let _ = context.report(
                    CrashReportKind::MainThreadPanic,
                    payload,
                    location,
                    backtrace_text.as_deref(),
                );
            }
        }

        previous(information);
    }));
    Ok(())
}

#[cfg(any(feature = "windows", test))]
fn panic_payload_text(payload: &(dyn std::any::Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}

#[cfg(any(feature = "windows", test))]
fn display_bounded(value: &dyn std::fmt::Display, limit: usize) -> String {
    let mut output = BoundedText::new(limit);
    let _ = write!(output, "{value}");
    String::from_utf8(output.into_bytes()).unwrap_or_else(|_| "<unavailable>".to_string())
}

#[cfg(any(feature = "windows", test))]
pub fn handle_native_loop_outcome<T, E>(
    context: &CrashReportContext,
    outcome: Result<T, E>,
) -> Result<T, E>
where
    E: std::fmt::Display,
{
    if let Err(error) = &outcome {
        let detail = display_bounded(error, MAX_PAYLOAD_BYTES);
        let _ = context.report(CrashReportKind::NativeLoopError, Some(&detail), None, None);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::{Command, Stdio};
    use std::time::{Duration, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "cpu-affinity-tool-{label}-{}-{nanos}",
                process::id()
            ));
            fs::create_dir(&path).expect("create temp directory");
            Self(path)
        }

        fn reports(&self) -> PathBuf {
            self.0.join(REPORT_DIRECTORY_NAME)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_input<'a>(payload: Option<&'a str>, backtrace: Option<&'a str>) -> ReportInput<'a> {
        ReportInput {
            kind: CrashReportKind::MainThreadPanic,
            timestamp: UNIX_EPOCH + Duration::from_millis(1_753_531_845_678),
            process_id: 42,
            app_version: "1.5.0",
            os: "windows",
            arch: "x86_64",
            phase: StartupPhase::RunningUi,
            payload,
            source: Some(ReportSource {
                file: "src/main_windows.rs",
                line: 12,
                column: 3,
            }),
            backtrace,
        }
    }

    #[test]
    fn report_contract_has_typed_kind_version_and_completion_marker() {
        let report = format_report(&sample_input(Some("boom"), Some("frame one")));
        let text = std::str::from_utf8(&report).expect("report must be UTF-8");

        assert!(text.starts_with(REPORT_MAGIC));
        assert!(text.contains("\nformat_version: 1\n"));
        assert!(text.contains("\nevent_kind: main_thread_panic\n"));
        assert!(text.contains("\nstartup_phase: running_ui\n"));
        assert!(text.ends_with(REPORT_COMPLETION_MARKER));
        assert!(report.len() <= MAX_REPORT_BYTES);
    }

    #[test]
    fn filename_is_utc_and_collision_suffix_is_bounded() {
        let timestamp = UNIX_EPOCH + Duration::from_millis(1_753_531_845_678);
        assert_eq!(
            report_file_name(timestamp, 42, 0),
            Some("crash-20250726T121045.678Z-p42-00.txt".to_string())
        );
        assert_eq!(
            report_file_name(UNIX_EPOCH - Duration::from_secs(1), 42, 1),
            Some("crash-19700101T000000.000Z-p42-01.txt".to_string())
        );
        assert!(report_file_name(timestamp, 42, MAX_NAME_ATTEMPTS).is_none());
    }

    #[test]
    fn payload_is_normalized_and_truncated_on_utf8_boundary() {
        let payload = format!("first\r\nsecond\rthird\u{0}{}", "🦀".repeat(4_000));
        let report = format_report(&sample_input(Some(&payload), None));
        let text = std::str::from_utf8(&report).expect("report must remain UTF-8");

        assert!(text.contains("first\nsecond\nthird�"));
        assert!(!text.contains('\r'));
        assert!(text.contains("[payload truncated]"));
        assert!(text.ends_with(REPORT_COMPLETION_MARKER));
        assert!(report.len() <= MAX_REPORT_BYTES);
    }

    #[test]
    fn native_loop_error_uses_same_privacy_and_size_bounds() {
        let detail = "x".repeat(MAX_REPORT_BYTES * 2);
        let mut input = sample_input(Some(&detail), None);
        input.kind = CrashReportKind::NativeLoopError;
        input.source = None;

        let report = format_report(&input);
        let text = std::str::from_utf8(&report).expect("report must remain UTF-8");
        assert!(text.contains("\nevent_kind: native_loop_error\n"));
        assert!(text.contains("[payload truncated]"));
        assert!(report.len() <= MAX_REPORT_BYTES);
        assert!(text.ends_with(REPORT_COMPLETION_MARKER));
    }

    #[test]
    fn report_size_boundary_keeps_utf8_and_completion_footer() {
        let backtrace = "frame 🦀\n".repeat(MAX_REPORT_BYTES);
        let report = format_report(&sample_input(Some("boom"), Some(&backtrace)));

        assert_eq!(report.len(), MAX_REPORT_BYTES);
        let text = std::str::from_utf8(&report).expect("bounded report must remain UTF-8");
        assert!(text.contains("[backtrace truncated]"));
        assert!(text.ends_with(REPORT_COMPLETION_MARKER));
    }

    #[test]
    fn parser_rejects_partial_or_unknown_reports() {
        let complete = format_report(&sample_input(Some("boom"), None));
        let metadata = parse_report(&complete).expect("complete report must parse");
        assert_eq!(metadata.kind, CrashReportKind::MainThreadPanic);
        assert_eq!(metadata.reason, "boom");

        let partial = &complete[..complete.len() - REPORT_COMPLETION_MARKER.len()];
        assert!(parse_report(partial).is_err());

        let mut unknown = complete.clone();
        let version = b"format_version: 1";
        let position = unknown
            .windows(version.len())
            .position(|window| window == version)
            .expect("version field");
        unknown[position + version.len() - 1] = b'9';
        assert!(parse_report(&unknown).is_err());
    }

    #[test]
    fn parser_bounds_untrusted_application_version_metadata() {
        let oversized_version = format!("{}{}", "v".repeat(512), "\u{0007}");
        let report = format!(
            "{REPORT_MAGIC}format_version: 1\n\
             event_kind: main_thread_panic\n\
             timestamp_utc: 20250726T121045.678Z\n\
             app_version: {oversized_version}\n\
             payload:\nboom\n\
             {REPORT_COMPLETION_MARKER}"
        );

        let parsed = parse_report(report.as_bytes()).expect("parse untrusted metadata");
        assert!(parsed.app_version.len() <= 128);
        assert!(!parsed.app_version.chars().any(char::is_control));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn rename_target_path_is_nul_terminated_but_length_excludes_terminator() {
        let (wide_path, name_length) = file_rename_path(Path::new(r"C:\\reports\\crash.txt"));

        assert_eq!(wide_path.last(), Some(&0));
        assert_eq!(
            name_length as usize,
            (wide_path.len() - 1) * size_of::<u16>()
        );
    }

    #[test]
    fn writer_uses_create_new_publish_and_scan_ignores_unrelated_files() {
        let temp = TempDir::new("write-scan");
        let input = sample_input(Some("boom"), None);
        let saved = write_report(&temp.reports(), &input).expect("write report");

        assert_eq!(saved.parent(), Some(temp.reports().as_path()));
        assert!(saved.is_file());
        fs::write(temp.reports().join("notes.txt"), b"not managed").expect("write unrelated file");

        let snapshot = scan_reports(&temp.reports()).expect("scan reports");
        assert_eq!(snapshot.reports.len(), 1);
        assert_eq!(snapshot.reports[0].reason, "boom");
        assert_eq!(snapshot.reports[0].kind, CrashReportKind::MainThreadPanic);
        assert!(!fs::read_dir(temp.reports())
            .expect("read report directory")
            .any(|entry| entry
                .expect("directory entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".partial")));
    }

    #[test]
    fn unsafe_or_incomplete_candidates_fail_closed() {
        let temp = TempDir::new("incomplete");
        fs::create_dir(temp.reports()).expect("create report directory");
        fs::write(
            temp.reports().join("crash-20250726T121045.678Z-p42-00.txt"),
            REPORT_MAGIC,
        )
        .expect("write partial report");

        assert!(matches!(
            scan_reports(&temp.reports()),
            Err(ScanError::InvalidReport { .. })
        ));

        let root_is_file = TempDir::new("root-file");
        fs::write(root_is_file.reports(), b"not a directory").expect("write root file");
        assert!(matches!(
            scan_reports(&root_is_file.reports()),
            Err(ScanError::UnsafeRoot(_))
        ));

        let mismatch = TempDir::new("timestamp-mismatch");
        fs::create_dir(mismatch.reports()).expect("create mismatch report directory");
        fs::write(
            mismatch
                .reports()
                .join("crash-20250726T121046.678Z-p42-00.txt"),
            format_report(&sample_input(Some("boom"), None)),
        )
        .expect("write mismatched report");
        assert!(matches!(
            scan_reports(&mismatch.reports()),
            Err(ScanError::InvalidReport { .. })
        ));
    }

    #[test]
    fn retention_keeps_newest_twenty_from_immutable_snapshot() {
        let temp = TempDir::new("retention");
        for index in 0..(RETAIN_REPORTS + 2) {
            let mut input = sample_input(Some("boom"), None);
            input.timestamp = UNIX_EPOCH + Duration::from_secs(index as u64);
            write_report(&temp.reports(), &input).expect("write report");
        }

        let snapshot = scan_reports(&temp.reports()).expect("scan reports");
        assert_eq!(snapshot.reports.len(), RETAIN_REPORTS + 2);
        let deleted = apply_retention(&snapshot).expect("apply retention");
        assert_eq!(deleted, 2);

        let after = scan_reports(&temp.reports()).expect("rescan reports");
        assert_eq!(after.reports.len(), RETAIN_REPORTS);
        assert_eq!(
            after.reports.first().expect("newest report").timestamp_utc,
            "19700101T000021.000Z"
        );
        assert_eq!(
            after.reports.last().expect("oldest retained").timestamp_utc,
            "19700101T000002.000Z"
        );
    }

    #[test]
    fn writer_caps_reports_created_before_the_ui_can_apply_retention() {
        let temp = TempDir::new("writer-cap");
        for index in 0..MAX_UNPRUNED_REPORTS {
            let mut input = sample_input(Some("startup boom"), None);
            input.timestamp = UNIX_EPOCH + Duration::from_secs(index as u64);
            write_report(&temp.reports(), &input).expect("write startup report");
        }

        let mut rejected = sample_input(Some("must be rejected"), None);
        rejected.timestamp = UNIX_EPOCH + Duration::from_secs(MAX_UNPRUNED_REPORTS as u64);
        assert!(write_report(&temp.reports(), &rejected).is_err());

        let snapshot = scan_reports(&temp.reports()).expect("scan bounded reports");
        assert_eq!(snapshot.reports.len(), MAX_UNPRUNED_REPORTS);
        let deleted = apply_retention(&snapshot).expect("apply normal retention");
        assert_eq!(deleted, MAX_UNPRUNED_REPORTS - RETAIN_REPORTS);
        assert_eq!(
            scan_reports(&temp.reports())
                .expect("scan retained reports")
                .reports
                .len(),
            RETAIN_REPORTS
        );
    }

    #[test]
    fn deletion_revalidates_file_identity_and_never_recurses() {
        let temp = TempDir::new("delete");
        let report_directory = temp.reports();
        fs::create_dir(&report_directory).expect("create report directory");
        let saved = report_directory.join("listed-report.txt");
        fs::write(&saved, b"listed report").expect("write listed report");
        let listed_file = open_report_file_for_read(&saved).expect("open listed report");
        let listed_identity = file_identity(&listed_file).expect("read listed report identity");
        drop(listed_file);
        let root = TrustedRoot::open_existing(&report_directory)
            .expect("open report root")
            .expect("report root exists");
        let snapshot = ReportSnapshot {
            report_directory,
            root_identity: Some(root.identity),
            reports: vec![CrashReportEntry {
                file_name: OsString::from("listed-report.txt"),
                identity: listed_identity,
                kind: CrashReportKind::MainThreadPanic,
                timestamp_utc: "20250726T121045.678Z".to_string(),
                app_version: "1.5.0".to_string(),
                reason: "listed report".to_string(),
                size_bytes: 13,
            }],
        };
        let report = snapshot.reports[0].clone();

        let replacement = temp.reports().join("replacement.txt");
        fs::write(&replacement, b"replacement report").expect("write replacement");
        // Create the replacement before unlinking the listed entry: otherwise a
        // filesystem is allowed to recycle the just-freed file identity and turn
        // this regression test into a false negative.
        fs::rename(&replacement, &saved).expect("replace original atomically");

        assert!(matches!(
            delete_report(&snapshot, &report),
            Err(DeleteError::IdentityChanged)
        ));
        assert!(saved.is_file());

        let nested = temp.reports().join("nested");
        fs::create_dir(&nested).expect("create nested directory");
        fs::write(nested.join("keep.txt"), b"keep").expect("write nested file");
        assert!(nested.join("keep.txt").is_file());
    }

    #[test]
    fn native_loop_outcome_is_reported_before_error_is_returned() {
        let temp = TempDir::new("native-loop");
        let context = CrashReportContext::new(temp.reports());
        context.set_phase(StartupPhase::RunningUi);

        let result: Result<(), &str> =
            handle_native_loop_outcome(&context, Err("runner\rfailed\u{0}"));
        assert_eq!(result, Err("runner\rfailed\u{0}"));

        let snapshot = scan_reports(&temp.reports()).expect("scan native loop report");
        assert_eq!(snapshot.reports.len(), 1);
        assert_eq!(snapshot.reports[0].kind, CrashReportKind::NativeLoopError);
        assert_eq!(snapshot.reports[0].reason, "runner");
    }

    #[test]
    fn panic_hook_subprocess_contract() {
        const CHILD_MODE: &str = "CPU_AFFINITY_TOOL_CRASH_TEST_CHILD";
        const CHILD_DIR: &str = "CPU_AFFINITY_TOOL_CRASH_TEST_DIR";
        const PREVIOUS_HOOK_MARKER: &str = "previous-hook-called";
        const TEST_NAME: &str =
            "app::features::diagnostics::crash_reports::tests::panic_hook_subprocess_contract";

        if let Ok(mode) = std::env::var(CHILD_MODE) {
            let report_directory =
                PathBuf::from(std::env::var_os(CHILD_DIR).expect("child report directory"));
            fs::create_dir_all(&report_directory).expect("create child report directory");
            let previous_hook_marker = report_directory.join(PREVIOUS_HOOK_MARKER);
            std::panic::set_hook(Box::new(move |_| {
                let _ = fs::write(&previous_hook_marker, b"called");
            }));
            let context = std::sync::Arc::new(CrashReportContext::new(report_directory.clone()));
            install_panic_hook(context).expect("install child panic hook");

            match mode.as_str() {
                "normal" => return,
                "background" => {
                    let _ = std::thread::spawn(|| panic!("background boom")).join();
                    assert!(scan_reports(&report_directory)
                        .expect("scan after background panic")
                        .reports
                        .is_empty());
                    return;
                }
                "main" => panic!("main boom"),
                _ => panic!("unknown child mode"),
            }
        }

        let executable = std::env::current_exe().expect("current test executable");
        for (mode, should_succeed, expected_reports) in [
            ("normal", true, 0usize),
            ("background", true, 0),
            ("main", false, 1),
        ] {
            let temp = TempDir::new(&format!("hook-{mode}"));
            let report_directory = temp.reports();
            let status = Command::new(&executable)
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(CHILD_MODE, mode)
                .env(CHILD_DIR, &report_directory)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("run panic hook child");
            assert_eq!(status.success(), should_succeed, "child mode {mode}");

            let snapshot = scan_reports(&report_directory).expect("scan child reports");
            assert_eq!(
                snapshot.reports.len(),
                expected_reports,
                "child mode {mode}"
            );
            if mode == "main" {
                assert_eq!(snapshot.reports[0].kind, CrashReportKind::MainThreadPanic);
                assert_eq!(snapshot.reports[0].reason, "main boom");
            }
            assert_eq!(
                report_directory.join(PREVIOUS_HOOK_MARKER).is_file(),
                mode != "normal",
                "previous hook marker for child mode {mode}"
            );
        }
    }

    #[test]
    fn indicator_distinguishes_loading_ready_and_incomplete_counts() {
        let report = CrashReportEntry {
            file_name: OsString::from("crash-20250726T121045.678Z-p42-00.txt"),
            identity: FileIdentity {
                volume: 1,
                index: 2,
            },
            kind: CrashReportKind::MainThreadPanic,
            timestamp_utc: "20250726T121045.678Z".to_string(),
            app_version: "1.5.0".to_string(),
            reason: "boom".to_string(),
            size_bytes: 100,
        };
        let snapshot = ReportSnapshot {
            report_directory: PathBuf::from("reports"),
            root_identity: Some(FileIdentity {
                volume: 1,
                index: 1,
            }),
            reports: vec![report],
        };

        assert_eq!(
            CrashReportIndexState::Loading {
                last_complete: None
            }
            .indicator()
            .label,
            "Crash reports: loading"
        );
        assert_eq!(
            CrashReportIndexState::Ready {
                snapshot: snapshot.clone()
            }
            .indicator()
            .label,
            "Saved crash reports: 1"
        );
        assert_eq!(
            CrashReportIndexState::Incomplete {
                last_complete: Some(snapshot),
                reason: "read failed".to_string()
            }
            .indicator()
            .label,
            "Saved crash reports: 1 (stale); some files could not be read"
        );
        assert_eq!(
            CrashReportIndexState::Incomplete {
                last_complete: None,
                reason: "read failed".to_string()
            }
            .indicator()
            .label,
            "Crash report count unavailable; some files could not be read"
        );
    }

    #[test]
    fn manager_coalesces_many_refreshes_into_one_follow_up() {
        let temp = TempDir::new("manager-refresh");
        let mut manager = CrashReportManager::new(temp.reports());
        for _ in 0..1_000 {
            manager.request_refresh();
        }
        assert!(manager.worker_is_active());
        assert!(manager.refresh_pending);

        let deadline = Instant::now() + Duration::from_secs(2);
        while manager.worker_is_active() && Instant::now() < deadline {
            manager.poll();
            std::thread::yield_now();
        }

        assert!(!manager.worker_is_active());
        assert!(!manager.refresh_pending);
        assert!(matches!(
            manager.state(),
            CrashReportIndexState::Ready { .. }
        ));
    }

    #[test]
    fn manager_uses_a_slow_poll_after_the_initial_scan_timeout() {
        let temp = TempDir::new("manager-poll-interval");
        let mut manager = CrashReportManager::new(temp.reports());

        assert_eq!(
            manager.worker_poll_interval(),
            Some(Duration::from_millis(100))
        );
        manager.worker_started = Some(Instant::now() - Duration::from_secs(3));
        assert_eq!(manager.worker_poll_interval(), Some(Duration::from_secs(2)));
        manager.worker = None;
        assert_eq!(manager.worker_poll_interval(), None);
    }

    #[test]
    fn inactive_manager_never_starts_or_deletes_reports() {
        let temp = TempDir::new("inactive-manager");
        let snapshot = ReportSnapshot {
            report_directory: temp.reports(),
            root_identity: None,
            reports: Vec::new(),
        };
        let mut manager = CrashReportManager::new_inactive(temp.reports());
        manager.state = CrashReportIndexState::Ready {
            snapshot: snapshot.clone(),
        };

        manager.request_refresh();
        assert!(!manager.worker_is_active());
        assert!(matches!(
            manager.delete_saved_reports_from(&snapshot),
            Err(DeleteError::Unavailable)
        ));
        assert_eq!(manager.state().snapshot(), Some(&snapshot));
    }

    #[test]
    fn manager_activity_message_uses_only_the_newest_validated_report() {
        let temp = TempDir::new("activity-message");
        let snapshot = ReportSnapshot {
            report_directory: temp.reports(),
            root_identity: None,
            reports: vec![
                CrashReportEntry {
                    file_name: OsString::from("crash-newer.txt"),
                    identity: FileIdentity {
                        volume: 1,
                        index: 2,
                    },
                    kind: CrashReportKind::MainThreadPanic,
                    timestamp_utc: "20250726T121046.678Z".to_string(),
                    app_version: "1.5.0".to_string(),
                    reason: "newer panic".to_string(),
                    size_bytes: 1,
                },
                CrashReportEntry {
                    file_name: OsString::from("crash-older.txt"),
                    identity: FileIdentity {
                        volume: 1,
                        index: 1,
                    },
                    kind: CrashReportKind::MainThreadPanic,
                    timestamp_utc: "20250726T121045.678Z".to_string(),
                    app_version: "1.5.0".to_string(),
                    reason: "older panic".to_string(),
                    size_bytes: 1,
                },
            ],
        };
        let mut manager = CrashReportManager::new_idle(temp.reports());
        manager.state = CrashReportIndexState::Ready { snapshot };

        let message = manager
            .latest_activity_message()
            .expect("latest report must produce an activity message");
        assert!(message.contains("newer panic"));
        assert!(!message.contains("older panic"));
        assert!(message.contains("Previous crash report"));
    }

    #[test]
    fn frozen_bulk_delete_never_removes_a_report_created_after_confirmation() {
        let temp = TempDir::new("frozen-delete");
        let mut first = sample_input(Some("first"), None);
        first.timestamp = UNIX_EPOCH + Duration::from_secs(1);
        write_report(&temp.reports(), &first).expect("write first report");
        let confirmed = scan_reports(&temp.reports()).expect("capture confirmation snapshot");

        let mut later = sample_input(Some("later"), None);
        later.timestamp = UNIX_EPOCH + Duration::from_secs(2);
        write_report(&temp.reports(), &later).expect("write later report");

        let mut manager = CrashReportManager::new_idle(temp.reports());
        manager.state = CrashReportIndexState::Loading {
            last_complete: Some(scan_reports(&temp.reports()).expect("scan latest reports")),
        };
        assert_eq!(
            manager
                .delete_saved_reports_from(&confirmed)
                .expect("delete frozen snapshot"),
            1
        );

        let remaining = scan_reports(&temp.reports()).expect("scan remaining reports");
        assert_eq!(remaining.reports.len(), 1);
        assert_eq!(remaining.reports[0].reason, "later");
    }
}
