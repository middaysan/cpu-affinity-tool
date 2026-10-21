use super::*;
use std::io::{Read, Write};
#[test]
fn managed_run_name_bounds_are_checked_before_admission() {
    let (jobs, _receiver) = mpsc::sync_channel(1);
    let ledger = Arc::new(Mutex::new(Ledger::new("epoch".into())));
    let handler = Handler {
        jobs,
        ledger: ledger.clone(),
        access: Arc::new(LiveAccess::new(AccessPolicy::default())),
        services: None,
    };
    for name in ["x".repeat(257), "😀".repeat(257), String::new()] {
        let args = json!({"path":"C:\\Tools\\child.exe","name":name,"request_key":"epoch:1"});
        assert!(handler.validate_arguments("launch_program", &args).is_err());
        assert_eq!(
            ledger.lock().unwrap().next_sequence(policy::LOCAL_AGENT_ID),
            Some(1)
        );
    }
    for name in ["x".repeat(256), "😀".repeat(256), "\u{1}".repeat(256)] {
        let args = json!({"path":"C:\\Tools\\child.exe","name":name,"request_key":"epoch:1"});
        handler.validate_arguments("launch_program", &args).unwrap();
    }
}
#[test]
fn managed_run_list_fits_wire_budget_with_all_retained_escaped_labels() {
    use crate::app::features::execution::managed_runs::maximum_run_list_fixture;
    for (text, error) in [
        ("\u{1}".repeat(15000), "\u{1}".repeat(15000)),
        ("\\\"".repeat(15000), "\\\"".repeat(15000)),
        ("😀".repeat(15000), "😀".repeat(15000)),
        (("\u{1}".repeat(128) + &"\"".repeat(128)), "\"".repeat(128)),
    ] {
        let list = maximum_run_list_fixture(&text, &error);
        assert_eq!(list["runs"].as_array().unwrap().len(), 64);
        let result = CallToolResult::structured(list);
        let size = serde_json::to_vec(&result).unwrap().len();
        assert!(
            size <= MAX_RESPONSE,
            "All retained runs must remain inspectable: {size}"
        );
    }
}
#[test]
fn tool_catalog_reuses_one_immutable_schema_allocation() {
    let first = tools();
    let second = tools();
    assert!(std::ptr::eq(first.as_ptr(), second.as_ptr()));
}
fn call(port: u16, name: &str, args: Value) -> Value {
    let response = post(
        port,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}}),
        "MCP-Protocol-Version: 2025-11-25\r\n",
    );
    serde_json::from_str(
        response
            .split_once("\r\n\r\n")
            .expect("HTTP response body")
            .1,
    )
    .expect("JSON-RPC response")
}
fn post(port: u16, body: &Value, extra: &str) -> String {
    post_with_token(port, body, extra, &"a".repeat(64))
}
fn post_with_token(port: u16, body: &Value, extra: &str, token: &str) -> String {
    let mut socket = std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let body = body.to_string();
    write!(socket, "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n{extra}\r\n{body}", body.len()).unwrap();
    let mut result = String::new();
    socket.read_to_string(&mut result).unwrap();
    result
}
#[test]
fn actual_http_supports_legacy_initialize_and_modern_discovery_and_revocation() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut policy = AccessPolicy::default(); policy.enabled = true;
        policy.token = "a".repeat(64);
        let live = Arc::new(LiveAccess::new(policy));
        let (jobs, _receiver) = mpsc::sync_channel(16);
        let ledger=Arc::new(Mutex::new(Ledger::new("epoch".into())));
        let server = HttpServer::start(0, live.clone(), ledger.clone(), jobs, None).await.unwrap();
        let result = post(server.port, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}), "");
        assert!(result.starts_with("HTTP/1.1 200"), "{result}"); assert!(result.contains("cpu-affinity-tool"), "{result}");
        let result = post(server.port, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_capabilities","arguments":{}}}), "MCP-Protocol-Version: 2025-11-25\r\n");
        assert!(result.contains("\"app_epoch\":\"epoch\""), "{result}");
        assert!(result.contains("\"access\":\"full\""), "{result}");
        assert!(!result.contains("permissions"), "{result}");
        let result = post(server.port, &json!({"jsonrpc":"2.0","id":4,"method":"tools/list","params":{}}), "MCP-Protocol-Version: 2025-11-25\r\n");
        for name in ["create_group", "launch_program", "focus_process", "stop_process", "tweak_apply"] {
            assert!(result.contains(&format!("\"name\":\"{name}\"")), "The connection must expose the full tool set: {name}");
        }
        let invalid=call(server.port,"get_capabilities",json!({"unexpected":true}));
        assert_eq!(invalid["result"]["structuredContent"]["code"],"validation");
        let invalid=call(server.port,"read_audit_log",json!({"limit":"many"}));
        assert_eq!(invalid["result"]["structuredContent"]["code"],"validation");
        let metadata = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"test","version":"1"}});
        let result = post(server.port, &json!({"jsonrpc":"2.0","id":3,"method":"server/discover","params":{"_meta":metadata}}), "MCP-Protocol-Version: 2026-07-28\r\nMcp-Method: server/discover\r\n");
        assert!(result.starts_with("HTTP/1.1 200"), "{result}"); assert!(result.contains("supportedVersions"), "{result}");
        assert!(post(server.port, &json!({}), "Origin: http://127.0.0.1\r\n").starts_with("HTTP/1.1 403"));
        assert!(post_with_token(server.port, &json!({}), "", "wrong").starts_with("HTTP/1.1 401"));
        let unknown=call(server.port,"secret-tool-name-must-not-be-retained",json!({"secret":"never retain this value"}));
        assert_eq!(unknown["result"]["structuredContent"]["code"],"unknown_tool");
        let journal=&ledger.lock().unwrap().commands;
        assert!(journal.rows.iter().any(|row| row.tool=="get_capabilities" && row.status==CommandStatus::Done));
        assert!(journal.rows.iter().any(|row| row.status==CommandStatus::Failed("validation")));
        assert!(journal.rows.iter().any(|row| row.tool=="unknown"));
        assert!(!format!("{:?}",journal.rows).contains("secret"));
        assert!(journal.last_received.is_some());
        live.replace(AccessPolicy::default());
        assert!(post(server.port, &json!({}), "").starts_with("HTTP/1.1 503"));
    });
}

#[test]
fn off_on_fences_old_requests_and_http_restart_keeps_the_same_key() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let policy = AccessPolicy {
            enabled: true,
            token: "a".repeat(64),
            ..Default::default()
        };
        let live = Arc::new(LiveAccess::new(policy.clone()));
        let generation = live.generation.load(Ordering::Acquire);
        let agent = policy.agent();
        assert!(live.current(&agent, generation));
        let mut off = policy.clone();
        off.enabled = false;
        live.replace(off);
        assert!(!live.current(&agent, generation));
        live.replace(policy.clone());
        assert!(
            !live.current(&agent, generation),
            "Re-enabling must not revive queued writes"
        );
        assert!(live.current(&agent, live.generation.load(Ordering::Acquire)));
        let serialized = serde_json::to_vec(&policy).unwrap();
        let (restored, _) =
            AccessPolicy::restore(Some(&serialized), "", || panic!("Unexpected key rotation"))
                .unwrap();
        let (jobs, _receiver) = mpsc::sync_channel(16);
        let server = HttpServer::start(
            0,
            Arc::new(LiveAccess::new(restored)),
            Arc::new(Mutex::new(Ledger::new("restart".into()))),
            jobs,
            None,
        )
        .await
        .unwrap();
        let result = call(server.port, "get_capabilities", json!({}));
        assert_eq!(result["result"]["structuredContent"]["access"], "full");
        assert!(!live.current(
            &AgentAccess {
                id: policy::LOCAL_AGENT_ID.into(),
                token: "b".repeat(64)
            },
            live.generation.load(Ordering::Acquire)
        ));
    });
}

#[test]
fn http_mutation_uses_shared_writer_and_returns_durable_receipt_without_ui_input() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async{
        use crate::app::{runtime::AppState,models::AppStateStorage};
        use std::sync::RwLock;
        let mut app=AppState::new_for_test(Arc::new(RwLock::new(AppStateStorage::test_fixture(|s|s.groups.clear()))),4);
        let services=super::super::services::Services::from_app(&app,Default::default(),"epoch".into(),Default::default()).unwrap();
        let mut policy=AccessPolicy::default();policy.enabled=true;
        policy.token = "a".repeat(64);
        let live=Arc::new(LiveAccess::new(policy));let (jobs,_receiver)=mpsc::sync_channel(16);
        let ledger=Arc::new(Mutex::new(Ledger::new("epoch".into())));
        let server=HttpServer::start(0,live,ledger.clone(),jobs,Some(services)).await.unwrap();
        let call=|name:&str,args:Value|{let response=post(server.port,&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}}),"MCP-Protocol-Version: 2025-11-25\r\n");let body=response.split_once("\r\n\r\n").unwrap().1;serde_json::from_str::<Value>(body).unwrap()};
        let first=call("create_group",json!({"name":"Agent group","cpus":[0,1],"expected_revision":"0","request_key":"epoch:1"}));
        let id=first["result"]["structuredContent"]["operation"]["id"].as_str().unwrap().to_owned();
        let repeated=call("create_group",json!({"name":"Agent group","cpus":[0,1],"expected_revision":"0","request_key":"epoch:1"}));
        assert_eq!(repeated["result"]["structuredContent"]["operation"]["id"],id);
        for _ in 0..100 {
            let receipt=call("get_operation",json!({"operation_id":id}));
            if receipt["result"]["structuredContent"]["operation"]["status"]=="completed" {
                assert_eq!(receipt["result"]["structuredContent"]["operation"]["result"]["persistence"],"durable");
                app.poll_tuning();assert_eq!(app.persistent_state.read().unwrap().groups.len(),1);
                let ledger=ledger.lock().unwrap();
                let writes:Vec<_>=ledger.commands.rows.iter().filter(|row|row.tool=="create_group").collect();
                assert_eq!(writes.len(),2);assert!(writes.iter().all(|row|row.status==CommandStatus::Done));
                assert!(ledger.commands.rows.iter().filter(|row|row.tool=="get_operation").all(|row|row.status==CommandStatus::Done));return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }panic!("Configuration receipt did not complete");
    });
}

#[test]
#[ignore = "Native managed launch requires an unrestricted interactive Windows session"]
fn managed_http_launch_is_idempotent_and_output_survives_revocation() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async{
        use crate::app::{runtime::AppState,models::AppStateStorage};
        use std::sync::RwLock;
        let app=AppState::new_for_test(Arc::new(RwLock::new(AppStateStorage::test_fixture(|s|s.groups.clear()))),4);
        let services=super::super::services::Services::from_app(&app,Default::default(),"epoch".into(),Default::default()).unwrap();
        let mut policy=AccessPolicy::default();policy.enabled=true;
        policy.token = "a".repeat(64);
        let live=Arc::new(LiveAccess::new(policy));let (jobs,_receiver)=mpsc::sync_channel(16);
        let server=HttpServer::start(0,live.clone(),Arc::new(Mutex::new(Ledger::new("epoch".into()))),jobs,Some(services.clone())).await.unwrap();
        let args=json!({"path":std::env::current_exe().unwrap(),"args":["--exact","app::features::mcp::server::tests::managed_http_output_child","--ignored","--nocapture"],"environment":{"CPU_AFFINITY_HTTP_CHILD_PROBE":"1"},"capture":true,"request_key":"epoch:1"});
        let first=call(server.port,"launch_program",args.clone());
        let id=first["result"]["structuredContent"]["operation"]["id"].as_str().expect("launch receipt").to_owned();
        assert_eq!(call(server.port,"launch_program",args)["result"]["structuredContent"]["operation"]["id"],id);
        let mut run_id=None;
        for _ in 0..100{
            let receipt=call(server.port,"get_operation",json!({"operation_id":id}));
            let operation=&receipt["result"]["structuredContent"]["operation"];
            assert_ne!(operation["status"],"failed","{receipt}");
            if operation["status"]=="completed"{run_id=operation["result"]["run_id"].as_str().map(str::to_owned);break;}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let run_id=run_id.expect("managed run started");live.replace(AccessPolicy::default());
        assert!(post(server.port,&json!({}),"").starts_with("HTTP/1.1 503"));
        for _ in 0..100{
            let stdout=services.runs.output(policy::LOCAL_AGENT_ID,&run_id,"stdout",0,4096).unwrap();
            let stderr=services.runs.output(policy::LOCAL_AGENT_ID,&run_id,"stderr",0,4096).unwrap();
            if stdout["eof"]==true&&stderr["eof"]==true{
                assert!(stdout["text"].as_str().unwrap().contains("after-revocation"));assert!(stderr["text"].as_str().unwrap().contains("stderr-probe"));
                let runs=services.runs.list(policy::LOCAL_AGENT_ID);assert_eq!(runs["runs"].as_array().unwrap().len(),1);assert_eq!(runs["runs"][0]["exit_code"],7);return;
            }tokio::time::sleep(Duration::from_millis(20)).await;
        }panic!("Output did not drain after revocation");
    });
}
#[test]
#[ignore = "Controlled native child fixture"]
fn managed_http_output_child() {
    if std::env::var("CPU_AFFINITY_HTTP_CHILD_PROBE").as_deref() != Ok("1") {
        return;
    }
    println!("before-revocation");
    std::thread::sleep(Duration::from_millis(150));
    println!("after-revocation");
    eprintln!("stderr-probe");
    std::process::exit(7);
}

#[test]
fn cancelled_command_is_uncertain_and_all_tools_have_human_labels() {
    let (jobs, _receiver) = mpsc::sync_channel(1);
    let ledger = Arc::new(Mutex::new(Ledger::new("epoch".into())));
    let handler = Handler {
        jobs,
        ledger: ledger.clone(),
        access: Arc::new(LiveAccess::new(AccessPolicy::default())),
        services: None,
    };
    {
        let _call = CommandCall::begin(&handler, "get_capabilities");
    }
    assert_eq!(
        ledger.lock().unwrap().commands.rows.back().unwrap().status,
        CommandStatus::Unknown
    );
    for tool in tools() {
        assert_ne!(
            super::super::activity::command_label(&tool.name),
            "Unknown command",
            "{}",
            tool.name
        );
    }
}

#[test]
fn advertised_contracts_have_output_schemas_and_described_input_fields() {
    fn described(schema: &Value, path: &str) {
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (key, value) in properties {
                assert!(
                    value
                        .get("description")
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty()),
                    "Missing description {path}.{key}"
                );
                described(value, &format!("{path}.{key}"));
            }
        }
        if let Some(items) = schema.get("items") {
            described(items, path);
        }
    }
    for tool in tools() {
        assert!(
            tool.output_schema.is_some(),
            "Missing outputSchema for {}",
            tool.name
        );
        described(
            &Value::Object(tool.input_schema.as_ref().clone()),
            &tool.name,
        );
    }
}
#[test]
fn partial_update_rule_is_rejected_before_operation_admission() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut policy=AccessPolicy::default();policy.enabled=true;policy.token="a".repeat(64);
        let ledger=Arc::new(Mutex::new(Ledger::new("epoch".into())));
        let (jobs,_receiver)=mpsc::sync_channel(16);
        let server=HttpServer::start(0,Arc::new(LiveAccess::new(policy)),ledger.clone(),jobs,None).await.unwrap();
        let result=call(server.port,"update_rule",json!({"group_id":"g","rule_id":"r","request_key":"epoch:1","expected_revision":"0","rule":{"name":"Game","path":"C:\\Games\\game.exe","tracked_process_names":["helper.exe"]}}));
        assert_eq!(result["result"]["structuredContent"]["code"],"validation","{result}");
        assert!(result["result"]["structuredContent"]["message"].as_str().unwrap().contains("complete"));
        assert_eq!(ledger.lock().unwrap().next_sequence(policy::LOCAL_AGENT_ID),Some(1));
    });
}

#[test]
fn advertised_schemas_validate_and_fit_the_tool_catalog_budget() {
    let values: Vec<_> = tools()
        .iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    assert!(
        serde_json::to_vec(&json!({"tools":values})).unwrap().len() < MAX_RESPONSE,
        "Tool discovery must fit the existing response budget: {} bytes",
        serde_json::to_vec(&json!({"tools":values})).unwrap().len()
    );
    println!(
        "MCP_TOOL_CATALOG_BYTES={}",
        serde_json::to_vec(&json!({"tools":values})).unwrap().len()
    );
    for tool in tools() {
        let input = Value::Object(tool.input_schema.as_ref().clone());
        jsonschema::validator_for(&input)
            .unwrap_or_else(|e| panic!("{} input schema: {e}", tool.name));
        let output = Value::Object(tool.output_schema.as_ref().unwrap().as_ref().clone());
        jsonschema::validator_for(&output)
            .unwrap_or_else(|e| panic!("{} output schema: {e}", tool.name));
    }
}
fn assert_output(name: &str, value: &Value) {
    super::assert_contract(name, value);
}

#[test]
fn cached_read_responses_and_receipt_variants_match_advertised_contracts() {
    use crate::app::{models::AppStateStorage, runtime::AppState};
    use std::sync::RwLock;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|s| {
                s.process_monitoring_enabled = false;
                s.cpu_load_monitoring_enabled = false;
            }))),
            4,
        );
        let services = super::super::services::Services::from_app(
            &app,
            Default::default(),
            "epoch".into(),
            Default::default(),
        )
        .unwrap();
        for name in [
            "get_app_state",
            "get_topology",
            "list_groups",
            "read_cpu_current",
            "read_system_tweaks",
        ] {
            let result = services.read(name, json!({})).unwrap().unwrap();
            assert_output(name, &result);
            if name == "list_groups" {
                for row in result["rows"].as_array().unwrap() {
                    let rules = services
                        .read("list_rules", json!({"group_id":row["group_id"]}))
                        .unwrap()
                        .unwrap();
                    assert_output("list_rules", &rules);
                }
            }
        }
        assert_output(
            "read_run_cpu",
            &services
                .read("read_run_cpu", json!({"run_id":"missing"}))
                .unwrap()
                .unwrap(),
        );
        assert_output("list_runs", &services.runs.list(policy::LOCAL_AGENT_ID));
        for name in ["list_tweak_profiles", "list_tweak_associations"] {
            assert_output(
                name,
                &services.tweak_configuration_read(name, json!({})).unwrap(),
            );
        }
        for name in ["read_app_logs"] {
            assert_output(
                name,
                &super::super::runtime::gui_read(&mut app, name, json!({})).unwrap(),
            );
        }
        let mut ledger = Ledger::new("epoch".into());
        let id = ledger
            .admit(policy::LOCAL_AGENT_ID, "epoch:1", "update_rule", json!({}))
            .unwrap()
            .id;
        assert_output(
            "get_operation",
            &ledger.receipt(&id, policy::LOCAL_AGENT_ID).unwrap(),
        );
        assert_output(
            "update_rule",
            &ledger.receipt(&id, policy::LOCAL_AGENT_ID).unwrap(),
        );
        ledger.finish(
            &id,
            Ok(json!({"configuration_revision":"2","persistence":"durable"})),
        );
        assert_output(
            "update_rule",
            &ledger.receipt(&id, policy::LOCAL_AGENT_ID).unwrap(),
        );
        assert_output(
            "read_audit_log",
            &ledger.audit(policy::LOCAL_AGENT_ID, None, 0, 50).unwrap(),
        );
        ledger.finish(&id, Err(ApiError::invalid("rejected")));
        assert_output(
            "get_operation",
            &ledger.receipt(&id, policy::LOCAL_AGENT_ID).unwrap(),
        );
    });
}

#[test]
fn http_rule_replacement_preserves_unrelated_settings_and_allows_explicit_clearing() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use crate::app::{runtime::AppState,models::AppStateStorage};use std::sync::RwLock;
        let app=AppState::new_for_test(Arc::new(RwLock::new(AppStateStorage::test_fixture(|s| {s.groups.clear();s.process_monitoring_enabled=false;s.cpu_load_monitoring_enabled=false;}))),4);
        let services=super::super::services::Services::from_app(&app,Default::default(),"epoch".into(),Default::default()).unwrap();
        let mut policy=AccessPolicy::default();policy.enabled=true;policy.token="a".repeat(64);
        let ledger=Arc::new(Mutex::new(Ledger::new("epoch".into())));let(jobs,_receiver)=mpsc::sync_channel(16);
        let server=HttpServer::start(0,Arc::new(LiveAccess::new(policy)),ledger.clone(),jobs,Some(services)).await.unwrap();
        async fn complete(port:u16,tool:&str,args:Value)->Value {
            let first=call(port,tool,args);assert!(!first["result"]["isError"].as_bool().unwrap_or(false),"{first}");
            let id=first["result"]["structuredContent"]["operation"]["id"].as_str().unwrap();
            for _ in 0..100 {
                let reply=call(port,"get_operation",json!({"operation_id":id}));let value=&reply["result"]["structuredContent"];
                assert_output("get_operation",value);assert_output(tool,value);
                let op=&value["operation"];assert!(!["failed","indeterminate"].contains(&op["status"].as_str().unwrap()),"{value}");
                if op["status"]=="completed" {return op["result"].clone();}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }panic!("Unresolved {tool}");
        }
        let group=complete(server.port,"create_group",json!({"request_key":"epoch:1","expected_revision":"0","name":"Games","cpus":[0,1],"run_all":true})).await;
        let group_id=&group["group_id"];
        let body=json!({"name":"Game","path":"C:\\Games\\game.exe","args":["--profile","keep me"],"tracked_process_names":["game.exe"],"manage_descendants":true,"autorun":true,"priority":"High"});
        let created=complete(server.port,"create_rule",json!({"request_key":"epoch:2","expected_revision":group["configuration_revision"],"group_id":group_id,"rule":body})).await;
        let rules=call(server.port,"list_rules",json!({"group_id":group_id}));let before=rules["result"]["structuredContent"]["rows"][0]["app"].clone();
        assert_output("list_rules",&rules["result"]["structuredContent"]);
        let mut replacement=json!({"name":before["name"],"path":before["launch_target"]["Path"]["bin_path"],"args":before["args"],"tracked_process_names":before["additional_processes"],"manage_descendants":before["manage_descendants"],"autorun":before["autorun"],"priority":before["priority"]});
        replacement["tracked_process_names"].as_array_mut().unwrap().push(json!("helper.exe"));
        for field in super::super::services::REPLACEMENT_RULE_FIELDS {
            let mut partial=replacement.clone();partial.as_object_mut().unwrap().remove(*field);
            let reply=call(server.port,"update_rule",json!({"request_key":"epoch:3","expected_revision":created["configuration_revision"],"group_id":group_id,"rule_id":created["rule_id"],"rule":partial}));
            assert_eq!(reply["result"]["structuredContent"]["code"],"validation","{reply}");
            assert_eq!(ledger.lock().unwrap().next_sequence(policy::LOCAL_AGENT_ID),Some(3));
        }
        let saved=complete(server.port,"update_rule",json!({"request_key":"epoch:3","expected_revision":created["configuration_revision"],"group_id":group_id,"rule_id":created["rule_id"],"rule":replacement})).await;
        let after=call(server.port,"list_rules",json!({"group_id":group_id}));let mut expected=before.clone();expected["additional_processes"]=json!(["game.exe","helper.exe"]);
        assert_eq!(after["result"]["structuredContent"]["rows"][0]["app"],expected);
        assert_eq!(after["result"]["structuredContent"]["rows"][0]["rule_id"],created["rule_id"]);
        replacement["tracked_process_names"]=json!([]);replacement["args"]=json!([]);replacement["autorun"]=json!(false);replacement["manage_descendants"]=json!(false);
        complete(server.port,"update_rule",json!({"request_key":"epoch:4","expected_revision":saved["configuration_revision"],"group_id":group_id,"rule_id":created["rule_id"],"rule":replacement})).await;
        let final_state=call(server.port,"list_rules",json!({"group_id":group_id}));let value=&final_state["result"]["structuredContent"]["rows"][0]["app"];
        assert_eq!(value["args"],json!([]));assert_eq!(value["additional_processes"],json!([]));assert_eq!(value["autorun"],false);assert_eq!(value["manage_descendants"],false);assert_eq!(value["priority"],"High");
    });
}

#[test]
fn http_export_downloads_original_process_history_as_utf8_csv() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use crate::app::{models::AppStateStorage, runtime::AppState};
        use std::sync::RwLock;
        let app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|s| {
                s.groups.clear();
                s.process_monitoring_enabled = false;
                s.cpu_load_monitoring_enabled = false;
            }))),
            1,
        );
        let mut services = super::super::services::Services::from_app(
            &app,
            Default::default(),
            "export-test".into(),
            Default::default(),
        )
        .unwrap();
        services.cpu = crate::app::features::cpu_load::CpuReadHandle::history_export_fixture();
        let mut policy = AccessPolicy::default();
        policy.enabled = true;
        policy.token = "a".repeat(64);
        let ledger = Arc::new(Mutex::new(Ledger::new("export-test".into())));
        let (jobs, _) = mpsc::sync_channel(16);
        let server = HttpServer::start(
            0,
            Arc::new(LiveAccess::new(policy)),
            ledger,
            jobs,
            Some(services),
        )
        .await
        .unwrap();
        let first = call(server.port, "export_cpu_history", json!({}));
        assert!(
            !first["result"]["isError"].as_bool().unwrap_or(false),
            "{first}"
        );
        let id = first["result"]["structuredContent"]["export_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut after = "0".to_string();
        let mut csv = String::new();
        let mut polls = 0;
        loop {
            let response = call(
                server.port,
                "export_cpu_history",
                json!({"export_id":id,"after":after,"limit":37}),
            );
            assert!(
                !response["result"]["isError"].as_bool().unwrap_or(false),
                "{response}"
            );
            let data = &response["result"]["structuredContent"];
            assert_output("export_cpu_history", data);
            if data["status"] == "preparing" {
                polls += 1;
                assert!(polls < 200);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                continue;
            }
            csv.push_str(data["text"].as_str().unwrap());
            after = data["next_after"].as_str().unwrap().to_owned();
            if data["eof"] == true {
                break;
            }
        }
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("1970-01-01T00:00:01.000Z"));
        assert!(csv.contains("\"пример.exe\",4242,7,0,12.5,"));
        drop(server);
    });
}

#[test]
#[ignore = "Controlled localhost official-SDK probe; needs its parent-provided port/stop paths and random token"]
fn output_schema_sdk_probe() {
    let port_path = std::path::PathBuf::from(
        std::env::var_os("CPU_AFFINITY_CONTRACT_PORT").expect("probe port path"),
    );
    let stop_path = std::path::PathBuf::from(
        std::env::var_os("CPU_AFFINITY_CONTRACT_STOP").expect("probe stop path"),
    );
    let token = std::env::var("CPU_AFFINITY_CONTRACT_TOKEN").expect("probe random token");
    assert_eq!(token.len(), 64);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut policy = AccessPolicy::default();
        policy.enabled = true;
        policy.token = token;
        let ledger = Arc::new(Mutex::new(Ledger::new("contract".into())));
        {
            let mut l = ledger.lock().unwrap();
            let id = l
                .admit(
                    policy::LOCAL_AGENT_ID,
                    "contract:1",
                    "update_rule",
                    json!({}),
                )
                .unwrap()
                .id;
            l.finish(
                &id,
                Ok(json!({"configuration_revision":"1","persistence":"durable"})),
            );
        }
        let (jobs, _receiver) = mpsc::sync_channel(16);
        let server = HttpServer::start(0, Arc::new(LiveAccess::new(policy)), ledger, jobs, None)
            .await
            .unwrap();
        std::fs::write(&port_path, server.port.to_string()).unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        while !stop_path.exists() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
}
