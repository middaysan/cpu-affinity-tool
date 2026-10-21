use super::{
    operations::{ApiError, ApiResult},
    services::{bounded_row, decode},
};
use crate::app::{
    adapters::discovery,
    models::{AppToRun, LaunchTarget},
};
use os_api::{InstalledAppCatalogEntry, InstalledAppCatalogTarget};
use serde::Deserialize;
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Query {
    query: String,
    catalog_id: Option<String>,
    offset: usize,
    limit: Option<usize>,
}
fn catalog_id(entry: &InstalledAppCatalogEntry) -> String {
    let (kind, target) = match &entry.target {
        InstalledAppCatalogTarget::Aumid(id) => ("aumid", id.to_lowercase()),
        InstalledAppCatalogTarget::Path(path) => ("path", path.to_string_lossy().to_lowercase()),
    };
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (kind, target).hash(&mut hash);
    format!("{:016x}", hash.finish())
}
fn template(app: &AppToRun) -> ApiResult {
    let mut rule = json!({"name":app.name,"args":app.args,"tracked_process_names":app.additional_processes,"manage_descendants":app.manage_descendants,"autorun":false,"priority":app.priority});
    match &app.launch_target {
        LaunchTarget::Path { bin_path, .. } => rule["path"] = json!(bin_path),
        LaunchTarget::Installed { aumid } => rule["aumid"] = json!(aumid),
    };
    bounded_row(&rule)
}
pub(super) fn query(name: &str, args: Value) -> ApiResult {
    let q: Query = decode(args)?;
    if q.query.len() > 256 || q.offset > 100000 || q.limit.is_some_and(|n| n == 0 || n > 256) {
        return Err(ApiError::invalid("Invalid discovery page/query"));
    }
    let entries =
        discovery::list_supported_start_apps().map_err(|e| ApiError::new("discovery", e))?;
    if name == "resolve_installed_app" {
        let id = q
            .catalog_id
            .ok_or_else(|| ApiError::invalid("catalog_id is required"))?;
        let entry = entries
            .into_iter()
            .find(|entry| catalog_id(entry) == id)
            .ok_or_else(|| {
                ApiError::new(
                    "catalog_changed",
                    "App is no longer in the supported catalog; list it again",
                )
            })?;
        let app = discovery::app_from_installed_entry(entry)
            .map_err(|e| ApiError::new("discovery", e))?;
        return Ok(
            json!({"rule_template":template(&app)?,"note":"Resolved without launching. Pass the desired editable values to create_rule."}),
        );
    }
    let needle = q.query.to_lowercase();
    let matches: Vec<_> = entries
        .iter()
        .filter(|entry| needle.is_empty() || entry.name.to_lowercase().contains(&needle))
        .collect();
    let rows:Vec<_>=matches.iter().skip(q.offset).take(q.limit.unwrap_or(50)).map(|entry|bounded_row(&json!({"catalog_id":catalog_id(entry),"name":entry.name,"source":entry.source.label()}))).collect::<Result<_,_>>()?;
    let next = q.offset + rows.len();
    Ok(
        json!({"rows":rows,"total":matches.len(),"next_offset":(next<matches.len()).then_some(next),"note":"Supported launch-safe catalog, not a complete installed-software inventory. Resolve a catalog_id before creating a rule."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovered_rule_template_uses_resolved_executable_and_keeps_arguments() {
        let app = AppToRun::new_path(
            r"C:\\Apps\\source.lnk".into(),
            vec!["--literal=one two".into()],
            r"C:\\Apps\\resolved.exe".into(),
            os_api::PriorityClass::Normal,
            false,
        );
        let rule = template(&app).unwrap();
        assert_eq!(rule["path"], r"C:\\Apps\\resolved.exe");
        assert_eq!(rule["args"][0], "--literal=one two");
        assert!(rule.get("dropped_path").is_none());
    }
}
