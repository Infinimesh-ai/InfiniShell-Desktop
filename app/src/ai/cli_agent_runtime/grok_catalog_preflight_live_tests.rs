//! 固定 Grok 的 SDK 目录零输入预检；真实名称只投影为固定集合的逐项匹配。

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{GrokProtocol, connect_protocol, validate_options};
use crate::ai::cli_agent_runtime::local_tools::LocalToolPermissions;
use crate::ai::cli_agent_runtime::{
    PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeError, RuntimeEventKind,
    SessionOptions, SessionTarget, managed_process,
};

const SCOPE: &str = "grok_sdk_catalog_zero_input";
const NAMES: [&str; 6] = [
    "read_file",
    "search_tool",
    "use_tool",
    "infinishell-local-tasks__inspect_local_tasks",
    "infinishell-local-tasks__run_agents",
    "infinishell-local-tasks__send_message_to_agent",
];
const FIXED_BINARY_SHA256: &str =
    "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb";

#[derive(Default)]
struct State {
    catalogs: usize,
    builtin_seen: bool,
    union_seen: bool,
    matching: [bool; 6],
    duplicate_count: usize,
    unknown_count: usize,
    non_string_count: usize,
    served_names_exact: bool,
    requests: usize,
    responses: usize,
    rejected: usize,
    reverse_ids: HashMap<String, Value>,
    initialized_session: Option<String>,
    pull_request: Option<Value>,
    pull_written: bool,
    pull_replied: bool,
    pull_verified: bool,
    acu_union_seen: bool,
}

#[derive(Clone, Default)]
pub(super) struct CatalogProbe(Arc<Mutex<State>>);

impl CatalogProbe {
    pub(super) fn poll_pull(&self, session: &str, served: &[String]) -> Option<Value> {
        let mut state = self.0.lock().expect("预检状态锁未损坏");
        if state.initialized_session.as_deref() != Some(session)
            || state.pull_request.is_some()
            || served.len() != 3
            || !NAMES[3..]
                .iter()
                .all(|name| served.iter().any(|served| served == name))
        {
            return None;
        }
        let request = json!({"jsonrpc":"2.0","id":"infinishell-catalog-preflight-pull",
            "method":"_x.ai/commands/list","params":{"sessionId":session}});
        state.pull_request = Some(request.clone());
        Some(request)
    }

    pub(super) fn observe_written(&self, message: &Value) {
        let mut state = self.0.lock().expect("预检状态锁未损坏");
        if state.pull_request.as_ref() == Some(message) {
            state.pull_written = true;
        }
    }

    pub(super) fn pull_response(
        &self,
        message: &Value,
        session: Option<&str>,
    ) -> Result<Option<Value>, RuntimeError> {
        if message["id"] != "infinishell-catalog-preflight-pull" {
            return Ok(None);
        }
        let mut state = self.0.lock().expect("预检状态锁未损坏");
        if !state.pull_written
            || state.pull_replied
            || state.initialized_session.as_deref() != session
            || message["jsonrpc"] != "2.0"
            || message.get("method").is_some()
            || message.get("error").is_some()
            || !message["result"]["tools"].is_array()
        {
            state.rejected += 1;
            return Err(RuntimeError::Protocol("目录预检只读响应未通过关联".into()));
        }
        state.pull_replied = true;
        Ok(Some(message["result"]["tools"].clone()))
    }

    pub(super) fn observe_pull(&self, tools: Value, served: &[String]) {
        let acu_seen = self.0.lock().expect("预检状态锁未损坏").acu_union_seen;
        self.observe_catalog(&json!({"update":{"_meta":{"tools":tools}}}), served);
        let mut state = self.0.lock().expect("预检状态锁未损坏");
        state.pull_verified = state.union_seen && state.served_names_exact;
        state.acu_union_seen = acu_seen;
    }

    pub(super) fn guard_write(&self, message: &Value) -> Result<(), RuntimeError> {
        let mut state = self.0.lock().expect("预检状态锁未损坏");
        let allowed = if let Some(method) = message["method"].as_str() {
            state.requests += 1;
            state.requests <= 8
                && (matches!(
                    method,
                    "initialize" | "authenticate" | "session/new" | "session/close"
                ) || (method == "_x.ai/commands/list"
                    && state.pull_request.as_ref() == Some(message)))
        } else {
            state.responses += 1;
            state.responses <= 8
                && state.reverse_ids.contains_key(&message["id"].to_string())
                && message["result"].is_object()
        };
        if allowed {
            Ok(())
        } else {
            state.rejected += 1;
            Err(RuntimeError::Protocol("目录零输入预检禁止该写入".into()))
        }
    }

    pub(super) fn guard_receive(&self, message: &Value) -> Result<(), RuntimeError> {
        let mut state = self.0.lock().expect("预检状态锁未损坏");
        let method = message["method"].as_str();
        if message["method"] == "_x.ai/mcp_initialized"
            && message["params"]["mcpToolCount"] == 3
            && message["params"]["sessionId"]
                .as_str()
                .is_some_and(super::valid_native_id)
        {
            let session = message["params"]["sessionId"].as_str().expect("已校验会话");
            if state
                .initialized_session
                .as_deref()
                .is_some_and(|old| old != session)
            {
                state.rejected += 1;
                return Err(RuntimeError::Protocol(
                    "目录预检的 MCP 初始化会话改变".into(),
                ));
            }
            state.initialized_session = Some(session.into());
        }
        let allowed = if method.is_some() && message.get("id").is_some() {
            let inner = &message["params"]["message"];
            let allowed = matches!(method, Some("_x.ai/mcp/sdk_call" | "x.ai/mcp/sdk_call"))
                && matches!(
                    inner["method"].as_str(),
                    Some("initialize" | "server/discover" | "tools/list" | "ping")
                )
                && state.reverse_ids.len() < 8;
            if allowed {
                let key = message["id"].to_string();
                if state
                    .reverse_ids
                    .get(&key)
                    .is_some_and(|old| old != message)
                {
                    false
                } else {
                    state.reverse_ids.insert(key, message.clone());
                    true
                }
            } else {
                false
            }
        } else {
            !matches!(
                message["params"]["update"]["sessionUpdate"].as_str(),
                Some(
                    "tool_call"
                        | "tool_call_update"
                        | "agent_message_chunk"
                        | "agent_thought_chunk"
                        | "user_message_chunk"
                        | "turn_started"
                        | "turn_completed"
                )
            )
        };
        if allowed {
            Ok(())
        } else {
            state.rejected += 1;
            Err(RuntimeError::Protocol(
                "目录零输入预检拒绝反向业务事件".into(),
            ))
        }
    }

    pub(super) fn observe_catalog(&self, params: &Value, served: &[String]) {
        let mut state = self.0.lock().expect("预检状态锁未损坏");
        state.catalogs += 1;
        let Some(items) = params["update"]["_meta"]["tools"].as_array() else {
            return;
        };
        let mut counts = [0; 6];
        let mut seen = HashSet::new();
        let mut unknown = 0;
        let mut non_string = 0;
        let mut duplicates = 0;
        for item in items {
            if let Some(name) = item.as_str() {
                duplicates += usize::from(!seen.insert(name));
                if let Some(index) = NAMES.iter().position(|expected| *expected == name) {
                    counts[index] += 1;
                } else {
                    unknown += 1;
                }
            } else {
                non_string += 1;
            }
        }
        state.duplicate_count += duplicates;
        state.unknown_count += unknown;
        state.non_string_count += non_string;
        state.builtin_seen |= items.len() == 3 && counts[..3] == [1; 3];
        let served_exact = served.len() == 3
            && NAMES[3..].iter().all(|expected| {
                served
                    .iter()
                    .filter(|actual| actual.as_str() == *expected)
                    .count()
                    == 1
            });
        let exact = items.len() == 6 && counts == [1; 6];
        state.union_seen |= exact && served_exact;
        state.acu_union_seen |= exact && served_exact;
        state.served_names_exact |= served_exact;
        if exact {
            state.matching = [true; 6];
        }
    }

    fn matched(&self) -> bool {
        let state = self.0.lock().expect("预检状态锁未损坏");
        state.union_seen
            && state.served_names_exact
            && state.rejected == 0
            && state.unknown_count == 0
            && state.non_string_count == 0
            && state.duplicate_count == 0
    }

    fn report(&self) -> Value {
        let state = self.0.lock().expect("预检状态锁未损坏");
        let matches: serde_json::Map<String, Value> = NAMES
            .iter()
            .zip(state.matching)
            .map(|(name, found)| ((*name).to_owned(), json!(found)))
            .collect();
        json!({"event":"catalog_observed","scope":SCOPE,"catalogs":state.catalogs,
            "builtin_seen":state.builtin_seen,"union_seen":state.union_seen,"name_matches":matches,
            "duplicate_count":state.duplicate_count,"unknown_count":state.unknown_count,
            "non_string_count":state.non_string_count,"served_names_exact":state.served_names_exact,
            "outbound_request_attempts":state.requests,"outbound_response_attempts":state.responses,
            "guard_rejections":state.rejected,"pull_verified":state.pull_verified,"acu_union_seen":state.acu_union_seen})
    }
}

fn record(file: &mut File, value: Value) {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .expect("安全证据写入失败");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "固定 macOS 原生路径与隔离官方认证；零用户输入，只验证 SDK 目录"]
async fn grok_sdk_catalog_zero_input() {
    assert!(cfg!(target_os = "macos"));
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("需要隔离运行器"))
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-catalog-probe")).unwrap(),
        SCOPE
    );
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-live-probe")).unwrap(),
        "isolated Grok Rust adapter verification\n"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_AUTH_MODE").unwrap(),
        "official-cached-token"
    );
    let executable = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").unwrap());
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(&executable).unwrap())),
        FIXED_BINARY_SHA256
    );
    let artifact = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").unwrap());
    assert_eq!(artifact, root.join("private-evidence.ndjson"));
    let mut file = File::create(artifact).unwrap();
    record(
        &mut file,
        json!({"event":"catalog_preflight_started","scope":SCOPE,
        "max_native_inputs":0,"production_prepare":true,"test_argv_override":false}),
    );
    let options = SessionOptions {
        executable,
        cwd: root.join("project"),
        state_dir: root.join("state"),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::GrokRestrictedReadV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: Some(LocalToolPermissions {
            allow_project_commands: false,
            allow_spawn: true,
            allow_message: true,
        }),
        selected_skills: Vec::new(),
    };
    validate_options(&options).unwrap();
    let generation = options.generation;
    let state_dir = options.state_dir.clone();
    let probe = CatalogProbe::default();
    let mut protocol = GrokProtocol::new(options);
    protocol.catalog_probe_for_live = Some(probe.clone());
    let connection = connect_protocol(protocol);
    let controller = connection.controller;
    let mut events = connection.events;
    let mut task = tokio::spawn(connection.task);
    let mut ready = false;
    let mut policy = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let verified = loop {
        tokio::select! {
            _ = tick.tick() => {
                if ready && probe.matched() { break true; }
                if tokio::time::Instant::now() >= deadline { break false; }
            }
            event = events.recv() => {
                let Some(event) = event else { break false; };
                if event.generation != generation { break false; }
                match event.kind {
                    RuntimeEventKind::SessionReady { effective_permissions, .. } => {
                        if ready || effective_permissions["appCreationPolicyApplied"] != true { break false; }
                        ready = true;
                        policy = effective_permissions["grokCreationPolicyV1"]["storageId"].as_str().map(str::to_owned);
                    }
                    RuntimeEventKind::CommandDispatched { .. } => {}
                    RuntimeEventKind::MessageAccepted { .. } | RuntimeEventKind::TurnStarted { .. }
                    | RuntimeEventKind::TextDelta { .. } | RuntimeEventKind::Progress { .. }
                    | RuntimeEventKind::ApprovalRequested { .. } | RuntimeEventKind::ApprovalResolved { .. }
                    | RuntimeEventKind::TurnFinished { .. } | RuntimeEventKind::ApprovalCancelled { .. }
                    | RuntimeEventKind::LocalToolCancelled { .. } | RuntimeEventKind::LocalToolRequested { .. }
                    | RuntimeEventKind::InputJoined { .. } | RuntimeEventKind::RequestFailed { .. }
                    | RuntimeEventKind::Disconnected { .. } => { break false; }
                }
            }
        }
    };
    let _ = controller
        .send(RuntimeCommand {
            generation,
            message_id: Uuid::new_v4(),
            action: RuntimeAction::Shutdown,
        })
        .await;
    let joined = tokio::time::timeout(Duration::from_secs(35), &mut task).await;
    let transport_closed = matches!(&joined, Ok(Ok(Ok(()))));
    if joined.is_err() {
        task.abort();
        let _ = task.await;
    }
    let cleanup = managed_process::confirmed_exit(&state_dir, generation)
        .ok()
        .flatten()
        .is_some_and(|receipt| receipt.cleanup_confirmed);
    let auth_removed = policy.is_some_and(|id| {
        !state_dir
            .join("grok-managed")
            .join(id)
            .join("grok/auth.json")
            .exists()
    });
    let passed = verified && transport_closed && cleanup && auth_removed;
    record(&mut file, probe.report());
    record(
        &mut file,
        json!({"event":"catalog_preflight_finished","scope":SCOPE,"passed":passed,
        "ready":ready,"native_inputs":0,"business_tool_dispatches":0,"transport_closed":transport_closed,
        "cleanup_confirmed":cleanup,"managed_auth_removed":auth_removed,
        "full_cli_parity_acceptance_passed":false}),
    );
    assert!(
        passed,
        "目录零输入预检未通过；仅查看安全投影，禁止补发模型输入"
    );
}

#[test]
fn zero_input_guard_rejects_prompt_business_tools_and_approval_before_write() {
    for message in [
        json!({"id":1,"method":"session/prompt"}),
        json!({"id":2,"method":"session/cancel"}),
        json!({"id":3,"result":{"outcome":"approved"}}),
    ] {
        assert!(CatalogProbe::default().guard_write(&message).is_err());
    }
    for message in [
        json!({"id":1,"method":"session/request_permission"}),
        json!({"id":2,"method":"_x.ai/mcp/sdk_call","params":{"message":{"method":"tools/call"}}}),
        json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call"}}}),
    ] {
        assert!(CatalogProbe::default().guard_receive(&message).is_err());
    }
}

#[test]
fn safe_catalog_report_requires_each_qualified_name_and_actual_served_set() {
    let probe = CatalogProbe::default();
    let params = json!({"update":{"_meta":{"tools":NAMES}}});
    probe.observe_catalog(&params, &[]);
    assert!(!probe.matched());
    probe.observe_catalog(
        &params,
        &NAMES[3..]
            .iter()
            .map(|name| (*name).into())
            .collect::<Vec<_>>(),
    );
    assert!(probe.matched());
    let bad = CatalogProbe::default();
    let mut params = params;
    params["update"]["_meta"]["tools"][3] = json!("PRIVATE_UNTRUSTED_NAME");
    bad.observe_catalog(&params, &[]);
    assert!(!bad.matched());
    assert!(!bad.report().to_string().contains("PRIVATE_UNTRUSTED_NAME"));
    assert_eq!(bad.report()["unknown_count"], 1);
}

#[test]
fn catalog_pull_is_once_bound_to_initialized_session_served_names_and_actual_write() {
    let probe = CatalogProbe::default();
    let served: Vec<String> = NAMES[3..].iter().map(|name| (*name).into()).collect();
    assert!(probe.poll_pull("native-session", &served).is_none());
    probe
        .guard_receive(&json!({"jsonrpc":"2.0","method":"_x.ai/mcp_initialized",
        "params":{"sessionId":"native-session","mcpToolCount":3}}))
        .unwrap();
    assert!(probe.poll_pull("old-session", &served).is_none());
    assert!(probe.poll_pull("native-session", &[]).is_none());
    let request = probe.poll_pull("native-session", &served).unwrap();
    assert!(probe.poll_pull("native-session", &served).is_none());
    probe.guard_write(&request).unwrap();
    let response = json!({"jsonrpc":"2.0","id":request["id"],"result":{"tools":NAMES}});
    assert!(
        probe
            .pull_response(&response, Some("native-session"))
            .is_err()
    );
    probe.observe_written(&request);
    assert!(probe.pull_response(&response, Some("old-session")).is_err());
    let tools = probe
        .pull_response(&response, Some("native-session"))
        .unwrap()
        .unwrap();
    probe.observe_pull(tools, &served);
    assert_eq!(probe.report()["pull_verified"], true);
    assert_eq!(probe.report()["acu_union_seen"], false);
    assert!(
        probe
            .pull_response(&response, Some("native-session"))
            .is_err()
    );
}
