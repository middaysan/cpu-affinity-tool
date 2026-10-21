use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};

const MAX_OPERATIONS: usize = 64;
const MAX_PENDING: usize = 16;
const MAX_INTENT_BYTES: usize = 16 * 1024;
pub(super) type ApiResult = Result<Value, ApiError>;

#[derive(Clone, Debug, Serialize)]
pub(super) struct ApiError {
    pub code: &'static str,
    pub message: String,
}
impl ApiError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("validation", message)
    }
    pub fn busy() -> Self {
        Self::new("busy", "The bounded operation queue is busy; retry with a new sequence after reading current operations")
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct Operation {
    pub id: String,
    pub agent: String,
    pub tool: String,
    pub status: &'static str,
    pub result: Option<Value>,
    pub error: Option<ApiError>,
    #[serde(skip)]
    key: String,
    #[serde(skip)]
    intent: Value,
}
pub(super) struct Admission {
    pub id: String,
    pub fresh: bool,
}
pub(super) struct Ledger {
    pub commands: super::activity::Journal,
    pub epoch: String,
    next: u64,
    high_water: HashMap<String, u64>,
    entries: VecDeque<Operation>,
    audit: VecDeque<Value>,
    audit_sequence: u64,
}
impl Ledger {
    pub fn rotate_epoch(&mut self, epoch: String) {
        self.epoch = epoch;
        self.high_water.clear();
    }
    pub fn new(epoch: String) -> Self {
        Self {
            commands: Default::default(),
            epoch,
            next: 1,
            high_water: HashMap::new(),
            entries: VecDeque::new(),
            audit: VecDeque::new(),
            audit_sequence: 0,
        }
    }
    fn record(&mut self, agent: &str, id: &str, tool: &str, status: &str) {
        self.audit_sequence = self.audit_sequence.saturating_add(1);
        if self.audit.len() == 256 {
            self.audit.pop_front();
        }
        self.audit.push_back(json!({"sequence":self.audit_sequence.to_string(),"unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().to_string(),"agent_id":agent,"operation_id":id,"tool":tool,"status":status}));
    }
    pub fn audit(&self, agent: &str, epoch: Option<&str>, after: u64, limit: usize) -> ApiResult {
        if limit == 0 || limit > 256 || after > self.audit_sequence {
            return Err(ApiError::invalid("Invalid audit page/cursor"));
        }
        if epoch.is_some_and(|epoch| epoch != self.epoch) {
            return Err(ApiError::new(
                "cursor_reset",
                "API epoch changed; restart audit pagination",
            ));
        }
        let rows: Vec<_> = self
            .audit
            .iter()
            .filter(|entry| {
                entry["agent_id"] == agent
                    && entry["sequence"]
                        .as_str()
                        .and_then(|n| n.parse::<u64>().ok())
                        .is_some_and(|seq| seq > after)
            })
            .take(limit)
            .cloned()
            .collect();
        let next = rows
            .last()
            .and_then(|row| row["sequence"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| self.audit_sequence.to_string());
        Ok(
            json!({"epoch":self.epoch,"next_after":next,"retained_from":self.audit.front().map(|r|&r["sequence"]),"rows":rows,"note":"Bounded RAM audit of accepted operations and transitions for the agent; arguments, credentials and program output are excluded"}),
        )
    }
    pub fn next_sequence(&self, agent: &str) -> Option<u64> {
        self.high_water
            .get(agent)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
    }
    pub fn admit(
        &mut self,
        agent: &str,
        key: &str,
        tool: &str,
        intent: Value,
    ) -> Result<Admission, ApiError> {
        let (epoch, sequence) = key
            .split_once(':')
            .ok_or_else(|| ApiError::invalid("request_key must be app_epoch:positive_sequence"))?;
        let sequence = sequence
            .parse::<u64>()
            .ok()
            .filter(|seq| *seq > 0)
            .ok_or_else(|| ApiError::invalid("request sequence must be a positive decimal u64"))?;
        if epoch != self.epoch {
            return Err(ApiError::new(
                "epoch_mismatch",
                "Application restarted; inspect runs before creating a fresh request key",
            ));
        }
        if let Some(old) = self
            .entries
            .iter()
            .find(|old| old.agent == agent && old.key == key)
        {
            if old.tool != tool || old.intent != intent {
                return Err(ApiError::new(
                    "request_conflict",
                    "This request key belongs to different arguments",
                ));
            }
            return Ok(Admission {
                id: old.id.clone(),
                fresh: false,
            });
        }
        if sequence <= self.high_water.get(agent).copied().unwrap_or(0) {
            return Err(ApiError::new(
                "receipt_expired",
                "The old receipt was evicted; its request cannot be executed again",
            ));
        }
        if serde_json::to_vec(&intent)
            .map_err(|_| ApiError::invalid("Invalid intent"))?
            .len()
            > MAX_INTENT_BYTES
        {
            return Err(ApiError::invalid(
                "Operation arguments exceed the 16 KiB intent budget",
            ));
        }
        if self.high_water.len() >= 4 && !self.high_water.contains_key(agent) {
            return Err(ApiError::busy());
        }
        if self
            .entries
            .iter()
            .filter(|op| op.status == "accepted" || op.status == "running")
            .count()
            >= MAX_PENDING
        {
            return Err(ApiError::busy());
        }
        while self.entries.len() >= MAX_OPERATIONS {
            let index = self
                .entries
                .iter()
                .position(|op| op.status != "accepted" && op.status != "running")
                .ok_or_else(ApiError::busy)?;
            self.entries.remove(index);
        }
        let id = format!("{}:{}", self.epoch, self.next);
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| ApiError::new("exhausted", "Operation identity exhausted"))?;
        self.high_water.insert(agent.into(), sequence);
        self.record(agent, &id, tool, "accepted");
        self.entries.push_back(Operation {
            id: id.clone(),
            agent: agent.into(),
            tool: tool.into(),
            status: "accepted",
            result: None,
            error: None,
            key: key.into(),
            intent,
        });
        Ok(Admission { id, fresh: true })
    }
    pub fn get(&self, id: &str) -> Option<&Operation> {
        self.entries.iter().find(|op| op.id == id)
    }
    pub fn running(&mut self, id: &str) {
        let metadata = if let Some(op) = self.entries.iter_mut().find(|op| op.id == id) {
            op.status = "running";
            Some((op.agent.clone(), op.tool.clone()))
        } else {
            None
        };
        if let Some((agent, tool)) = metadata {
            self.commands
                .operation(id, super::activity::CommandStatus::Running);
            self.record(&agent, id, &tool, "running");
        }
    }
    pub fn finish(&mut self, id: &str, outcome: ApiResult) {
        if let Some(op) = self.entries.iter_mut().find(|op| op.id == id) {
            match outcome {
                Ok(value) => {
                    if serde_json::to_vec(&value).is_ok_and(|bytes| bytes.len() <= 32 * 1024) {
                        op.result = Some(value);
                        op.status = "completed";
                    } else {
                        op.error = Some(ApiError::new("result_too_large", "The operation completed but its receipt exceeded the result budget; inspect the target"));
                        op.status = "indeterminate";
                    }
                }
                Err(error) => {
                    op.status = if error.code == "indeterminate" {
                        "indeterminate"
                    } else {
                        "failed"
                    };
                    op.error = Some(error);
                }
            }
        }
        if let Some(op) = self.get(id) {
            let (agent, tool, status) = (op.agent.clone(), op.tool.clone(), op.status);
            let command_status = super::activity::CommandStatus::from_operation(
                status,
                op.error.as_ref().map(|e| e.code),
            );
            self.commands.operation(id, command_status);
            self.record(&agent, id, &tool, status);
        }
    }
    pub fn receipt(&self, id: &str, agent: &str) -> ApiResult {
        let op = self.get(id).filter(|op| op.agent == agent).ok_or_else(|| {
            ApiError::new(
                "receipt_expired",
                "No retained receipt for this agent and operation",
            )
        })?;
        Ok(json!({"operation":op, "app_epoch":self.epoch}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn retries_return_the_same_operation_and_changed_payload_conflicts() {
        let mut ledger = Ledger::new("epoch".into());
        let first = ledger
            .admit("client", "epoch:1", "launch_program", json!({"path":"a"}))
            .unwrap();
        let repeated = ledger
            .admit("client", "epoch:1", "launch_program", json!({"path":"a"}))
            .unwrap();
        assert_eq!(first.id, repeated.id);
        assert!(first.fresh && !repeated.fresh);
        assert!(ledger
            .admit("client", "epoch:1", "launch_program", json!({"path":"b"}))
            .is_err());
    }
    #[test]
    fn eviction_cannot_turn_an_old_request_into_a_new_launch() {
        let mut ledger = Ledger::new("epoch".into());
        for sequence in 1..=100 {
            let a = ledger
                .admit(
                    "client",
                    &format!("epoch:{sequence}"),
                    "launch_program",
                    json!({}),
                )
                .unwrap();
            ledger.finish(&a.id, Ok(json!({"pid":1})));
        }
        assert!(ledger
            .admit("client", "epoch:1", "launch_program", json!({}))
            .is_err());
        assert!(ledger
            .admit("client", "old-epoch:101", "launch_program", json!({}))
            .is_err());
        assert!(ledger.entries.len() <= MAX_OPERATIONS);
    }
    #[test]
    fn pending_work_is_bounded_and_cannot_be_evicted() {
        let mut ledger = Ledger::new("epoch".into());
        for sequence in 1..=MAX_PENDING {
            ledger
                .admit(
                    "client",
                    &format!("epoch:{sequence}"),
                    "launch_program",
                    json!({}),
                )
                .unwrap();
        }
        assert!(ledger
            .admit("client", "epoch:999", "launch_program", json!({}))
            .is_err());
        assert!(ledger.get("epoch:1").is_some());
    }
    #[test]
    fn epoch_rotation_keeps_pending_receipts_and_audit_without_replaying_old_keys() {
        let mut ledger = Ledger::new("old".into());
        let op = ledger
            .admit(
                "client",
                "old:1",
                "launch_program",
                json!({"secret_argument":"must not enter audit"}),
            )
            .unwrap();
        ledger.rotate_epoch("new".into());
        ledger.finish(&op.id, Ok(json!({"pid":42})));
        assert_eq!(
            ledger.receipt(&op.id, "client").unwrap()["operation"]["status"],
            "completed"
        );
        assert!(ledger
            .admit("client", "old:1", "launch_program", json!({}))
            .is_err());
        assert!(ledger
            .admit("client", "new:1", "launch_program", json!({}))
            .is_ok());
        let audit = ledger.audit("client", Some("new"), 0, 256).unwrap();
        assert!(!audit.to_string().contains("secret_argument"));
        assert!(!audit["rows"].as_array().unwrap().is_empty());
    }
}
