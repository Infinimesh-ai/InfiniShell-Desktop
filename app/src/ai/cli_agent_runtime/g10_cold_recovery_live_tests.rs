//! 固定 Claude 父子技能链在待审批点跨进程冷恢复的真实验收。

use std::env;
use std::fs::{self, File};
use std::io::Write as _;

use ai::skills::parse_skill;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::{DirectoryWatcher, RepoMetadataModel};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
use crate::terminal::cli_agent::{
    CLIAgentInstallModel, CLIAgentInstallation, CLIAgentVersionStatus,
};
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;

const SCOPE: &str = "real_claude_g10_skill_parent_child_cold_recovery";
const MARKER: &str = "G10_COLD_SKILL_BODY_09c7fa62";
type Wake = Option<(LocalCliTask, RuntimeEvent)>;

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn root() -> PathBuf {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("缺少隔离工作目录"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-g10-cold-probe")).unwrap(),
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
    let auth_home = PathBuf::from(env::var_os("HOME").unwrap())
        .canonicalize()
        .unwrap();
    assert!(!auth_home.starts_with(&root));
    let profile = env::var("INFINISHELL_CLAUDE_LIVE_STATE_PROFILE").unwrap();
    let nonce = profile
        .strip_prefix("claude-g10-cold-")
        .expect("缺少独立数据 profile");
    assert_eq!(nonce.len(), 32);
    assert!(
        nonce
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert_eq!(env::var("WARP_DATA_PROFILE").unwrap(), profile);
    let state = current_state_dir();
    assert!(state.starts_with(&auth_home) && !state.starts_with(&root));
    assert!(state.to_string_lossy().contains(&profile));
    root
}

fn artifact(root: &Path) -> File {
    let path = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").unwrap())
        .canonicalize()
        .unwrap();
    assert!(path.starts_with(root));
    File::create(path).unwrap()
}

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(
        file,
        "{}",
        serde_json::to_string(&value).map_err(|error| error.to_string())?
    )
    .map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}

fn skill(root: &Path) -> SelectedLocalSkill {
    let path = root.join("project/.claude/skills/alpha/SKILL.md");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(
        &path,
        format!("---\nname: alpha\ndescription: Isolated cold recovery probe.\n---\nReply with the exact marker {MARKER}. Do not use tools, commands, or other skills.\n"),
    )
    .unwrap();
    SelectedLocalSkill {
        name: "alpha".into(),
        path,
    }
}

fn app_models(app: &mut App, selected: &SelectedLocalSkill) {
    app.add_singleton_model(DirectoryWatcher::new);
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
    skills.update(app, |manager, _| {
        manager.handle_skills_added(vec![parse_skill(&selected.path).unwrap()])
    });
}

fn subscribe(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
) -> mpsc::UnboundedReceiver<Wake> {
    let (wake, receiver) = mpsc::unbounded_channel();
    app.update(|ctx| {
        ctx.subscribe_to_model(coordinator, move |_, event, _| {
            let runtime = if let LocalCLITaskCoordinatorEvent::Runtime { task, event } = event {
                Some((task.clone(), event.clone()))
            } else {
                None
            };
            let _ = wake.send(runtime);
        })
    });
    receiver
}

async fn approve(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    task: &LocalCliTask,
    approval_id: &str,
) -> Result<(), String> {
    coordinator
        .update(app, |model, _| {
            model.request_for_generation(
                &task.task_id,
                task.generation,
                Uuid::new_v4(),
                RuntimeAction::RespondApproval {
                    approval_id: approval_id.to_owned(),
                    decision: ApprovalDecision::AllowOnce,
                },
            )
        })?
        .with_timeout(Duration::from_secs(20))
        .await
        .map_err(|_| "真实审批确认超时")?
        .map_err(|_| "真实审批确认通道关闭")?
}

async fn records(sender: &SyncSender<ModelEvent>) -> Result<Vec<LocalCliTask>, String> {
    load_tasks(sender, false)?
        .await
        .map_err(|_| "任务记录确认通道关闭")?
}

async fn messages(
    sender: &SyncSender<ModelEvent>,
    task_id: &str,
) -> Result<Vec<LocalCliMessage>, String> {
    load_task_messages(sender, task_id.to_owned())?
        .await
        .map_err(|_| "任务消息确认通道关闭")?
}

async fn wait_runtime(
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    deadline: Instant,
) -> Result<Wake, String> {
    receiver
        .recv()
        .with_timeout(
            Duration::from_secs(2).min(deadline.saturating_duration_since(Instant::now())),
        )
        .await
        .map_err(|_| "运行事件暂未到达")?
        .ok_or("运行事件通道关闭".into())
}

async fn prepare(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    root: &Path,
    selected: &SelectedLocalSkill,
    file: &mut File,
) -> Result<(), String> {
    let model = env::var("INFINISHELL_CLAUDE_LIVE_MODEL").map_err(|_| "缺少固定模型")?;
    let parent_id = Uuid::new_v4().to_string();
    let child_label = format!("G10_COLD_CHILD_{}", Uuid::new_v4().simple());
    let child_prompt = format!(
        "This is an isolated parent-child cold recovery test. Invoke the native Skill tool for infinishell-local-skills:alpha exactly once. The host will pause at the permission request, then restart and allow it. After reading the allowed skill body, reply with its exact marker followed by {child_label}. Do not spawn agents, run commands, edit files, or load another skill."
    );
    let spawn = json!({"summary":"isolated G10 cold recovery",
        "base_prompt":"", "harness":"claude", "model_id":model,
        "skills":[selected.path],
        "agent_run_configs":[{"name":"cold-skill","prompt":child_prompt}]});
    let prompt = format!(
        "Call run_agents exactly once with these arguments: {spawn}. The child handles the fixed Skill test. Do not load skills yourself, send messages, inspect tasks, run commands, or spawn another child. After the tool reports one queued child, reply only PARENT_QUEUED."
    );
    let cwd = root
        .join("project")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少固定 CLI")?,
        ),
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
        selected_skills: vec![selected.clone()],
    };
    let task = LocalCliTask {
        version: 1,
        task_id: parent_id.clone(),
        parent_task_id: None,
        parent_generation: None,
        harness: "claude".into(),
        working_directory: cwd.to_string_lossy().into(),
        config_json: json!({"model":model,"permission_policy":"ClaudeRestrictedSkillsV1"})
            .to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    coordinator.update(app, |model, ctx| {
        model.start_with_input(
            task,
            options,
            None,
            Uuid::new_v4(),
            vec![
                InputContent::Text(prompt),
                InputContent::Skill {
                    name: selected.name.clone(),
                    path: selected.path.clone(),
                },
            ],
            ctx,
        )
    })?;
    record(
        file,
        json!({"event":"parent_started","parent_task_id":parent_id}),
    )?;

    let deadline = Instant::now() + Duration::from_secs(420);
    let mut parent_approved = false;
    let mut child_id = None;
    loop {
        if Instant::now() >= deadline {
            return Err("待审批父子链超过 420 秒".into());
        }
        if let Some(error) = coordinator.read(app, |model, _| {
            model.snapshots().find_map(|item| item.error.clone())
        }) {
            return Err(format!("协调器报告错误：{error}"));
        }
        let wake = match wait_runtime(receiver, deadline).await {
            Ok(wake) => wake,
            Err(error) if error == "运行事件暂未到达" => continue,
            Err(error) => return Err(error),
        };
        let Some((task, event)) = wake else { continue };
        if task.task_id != parent_id {
            if task.parent_task_id.as_deref() != Some(&parent_id)
                || child_id.as_ref().is_some_and(|id| id != &task.task_id)
            {
                return Err("出现额外或身份不符的子任务".into());
            }
            child_id = Some(task.task_id.clone());
        }
        if let RuntimeEventKind::ApprovalRequested {
            approval_id,
            method,
            details,
            ..
        } = event.kind
        {
            if method != "can_use_tool" {
                return Err("原生审批类型不符".into());
            }
            if task.task_id == parent_id {
                if parent_approved
                    || details["tool_name"] != "mcp__infinishell-local-tasks__run_agents"
                    || details["input"] != spawn
                {
                    return Err("父任务请求了额外或不符的工具".into());
                }
                approve(app, coordinator, &task, &approval_id).await?;
                parent_approved = true;
                record(
                    file,
                    json!({"event":"parent_spawn_approved","approval_sha256":hash(&approval_id)}),
                )?;
            } else {
                if !parent_approved
                    || details["tool_name"] != "Skill"
                    || details["input"]["skill"] != "infinishell-local-skills:alpha"
                    || details["tool_use_id"].as_str().is_none_or(str::is_empty)
                {
                    return Err("子任务待审批技能不符".into());
                }
                let saved = records(sender).await?;
                let parent = saved
                    .iter()
                    .find(|item| item.task_id == parent_id)
                    .ok_or("父任务未落盘")?;
                let child = saved
                    .iter()
                    .find(|item| item.task_id == task.task_id)
                    .ok_or("子任务未落盘")?;
                let ceiling =
                    ceiling_from_parent(parent, "claude").map_err(|error| error.to_string())?;
                let child_config: Value =
                    serde_json::from_str(&child.config_json).map_err(|error| error.to_string())?;
                if saved.len() != 2
                    || child.parent_task_id.as_deref() != Some(&parent_id)
                    || child.parent_generation != Some(1)
                    || child_config["permission_ceiling"] != json!(ceiling)
                    || child.native_session_id.is_none()
                    || parent.native_session_id.is_none()
                    || child.native_session_id == parent.native_session_id
                {
                    return Err("待审批父子持久身份与上限不符".into());
                }
                let checkpoint = json!({"scope":SCOPE,"parent_task_id":parent_id,
                    "child_task_id":child.task_id,"child_approval_id":approval_id,
                    "parent_native_session_id":parent.native_session_id,
                    "child_native_session_id":child.native_session_id,
                    "parent_generation":parent.generation,"child_generation":child.generation,
                    "child_label":child_label,
                    "parent_user_inputs":messages(sender, &parent_id).await?.iter().filter(|row| row.subject == "user_input").count(),
                    "child_user_inputs":messages(sender, &child.task_id).await?.iter().filter(|row| row.subject == "user_input").count()});
                fs::write(
                    root.join("pending-checkpoint.raw.json"),
                    serde_json::to_vec_pretty(&checkpoint).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                record(
                    file,
                    json!({"event":"child_skill_pending","parent_task_id":parent_id,
                    "child_task_id":child.task_id,"approval_sha256":hash(&approval_id),
                    "parent_native_sha256":parent.native_session_id.as_deref().map(hash),
                    "child_native_sha256":child.native_session_id.as_deref().map(hash),
                    "parent_ceiling_saved":true,"child_skill_unresolved":true}),
                )?;
                return Ok(());
            }
        }
    }
}

async fn cleanup(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    file: &mut File,
) -> Result<(), String> {
    let snapshots = coordinator.read(app, |model, _| {
        model.snapshots().cloned().collect::<Vec<_>>()
    });
    for snapshot in &snapshots {
        if snapshot.connected {
            let reply = coordinator.update(app, |model, _| {
                model.request(
                    &snapshot.task.task_id,
                    Uuid::new_v4(),
                    RuntimeAction::Shutdown,
                )
            });
            if let Ok(reply) = reply {
                let _ = reply.with_timeout(Duration::from_secs(10)).await;
            }
        }
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    while coordinator.read(app, |model, _| {
        model.snapshots().any(|snapshot| snapshot.connected)
    }) {
        if Instant::now() >= deadline {
            return Err("父子宿主关闭超过 90 秒".into());
        }
        let _ = wait_runtime(receiver, deadline).await;
    }
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
                return Err("父子原生退出回执超时".into());
            }
            Timer::after(Duration::from_millis(50)).await;
        };
        if receipt.native_process != NativeProcessCompletion::Exited
            || receipt.native_cleanup_sha256.is_none()
            || !receipt.adapter_task_terminated
            || !receipt.adapter_succeeded
            || !receipt.event_journal_completed
        {
            return Err("父子原生退出回执不完整".into());
        }
        record(
            file,
            json!({"event":"cleanup_confirmed","task_id":snapshot.task.task_id,
            "native_cleanup_sha256":receipt.native_cleanup_sha256}),
        )?;
    }
    Ok(())
}

async fn recover(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    root: &Path,
    file: &mut File,
) -> Result<(), String> {
    let checkpoint: Value = serde_json::from_slice(
        &fs::read(root.join("pending-checkpoint.raw.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if checkpoint["scope"] != SCOPE {
        return Err("恢复检查点范围不符".into());
    }
    let parent_id = checkpoint["parent_task_id"]
        .as_str()
        .ok_or("缺少父任务 ID")?;
    let child_id = checkpoint["child_task_id"]
        .as_str()
        .ok_or("缺少子任务 ID")?;
    let approval_id = checkpoint["child_approval_id"]
        .as_str()
        .ok_or("缺少待审批 ID")?;
    let child_label = checkpoint["child_label"].as_str().ok_or("缺少子结果标记")?;
    let before = records(sender).await?;
    if before.len() != 2 {
        return Err("冷恢复前任务数不符".into());
    }
    coordinator.update(app, |model, ctx| model.refresh_records(ctx));
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if Instant::now() >= deadline {
            return Err("父子宿主冷重接超过 120 秒".into());
        }
        if let Some(error) =
            coordinator.read(app, |model, _| model.recovery_error().map(str::to_owned))
        {
            return Err(format!("冷恢复失败：{error}"));
        }
        let ready = coordinator.read(app, |model, _| {
            let parent = model.snapshot(parent_id);
            let child = model.snapshot(child_id);
            (
                model.recovery_complete,
                parent.cloned(),
                child.cloned(),
                model.restored_tasks().len(),
            )
        });
        if ready.0
            && ready.1.as_ref().is_some_and(|item| item.connected)
            && ready.2.as_ref().is_some_and(|item| item.connected)
        {
            let parent = ready.1.unwrap();
            let child = ready.2.unwrap();
            let approval = child
                .approvals
                .iter()
                .find(|item| item.approval_id == approval_id)
                .ok_or("冷恢复没有保留子技能待审批")?;
            let saved = records(sender).await?;
            let source = saved
                .iter()
                .find(|item| item.task_id == parent_id)
                .ok_or("父记录丢失")?;
            let target = saved
                .iter()
                .find(|item| item.task_id == child_id)
                .ok_or("子记录丢失")?;
            let ceiling =
                ceiling_from_parent(source, "claude").map_err(|error| error.to_string())?;
            let child_config: Value =
                serde_json::from_str(&target.config_json).map_err(|error| error.to_string())?;
            if ready.3 != 2
                || saved.len() != 2
                || parent.task.native_session_id
                    != checkpoint["parent_native_session_id"]
                        .as_str()
                        .map(str::to_owned)
                || child.task.native_session_id
                    != checkpoint["child_native_session_id"]
                        .as_str()
                        .map(str::to_owned)
                || child.task.generation
                    != checkpoint["child_generation"].as_i64().unwrap_or_default()
                || child.task.parent_task_id.as_deref() != Some(parent_id)
                || child_config["permission_ceiling"] != json!(ceiling)
                || approval.method != "can_use_tool"
                || approval.details["tool_name"] != "Skill"
                || approval.details["input"]["skill"] != "infinishell-local-skills:alpha"
                || messages(sender, parent_id)
                    .await?
                    .iter()
                    .filter(|row| row.subject == "user_input")
                    .count()
                    != checkpoint["parent_user_inputs"]
                        .as_u64()
                        .unwrap_or_default() as usize
                || messages(sender, child_id)
                    .await?
                    .iter()
                    .filter(|row| row.subject == "user_input")
                    .count()
                    != checkpoint["child_user_inputs"].as_u64().unwrap_or_default() as usize
            {
                return Err("冷恢复身份、待审批、父上限或重投断言不符".into());
            }
            record(
                file,
                json!({"event":"cold_reattach_verified","parent_task_id":parent_id,
                "child_task_id":child_id,"same_native_sessions":true,"same_pending_approval":true,
                "parent_ceiling_saved":true,"user_inputs_not_replayed":true,
                "app_process_restart_verified":true}),
            )?;
            approve(app, coordinator, &child.task, approval_id).await?;
            record(
                file,
                json!({"event":"recovered_skill_approved","approval_sha256":hash(approval_id)}),
            )?;
            break;
        }
        match wait_runtime(receiver, deadline).await {
            Ok(_) => {}
            Err(error) if error == "运行事件暂未到达" => {}
            Err(error) => return Err(error),
        }
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut resolved = false;
    let mut finished = false;
    loop {
        if Instant::now() >= deadline {
            return Err("恢复后技能结果超过 180 秒".into());
        }
        if let Some(error) = coordinator.read(app, |model, _| {
            model.snapshots().find_map(|item| item.error.clone())
        }) {
            return Err(format!("恢复后协调器错误：{error}"));
        }
        let wake = match wait_runtime(receiver, deadline).await {
            Ok(wake) => wake,
            Err(error) if error == "运行事件暂未到达" => continue,
            Err(error) => return Err(error),
        };
        if let Some((task, event)) = wake {
            if task.task_id == child_id {
                if let RuntimeEventKind::ApprovalResolved {
                    approval_id: seen,
                    decision,
                } = &event.kind
                    && seen == approval_id
                    && *decision == ApprovalDecision::AllowOnce
                {
                    resolved = true;
                }
                if let RuntimeEventKind::TurnFinished {
                    outcome, output, ..
                } = &event.kind
                    && *outcome == TurnOutcome::Completed
                    && output.contains(MARKER)
                    && output.contains(child_label)
                {
                    finished = true;
                }
                if matches!(event.kind, RuntimeEventKind::ApprovalRequested { .. }) {
                    return Err("恢复后出现额外技能审批".into());
                }
                if matches!(event.kind, RuntimeEventKind::LocalToolRequested { .. }) {
                    return Err("恢复后子任务请求了本地工具".into());
                }
            }
        }
        if resolved && finished {
            let saved = records(sender).await?;
            let child = saved
                .iter()
                .find(|item| item.task_id == child_id)
                .ok_or("子任务持久记录丢失")?;
            if child.state != LocalCliTaskState::Completed
                || child
                    .result
                    .as_ref()
                    .is_none_or(|value| !value.contains(MARKER))
                || child
                    .result
                    .as_ref()
                    .is_none_or(|value| !value.contains(child_label))
            {
                continue;
            }
            record(
                file,
                json!({"event":"recovered_skill_result_verified","child_task_id":child_id,
                "native_skill_resolved":true,"saved_result_contains_marker":true,
                "new_child_count":0}),
            )?;
            return Ok(());
        }
    }
}

#[test]
#[ignore = "仅由固定 Claude 已认证隔离运行器执行，调用真实模型并产生费用"]
fn real_claude_g10_cold_prepare_pending_skill() {
    let root = root();
    let state = current_state_dir();
    assert_eq!(
        fs::symlink_metadata(&state).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    fs::create_dir_all(&state).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        state.join(super::super::claude::NATIVE_RESULT_EVIDENCE_MARKER),
        format!("{SCOPE}\n"),
    )
    .unwrap();
    let selected = skill(&root);
    let mut file = artifact(&root);
    record(
        &mut file,
        json!({"event":"acceptance_started","scope":SCOPE,"phase":"prepare",
        "real_gui_verified":false}),
    )
    .unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let _bundled = FeatureFlag::BundledSkills.override_enabled(false);
        app_models(&mut app, &selected);
        let writer =
            crate::persistence::start_test_writer(&root.join("coordinator.sqlite")).unwrap();
        let coordinator =
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
        let mut receiver = subscribe(&mut app, &coordinator);
        let result = prepare(
            &mut app,
            &coordinator,
            &writer.sender,
            &mut receiver,
            &root,
            &selected,
            &mut file,
        )
        .await;
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result {
            Ok(()) => record(
                &mut file,
                json!({"event":"prepare_passed","native_hosts_intentionally_live":true}),
            )
            .unwrap(),
            Err(error) => {
                record(
                    &mut file,
                    json!({"event":"prepare_failed","reason_sha256":hash(&error)}),
                )
                .unwrap();
                panic!("G10 冷恢复准备阶段未通过；检查私有证据");
            }
        }
    });
}

#[test]
#[ignore = "仅由固定 Claude 已认证隔离运行器执行，调用真实模型并产生费用"]
fn real_claude_g10_cold_recover_pending_skill() {
    let root = root();
    assert!(current_state_dir().exists());
    let selected = SelectedLocalSkill {
        name: "alpha".into(),
        path: root.join("project/.claude/skills/alpha/SKILL.md"),
    };
    assert!(selected.path.is_file());
    let mut file = artifact(&root);
    record(
        &mut file,
        json!({"event":"acceptance_started","scope":SCOPE,"phase":"recover",
        "real_gui_verified":false}),
    )
    .unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let _bundled = FeatureFlag::BundledSkills.override_enabled(false);
        app_models(&mut app, &selected);
        let writer =
            crate::persistence::start_test_writer(&root.join("coordinator.sqlite")).unwrap();
        let coordinator =
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
        let mut receiver = subscribe(&mut app, &coordinator);
        let result = recover(
            &mut app,
            &coordinator,
            &writer.sender,
            &mut receiver,
            &root,
            &mut file,
        )
        .await;
        let cleaned = cleanup(&mut app, &coordinator, &mut receiver, &mut file).await;
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result.and(cleaned) {
            Ok(()) => record(
                &mut file,
                json!({"event":"acceptance_passed","scope":SCOPE,
                "app_process_restart_verified":true,"same_native_sessions":true,
                "same_pending_approval":true,"parent_ceiling_saved":true,
                "saved_skill_result_verified":true,"all_runtime_hosts_cleaned":true,
                "real_gui_verified":false}),
            )
            .unwrap(),
            Err(error) => {
                record(
                    &mut file,
                    json!({"event":"acceptance_failed","reason_sha256":hash(&error)}),
                )
                .unwrap();
                panic!("G10 冷恢复验收未通过；检查私有证据");
            }
        }
    });
}

#[test]
#[ignore = "仅供 G10 冷恢复隔离运行器在失败后重接并清理本轮宿主"]
fn real_claude_g10_cold_cleanup_after_failure() {
    let root = root();
    let mut file = artifact(&root);
    record(
        &mut file,
        json!({"event":"failure_cleanup_started","scope":SCOPE}),
    )
    .unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let writer =
            crate::persistence::start_test_writer(&root.join("coordinator.sqlite")).unwrap();
        let coordinator =
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
        let mut receiver = subscribe(&mut app, &coordinator);
        coordinator.update(&mut app, |model, ctx| model.refresh_records(ctx));
        let deadline = Instant::now() + Duration::from_secs(120);
        let result = loop {
            if Instant::now() >= deadline {
                break Err("失败轮宿主重接超过 120 秒".to_owned());
            }
            let status = coordinator.read(&app, |model, _| {
                (
                    model.recovery_complete,
                    model.recovery_error().map(str::to_owned),
                    model.unconfirmed_hosts.len(),
                )
            });
            if let Some(error) = status.1 {
                break Err(format!("失败轮宿主重接错误：{error}"));
            }
            if status.0 {
                if status.2 != 0 {
                    break Err("失败轮含未确认宿主，禁止声称清理".to_owned());
                }
                if let Err(error) = cleanup(&mut app, &coordinator, &mut receiver, &mut file).await
                {
                    break Err(error);
                }
                let saved = match records(&writer.sender).await {
                    Ok(saved) => saved,
                    Err(error) => break Err(error),
                };
                let mut verified = 0;
                let mut failure = None;
                for task in saved {
                    let config: Value = match serde_json::from_str(&task.config_json) {
                        Ok(config) => config,
                        Err(error) => {
                            failure = Some(error.to_string());
                            break;
                        }
                    };
                    let Some(generation) = config["runtime_generation"].as_str() else {
                        continue;
                    };
                    let generation = match Uuid::parse_str(generation) {
                        Ok(generation) => generation,
                        Err(error) => {
                            failure = Some(error.to_string());
                            break;
                        }
                    };
                    match confirmed_exit(&current_state_dir(), generation) {
                        Ok(Some(receipt)) => {
                            if receipt.native_process == NativeProcessCompletion::Exited
                                && receipt.native_cleanup_sha256.is_some()
                                && receipt.adapter_task_terminated
                                && receipt.adapter_succeeded
                                && receipt.event_journal_completed
                            {
                                verified += 1;
                            } else {
                                failure = Some("失败轮存在不完整的原生退出回执".to_owned());
                                break;
                            }
                        }
                        Ok(None) => {
                            failure = Some("失败轮存在未确认退出的原生宿主".to_owned());
                            break;
                        }
                        Err(error) => {
                            failure = Some(error.to_string());
                            break;
                        }
                    }
                }
                if let Some(error) = failure {
                    break Err(error);
                }
                record(
                    &mut file,
                    json!({"event":"failure_cleanup_receipts_verified",
                    "runtime_count":verified}),
                )
                .unwrap();
                break Ok(());
            }
            let _ = wait_runtime(&mut receiver, deadline).await;
        };
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result {
            Ok(()) => record(
                &mut file,
                json!({"event":"failure_cleanup_passed","scope":SCOPE,
                "all_recovered_hosts_exited":true}),
            )
            .unwrap(),
            Err(error) => {
                record(
                    &mut file,
                    json!({"event":"failure_cleanup_unconfirmed",
                    "reason_sha256":hash(&error)}),
                )
                .unwrap();
                panic!("G10 失败轮原生宿主未确认退出；保留隔离现场");
            }
        }
    });
}
