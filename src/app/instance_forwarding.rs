use crate::app::runtime::RunRuleOutcome;
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::startup::{is_cli_safe_id, StartupIntent};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::sync::mpsc::Sender;
use std::time::Duration;

const IPC_PROTOCOL_VERSION: u16 = 1;
const MAX_FRAME_BYTES: usize = 4096;
pub(crate) const EXIT_GUI_STARTUP_ERROR: i32 = 1;
pub(crate) const EXIT_CLI_PARSE_ERROR: i32 = 2;
pub(crate) const EXIT_ACCEPTED: i32 = 0;
pub(crate) const EXIT_MISSING_GROUP: i32 = 20;
pub(crate) const EXIT_MISSING_RULE: i32 = 21;
pub(crate) const EXIT_LAUNCH_REJECTED: i32 = 22;
pub(crate) const EXIT_SERVER_NOT_READY: i32 = 23;
pub(crate) const EXIT_TIMEOUT: i32 = 24;
pub(crate) const EXIT_PROTOCOL_ERROR: i32 = 25;
pub(crate) const EXIT_AUTH_FAILED: i32 = 26;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IpcCommand {
    RunRule { group_id: GroupId, rule_id: RuleId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IpcResponseCode {
    Accepted,
    MissingGroup,
    MissingRule,
    LaunchRejected,
    ServerNotReady,
    Timeout,
    ProtocolError,
    AuthFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IpcResponse {
    pub code: IpcResponseCode,
    pub detail: Option<String>,
}

#[cfg(test)]
pub(crate) struct ForwardedIpcCommand {
    pub command: IpcCommand,
    pub response_tx: Sender<IpcResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IpcProtocolError {
    OversizedFrame,
    InvalidUtf8,
    InvalidJson,
    UnsupportedVersion,
    InvalidId,
    SerializeFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForwardingProbeResult {
    Forwarded(IpcResponse),
    NoActiveInstance,
    ServerNotReady,
    ProtocolError,
    AuthFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForwardingClientError {
    NoServer,
    ServerNotReady,
    Timeout,
    SecurityRejected(String),
    #[cfg_attr(not(feature = "windows"), allow(dead_code))]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EntryAction {
    RunGui(StartupIntent),
    Exit(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedStartupForwarding<R> {
    pub action: EntryAction,
    pub forwarding_runtime: Option<R>,
    pub forwarding_warning: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ForwardingRetryPolicy {
    pub total_timeout: Duration,
    pub request_timeout: Duration,
    pub retry_sleep: Duration,
}

pub(crate) trait StartupForwardingPlatform {
    type Guard;
    type Runtime;

    fn resolve_endpoint(&mut self) -> Result<(), String>;
    fn try_claim_primary_guard(&mut self) -> Result<Option<Self::Guard>, String>;
    fn start_forwarding_runtime(&mut self, guard: Self::Guard) -> Result<Self::Runtime, String>;
    fn send_request(
        &mut self,
        request: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, ForwardingClientError>;
}

pub(crate) trait StartupForwardingClock {
    fn now(&self) -> Duration;
    fn sleep(&mut self, duration: Duration);
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCommandFrame {
    version: u16,
    command: WireCommand,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireCommand {
    RunRule { group_id: String, rule_id: String },
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct WireCommandFrameOut {
    version: u16,
    command: WireCommandOut,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireCommandOut {
    RunRule { group_id: String, rule_id: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireResponseFrame {
    version: u16,
    code: IpcResponseCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

pub(crate) fn parse_ipc_command_frame(frame: &[u8]) -> Result<IpcCommand, IpcProtocolError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(IpcProtocolError::OversizedFrame);
    }

    let frame = std::str::from_utf8(frame).map_err(|_| IpcProtocolError::InvalidUtf8)?;
    let frame: WireCommandFrame =
        serde_json::from_str(frame).map_err(|_| IpcProtocolError::InvalidJson)?;

    if frame.version != IPC_PROTOCOL_VERSION {
        return Err(IpcProtocolError::UnsupportedVersion);
    }

    match frame.command {
        WireCommand::RunRule { group_id, rule_id } => {
            if !is_cli_safe_id(&group_id) || !is_cli_safe_id(&rule_id) {
                return Err(IpcProtocolError::InvalidId);
            }

            Ok(IpcCommand::RunRule {
                group_id: GroupId(group_id),
                rule_id: RuleId(rule_id),
            })
        }
    }
}

pub(crate) fn serialize_ipc_command_frame(
    command: &IpcCommand,
) -> Result<Vec<u8>, IpcProtocolError> {
    let command = match command {
        IpcCommand::RunRule { group_id, rule_id } => WireCommandOut::RunRule {
            group_id: group_id.0.clone(),
            rule_id: rule_id.0.clone(),
        },
    };
    let frame = WireCommandFrameOut {
        version: IPC_PROTOCOL_VERSION,
        command,
    };

    serde_json::to_vec(&frame).map_err(|_| IpcProtocolError::SerializeFailed)
}

pub(crate) fn parse_ipc_response_frame(frame: &[u8]) -> Result<IpcResponse, IpcProtocolError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(IpcProtocolError::OversizedFrame);
    }

    let frame = std::str::from_utf8(frame).map_err(|_| IpcProtocolError::InvalidUtf8)?;
    let frame: WireResponseFrame =
        serde_json::from_str(frame).map_err(|_| IpcProtocolError::InvalidJson)?;

    if frame.version != IPC_PROTOCOL_VERSION {
        return Err(IpcProtocolError::UnsupportedVersion);
    }

    Ok(IpcResponse {
        code: frame.code,
        detail: frame.detail.map(|detail| sanitize_detail(&detail)),
    })
}

pub(crate) fn serialize_ipc_response_frame(
    response: &IpcResponse,
) -> Result<Vec<u8>, IpcProtocolError> {
    let frame = WireResponseFrame {
        version: IPC_PROTOCOL_VERSION,
        code: response.code,
        detail: response.detail.as_deref().map(sanitize_detail),
    };

    serde_json::to_vec(&frame).map_err(|_| IpcProtocolError::SerializeFailed)
}

pub(crate) fn run_rule_outcome_to_response(outcome: RunRuleOutcome) -> IpcResponse {
    match outcome {
        RunRuleOutcome::Accepted => IpcResponse {
            code: IpcResponseCode::Accepted,
            detail: None,
        },
        RunRuleOutcome::MissingGroup => IpcResponse {
            code: IpcResponseCode::MissingGroup,
            detail: None,
        },
        RunRuleOutcome::MissingRule => IpcResponse {
            code: IpcResponseCode::MissingRule,
            detail: None,
        },
        RunRuleOutcome::LaunchRejected(message) => IpcResponse {
            code: IpcResponseCode::LaunchRejected,
            detail: Some(sanitize_detail(&message)),
        },
    }
}

pub(crate) fn forwarding_exit_code(response: &IpcResponse) -> i32 {
    match response.code {
        IpcResponseCode::Accepted => EXIT_ACCEPTED,
        IpcResponseCode::MissingGroup => EXIT_MISSING_GROUP,
        IpcResponseCode::MissingRule => EXIT_MISSING_RULE,
        IpcResponseCode::LaunchRejected => EXIT_LAUNCH_REJECTED,
        IpcResponseCode::ServerNotReady => EXIT_SERVER_NOT_READY,
        IpcResponseCode::Timeout => EXIT_TIMEOUT,
        IpcResponseCode::ProtocolError => EXIT_PROTOCOL_ERROR,
        IpcResponseCode::AuthFailed => EXIT_AUTH_FAILED,
    }
}

pub(crate) fn decide_entry_action(
    startup_intent: StartupIntent,
    probe_result: ForwardingProbeResult,
) -> EntryAction {
    if matches!(startup_intent, StartupIntent::NormalGui) {
        return EntryAction::RunGui(startup_intent);
    }

    match probe_result {
        ForwardingProbeResult::Forwarded(response) => {
            EntryAction::Exit(forwarding_exit_code(&response))
        }
        ForwardingProbeResult::NoActiveInstance => EntryAction::RunGui(startup_intent),
        ForwardingProbeResult::ServerNotReady => EntryAction::Exit(EXIT_SERVER_NOT_READY),
        ForwardingProbeResult::ProtocolError => EntryAction::Exit(EXIT_PROTOCOL_ERROR),
        ForwardingProbeResult::AuthFailed => EntryAction::Exit(EXIT_AUTH_FAILED),
    }
}

pub(crate) fn prepare_startup_forwarding_with<P, C>(
    startup_intent: StartupIntent,
    platform: &mut P,
    clock: &mut C,
    retry_policy: ForwardingRetryPolicy,
) -> PreparedStartupForwarding<P::Runtime>
where
    P: StartupForwardingPlatform,
    C: StartupForwardingClock,
{
    if let Err(err) = platform.resolve_endpoint() {
        return match startup_intent {
            StartupIntent::NormalGui => PreparedStartupForwarding {
                action: EntryAction::RunGui(StartupIntent::NormalGui),
                forwarding_runtime: None,
                forwarding_warning: Some(err),
            },
            intent @ StartupIntent::RunRule { .. } => PreparedStartupForwarding {
                action: decide_entry_action(intent, ForwardingProbeResult::AuthFailed),
                forwarding_runtime: None,
                forwarding_warning: None,
            },
        };
    }

    match startup_intent.clone() {
        StartupIntent::NormalGui => {
            let (forwarding_runtime, forwarding_warning) =
                prepare_normal_gui_forwarding_with(platform);
            PreparedStartupForwarding {
                action: EntryAction::RunGui(startup_intent),
                forwarding_runtime,
                forwarding_warning,
            }
        }
        StartupIntent::RunRule { group_id, rule_id } => prepare_run_rule_forwarding_with(
            startup_intent,
            group_id,
            rule_id,
            platform,
            clock,
            retry_policy,
        ),
    }
}

fn prepare_normal_gui_forwarding_with<P>(platform: &mut P) -> (Option<P::Runtime>, Option<String>)
where
    P: StartupForwardingPlatform,
{
    match platform.try_claim_primary_guard() {
        Ok(Some(guard)) => match platform.start_forwarding_runtime(guard) {
            Ok(runtime) => (Some(runtime), None),
            Err(err) => (None, Some(err)),
        },
        Ok(None) => (
            None,
            Some("another instance owns the local shortcut endpoint".to_string()),
        ),
        Err(err) => (None, Some(err)),
    }
}

fn prepare_run_rule_forwarding_with<P, C>(
    startup_intent: StartupIntent,
    group_id: GroupId,
    rule_id: RuleId,
    platform: &mut P,
    clock: &mut C,
    retry_policy: ForwardingRetryPolicy,
) -> PreparedStartupForwarding<P::Runtime>
where
    P: StartupForwardingPlatform,
    C: StartupForwardingClock,
{
    match platform.try_claim_primary_guard() {
        Ok(Some(guard)) => match platform.start_forwarding_runtime(guard) {
            Ok(runtime) => PreparedStartupForwarding {
                action: decide_entry_action(
                    startup_intent,
                    ForwardingProbeResult::NoActiveInstance,
                ),
                forwarding_runtime: Some(runtime),
                forwarding_warning: None,
            },
            Err(_) => PreparedStartupForwarding {
                action: decide_entry_action(startup_intent, ForwardingProbeResult::AuthFailed),
                forwarding_runtime: None,
                forwarding_warning: None,
            },
        },
        Ok(None) => {
            let probe_result =
                forward_run_rule_to_primary_with(platform, clock, retry_policy, group_id, rule_id);
            PreparedStartupForwarding {
                action: decide_entry_action(startup_intent, probe_result),
                forwarding_runtime: None,
                forwarding_warning: None,
            }
        }
        Err(_) => PreparedStartupForwarding {
            action: decide_entry_action(startup_intent, ForwardingProbeResult::AuthFailed),
            forwarding_runtime: None,
            forwarding_warning: None,
        },
    }
}

fn forward_run_rule_to_primary_with<P, C>(
    platform: &mut P,
    clock: &mut C,
    retry_policy: ForwardingRetryPolicy,
    group_id: GroupId,
    rule_id: RuleId,
) -> ForwardingProbeResult
where
    P: StartupForwardingPlatform,
    C: StartupForwardingClock,
{
    let request = match serialize_ipc_command_frame(&IpcCommand::RunRule { group_id, rule_id }) {
        Ok(request) => request,
        Err(_) => return ForwardingProbeResult::ProtocolError,
    };

    let deadline = clock.now() + retry_policy.total_timeout;
    loop {
        match platform.send_request(&request, retry_policy.request_timeout) {
            Ok(response) => {
                return parse_ipc_response_frame(&response)
                    .map(ForwardingProbeResult::Forwarded)
                    .unwrap_or(ForwardingProbeResult::ProtocolError);
            }
            Err(ForwardingClientError::NoServer | ForwardingClientError::ServerNotReady)
                if clock.now() < deadline =>
            {
                clock.sleep(retry_policy.retry_sleep);
            }
            Err(ForwardingClientError::NoServer | ForwardingClientError::ServerNotReady) => {
                return ForwardingProbeResult::ServerNotReady;
            }
            Err(ForwardingClientError::Timeout) => {
                return ForwardingProbeResult::Forwarded(IpcResponse {
                    code: IpcResponseCode::Timeout,
                    detail: None,
                });
            }
            Err(ForwardingClientError::SecurityRejected(_)) => {
                return ForwardingProbeResult::AuthFailed;
            }
            Err(ForwardingClientError::Io(_)) => {
                return ForwardingProbeResult::ServerNotReady;
            }
        }
    }
}

fn sanitize_detail(message: &str) -> String {
    const MAX_DETAIL_CHARS: usize = 240;

    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_DETAIL_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::time::Duration;

    fn run_rule_intent() -> StartupIntent {
        StartupIntent::RunRule {
            group_id: GroupId("group-1".to_string()),
            rule_id: RuleId("rule-1".to_string()),
        }
    }

    #[test]
    fn test_ipc_command_accepts_valid_run_rule_frame() {
        let frame = br#"{"version":1,"command":{"type":"run_rule","group_id":"group-1","rule_id":"rule-1"}}"#;

        assert_eq!(
            parse_ipc_command_frame(frame),
            Ok(IpcCommand::RunRule {
                group_id: GroupId("group-1".to_string()),
                rule_id: RuleId("rule-1".to_string())
            })
        );
    }

    #[test]
    fn test_ipc_command_rejects_unknown_fields_and_versions() {
        let unknown_field =
            br#"{"version":1,"command":{"type":"run_rule","group_id":"group-1","rule_id":"rule-1"},"extra":true}"#;
        let unsupported_version =
            br#"{"version":2,"command":{"type":"run_rule","group_id":"group-1","rule_id":"rule-1"}}"#;

        assert_eq!(
            parse_ipc_command_frame(unknown_field),
            Err(IpcProtocolError::InvalidJson)
        );
        assert_eq!(
            parse_ipc_command_frame(unsupported_version),
            Err(IpcProtocolError::UnsupportedVersion)
        );
    }

    #[test]
    fn test_ipc_command_serialization_roundtrips_run_rule_frame() {
        let command = IpcCommand::RunRule {
            group_id: GroupId("group-1".to_string()),
            rule_id: RuleId("rule-1".to_string()),
        };

        let frame = serialize_ipc_command_frame(&command).unwrap();

        assert_eq!(parse_ipc_command_frame(&frame), Ok(command));
    }

    #[test]
    fn test_ipc_command_rejects_invalid_ids_and_oversized_frames() {
        let invalid_id =
            br#"{"version":1,"command":{"type":"run_rule","group_id":"group 1","rule_id":"rule-1"}}"#;
        let oversized = vec![b' '; 4097];

        assert_eq!(
            parse_ipc_command_frame(invalid_id),
            Err(IpcProtocolError::InvalidId)
        );
        assert_eq!(
            parse_ipc_command_frame(&oversized),
            Err(IpcProtocolError::OversizedFrame)
        );
    }

    #[test]
    fn test_run_rule_outcome_maps_to_stable_response_codes_and_exit_codes() {
        let accepted = run_rule_outcome_to_response(RunRuleOutcome::Accepted);
        let missing_group = run_rule_outcome_to_response(RunRuleOutcome::MissingGroup);
        let missing_rule = run_rule_outcome_to_response(RunRuleOutcome::MissingRule);
        let rejected = run_rule_outcome_to_response(RunRuleOutcome::LaunchRejected(
            "line 1\r\nline 2".to_string(),
        ));

        assert_eq!(accepted.code, IpcResponseCode::Accepted);
        assert_eq!(forwarding_exit_code(&accepted), 0);
        assert_eq!(missing_group.code, IpcResponseCode::MissingGroup);
        assert_ne!(forwarding_exit_code(&missing_group), 0);
        assert_eq!(missing_rule.code, IpcResponseCode::MissingRule);
        assert_ne!(forwarding_exit_code(&missing_rule), 0);
        assert_eq!(rejected.code, IpcResponseCode::LaunchRejected);
        assert_eq!(rejected.detail.as_deref(), Some("line 1 line 2"));
        assert_ne!(forwarding_exit_code(&rejected), 0);
    }

    #[test]
    fn test_ipc_response_serialization_roundtrips_and_rejects_unknown_codes() {
        let response = IpcResponse {
            code: IpcResponseCode::MissingRule,
            detail: Some("line 1\r\nline 2".to_string()),
        };

        let frame = serialize_ipc_response_frame(&response).unwrap();

        assert_eq!(
            parse_ipc_response_frame(&frame),
            Ok(IpcResponse {
                code: IpcResponseCode::MissingRule,
                detail: Some("line 1 line 2".to_string())
            })
        );

        assert_eq!(
            parse_ipc_response_frame(br#"{"version":1,"code":"bogus"}"#),
            Err(IpcProtocolError::InvalidJson)
        );
        assert_eq!(
            parse_ipc_response_frame(br#"{"version":1,"code":"accepted","extra":true}"#),
            Err(IpcProtocolError::InvalidJson)
        );
    }

    #[test]
    fn test_startup_decision_never_forwards_normal_gui() {
        assert_eq!(
            decide_entry_action(
                StartupIntent::NormalGui,
                ForwardingProbeResult::Forwarded(IpcResponse {
                    code: IpcResponseCode::Accepted,
                    detail: None
                })
            ),
            EntryAction::RunGui(StartupIntent::NormalGui)
        );
    }

    #[test]
    fn test_startup_decision_for_run_rule_forwards_or_falls_back() {
        let intent = run_rule_intent();

        assert_eq!(
            decide_entry_action(
                intent.clone(),
                ForwardingProbeResult::Forwarded(IpcResponse {
                    code: IpcResponseCode::Accepted,
                    detail: None
                })
            ),
            EntryAction::Exit(0)
        );
        assert_eq!(
            decide_entry_action(intent.clone(), ForwardingProbeResult::NoActiveInstance),
            EntryAction::RunGui(intent.clone())
        );
        assert!(matches!(
            decide_entry_action(intent.clone(), ForwardingProbeResult::ServerNotReady),
            EntryAction::Exit(code) if code != 0
        ));
        assert!(matches!(
            decide_entry_action(intent.clone(), ForwardingProbeResult::AuthFailed),
            EntryAction::Exit(code) if code != 0
        ));
    }

    #[test]
    fn test_forwarding_error_responses_use_distinct_nonzero_exit_codes() {
        let responses = [
            IpcResponse {
                code: IpcResponseCode::ServerNotReady,
                detail: None,
            },
            IpcResponse {
                code: IpcResponseCode::Timeout,
                detail: None,
            },
            IpcResponse {
                code: IpcResponseCode::ProtocolError,
                detail: None,
            },
            IpcResponse {
                code: IpcResponseCode::AuthFailed,
                detail: None,
            },
        ];

        let exit_codes = responses
            .iter()
            .map(forwarding_exit_code)
            .collect::<Vec<_>>();

        assert!(exit_codes.iter().all(|code| *code != 0));
        assert_eq!(exit_codes.len(), 4);
        for (index, code) in exit_codes.iter().enumerate() {
            assert!(!exit_codes[..index].contains(code));
        }

        assert!(matches!(
            decide_entry_action(run_rule_intent(), ForwardingProbeResult::ProtocolError),
            EntryAction::Exit(code) if code != 0
        ));
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum StartupCall {
        ResolveEndpoint,
        TryClaimPrimaryGuard,
        StartForwardingRuntime,
        SendRequest(Duration),
        Sleep(Duration),
    }

    #[derive(Debug, Clone)]
    enum FakeSendOutcome {
        Response(Vec<u8>),
        Error(ForwardingClientError),
    }

    struct FakeStartupPlatform {
        calls: Rc<RefCell<Vec<StartupCall>>>,
        endpoint_result: Result<(), String>,
        guard_result: Result<Option<&'static str>, String>,
        server_result: Result<&'static str, String>,
        send_outcomes: VecDeque<FakeSendOutcome>,
        sent_commands: Rc<RefCell<Vec<IpcCommand>>>,
    }

    impl FakeStartupPlatform {
        fn primary() -> Self {
            Self {
                calls: Rc::new(RefCell::new(Vec::new())),
                endpoint_result: Ok(()),
                guard_result: Ok(Some("guard")),
                server_result: Ok("runtime"),
                send_outcomes: VecDeque::new(),
                sent_commands: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn secondary(send_outcomes: Vec<FakeSendOutcome>) -> Self {
            Self {
                calls: Rc::new(RefCell::new(Vec::new())),
                endpoint_result: Ok(()),
                guard_result: Ok(None),
                server_result: Ok("runtime"),
                send_outcomes: send_outcomes.into(),
                sent_commands: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl StartupForwardingPlatform for FakeStartupPlatform {
        type Guard = &'static str;
        type Runtime = &'static str;

        fn resolve_endpoint(&mut self) -> Result<(), String> {
            self.calls.borrow_mut().push(StartupCall::ResolveEndpoint);
            self.endpoint_result.clone()
        }

        fn try_claim_primary_guard(&mut self) -> Result<Option<Self::Guard>, String> {
            self.calls
                .borrow_mut()
                .push(StartupCall::TryClaimPrimaryGuard);
            self.guard_result.clone()
        }

        fn start_forwarding_runtime(
            &mut self,
            _guard: Self::Guard,
        ) -> Result<Self::Runtime, String> {
            self.calls
                .borrow_mut()
                .push(StartupCall::StartForwardingRuntime);
            self.server_result.clone()
        }

        fn send_request(
            &mut self,
            request: &[u8],
            timeout: Duration,
        ) -> Result<Vec<u8>, ForwardingClientError> {
            self.calls
                .borrow_mut()
                .push(StartupCall::SendRequest(timeout));
            self.sent_commands
                .borrow_mut()
                .push(parse_ipc_command_frame(request).unwrap());

            match self.send_outcomes.pop_front() {
                Some(FakeSendOutcome::Response(response)) => Ok(response),
                Some(FakeSendOutcome::Error(error)) => Err(error),
                None => Err(ForwardingClientError::NoServer),
            }
        }
    }

    struct FakeStartupClock {
        now: Duration,
        calls: Rc<RefCell<Vec<StartupCall>>>,
    }

    impl FakeStartupClock {
        fn for_platform(platform: &FakeStartupPlatform) -> Self {
            Self {
                now: Duration::ZERO,
                calls: Rc::clone(&platform.calls),
            }
        }
    }

    impl StartupForwardingClock for FakeStartupClock {
        fn now(&self) -> Duration {
            self.now
        }

        fn sleep(&mut self, duration: Duration) {
            self.calls.borrow_mut().push(StartupCall::Sleep(duration));
            self.now += duration;
        }
    }

    fn short_retry_policy() -> ForwardingRetryPolicy {
        ForwardingRetryPolicy {
            total_timeout: Duration::from_millis(250),
            request_timeout: Duration::from_millis(50),
            retry_sleep: Duration::from_millis(100),
        }
    }

    fn response_bytes(code: IpcResponseCode) -> Vec<u8> {
        serialize_ipc_response_frame(&IpcResponse { code, detail: None }).unwrap()
    }

    fn prepare_with_fake(
        intent: StartupIntent,
        platform: &mut FakeStartupPlatform,
    ) -> PreparedStartupForwarding<&'static str> {
        let mut clock = FakeStartupClock::for_platform(platform);
        prepare_startup_forwarding_with(intent, platform, &mut clock, short_retry_policy())
    }

    #[test]
    fn test_startup_boundary_normal_gui_never_forwards_exits_or_retries() {
        let mut platform = FakeStartupPlatform::primary();
        platform.guard_result = Ok(None);

        let prepared = prepare_with_fake(StartupIntent::NormalGui, &mut platform);

        assert_eq!(
            prepared.action,
            EntryAction::RunGui(StartupIntent::NormalGui)
        );
        assert_eq!(prepared.forwarding_runtime, None);
        assert_eq!(
            prepared.forwarding_warning.as_deref(),
            Some("another instance owns the local shortcut endpoint")
        );
        assert_eq!(
            *platform.calls.borrow(),
            vec![
                StartupCall::ResolveEndpoint,
                StartupCall::TryClaimPrimaryGuard
            ]
        );
        assert!(platform.sent_commands.borrow().is_empty());
    }

    #[test]
    fn test_startup_boundary_run_rule_primary_cold_starts_gui_with_server() {
        let intent = run_rule_intent();
        let mut platform = FakeStartupPlatform::primary();

        let prepared = prepare_with_fake(intent.clone(), &mut platform);

        assert_eq!(prepared.action, EntryAction::RunGui(intent));
        assert_eq!(prepared.forwarding_runtime, Some("runtime"));
        assert_eq!(prepared.forwarding_warning, None);
        assert_eq!(
            *platform.calls.borrow(),
            vec![
                StartupCall::ResolveEndpoint,
                StartupCall::TryClaimPrimaryGuard,
                StartupCall::StartForwardingRuntime
            ]
        );
        assert!(platform.sent_commands.borrow().is_empty());
    }

    #[test]
    fn test_startup_boundary_secondary_forwarded_responses_use_exact_exit_codes() {
        let cases = [
            (IpcResponseCode::Accepted, 0),
            (IpcResponseCode::MissingGroup, 20),
            (IpcResponseCode::MissingRule, 21),
            (IpcResponseCode::LaunchRejected, 22),
        ];

        for (response_code, expected_exit_code) in cases {
            let intent = run_rule_intent();
            let mut platform = FakeStartupPlatform::secondary(vec![FakeSendOutcome::Response(
                response_bytes(response_code),
            )]);

            let prepared = prepare_with_fake(intent.clone(), &mut platform);

            assert_eq!(prepared.action, EntryAction::Exit(expected_exit_code));
            assert_eq!(prepared.forwarding_runtime, None);
            assert_eq!(
                *platform.sent_commands.borrow(),
                vec![IpcCommand::RunRule {
                    group_id: GroupId("group-1".to_string()),
                    rule_id: RuleId("rule-1".to_string())
                }]
            );
        }
    }

    #[test]
    fn test_startup_boundary_guard_present_pipe_absent_retries_then_exits_23() {
        let mut platform = FakeStartupPlatform::secondary(vec![
            FakeSendOutcome::Error(ForwardingClientError::NoServer),
            FakeSendOutcome::Error(ForwardingClientError::ServerNotReady),
            FakeSendOutcome::Error(ForwardingClientError::NoServer),
        ]);

        let prepared = prepare_with_fake(run_rule_intent(), &mut platform);

        assert_eq!(prepared.action, EntryAction::Exit(23));
        assert_eq!(
            *platform.calls.borrow(),
            vec![
                StartupCall::ResolveEndpoint,
                StartupCall::TryClaimPrimaryGuard,
                StartupCall::SendRequest(Duration::from_millis(50)),
                StartupCall::Sleep(Duration::from_millis(100)),
                StartupCall::SendRequest(Duration::from_millis(50)),
                StartupCall::Sleep(Duration::from_millis(100)),
                StartupCall::SendRequest(Duration::from_millis(50)),
                StartupCall::Sleep(Duration::from_millis(100)),
                StartupCall::SendRequest(Duration::from_millis(50)),
            ]
        );
    }

    #[test]
    fn test_startup_boundary_timeout_protocol_and_auth_use_exact_exit_codes() {
        let cases = [
            (
                FakeSendOutcome::Error(ForwardingClientError::Timeout),
                EntryAction::Exit(24),
            ),
            (
                FakeSendOutcome::Response(b"not-json".to_vec()),
                EntryAction::Exit(25),
            ),
            (
                FakeSendOutcome::Error(ForwardingClientError::SecurityRejected(
                    "denied".to_string(),
                )),
                EntryAction::Exit(26),
            ),
        ];

        for (send_outcome, expected_action) in cases {
            let mut platform = FakeStartupPlatform::secondary(vec![send_outcome]);

            let prepared = prepare_with_fake(run_rule_intent(), &mut platform);

            assert_eq!(prepared.action, expected_action);
        }
    }

    #[test]
    fn test_startup_boundary_parse_and_gui_error_exit_codes_stay_distinct() {
        assert_eq!(EXIT_CLI_PARSE_ERROR, 2);
        assert_eq!(EXIT_GUI_STARTUP_ERROR, 1);

        let forwarding_exit_codes = [
            IpcResponseCode::Accepted,
            IpcResponseCode::MissingGroup,
            IpcResponseCode::MissingRule,
            IpcResponseCode::LaunchRejected,
            IpcResponseCode::ServerNotReady,
            IpcResponseCode::Timeout,
            IpcResponseCode::ProtocolError,
            IpcResponseCode::AuthFailed,
        ]
        .map(|code| forwarding_exit_code(&IpcResponse { code, detail: None }));

        assert!(!forwarding_exit_codes.contains(&EXIT_CLI_PARSE_ERROR));
        assert!(!forwarding_exit_codes.contains(&EXIT_GUI_STARTUP_ERROR));
    }
}
