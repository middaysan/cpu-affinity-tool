use super::{
    operations::{ApiError, ApiResult},
    services::decode,
};
use crate::app::{
    features::diagnostics::crash_reports::{self, CrashReportEntry, ReportSnapshot},
    runtime::AppState,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub(super) enum GuiReply {
    Json(Value),
    Report {
        snapshot: ReportSnapshot,
        report: CrashReportEntry,
        after: u64,
        limit: usize,
    },
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Args {
    report_id: Option<String>,
    offset: usize,
    limit: Option<usize>,
    after: Option<String>,
}
pub(super) fn snapshot(app: &AppState, name: &str, args: Value) -> Result<GuiReply, ApiError> {
    let a: Args = decode(args)?;
    let snapshot = app.crash_reports.state().snapshot().ok_or_else(|| {
        ApiError::new(
            "not_ready",
            "Crash report discovery has no validated snapshot yet",
        )
    })?;
    if name == "list_crash_reports" {
        let limit = a.limit.unwrap_or(20);
        if limit == 0 || limit > 256 || a.offset > 100000 {
            return Err(ApiError::invalid("Invalid report page"));
        }
        let rows:Vec<_>=snapshot.reports.iter().skip(a.offset).take(limit).map(|r|json!({"report_id":r.opaque_id(),"event":r.kind.user_title(),"utc":r.timestamp_utc,"app_version":r.app_version,"reason":r.reason,"bytes":r.size_bytes})).collect();
        return Ok(GuiReply::Json(
            json!({"reports":rows,"total":snapshot.reports.len(),"discovery":"Existing cached validated report index; a removed or changed file is refused at read time","untrusted_data":true}),
        ));
    }
    let id = a
        .report_id
        .ok_or_else(|| ApiError::invalid("report_id is required"))?;
    let report = snapshot
        .reports
        .iter()
        .find(|r| r.opaque_id() == id)
        .ok_or_else(|| {
            ApiError::new(
                "not_found",
                "Report is no longer in the validated report list",
            )
        })?
        .clone();
    let after = a
        .after
        .as_deref()
        .unwrap_or("0")
        .parse()
        .map_err(|_| ApiError::invalid("Invalid byte cursor"))?;
    let limit = a.limit.unwrap_or(4096);
    if limit == 0 || limit > 16384 || after > report.size_bytes {
        return Err(ApiError::invalid("Invalid report byte page"));
    }
    Ok(GuiReply::Report {
        snapshot: snapshot.clone(),
        report,
        after,
        limit,
    })
}
impl GuiReply {
    pub fn resolve(self) -> ApiResult {
        match self {
            Self::Json(value) => Ok(value),
            Self::Report {
                snapshot,
                report,
                after,
                limit,
            } => {
                let bytes = crash_reports::read_report_page(&snapshot, &report, after, limit)
                    .map_err(|e| ApiError::new("report_changed", e.to_string()))?;
                Ok(
                    json!({"report_id":report.opaque_id(),"start":after.to_string(),"next":(after+bytes.len()as u64).to_string(),"total_bytes":report.size_bytes,"eof":after+bytes.len()as u64==report.size_bytes,"text":String::from_utf8_lossy(&bytes),"encoding":"UTF-8 lossy; cursors count original bytes","untrusted_data":true}),
                )
            }
        }
    }
}
