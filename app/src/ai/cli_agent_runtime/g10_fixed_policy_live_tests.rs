//! 固定 Claude 技能父子链的真实审批与父上限验收；只由隔离运行器显式执行。

use std::env;
use std::fs::{self, File};
use std::io::Write as _;

use ai::skills::parse_skill;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::{DirectoryWatcher, RepoMetadataModel};
use serde_json::{Value, json};
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};
use watcher::HomeDirectoryWatcher;

use super::*;
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::local_tools::LocalToolPermissions;
use crate::ai::cli_agent_runtime::permissions::ceiling_from_parent;
use crate::ai::cli_agent_runtime::runtime_host::{NativeProcessCompletion, confirmed_exit};
use crate::ai::cli_agent_runtime::{ApprovalDecision, current_state_dir};
use crate::ai::skills::SkillManager;
use crate::settings::AISettings;
use crate::terminal::cli_agent::{
    CLIAgentInstallModel, CLIAgentInstallation, CLIAgentVersionStatus,
};
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;

const SCOPE: &str = "real_claude_g10_fixed_skill_parent_child";
const MARKER: &str = "G10_SKILL_BODY_7f3b8e24";
type Wake = Option<(LocalCliTask, RuntimeEvent)>;

struct Evidence {
    file: File,
    root: PathBuf,
}

impl Evidence {
    fn record(&mut self, value: Value) -> Result<(), String> {
        let encoded = serde_json::to_string(&value)
            .map_err(|error| error.to_string())?
            .replace(&*self.root.to_string_lossy(), "<probe-root>");
        writeln!(self.file, "{encoded}").map_err(|error| error.to_string())?;
        self.file.flush().map_err(|error| error.to_string())
    }
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn record_runtime(
    evidence: &mut Evidence,
    task: &LocalCliTask,
    event: &RuntimeEvent,
) -> Result<(), String> {
    let kind = if matches!(event.kind, RuntimeEventKind::SessionReady { .. }) {
        "session_ready"
    } else if matches!(event.kind, RuntimeEventKind::TurnStarted { .. }) {
        "turn_started"
    } else if matches!(event.kind, RuntimeEventKind::ApprovalRequested { .. }) {
        "approval_requested"
    } else if matches!(event.kind, RuntimeEventKind::ApprovalResolved { .. }) {
        "approval_resolved"
    } else if matches!(event.kind, RuntimeEventKind::TurnFinished { .. }) {
        "turn_finished"
    } else if matches!(event.kind, RuntimeEventKind::Disconnected { .. }) {
        "disconnected"
    } else {
        return Ok(());
    };
    let output_sha256 = if let RuntimeEventKind::TurnFinished { output, .. } = &event.kind {
        Some(digest(output))
    } else {
        None
    };
    evidence.record(json!({"event":"runtime_state","task_id":task.task_id,
        "generation":task.generation,"kind":kind,
        "native_session_sha256":event.native_session_id.as_deref().map(digest),
        "output_sha256":output_sha256}))
}

fn write_skill(project: &Path, name: &str, marker: &str) -> SelectedLocalSkill {
    let directory = project.join(".claude/skills").join(name);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("SKILL.md");
    fs::write(
        &path,
        format!(
            "---\nname: {name}\ndescription: Isolated G10 approval probe.\n---\n\
             Reply with the exact marker {marker}. Do not use tools, commands, or other skills.\n"
        ),
    )
    .unwrap();
    SelectedLocalSkill {
        name: name.into(),
        path,
    }
}

async fn control(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    task: &LocalCliTask,
    action: RuntimeAction,
) -> Result<(), String> {
    coordinator
        .update(app, |model, _| {
            model.request_for_generation(&task.task_id, task.generation, Uuid::new_v4(), action)
        })?
        .await
        .map_err(|_| "真实控制确认已关闭".to_owned())?
}

async fn drive(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    root: &Path,
    alpha: &SelectedLocalSkill,
    beta: &SelectedLocalSkill,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let model =
        env::var("INFINISHELL_CLAUDE_LIVE_MODEL").map_err(|_| "G10 验收必须固定模型".to_owned())?;
    let executable =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少固定 CLI")?);
    let cwd = root
        .join("project")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let parent_id = Uuid::new_v4().to_string();
    let child_label = format!("G10_CHILD_FINISHED_{}", Uuid::new_v4().simple());
    let child_prompt = format!(
        "This is an isolated fixed-skill approval test. Invoke the native Skill tool for \
         infinishell-local-skills:alpha twice in order. The host will deny the first request. \
         Accept the denial, do not change policy or use another tool, then make exactly one \
         new Skill request for the same skill. The host will allow that second request. \
         After reading the allowed skill body, reply with its exact marker followed by {child_label}. \
         Do not spawn agents, run commands, edit files, or load another skill."
    );
    let spawn = json!({"summary":"isolated G10 fixed skill approval",
        "base_prompt":"", "harness":"claude", "model_id":model,
        "skills":[alpha.path],
        "agent_run_configs":[{"name":"fixed-skill","prompt":child_prompt}]});
    let prompt = format!(
        "Call run_agents exactly once with these arguments: {spawn}. The child handles the \
         fixed-skill test. Do not load skills yourself, send messages, inspect tasks, run \
         commands, or spawn another child. After the tool reports one queued child, reply only PARENT_QUEUED."
    );
    let options = SessionOptions {
        executable,
        cwd: cwd.clone(),
        state_dir: current_state_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::ClaudeRestrictedSkillsV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: Some(model.clone()),
        local_tools: Some(LocalToolPermissions {
            allow_project_commands: false,
            allow_spawn: true,
            allow_message: false,
        }),
        selected_skills: vec![alpha.clone()],
    };
    let task = LocalCliTask {
        version: 1,
        task_id: parent_id.clone(),
        parent_task_id: None,
        parent_generation: None,
        harness: "claude".into(),
        working_directory: cwd.to_string_lossy().into(),
        config_json: json!({"model":model,
            "permission_policy":"ClaudeRestrictedSkillsV1"})
        .to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    coordinator.update(app, |model, ctx| model.start(task, options, None, ctx))?;
    evidence.record(json!({"event":"parent_started","scope":SCOPE,
        "parent_task_id":parent_id,"selected_skill":"alpha",
        "skill_body_marker_sha256":format!("{:x}", Sha256::digest(MARKER))}))?;

    let deadline = Instant::now() + Duration::from_secs(450);
    let mut submitted = false;
    let mut parent_spawn_approved = false;
    let mut child_id = None;
    let mut child_turn = None;
    let mut first_denied = false;
    let mut second_allowed = false;
    let mut denied_id = None;
    let mut allowed_id = None;
    let mut resolved_denial = false;
    let mut resolved_allow = false;
    let mut child_finished = false;
    let mut pre_allow_text = String::new();
    let mut approvals = HashSet::new();
    let mut events_seen = 0;
    loop {
        if Instant::now() >= deadline {
            return Err("G10 父子技能整链超过 450 秒".into());
        }
        let wake = receiver
            .recv()
            .with_timeout(
                Duration::from_secs(2).min(deadline.saturating_duration_since(Instant::now())),
            )
            .await;
        let wake = match wake {
            Ok(Some(wake)) => wake,
            Ok(None) => return Err("G10 协调器事件通道已关闭".into()),
            Err(_) => continue,
        };
        if let Some((task, event)) = wake {
            events_seen += 1;
            if events_seen > 512 {
                return Err("G10 原生事件超过验收预算".into());
            }
            record_runtime(evidence, &task, &event)?;
            let is_parent = task.task_id == parent_id;
            if !is_parent {
                if task.parent_task_id.as_deref() != Some(&parent_id)
                    || child_id.as_ref().is_some_and(|id| id != &task.task_id)
                {
                    return Err("出现父上限之外或重复的子任务".into());
                }
                child_id = Some(task.task_id.clone());
            }
            if is_parent
                && !submitted
                && matches!(event.kind, RuntimeEventKind::SessionReady { .. })
            {
                submitted = true;
                control(
                    app,
                    coordinator,
                    &task,
                    RuntimeAction::Submit {
                        input: vec![InputContent::Text(prompt.clone())],
                    },
                )
                .await?;
                evidence.record(json!({"event":"parent_input_submitted",
                    "parent_task_id":parent_id}))?;
            }
            if !is_parent {
                if let RuntimeEventKind::TurnStarted { turn_id } = &event.kind {
                    if child_turn.replace(turn_id.clone()).is_some() {
                        return Err("子任务启动了额外回合".into());
                    }
                }
                if let RuntimeEventKind::TextDelta { text, .. } = &event.kind {
                    if !resolved_allow {
                        pre_allow_text.push_str(text);
                        if pre_allow_text.contains(MARKER) {
                            return Err("原生允许确认前已出现技能正文标记".into());
                        }
                        if pre_allow_text.len() > 8192 {
                            return Err("技能允许前的模型输出超过验收预算".into());
                        }
                    }
                }
            }
            if let RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } = &event.kind
            {
                if !approvals.insert((task.task_id.clone(), approval_id.clone())) {
                    return Err("原生审批 ID 重复".into());
                }
                if method != "can_use_tool" || approvals.len() > 3 {
                    return Err("原生审批类型或次数越界".into());
                }
                let decision = if is_parent {
                    if parent_spawn_approved
                        || details["tool_name"] != "mcp__infinishell-local-tasks__run_agents"
                        || details["input"] != spawn
                    {
                        return Err("父任务请求了非唯一固定子任务".into());
                    }
                    parent_spawn_approved = true;
                    ApprovalDecision::AllowOnce
                } else {
                    if details["tool_name"] != "Skill"
                        || child_turn.as_deref() != Some(turn_id)
                        || details["input"]["skill"] != "infinishell-local-skills:alpha"
                        || details["input"].as_object().is_none_or(|input| {
                            input
                                .keys()
                                .any(|key| !matches!(key.as_str(), "skill" | "args"))
                        })
                        || details["input"].get("args").is_some_and(|args| args != "")
                        || details["tool_use_id"].as_str().is_none_or(str::is_empty)
                    {
                        return Err("子任务请求了固定技能之外的工具或参数".into());
                    }
                    if !first_denied {
                        first_denied = true;
                        denied_id = Some(approval_id.clone());
                        ApprovalDecision::DenyOnce
                    } else if !second_allowed {
                        second_allowed = true;
                        allowed_id = Some(approval_id.clone());
                        ApprovalDecision::AllowOnce
                    } else {
                        return Err("子任务重复请求技能审批".into());
                    }
                };
                evidence.record(json!({"event":"native_approval_decision",
                    "task_id":task.task_id,"approval_id_sha256":digest(approval_id),
                    "turn_id_sha256":digest(turn_id),
                    "tool_use_id_sha256":details["tool_use_id"].as_str().map(digest),
                    "tool_name":details["tool_name"],
                    "input_sha256":digest(&details["input"].to_string()),
                    "exact_scope_verified":true,"decision":decision}))?;
                control(
                    app,
                    coordinator,
                    &task,
                    RuntimeAction::RespondApproval {
                        approval_id: approval_id.clone(),
                        decision,
                    },
                )
                .await?;
            }
            if !is_parent {
                if let RuntimeEventKind::ApprovalResolved {
                    approval_id,
                    decision,
                } = &event.kind
                {
                    if *decision == ApprovalDecision::DenyOnce
                        && denied_id.as_ref() == Some(approval_id)
                    {
                        resolved_denial = true;
                    } else if *decision == ApprovalDecision::AllowOnce
                        && allowed_id.as_ref() == Some(approval_id)
                    {
                        resolved_allow = true;
                    } else {
                        return Err("子任务审批决定与原生请求身份不一致".into());
                    }
                }
                if let RuntimeEventKind::TurnFinished {
                    turn_id,
                    outcome,
                    output,
                } = &event.kind
                {
                    if child_turn.as_deref() != Some(turn_id)
                        || *outcome != TurnOutcome::Completed
                        || !output.contains(MARKER)
                        || !output.contains(&child_label)
                    {
                        return Err("子任务没有完成已批准技能结果".into());
                    }
                    child_finished = true;
                }
                if matches!(event.kind, RuntimeEventKind::LocalToolRequested { .. }) {
                    return Err("子任务请求了非技能本地工具".into());
                }
            }
        }
        if !(submitted
            && parent_spawn_approved
            && first_denied
            && second_allowed
            && resolved_denial
            && resolved_allow
            && child_finished)
        {
            continue;
        }
        let saved = load_tasks(sender, false)?
            .await
            .map_err(|_| "任务读取确认已关闭")??;
        let Some(parent) = saved.iter().find(|task| task.task_id == parent_id) else {
            continue;
        };
        let Some(child) = child_id
            .as_ref()
            .and_then(|id| saved.iter().find(|task| &task.task_id == id))
        else {
            continue;
        };
        if child.state != LocalCliTaskState::Completed
            || parent.state != LocalCliTaskState::Completed
        {
            continue;
        }
        let parent_config: Value =
            serde_json::from_str(&parent.config_json).map_err(|error| error.to_string())?;
        let child_config: Value =
            serde_json::from_str(&child.config_json).map_err(|error| error.to_string())?;
        let ceiling = ceiling_from_parent(parent, "claude").map_err(|error| error.to_string())?;
        let profile = ceiling.claude_profile().ok_or("父任务缺少固定技能上限")?;
        let overbound = profile
            .derive_child(&[beta.clone()])
            .err()
            .ok_or("父上限意外允许未选定技能")?;
        let evidence_overbound = overbound
            .permission_ceiling_evidence()
            .ok_or("越界拒绝缺少父上限证据")?;
        if saved.len() != 2
            || child.parent_task_id.as_deref() != Some(&parent_id)
            || child.parent_generation != Some(1)
            || parent.native_session_id.is_none()
            || child.native_session_id.is_none()
            || parent.native_session_id == child.native_session_id
            || child_config["permission_policy"] != "ClaudeRestrictedSkillsV1"
            || child_config["permission_ceiling"] != json!(ceiling)
            || child_config["claude_profile"] != parent_config["claude_profile"]
            || child_config["effective_permissions"]["claudeRestrictedSkillsV1"]
                != parent_config["claude_profile"]
            || child_config["effective_permissions"]["fixedProfileVerified"] != true
            || parent
                .result
                .as_ref()
                .is_none_or(|result| !result.contains("PARENT_QUEUED"))
            || child
                .result
                .as_ref()
                .is_none_or(|result| !result.contains(MARKER))
            || evidence_overbound["reason"] != "claude_skills_parent_ceiling"
        {
            return Err("固定技能父子身份、结果或上限证据不符".into());
        }
        evidence.record(json!({"event":"g10_saved_chain_verified",
            "parent_task_id":parent_id,"child_task_id":child.task_id,
            "parent_native_session_sha256":parent.native_session_id.as_deref().map(digest),
            "child_native_session_sha256":child.native_session_id.as_deref().map(digest),
            "parent_ceiling_saved":true,"child_profile_subset":true,
            "skill_deny_resolved":true,"skill_allow_resolved":true,
            "child_result_contains_marker":true,"parent_ceiling_derive_rejected":true,
            "parent_ceiling_rejection_reason":evidence_overbound["reason"]}))?;
        return Ok(());
    }
}

async fn shutdown(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let ids = coordinator.read(app, |model, _| {
        model
            .snapshots()
            .filter(|snapshot| snapshot.connected)
            .map(|snapshot| snapshot.task.task_id.clone())
            .collect::<Vec<_>>()
    });
    for id in ids {
        let reply = coordinator.update(app, |model, _| {
            model.request(&id, Uuid::new_v4(), RuntimeAction::Shutdown)
        });
        if let Ok(reply) = reply {
            let _ = reply.with_timeout(Duration::from_secs(5)).await;
        }
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while coordinator.read(app, |model, _| {
        model.snapshots().any(|snapshot| snapshot.connected)
    }) {
        receiver
            .recv()
            .with_timeout(deadline.saturating_duration_since(Instant::now()))
            .await
            .map_err(|_| "G10 协调器清理未确认")?
            .ok_or("G10 清理事件通道关闭")?;
    }
    let snapshots = coordinator.read(app, |model, _| {
        model.snapshots().cloned().collect::<Vec<_>>()
    });
    for snapshot in snapshots {
        let config: Value =
            serde_json::from_str(&snapshot.task.config_json).map_err(|error| error.to_string())?;
        let generation: Uuid = serde_json::from_value(config["runtime_generation"].clone())
            .map_err(|error| error.to_string())?;
        let receipt = loop {
            if let Some(receipt) = confirmed_exit(&current_state_dir(), generation)
                .map_err(|error| error.to_string())?
            {
                break receipt;
            }
            if Instant::now() >= deadline {
                return Err("G10 父子原生进程缺少退出回执".into());
            }
            Timer::after(Duration::from_millis(50)).await;
        };
        if receipt.native_process != NativeProcessCompletion::Exited
            || receipt.native_cleanup_sha256.is_none()
            || !receipt.adapter_task_terminated
            || !receipt.adapter_succeeded
            || !receipt.event_journal_completed
        {
            return Err("G10 父子监督清理未确认".into());
        }
        evidence.record(json!({"event":"cleanup_confirmed",
            "task_id":snapshot.task.task_id,"receipt":receipt}))?;
    }
    Ok(())
}

#[test]
#[ignore = "仅由固定 Claude 2.1.280 已认证隔离运行器执行，调用真实模型并产生费用"]
fn real_claude_g10_fixed_skill_parent_child() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-g10-skill-probe")).unwrap(),
        SCOPE
    );
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION").unwrap(),
        "2.1.280"
    );
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_LIVE_AUTH_MODE").unwrap(),
        "authorized_default_account"
    );
    assert!(env::var_os("CLAUDE_CONFIG_DIR").is_none());
    assert!(env::var_os("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR").is_none());
    let auth_home = PathBuf::from(env::var_os("HOME").expect("缺少认证 HOME"))
        .canonicalize()
        .unwrap();
    assert!(!auth_home.starts_with(&root));
    let profile = env::var("INFINISHELL_CLAUDE_LIVE_STATE_PROFILE").unwrap();
    let nonce = profile
        .strip_prefix("claude-g10-skill-")
        .expect("G10 必须使用本轮独立 profile");
    assert_eq!(nonce.len(), 32);
    assert!(
        nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(env::var("WARP_DATA_PROFILE").unwrap(), profile);
    let state = current_state_dir();
    assert!(state.starts_with(&auth_home));
    assert!(!state.starts_with(&root));
    let state_parent = state.parent().expect("G10 数据目录缺少父目录");
    assert_eq!(state_parent.canonicalize().unwrap().as_path(), state_parent);
    assert!(state_parent.starts_with(&auth_home));
    assert!(
        state
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(&format!("-{profile}")))
    );
    assert_eq!(
        fs::symlink_metadata(&state).unwrap_err().kind(),
        std::io::ErrorKind::NotFound,
        "G10 数据 profile 已存在，不能复用或修改旧目录"
    );
    fs::create_dir_all(&state).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        state.join(super::super::claude::NATIVE_RESULT_EVIDENCE_MARKER),
        format!("{SCOPE}\n"),
    )
    .unwrap();
    let artifact =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").expect("缺少私有证据路径"))
            .canonicalize()
            .unwrap();
    assert!(artifact.starts_with(&root));
    let project = root.join("project");
    let alpha = write_skill(&project, "alpha", MARKER);
    let beta = write_skill(&project, "beta", "G10_UNSELECTED_BODY_4a1e2b6c");
    let mut evidence = Evidence {
        file: File::create(artifact).unwrap(),
        root: root.clone(),
    };
    evidence
        .record(json!({"event":"acceptance_started","scope":SCOPE,
        "cli_version":"2.1.280","real_gui_verified":false,
        "max_native_children":1,"max_native_skill_approvals":2,
        "http_request_count_verified":false}))
        .unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let _bundled = FeatureFlag::BundledSkills.override_enabled(false);
        app.add_singleton_model(DirectoryWatcher::new);
        app.add_singleton_model(AISettings::new_with_defaults);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let executable = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").unwrap());
        app.add_singleton_model(|_| {
            CLIAgentInstallModel::with_installation_for_test(
                CLIAgent::Claude,
                CLIAgentInstallation {
                    executable: Some(executable),
                    version: CLIAgentVersionStatus::Detected("2.1.280".into()),
                },
            )
        });
        let skills = app.add_singleton_model(SkillManager::new);
        skills.update(&mut app, |manager, _| {
            manager.handle_skills_added(vec![
                parse_skill(&alpha.path).unwrap(),
                parse_skill(&beta.path).unwrap(),
            ])
        });
        let writer =
            crate::persistence::start_test_writer(&root.join("coordinator.sqlite")).unwrap();
        let coordinator =
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
        let (wake, mut receiver) = mpsc::unbounded_channel();
        app.update(|ctx| {
            ctx.subscribe_to_model(&coordinator, move |_, event, _| {
                let event = if let LocalCLITaskCoordinatorEvent::Runtime { task, event } = event {
                    Some((task.clone(), event.clone()))
                } else {
                    None
                };
                let _ = wake.send(event);
            })
        });
        let result = drive(
            &mut app,
            &coordinator,
            &writer.sender,
            &mut receiver,
            &root,
            &alpha,
            &beta,
            &mut evidence,
        )
        .await;
        let cleanup = shutdown(&mut app, &coordinator, &mut receiver, &mut evidence).await;
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result.and(cleanup) {
            Ok(()) => evidence
                .record(json!({"event":"acceptance_passed","scope":SCOPE,
                "parent_child_ceiling_verified":true,"native_skill_deny_verified":true,
                "native_skill_allow_verified":true,"parent_ceiling_derive_rejected":true,
                "all_runtime_hosts_cleaned":true,"real_gui_verified":false}))
                .unwrap(),
            Err(error) => {
                evidence
                    .record(json!({"event":"acceptance_failed","reason_sha256":digest(&error)}))
                    .unwrap();
                panic!("真实 Claude G10 固定技能父子验收未通过；检查隔离证据");
            }
        }
    });
}
