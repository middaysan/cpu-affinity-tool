use super::{
    operations::{ApiError, ApiResult},
    services::Services,
};
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};
fn prepare(epoch: &str, name: &str, mut args: Value) -> Result<(Value, String), ApiError> {
    let scope = (
        args.get("source")
            .and_then(Value::as_str)
            .unwrap_or("tracked"),
        args.get("group_id").and_then(Value::as_str),
        args.get("rule_id").and_then(Value::as_str),
        args.get("run_id").and_then(Value::as_str),
        args.get("cpu").and_then(Value::as_u64).unwrap_or(0),
    );
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    scope.hash(&mut hash);
    let scope_key = format!("{:016x}", hash.finish());
    let incremental = name == "read_cpu_changes"
        && args
            .get("since_revision")
            .and_then(Value::as_str)
            .unwrap_or("0")
            != "0";
    if let Some(provided) = args.get("epoch").and_then(Value::as_str) {
        let (api, native) = provided.split_once(':').ok_or_else(|| {
            ApiError::new(
                "cursor_reset",
                "CPU cursor needs its full API/history epoch",
            )
        })?;
        if api != epoch {
            return Err(ApiError::new(
                "cursor_reset",
                "API epoch changed; restart CPU history inspection",
            ));
        }
        args["epoch"] = json!(native.to_owned());
    } else if incremental {
        return Err(ApiError::new(
            "cursor_reset",
            "Incremental CPU reads require the returned epoch",
        ));
    }
    if incremental && args.get("scope_key").and_then(Value::as_str) != Some(&scope_key) {
        return Err(ApiError::new(
            "scope_changed",
            "Restart the CPU baseline when its source or CPU changes",
        ));
    }
    args.as_object_mut()
        .ok_or_else(|| ApiError::invalid("Expected CPU argument object"))?
        .remove("scope_key");
    Ok((args, scope_key))
}
impl Services {
    pub(super) fn read_cpu(&self, name: &str, args: Value) -> ApiResult {
        if matches!(name, "read_cpu_current" | "read_run_cpu") {
            let mut result = self
                .cpu
                .read(name, args)
                .map_err(|e| ApiError::new("cpu_unavailable", e))?;
            result["api_epoch"] = json!(self.api_epoch);
            return Ok(result);
        }
        let (args, scope) = prepare(&self.api_epoch, name, args)?;
        let mut result = self
            .cpu
            .read(name, args)
            .map_err(|e| ApiError::new("cpu_unavailable", e))?;
        if let Some(epoch) = result.get("epoch").and_then(Value::as_str) {
            result["epoch"] = json!(format!("{}:{epoch}", self.api_epoch));
        }
        result["scope_key"] = json!(scope);
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_cursor_cannot_silently_cross_restart_or_scope() {
        let (_, scope) = prepare(
            "api",
            "read_cpu_history",
            json!({"source":"system","cpu":0}),
        )
        .unwrap();
        let query = json!({"source":"system","cpu":0,"epoch":"api:1","scope_key":scope,"since_revision":"4"});
        assert!(prepare("api", "read_cpu_changes", query.clone()).is_ok());
        assert!(prepare("other", "read_cpu_changes", query.clone()).is_err());
        let mut changed = query.clone();
        changed["cpu"] = json!(1);
        assert!(prepare("api", "read_cpu_changes", changed).is_err());
        let mut changed = query;
        changed.as_object_mut().unwrap().remove("epoch");
        assert!(prepare("api", "read_cpu_changes", changed).is_err());
    }
}
