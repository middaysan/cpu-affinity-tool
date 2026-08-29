use std::ffi::OsStr;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, ERROR_TIMEOUT, FILETIME, SYSTEMTIME,
};
use windows::Win32::Globalization::CompareStringOrdinal;
use windows::Win32::System::EventLog::{
    EVT_HANDLE, EVT_VARIANT, EVT_VARIANT_TYPE_ARRAY, EVT_VARIANT_TYPE_MASK, EvtCreateRenderContext,
    EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender,
    EvtRenderContextSystem, EvtRenderContextUser, EvtRenderContextValues, EvtRenderEventValues,
    EvtSystemEventID, EvtSystemEventRecordId, EvtSystemPropertyIdEND, EvtSystemProviderName,
    EvtSystemTimeCreated, EvtSystemVersion, EvtVarTypeByte, EvtVarTypeFileTime, EvtVarTypeString,
    EvtVarTypeSysTime, EvtVarTypeUInt16, EvtVarTypeUInt64,
};
use windows::Win32::System::Time::FileTimeToSystemTime;
use windows::core::{HRESULT, PCWSTR};

use super::OS;
use super::common::to_wide_z_str;

const APPLICATION_ERROR_PROVIDER: &str = "Application Error";
const APPLICATION_ERROR_EVENT_ID: u16 = 1000;
const APPLICATION_ERROR_V0_DATA_COUNT: usize = 15;
const MAX_EVENT_TEXT_CHARS: usize = 256;
const MAX_RENDER_BUFFER_BYTES: usize = 64 * 1024;
const MAX_SCANNED_EVENTS: usize = 64;
const EVENT_BATCH_SIZE: usize = 8;
const EVENT_QUERY: &str = "*[System[Provider[@Name='Application Error'] and EventID=1000 and TimeCreated[timediff(@SystemTime) <= 604800000]]]";
const RENDER_PATHS: [&str; 5] = [
    "Event/EventData/Data[@Name='AppName']",
    "Event/EventData/Data[@Name='AppPath']",
    "Event/EventData/Data[@Name='ModuleName']",
    "Event/EventData/Data[@Name='ModulePath']",
    "Event/EventData/Data[@Name='ExceptionCode']",
];
const SYSTEM_PROPERTY_COUNT: usize = EvtSystemPropertyIdEND.0 as usize;

struct EventLogHandle(EVT_HANDLE);

impl Drop for EventLogHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::EventLog::EvtClose(self.0);
        }
    }
}

struct RenderContexts {
    system: EventLogHandle,
    named: EventLogHandle,
    user: Option<EventLogHandle>,
}

impl RenderContexts {
    fn user_context(&mut self) -> Result<EVT_HANDLE, String> {
        if self.user.is_none() {
            self.user = Some(create_user_render_context()?);
        }
        Ok(self.user.as_ref().expect("context was initialized").0)
    }
}

struct RenderedValues {
    buffer: Vec<EVT_VARIANT>,
    property_count: usize,
    buffer_used: usize,
}

impl RenderedValues {
    fn values(&self) -> &[EVT_VARIANT] {
        &self.buffer[..self.property_count]
    }

    fn utf16_z(&self, pointer: *const u16) -> Option<String> {
        let buffer_start = self.buffer.as_ptr().cast::<u8>().addr();
        let buffer_end = buffer_start.checked_add(self.buffer_used)?;
        let pointer_address = pointer.addr();
        if pointer_address < buffer_start
            || pointer_address >= buffer_end
            || !pointer_address.is_multiple_of(std::mem::align_of::<u16>())
        {
            return None;
        }

        let available_units = (buffer_end.checked_sub(pointer_address)?) / size_of::<u16>();
        let units_to_scan = available_units.min(MAX_EVENT_TEXT_CHARS + 1);
        if units_to_scan == 0 {
            return None;
        }

        let units = unsafe { std::slice::from_raw_parts(pointer, units_to_scan) };
        let length = units.iter().position(|unit| *unit == 0)?;
        String::from_utf16(&units[..length]).ok()
    }

    fn system_time(&self, pointer: *const SYSTEMTIME) -> Option<SYSTEMTIME> {
        let buffer_start = self.buffer.as_ptr().cast::<u8>().addr();
        let buffer_end = buffer_start.checked_add(self.buffer_used)?;
        let pointer_address = pointer.addr();
        let system_time_end = pointer_address.checked_add(size_of::<SYSTEMTIME>())?;
        if pointer_address < buffer_start
            || system_time_end > buffer_end
            || !pointer_address.is_multiple_of(std::mem::align_of::<SYSTEMTIME>())
        {
            return None;
        }

        Some(unsafe { *pointer })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsApplicationFailure {
    pub event_record_id: u64,
    pub timestamp_utc: String,
    pub exception_code: u32,
    pub faulting_module: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventSystemValues {
    provider: String,
    event_id: u16,
    version: u8,
    event_record_id: u64,
    timestamp_utc: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NamedEventData {
    app_name: Option<String>,
    app_path: Option<String>,
    faulting_module_name: Option<String>,
    faulting_module_path: Option<String>,
    exception_code: Option<String>,
}

fn parse_exception_code(value: &str) -> Option<u32> {
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }

    u32::from_str_radix(value, 16).ok()
}

fn application_error_from_values(
    system: EventSystemValues,
    named: NamedEventData,
    positional: &[Option<String>],
    executable_path: &Path,
) -> Option<WindowsApplicationFailure> {
    if system.provider != APPLICATION_ERROR_PROVIDER
        || system.event_id != APPLICATION_ERROR_EVENT_ID
        || system.timestamp_utc.is_empty()
    {
        return None;
    }

    let (app_name, app_path, faulting_module, faulting_module_path, exception_code) = match named {
        NamedEventData {
            app_name: Some(app_name),
            app_path: Some(app_path),
            faulting_module_name: Some(faulting_module_name),
            faulting_module_path,
            exception_code: Some(exception_code),
        } => (
            app_name,
            app_path,
            faulting_module_name,
            faulting_module_path,
            exception_code,
        ),
        _ if system.version == 0
            && positional.len() == APPLICATION_ERROR_V0_DATA_COUNT
            && positional.iter().all(Option::is_some) =>
        {
            (
                positional[0].clone()?,
                positional[10].clone()?,
                positional[3].clone()?,
                positional[11].clone(),
                positional[6].clone()?,
            )
        }
        _ => return None,
    };

    let expected_name = executable_path.file_name()?.to_str()?;
    if !windows_case_insensitive_eq(&app_name, expected_name)
        || !matches_executable_path(&app_path, executable_path)
    {
        return None;
    }

    let exception_code = parse_exception_code(&exception_code)?;
    let faulting_module = sanitized_basename(&faulting_module)
        .or_else(|| faulting_module_path.as_deref().and_then(sanitized_basename))?;

    Some(WindowsApplicationFailure {
        event_record_id: system.event_record_id,
        timestamp_utc: system.timestamp_utc,
        exception_code,
        faulting_module,
    })
}

fn windows_case_insensitive_eq(left: &str, right: &str) -> bool {
    let left: Vec<u16> = OsStr::new(left).encode_wide().collect();
    let right: Vec<u16> = OsStr::new(right).encode_wide().collect();
    unsafe { CompareStringOrdinal(&left, &right, true).0 == 2 }
}

fn matches_executable_path(event_path: &str, executable_path: &Path) -> bool {
    let Some(event_path) = normalize_windows_path(event_path) else {
        return false;
    };
    let Some(executable_path) = normalize_windows_path(&executable_path.to_string_lossy()) else {
        return false;
    };

    if let (Ok(event_canonical), Ok(executable_canonical)) = (
        std::fs::canonicalize(&event_path),
        std::fs::canonicalize(&executable_path),
    ) {
        let Some(event_canonical) = normalize_windows_path(&event_canonical.to_string_lossy())
        else {
            return false;
        };
        let Some(executable_canonical) =
            normalize_windows_path(&executable_canonical.to_string_lossy())
        else {
            return false;
        };
        return windows_case_insensitive_eq(&event_canonical, &executable_canonical);
    }

    windows_case_insensitive_eq(&event_path, &executable_path)
}

fn normalize_windows_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || !value.contains(':') || value.starts_with("\\\\.") {
        return None;
    }

    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    let value = value.replace('/', r"\");
    let path = PathBuf::from(&value);
    if !path.is_absolute() {
        return None;
    }

    Some(value.trim_end_matches('\\').to_string())
}

fn sanitized_basename(value: &str) -> Option<String> {
    let normalized = value.replace('/', r"\");
    let basename = Path::new(&normalized).file_name()?.to_str()?;
    let sanitized: String = basename
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    *character,
                    '\u{00ad}'
                        | '\u{061c}'
                        | '\u{180e}'
                        | '\u{200b}'..='\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2060}'..='\u{206f}'
                        | '\u{feff}'
                )
        })
        .take(MAX_EVENT_TEXT_CHARS)
        .collect();
    (!sanitized.is_empty()).then_some(sanitized)
}

impl OS {
    /// Looks up the newest local Application Error record that securely matches this executable.
    ///
    /// This is a bounded, read-only Event Log query. Callers must treat a returned record as
    /// supplemental diagnostic evidence rather than proof of a crash cause.
    pub fn find_latest_application_error(
        executable_path: &Path,
    ) -> Result<Option<WindowsApplicationFailure>, String> {
        let channel = to_wide_z_str("Application");
        let query = to_wide_z_str(EVENT_QUERY);
        let query = unsafe {
            EvtQuery(
                None,
                PCWSTR(channel.as_ptr()),
                PCWSTR(query.as_ptr()),
                EvtQueryChannelPath.0 | EvtQueryReverseDirection.0,
            )
            .map_err(|_| "could not query the local Windows Application log".to_string())?
        };
        let query = EventLogHandle(query);

        let mut contexts = create_render_contexts()?;
        let mut scanned = 0usize;
        let mut system_render_failures = 0usize;
        let mut matching_candidates = 0usize;
        let mut data_render_successes = 0usize;
        let mut data_render_failures = 0usize;

        while scanned < MAX_SCANNED_EVENTS {
            let mut events = [0isize; EVENT_BATCH_SIZE];
            let mut returned = 0u32;
            let next = unsafe { EvtNext(query.0, &mut events, 0, 0, &mut returned) };
            if let Err(error) = next {
                if is_query_complete_error(error.code()) {
                    break;
                }
                return Err("could not read the local Windows Application log".to_string());
            }

            let returned = returned as usize;
            if returned == 0 || returned > EVENT_BATCH_SIZE {
                return Err("Windows Event Log returned an invalid event batch".to_string());
            }
            let event_handles: Vec<EventLogHandle> = events[..returned]
                .iter()
                .copied()
                .map(|handle| EventLogHandle(EVT_HANDLE(handle)))
                .collect();

            for event in &event_handles {
                if scanned >= MAX_SCANNED_EVENTS {
                    break;
                }
                scanned += 1;
                let system = match render_event_values(contexts.system.0, event.0) {
                    Ok(values) => system_values_from_rendered(&values),
                    Err(_) => {
                        system_render_failures += 1;
                        continue;
                    }
                };
                let Some(system) = system else {
                    continue;
                };
                if system.provider != APPLICATION_ERROR_PROVIDER
                    || system.event_id != APPLICATION_ERROR_EVENT_ID
                {
                    continue;
                }
                matching_candidates += 1;

                let named = match render_event_values(contexts.named.0, event.0) {
                    Ok(values) => {
                        data_render_successes += 1;
                        values
                    }
                    Err(_) => {
                        data_render_failures += 1;
                        if system.version != 0 {
                            continue;
                        }
                        RenderedValues {
                            buffer: Vec::new(),
                            property_count: 0,
                            buffer_used: 0,
                        }
                    }
                };
                let named_values = named_event_data_from_rendered(&named).unwrap_or_default();
                if let Some(failure) = application_error_from_values(
                    system.clone(),
                    named_values,
                    &[],
                    executable_path,
                ) {
                    return Ok(Some(failure));
                }

                if system.version != 0 {
                    continue;
                }

                let user_context = match contexts.user_context() {
                    Ok(context) => context,
                    Err(_) => {
                        data_render_failures += 1;
                        continue;
                    }
                };
                let positional = match render_event_values(user_context, event.0) {
                    Ok(values) => {
                        data_render_successes += 1;
                        positional_event_data_from_rendered(&values)
                    }
                    Err(_) => {
                        data_render_failures += 1;
                        continue;
                    }
                };
                if let Some(failure) = application_error_from_values(
                    system,
                    NamedEventData::default(),
                    &positional,
                    executable_path,
                ) {
                    return Ok(Some(failure));
                }
            }
        }

        if scanned > 0 && system_render_failures == scanned {
            return Err("could not render any Windows Event Log records".to_string());
        }
        if matching_candidates > 0 && data_render_successes == 0 && data_render_failures > 0 {
            return Err("could not render any matching Windows Event Log records".to_string());
        }
        Ok(None)
    }
}

fn is_query_complete_error(code: HRESULT) -> bool {
    code == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0)
        || code == HRESULT::from_win32(ERROR_TIMEOUT.0)
}

fn create_render_contexts() -> Result<RenderContexts, String> {
    Ok(RenderContexts {
        system: create_system_render_context()?,
        named: create_named_render_context()?,
        user: None,
    })
}

fn create_system_render_context() -> Result<EventLogHandle, String> {
    let context = unsafe { EvtCreateRenderContext(None, EvtRenderContextSystem.0) }
        .map_err(|_| "could not prepare the Windows Event Log system reader".to_string())?;
    Ok(EventLogHandle(context))
}

fn create_named_render_context() -> Result<EventLogHandle, String> {
    let wide_paths: Vec<Vec<u16>> = RENDER_PATHS
        .iter()
        .map(|path| to_wide_z_str(path))
        .collect();
    let paths: Vec<PCWSTR> = wide_paths
        .iter()
        .map(|path| PCWSTR(path.as_ptr()))
        .collect();
    let context = unsafe { EvtCreateRenderContext(Some(&paths), EvtRenderContextValues.0) }
        .map_err(|_| "could not prepare the Windows Event Log named-value reader".to_string())?;
    Ok(EventLogHandle(context))
}

fn create_user_render_context() -> Result<EventLogHandle, String> {
    let context = unsafe { EvtCreateRenderContext(None, EvtRenderContextUser.0) }
        .map_err(|_| "could not prepare the Windows Event Log user-data reader".to_string())?;
    Ok(EventLogHandle(context))
}

fn render_event_values(context: EVT_HANDLE, event: EVT_HANDLE) -> Result<RenderedValues, String> {
    let mut buffer_used = 0u32;
    let mut property_count = 0u32;
    match unsafe {
        EvtRender(
            Some(context),
            event,
            EvtRenderEventValues.0,
            0,
            None,
            &mut buffer_used,
            &mut property_count,
        )
    } {
        Ok(()) if buffer_used == 0 && property_count == 0 => {
            return Ok(RenderedValues {
                buffer: Vec::new(),
                property_count: 0,
                buffer_used: 0,
            });
        }
        Ok(()) => return Err("Windows Event Log record has an invalid render probe".to_string()),
        Err(error) if error.code() != HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => {
            return Err("Windows Event Log record is not available".to_string());
        }
        Err(_) => {}
    }
    if buffer_used == 0 || buffer_used as usize > MAX_RENDER_BUFFER_BYTES {
        return Err("Windows Event Log record is not available".to_string());
    }

    let variant_size = size_of::<EVT_VARIANT>();
    let variants_needed = (buffer_used as usize).div_ceil(variant_size);
    let mut buffer = vec![EVT_VARIANT::default(); variants_needed];
    unsafe {
        EvtRender(
            Some(context),
            event,
            EvtRenderEventValues.0,
            (buffer.len() * variant_size) as u32,
            Some(buffer.as_mut_ptr().cast()),
            &mut buffer_used,
            &mut property_count,
        )
    }
    .map_err(|_| "Windows Event Log record is not available".to_string())?;

    let buffer_used = buffer_used as usize;
    let property_count = property_count as usize;
    if buffer_used > MAX_RENDER_BUFFER_BYTES
        || buffer_used > buffer.len() * variant_size
        || property_count > buffer_used / variant_size
    {
        return Err("Windows Event Log record has an unexpected shape".to_string());
    }
    Ok(RenderedValues {
        buffer,
        property_count,
        buffer_used,
    })
}

fn system_values_from_rendered(values: &RenderedValues) -> Option<EventSystemValues> {
    if values.values().len() != SYSTEM_PROPERTY_COUNT {
        return None;
    }

    Some(EventSystemValues {
        provider: variant_string(values, EvtSystemProviderName.0 as usize)?,
        event_id: variant_u16(values.values().get(EvtSystemEventID.0 as usize)?)?,
        version: variant_u8(values.values().get(EvtSystemVersion.0 as usize)?)?,
        event_record_id: variant_u64(values.values().get(EvtSystemEventRecordId.0 as usize)?)?,
        timestamp_utc: variant_timestamp_utc(values, EvtSystemTimeCreated.0 as usize)?,
    })
}

fn named_event_data_from_rendered(values: &RenderedValues) -> Option<NamedEventData> {
    if values.values().len() != RENDER_PATHS.len() {
        return None;
    }

    Some(NamedEventData {
        app_name: variant_string(values, 0),
        app_path: variant_string(values, 1),
        faulting_module_name: variant_string(values, 2),
        faulting_module_path: variant_string(values, 3),
        exception_code: variant_string(values, 4),
    })
}

fn positional_event_data_from_rendered(values: &RenderedValues) -> Vec<Option<String>> {
    values
        .values()
        .iter()
        .enumerate()
        .map(|(index, _)| variant_string(values, index))
        .collect()
}

fn variant_type(value: &EVT_VARIANT) -> u32 {
    value.Type & EVT_VARIANT_TYPE_MASK
}

fn is_scalar(value: &EVT_VARIANT, expected_type: i32) -> bool {
    value.Type & EVT_VARIANT_TYPE_ARRAY == 0 && variant_type(value) == expected_type as u32
}

fn variant_string(values: &RenderedValues, index: usize) -> Option<String> {
    let value = values.values().get(index)?;
    if !is_scalar(value, EvtVarTypeString.0) {
        return None;
    }
    let pointer = unsafe { value.Anonymous.StringVal.0 };
    if pointer.is_null() {
        return None;
    }
    // EvtRender guarantees string pointers refer to its output buffer. The caller keeps the
    // rendered buffer alive and bounds the read to the bytes Windows reported as initialized.
    values.utf16_z(pointer)
}

fn variant_u16(value: &EVT_VARIANT) -> Option<u16> {
    if !is_scalar(value, EvtVarTypeUInt16.0) {
        return None;
    }
    Some(unsafe { value.Anonymous.UInt16Val })
}

fn variant_u8(value: &EVT_VARIANT) -> Option<u8> {
    if !is_scalar(value, EvtVarTypeByte.0) {
        return None;
    }
    Some(unsafe { value.Anonymous.ByteVal })
}

fn variant_u64(value: &EVT_VARIANT) -> Option<u64> {
    if !is_scalar(value, EvtVarTypeUInt64.0) {
        return None;
    }
    Some(unsafe { value.Anonymous.UInt64Val })
}

fn variant_timestamp_utc(values: &RenderedValues, index: usize) -> Option<String> {
    let value = values.values().get(index)?;
    let system_time = if is_scalar(value, EvtVarTypeFileTime.0) {
        let raw = unsafe { value.Anonymous.FileTimeVal };
        let file_time = FILETIME {
            dwLowDateTime: raw as u32,
            dwHighDateTime: (raw >> 32) as u32,
        };
        let mut system_time = SYSTEMTIME::default();
        unsafe { FileTimeToSystemTime(&file_time, &mut system_time).ok()? };
        system_time
    } else if is_scalar(value, EvtVarTypeSysTime.0) {
        let pointer = unsafe { value.Anonymous.SysTimeVal };
        if pointer.is_null() {
            return None;
        }
        values.system_time(pointer)?
    } else {
        return None;
    };

    (system_time.wMonth >= 1 && system_time.wMonth <= 12 && system_time.wDay >= 1).then(|| {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            system_time.wYear,
            system_time.wMonth,
            system_time.wDay,
            system_time.wHour,
            system_time.wMinute,
            system_time.wSecond,
            system_time.wMilliseconds
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        EventSystemValues, NamedEventData, RENDER_PATHS, RenderedValues,
        application_error_from_values, create_render_contexts, is_query_complete_error, is_scalar,
        parse_exception_code, variant_string,
    };
    use std::mem::size_of;
    use std::path::Path;
    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_NO_MORE_ITEMS, ERROR_TIMEOUT};
    use windows::Win32::System::EventLog::{
        EVT_VARIANT, EVT_VARIANT_0, EVT_VARIANT_TYPE_ARRAY, EvtVarTypeUInt16,
    };
    use windows::core::{HRESULT, PCWSTR};

    #[test]
    fn query_completion_accepts_end_of_results_and_zero_timeout() {
        assert!(is_query_complete_error(HRESULT::from_win32(
            ERROR_NO_MORE_ITEMS.0
        )));
        assert!(is_query_complete_error(HRESULT::from_win32(
            ERROR_TIMEOUT.0
        )));
        assert!(!is_query_complete_error(HRESULT::from_win32(
            ERROR_ACCESS_DENIED.0
        )));
    }

    #[test]
    fn parses_the_application_error_access_violation_code() {
        assert_eq!(parse_exception_code("c0000005"), Some(0xc000_0005));
        assert_eq!(parse_exception_code("0xC0000005"), Some(0xc000_0005));
        assert_eq!(parse_exception_code("not-a-code"), None);
    }

    fn valid_system(version: u8) -> EventSystemValues {
        EventSystemValues {
            provider: "Application Error".to_string(),
            event_id: 1000,
            version,
            event_record_id: 42,
            timestamp_utc: "2026-08-29T12:00:00.000Z".to_string(),
        }
    }

    #[test]
    fn accepts_complete_named_values_for_the_current_executable() {
        let result = application_error_from_values(
            valid_system(1),
            NamedEventData {
                app_name: Some("cpu-affinity-tool.exe".to_string()),
                app_path: Some(r"C:\Tools\cpu-affinity-tool.exe".to_string()),
                faulting_module_name: Some("kernelbase.dll".to_string()),
                faulting_module_path: None,
                exception_code: Some("c0000005".to_string()),
            },
            &[],
            Path::new(r"C:\Tools\cpu-affinity-tool.exe"),
        )
        .unwrap();

        assert_eq!(result.event_record_id, 42);
        assert_eq!(result.exception_code, 0xc000_0005);
        assert_eq!(result.faulting_module, "kernelbase.dll");
    }

    #[test]
    fn accepts_only_the_known_version_zero_positional_layout() {
        let mut positional = vec![Some(String::new()); 15];
        positional[0] = Some("cpu-affinity-tool.exe".to_string());
        positional[3] = Some("ntdll.dll".to_string());
        positional[6] = Some("0xc0000005".to_string());
        positional[10] = Some(r"C:\Tools\cpu-affinity-tool.exe".to_string());
        positional[11] = Some(r"C:\Windows\System32\ntdll.dll".to_string());

        assert!(
            application_error_from_values(
                valid_system(0),
                NamedEventData::default(),
                &positional,
                Path::new(r"C:\Tools\cpu-affinity-tool.exe"),
            )
            .is_some()
        );
        assert!(
            application_error_from_values(
                valid_system(1),
                NamedEventData::default(),
                &positional,
                Path::new(r"C:\Tools\cpu-affinity-tool.exe"),
            )
            .is_none()
        );

        let mut malformed = positional[..15].to_vec();
        malformed[14] = None;
        assert!(
            application_error_from_values(
                valid_system(0),
                NamedEventData::default(),
                &malformed,
                Path::new(r"C:\Tools\cpu-affinity-tool.exe"),
            )
            .is_none()
        );

        positional.push(Some("unexpected extra data".to_string()));
        assert!(
            application_error_from_values(
                valid_system(0),
                NamedEventData::default(),
                &positional,
                Path::new(r"C:\Tools\cpu-affinity-tool.exe"),
            )
            .is_none()
        );
    }

    #[test]
    fn only_renders_named_event_1000_fields_from_the_event_root() {
        assert_eq!(RENDER_PATHS.len(), 5);
        assert!(
            RENDER_PATHS
                .iter()
                .all(|path| path.starts_with("Event/EventData/Data[@Name='"))
        );
        assert!(RENDER_PATHS.contains(&"Event/EventData/Data[@Name='AppPath']"));
        assert!(RENDER_PATHS.contains(&"Event/EventData/Data[@Name='ModuleName']"));
        assert!(RENDER_PATHS.contains(&"Event/EventData/Data[@Name='ModulePath']"));
    }

    #[test]
    fn scalar_type_check_ignores_count_but_rejects_arrays() {
        let scalar_with_zero_count = EVT_VARIANT {
            Anonymous: EVT_VARIANT_0 { UInt16Val: 1000 },
            Count: 0,
            Type: EvtVarTypeUInt16.0 as u32,
        };
        let array = EVT_VARIANT {
            Anonymous: EVT_VARIANT_0 { UInt16Val: 1000 },
            Count: 1,
            Type: EvtVarTypeUInt16.0 as u32 | EVT_VARIANT_TYPE_ARRAY,
        };

        assert!(is_scalar(&scalar_with_zero_count, EvtVarTypeUInt16.0));
        assert!(!is_scalar(&array, EvtVarTypeUInt16.0));
    }

    #[test]
    fn string_values_must_point_inside_the_rendered_buffer() {
        let mut buffer = vec![EVT_VARIANT::default(); 2];
        let text = ['A' as u16, 0];
        let text_offset = size_of::<EVT_VARIANT>();
        unsafe {
            std::ptr::copy_nonoverlapping(
                text.as_ptr().cast::<u8>(),
                buffer.as_mut_ptr().cast::<u8>().add(text_offset),
                size_of_val(&text),
            );
        }
        let valid_pointer = unsafe { buffer.as_ptr().cast::<u8>().add(text_offset).cast::<u16>() };
        buffer[0] = EVT_VARIANT {
            Anonymous: EVT_VARIANT_0 {
                StringVal: PCWSTR(valid_pointer),
            },
            Count: 0,
            Type: windows::Win32::System::EventLog::EvtVarTypeString.0 as u32,
        };
        let rendered = RenderedValues {
            buffer,
            property_count: 1,
            buffer_used: text_offset + size_of_val(&text),
        };

        assert_eq!(variant_string(&rendered, 0), Some("A".to_string()));

        let outside = EVT_VARIANT {
            Anonymous: EVT_VARIANT_0 {
                StringVal: PCWSTR(0x10usize as *const u16),
            },
            Count: 0,
            Type: windows::Win32::System::EventLog::EvtVarTypeString.0 as u32,
        };
        let rendered_with_bad_pointer = RenderedValues {
            buffer: vec![outside],
            property_count: 1,
            buffer_used: size_of::<EVT_VARIANT>(),
        };
        assert_eq!(variant_string(&rendered_with_bad_pointer, 0), None);
    }

    #[test]
    fn creates_the_three_supported_windows_event_log_render_contexts() {
        let mut contexts =
            create_render_contexts().expect("system and named Event Log contexts must be valid");
        let _user = contexts
            .user_context()
            .expect("the fallback user Event Log context must be valid");
    }

    #[test]
    fn rejects_a_matching_filename_at_a_different_path() {
        let result = application_error_from_values(
            valid_system(0),
            NamedEventData {
                app_name: Some("cpu-affinity-tool.exe".to_string()),
                app_path: Some(r"D:\Other\cpu-affinity-tool.exe".to_string()),
                faulting_module_name: Some("ntdll.dll".to_string()),
                faulting_module_path: None,
                exception_code: Some("c0000005".to_string()),
            },
            &[],
            Path::new(r"C:\Tools\cpu-affinity-tool.exe"),
        );

        assert!(result.is_none());
    }

    #[test]
    fn strips_control_and_bidirectional_characters_from_module_basename() {
        let result = application_error_from_values(
            valid_system(1),
            NamedEventData {
                app_name: Some("cpu-affinity-tool.exe".to_string()),
                app_path: Some(r"C:\Tools\cpu-affinity-tool.exe".to_string()),
                faulting_module_name: Some(
                    "C:\\Windows\\kernel\u{200b}\u{202e}base.dll\n".to_string(),
                ),
                faulting_module_path: None,
                exception_code: Some("c0000005".to_string()),
            },
            &[],
            Path::new(r"C:\Tools\cpu-affinity-tool.exe"),
        )
        .unwrap();

        assert_eq!(result.faulting_module, "kernelbase.dll");
    }
}
