//! 固定 Claude V2 父子链的待审批 Write 取消；保留真实原生终态和监督清理收据。

use super::*;

const SCOPE: &str = "real_claude_production_v2_write_cancel";
const CONTENT: &str = "V2_WRITE_CANCEL_MUST_NOT_APPLY";

async fn control(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    task: &LocalCliTask,
    message_id: Uuid,
    action: RuntimeAction,
) -> Result<Result<(), String>, String> {
    coordinator
        .update(app, |model, _| {
            model.request_for_generation(&task.task_id, task.generation, message_id, action)
        })?
        .await
        .map_err(|_| "受审控制请求确认已关闭".to_owned())
}

async fn drive_cancel(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    root: &Path,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let parent_id = Uuid::new_v4().to_string();
    let model = env::var("INFINISHELL_CLAUDE_LIVE_MODEL")
        .map_err(|_| "V2 取消验收必须固定模型".to_owned())?;
    let executable =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少固定 CLI")?);
    let cwd = fs::canonicalize(root.join("project")).map_err(|error| error.to_string())?;
    let target = cwd.join("cancelled-write.txt");
    if target.exists() || target.is_symlink() {
        return Err("取消目标文件开始前必须不存在".into());
    }
    let marker = format!("CANCEL_PARENT_{}", Uuid::new_v4().simple());
    let child_prompt = format!(
        "This is an isolated approval cancellation test. Request the Write tool exactly once \
         with file_path {} and content exactly {:?}. Wait for the host decision. Do not retry, \
         read, edit, run shell commands, use skills, or spawn another agent.",
        target.display(),
        CONTENT,
    );
    let spawn = json!({"summary":"isolated pending Write cancellation", "base_prompt":"",
        "harness":"claude", "model_id":model, "skills":[],
        "agent_run_configs":[{"name":"pending-write","prompt":child_prompt}]});
    let prompt = format!(
        "Call run_agents exactly once with {spawn}. The child will wait for a host reviewed \
         Write. Do not send messages, use other tools, or edit files. Once the child is queued, \
         reply only {marker}. If a cancellation result later arrives, do not start new work."
    );
    let options = SessionOptions {
        executable,
        cwd: cwd.clone(),
        state_dir: current_state_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::ClaudeRestrictedFilesV2,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: Some(model.clone()),
        local_tools: Some(LocalToolPermissions {
            allow_project_commands: false,
            allow_spawn: true,
            allow_message: false,
        }),
        selected_skills: Vec::new(),
    };
    let task = LocalCliTask {
        version: 1,
        task_id: parent_id.clone(),
        parent_task_id: None,
        parent_generation: None,
        harness: "claude".into(),
        working_directory: cwd.to_string_lossy().into(),
        config_json: json!({"model":model,"permission_policy":"ClaudeRestrictedFilesV2"})
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
            vec![InputContent::Text(prompt)],
            ctx,
        )
    })?;
    evidence.record(json!({"event":"parent_started","parent_task_id":parent_id,
        "policy":"ClaudeRestrictedFilesV2","target_absent":true,
        "target_ref":"cancelled-write.txt","content_sha256":format!("{:x}", Sha256::digest(CONTENT))}))?;

    let deadline = Instant::now() + Duration::from_secs(360);
    let mut child_id = None;
    let mut parent_tool_approved = false;
    let mut spawn_called = false;
    let mut child_turn_started = None;
    let mut pending_approval: Option<(String, String, String, i64)> = None;
    let mut interrupt_id = None;
    let mut interrupt_ack = false;
    let mut approval_cancelled = false;
    let mut late_id = None;
    let mut late_rejected = false;
    let mut child_cancelled = false;
    let mut child_native_id = None;
    let mut parent_native_id = None;
    let mut events_seen = 0;
    while !(child_cancelled && late_rejected && interrupt_ack) {
        if Instant::now() >= deadline {
            return Err("V2 父子待审批取消超过 360 秒".into());
        }
        if pending_approval.is_none() {
            let startup_error = coordinator.read(app, |model, _| {
                model
                    .snapshots()
                    .find_map(|snapshot| snapshot.error.clone())
            });
            if let Some(error) = startup_error {
                return Err(format!("真实原生任务在 Write 待审批前启动失败：{error}"));
            }
        }
        let wake = receiver
            .recv()
            .with_timeout(
                Duration::from_secs(2).min(deadline.saturating_duration_since(Instant::now())),
            )
            .await;
        let wake = match wake {
            Ok(Some(wake)) => wake,
            Ok(None) => return Err("V2 父子事件通道已关闭".into()),
            Err(_) => continue,
        };
        let Some((task, event)) = wake else {
            continue;
        };
        events_seen += 1;
        if events_seen > 512 {
            return Err("真实父子原生事件超过验收预算".into());
        }
        evidence.record(json!({"event":"runtime","task":task,"runtime":event}))?;
        let is_parent = task.task_id == parent_id;
        if is_parent {
            parent_native_id = event.native_session_id.clone().or(parent_native_id);
        } else if task.parent_task_id.as_deref() == Some(&parent_id) {
            if child_id.as_ref().is_some_and(|id| id != &task.task_id) {
                return Err("派生了不止一个子任务".into());
            }
            child_id = Some(task.task_id.clone());
            child_native_id = event.native_session_id.clone().or(child_native_id);
        } else {
            return Err("收到无关任务事件".into());
        }
        match &event.kind {
            RuntimeEventKind::TurnStarted { turn_id } if !is_parent => {
                if child_turn_started.replace(turn_id.clone()).is_some() {
                    return Err("子任务启动了额外原生执行".into());
                }
                evidence.record(json!({"event":"child_turn_started","task_id":task.task_id,
                    "turn_id":turn_id,"native_session_id":event.native_session_id}))?;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } if is_parent => {
                if parent_tool_approved
                    || method != "can_use_tool"
                    || details["tool_name"] != "mcp__infinishell-local-tasks__run_agents"
                    || details["input"]["harness"] != "claude"
                    || details["input"]["model_id"] != model
                    || details["input"]["agent_run_configs"]
                        .as_array()
                        .is_none_or(|configs| configs.len() != 1)
                {
                    return Err("父任务请求了范围外工具或参数".into());
                }
                parent_tool_approved = true;
                evidence.record(json!({"event":"parent_spawn_approved",
                    "approval_id":approval_id,"turn_id":turn_id,"decision":"AllowOnce"}))?;
                let result = control(
                    app,
                    coordinator,
                    &task,
                    Uuid::new_v4(),
                    RuntimeAction::RespondApproval {
                        approval_id: approval_id.clone(),
                        decision: ApprovalDecision::AllowOnce,
                    },
                )
                .await?;
                result?;
            }
            RuntimeEventKind::LocalToolRequested { request } if is_parent => {
                if spawn_called || request.tool != "run_agents" {
                    return Err("父任务没有唯一真实派发调用".into());
                }
                spawn_called = true;
                evidence.record(
                    json!({"event":"parent_spawn_called","call_id":request.call_id,
                    "turn_id":request.turn_id,"tool":"run_agents"}),
                )?;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } if !is_parent => {
                if !spawn_called
                    || child_turn_started.as_deref() != Some(turn_id)
                    || pending_approval.is_some()
                    || method != "can_use_tool"
                    || details["tool_name"] != "Write"
                    || details["input"] != json!({"file_path":target,"content":CONTENT})
                    || details["tool_use_id"].as_str().is_none_or(|id| {
                        !id.starts_with("toolu_")
                            || id.len() > 160
                            || !id.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                            })
                    })
                    || target.exists()
                {
                    return Err("子任务未在唯一精确 Write 待审批点停住".into());
                }
                pending_approval = Some((
                    approval_id.clone(),
                    turn_id.clone(),
                    task.task_id.clone(),
                    task.generation,
                ));
                evidence.record(json!({"event":"child_write_pending","task_id":task.task_id,
                    "generation":task.generation,"approval_id":approval_id,"turn_id":turn_id,
                    "tool_use_id":details["tool_use_id"],"exact_write_verified":true,
                    "target_absent":true,"write_allowed":false}))?;
                let id = Uuid::new_v4();
                interrupt_id = Some(id);
                control(
                    app,
                    coordinator,
                    &task,
                    id,
                    RuntimeAction::Interrupt {
                        turn_id: turn_id.clone(),
                    },
                )
                .await??;
                evidence.record(
                    json!({"event":"child_interrupt_submitted", "task_id":task.task_id,
                    "generation":task.generation,"turn_id":turn_id,"message_id":id,
                    "approval_id":approval_id,"write_allowed":false}),
                )?;
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } if !is_parent => {
                if Some(*message_id) == interrupt_id {
                    if interrupt_ack
                        || pending_approval
                            .as_ref()
                            .is_none_or(|(_, pending_turn, _, _)| {
                                turn_id.as_deref() != Some(pending_turn)
                            })
                    {
                        return Err("取消 ACK 与子执行不匹配".into());
                    }
                    interrupt_ack = true;
                    evidence.record(
                        json!({"event":"child_interrupt_ack","message_id":message_id,
                        "turn_id":turn_id,"native_session_id":event.native_session_id}),
                    )?;
                }
            }
            RuntimeEventKind::ApprovalCancelled { approval_id } if !is_parent => {
                let Some((pending_id, pending_turn, pending_task, pending_generation)) =
                    &pending_approval
                else {
                    return Err("子审批未请求就被撤销".into());
                };
                if approval_cancelled
                    || approval_id != pending_id
                    || task.task_id != *pending_task
                    || task.generation != *pending_generation
                {
                    return Err("撤销的审批 ID 或任务代次错误".into());
                }
                approval_cancelled = true;
                evidence.record(json!({"event":"child_write_approval_cancelled",
                    "approval_id":approval_id,"turn_id":pending_turn,
                    "native_session_id":event.native_session_id,"target_absent":!target.exists()}))?;
                if target.exists() {
                    return Err("撤销时目标文件已被创建".into());
                }
                let id = Uuid::new_v4();
                late_id = Some(id);
                control(
                    app,
                    coordinator,
                    &task,
                    id,
                    RuntimeAction::RespondApproval {
                        approval_id: approval_id.clone(),
                        decision: ApprovalDecision::AllowOnce,
                    },
                )
                .await??;
                evidence.record(json!({"event":"late_allow_submitted",
                    "approval_id":approval_id,"message_id":id,
                    "decision":"AllowOnce","same_cancelled_request_id":true}))?;
            }
            RuntimeEventKind::RequestFailed {
                message_id,
                message,
            } if !is_parent => {
                if Some(*message_id) != late_id || !approval_cancelled || late_rejected {
                    return Err("非迟到允许控制请求失败".into());
                }
                late_rejected = true;
                evidence.record(json!({"event":"late_allow_rejected",
                    "approval_id":pending_approval.as_ref().map(|pending| &pending.0),
                    "message_id":message_id,"reason_sha256":format!("{:x}", Sha256::digest(message)),
                    "native_request_failed":true,"target_absent":!target.exists()}))?;
                if target.exists() {
                    return Err("迟到允许后目标文件已被创建".into());
                }
            }
            RuntimeEventKind::TurnFinished {
                turn_id, outcome, ..
            } if !is_parent => {
                if child_cancelled
                    || *outcome != TurnOutcome::Cancelled
                    || pending_approval
                        .as_ref()
                        .is_none_or(|(_, pending_turn, _, _)| turn_id != pending_turn)
                    || !approval_cancelled
                {
                    return Err("子任务没有真实取消终态".into());
                }
                child_cancelled = true;
                evidence.record(
                    json!({"event":"child_turn_cancelled","task_id":task.task_id,
                    "turn_id":turn_id,"native_session_id":event.native_session_id,
                    "outcome":"Cancelled","target_absent":!target.exists()}),
                )?;
                if target.exists() {
                    return Err("子任务取消终态后目标文件已被创建".into());
                }
            }
            RuntimeEventKind::ApprovalResolved { .. } if !is_parent => {
                return Err("子任务 Write 意外取得审批决定".into());
            }
            RuntimeEventKind::LocalToolRequested { .. } if !is_parent => {
                return Err("子任务请求了范围外本地工具".into());
            }
            RuntimeEventKind::Disconnected { .. } if !child_cancelled => {
                return Err("真实连接在取消确认前断开".into());
            }
            _ => {}
        }
    }
    let child_id = child_id.ok_or("缺少真实子任务")?;
    let saved = records(sender).await?;
    let parent = saved
        .iter()
        .find(|task| task.task_id == parent_id)
        .ok_or("缺少父记录")?;
    let child = saved
        .iter()
        .find(|task| task.task_id == child_id)
        .ok_or("缺少子记录")?;
    let parent_config: Value =
        serde_json::from_str(&parent.config_json).map_err(|error| error.to_string())?;
    let child_config: Value =
        serde_json::from_str(&child.config_json).map_err(|error| error.to_string())?;
    let ceiling = ceiling_from_parent(parent, "claude").map_err(|error| error.to_string())?;
    if !parent_tool_approved
        || !spawn_called
        || saved.len() != 2
        || child.parent_task_id.as_deref() != Some(&parent_id)
        || child.parent_generation != Some(1)
        || child_config["permission_policy"] != "ClaudeRestrictedFilesV2"
        || child_config["permission_ceiling"] != json!(ceiling)
        || child_config["claude_profile"] != parent_config["claude_profile"]
        || child_config["effective_permissions"]["claudeRestrictedFilesV2"]
            != parent_config["claude_profile"]
        || child_config["effective_permissions"]["fixedProfileVerified"] != true
        || child_config["effective_permissions"]["permissionMode"] != "default"
        || child_config["cli_version"] != "2.1.280"
        || child.state != LocalCliTaskState::Cancelled
        || child_native_id.is_none()
        || parent_native_id.is_none()
        || child_native_id == parent_native_id
        || target.exists()
    {
        return Err("父子亲缘、权限上限、原生会话或文件终态不符".into());
    }
    evidence.record(json!({"event":"saved_cancel_chain_verified",
        "parent_task_id":parent_id,"child_task_id":child_id,
        "parent_generation":parent.generation,"child_generation":child.generation,
        "parent_native_session_id":parent_native_id,"child_native_session_id":child_native_id,
        "same_fixed_profile":true,"parent_ceiling_saved":true,"child_state":"cancelled",
        "late_allow_rejected":late_rejected,"target_absent":true}))?;
    Ok(())
}

#[test]
#[ignore = "仅由固定 Claude 2.1.280 已授权隔离运行器执行，调用真实模型"]
fn real_claude_v2_parent_child_pending_write_cancel() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-v2-cancel-probe")).unwrap(),
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
    let profile = env::var("INFINISHELL_CLAUDE_LIVE_STATE_PROFILE").unwrap();
    assert_eq!(env::var("WARP_DATA_PROFILE").unwrap(), profile);
    let state = current_state_dir();
    assert!(state.to_string_lossy().contains(&profile));
    fs::create_dir_all(&state).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(
        state.join(super::super::super::claude::NATIVE_RESULT_EVIDENCE_MARKER),
        b"real_claude_production_v2_write_cancel\n",
    )
    .unwrap();
    let artifact =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").expect("缺少隔离证据路径"))
            .canonicalize()
            .unwrap();
    assert!(artifact.starts_with(&root));
    let mut evidence = Evidence {
        file: File::create(artifact).unwrap(),
        root: root.clone(),
    };
    evidence
        .record(json!({"event":"acceptance_started","scope":SCOPE,
            "cli_version":"2.1.280","real_gui_verified":false,
            "max_native_children":1,"max_native_write_requests":1,
            "http_request_count_verified":false}))
        .unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
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
        let writer =
            crate::persistence::start_test_writer(&root.join("coordinator.sqlite")).unwrap();
        let coordinator =
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
        let (wake, mut receiver) = mpsc::unbounded_channel();
        app.update(|ctx| {
            ctx.subscribe_to_model(&coordinator, move |_, event, _| {
                let event = match event {
                    LocalCLITaskCoordinatorEvent::Runtime { task, event } => {
                        Some((task.clone(), event.clone()))
                    }
                    _ => None,
                };
                let _ = wake.send(event);
            })
        });
        let result = drive_cancel(
            &mut app,
            &coordinator,
            &writer.sender,
            &mut receiver,
            &root,
            &mut evidence,
        )
        .await;
        let cleanup = shutdown(&mut app, &coordinator, &mut receiver, &mut evidence).await;
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result.and(cleanup) {
            Ok(()) => evidence
                .record(json!({"event":"acceptance_passed","scope":SCOPE,
                    "waiting_write_cancel_verified":true,"late_allow_rejected":true,
                    "target_file_unchanged":true,"parent_child_ceiling_verified":true,
                    "all_runtime_hosts_cleaned":true,"real_gui_verified":false}))
                .unwrap(),
            Err(error) => {
                evidence
                    .record(json!({"event":"acceptance_failed","reason":error}))
                    .unwrap();
                panic!("真实 Claude V2 父子待审批取消验收未通过；检查隔离证据");
            }
        }
    });
}
