use super::super::RuntimeEvent;
use super::*;

fn task(id: &str) -> LocalCliTask {
    LocalCliTask {
        version: 1,
        task_id: id.into(),
        parent_task_id: Some("parent".into()),
        parent_generation: Some(1),
        harness: "codex".into(),
        working_directory: std::env::temp_dir().to_string_lossy().to_string(),
        config_json: "{\"permission_policy\":\"Inherit\",\"model\":null}".into(),
        native_session_id: Some("native-session".into()),
        generation: 4,
        revision: 8,
        state: LocalCliTaskState::Completed,
        result: Some("previous result".into()),
        terminal_evidence: Some("native completed".into()),
    }
}

#[test]
fn live_records_replace_restored_duplicates_without_losing_other_tasks() {
    let old = task("one");
    let mut live = old.clone();
    live.state = LocalCliTaskState::Running;
    live.generation = 5;
    let merged = merge_tasks(&[old, task("two")], [live.clone()]);
    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged.iter().find(|record| record.task_id == "one"),
        Some(&live)
    );
    assert!(merged.iter().any(|record| record.task_id == "two"));
}

#[test]
fn resume_retains_native_and_parent_identity_and_advances_generation() {
    let original = task("one");
    let (resumed, target) = resumed_task(&original).unwrap();
    assert_eq!(resumed.task_id, original.task_id);
    assert_eq!(resumed.parent_task_id, original.parent_task_id);
    assert_eq!(resumed.native_session_id, original.native_session_id);
    assert_eq!(resumed.generation, original.generation + 1);
    assert_eq!(resumed.revision, 0);
    assert_eq!(resumed.state, LocalCliTaskState::Queued);
    assert!(resumed.result.is_none());
    assert!(resumed.terminal_evidence.is_none());
    assert_eq!(
        target,
        SessionTarget::Resume {
            native_session_id: "native-session".into()
        }
    );
    assert_eq!(original.result.as_deref(), Some("previous result"));
}

#[test]
fn resume_rejects_unfinished_and_unknown_records_without_replacing_the_run() {
    for state in [
        LocalCliTaskState::Queued,
        LocalCliTaskState::Running,
        LocalCliTaskState::WaitingForUser,
        LocalCliTaskState::Unconfirmed,
        LocalCliTaskState::Unknown,
    ] {
        let mut original = task("still-owned");
        original.state = state;
        let before = original.clone();
        let error = resumed_task(&original).unwrap_err();
        if state == LocalCliTaskState::Unconfirmed {
            assert_eq!(error, crate::t!("cli-agent-task-outcome-unconfirmed"));
        }
        assert_eq!(original, before);
    }
}

#[test]
fn missing_native_session_and_generation_overflow_never_fall_back_to_new() {
    let mut original = task("one");
    original.native_session_id = None;
    assert!(resumed_task(&original).is_err());
    original.native_session_id = Some(" ".into());
    assert!(resumed_task(&original).is_err());
    original.native_session_id = Some("native".into());
    original.generation = i64::MAX;
    assert!(resumed_task(&original).is_err());
}

#[test]
fn running_codex_steers_and_claude_submits_a_separate_turn() {
    let text = "中文与English\n第二行 🧪";
    assert_eq!(
        input_action(Harness::Codex, Some("turn-1"), text).unwrap(),
        RuntimeAction::Steer {
            expected_turn_id: "turn-1".into(),
            input: vec![InputContent::Text(text.into())],
        }
    );
    assert_eq!(
        input_action(Harness::Claude, Some("turn-1"), text).unwrap(),
        RuntimeAction::Submit {
            input: vec![InputContent::Text(text.into())],
        }
    );
    assert!(matches!(
        input_action(Harness::Claude, None, text).unwrap(),
        RuntimeAction::Submit { .. }
    ));
    assert!(matches!(
        input_action(Harness::Codex, None, text).unwrap(),
        RuntimeAction::Submit { .. }
    ));
    assert!(input_action(Harness::Grok, None, text).is_err());
}

#[test]
fn installed_or_newer_versions_do_not_automatically_enable_managed_launch() {
    assert!(verified_version(
        Harness::Codex,
        &CLIAgentVersionStatus::Detected("0.147.0".into())
    ));
    assert!(!verified_version(
        Harness::Codex,
        &CLIAgentVersionStatus::Detected("0.148.0".into())
    ));
    assert!(verified_version(
        Harness::Claude,
        &CLIAgentVersionStatus::Detected("2.1.273".into())
    ));
    assert!(!verified_version(
        Harness::Claude,
        &CLIAgentVersionStatus::Unknown
    ));
    assert!(!verified_version(
        Harness::Grok,
        &CLIAgentVersionStatus::Detected("1.0.30".into())
    ));
}

fn message(state: LocalCliMessageState) -> LocalCliMessage {
    LocalCliMessage {
        version: 1,
        message_id: Uuid::from_u128(10).to_string(),
        sender_task_id: "task".into(),
        recipient_task_id: "task".into(),
        sender_generation: 1,
        recipient_generation: 1,
        subject: "user_input".into(),
        state,
        receipt_kind: None,
        body: serde_json::to_string(&RuntimeAction::Submit {
            input: vec![InputContent::Text("中文 🧪\nEnglish\n$(literal)".into())],
        })
        .unwrap(),
    }
}

#[test]
fn stale_message_loads_cannot_replace_another_task_or_generation() {
    let token = Uuid::from_u128(1);
    assert!(message_read_is_current(
        token,
        token,
        ("task", 2),
        Some(("task", 2))
    ));
    assert!(!message_read_is_current(
        token,
        Uuid::from_u128(2),
        ("task", 2),
        Some(("task", 2))
    ));
    assert!(!message_read_is_current(
        token,
        token,
        ("task", 1),
        Some(("task", 2))
    ));
    assert!(!message_read_is_current(
        token,
        token,
        ("task", 2),
        Some(("other", 2))
    ));
    assert!(!message_read_is_current(token, token, ("task", 2), None));
}

#[test]
fn only_unconfirmed_text_input_is_recovered_and_contents_are_preserved() {
    for state in [LocalCliMessageState::Queued, LocalCliMessageState::Sent] {
        assert_eq!(
            recoverable_input(&message(state)).as_deref(),
            Some("中文 🧪\nEnglish\n$(literal)")
        );
    }
    for state in [
        LocalCliMessageState::Acknowledged,
        LocalCliMessageState::Failed,
        LocalCliMessageState::Cancelled,
        LocalCliMessageState::Unknown,
    ] {
        assert!(recoverable_input(&message(state)).is_none());
        assert!(input_text(&message(state)).is_some());
    }
    let mut malformed = message(LocalCliMessageState::Sent);
    malformed.version = 2;
    assert!(recoverable_input(&malformed).is_none());
    malformed.version = 1;
    malformed.body = "not a runtime action".into();
    assert!(recoverable_input(&malformed).is_none());
    malformed.body = serde_json::to_string(&RuntimeAction::Submit {
        input: vec![InputContent::LocalImage(PathBuf::from("/image.png"))],
    })
    .unwrap();
    assert!(recoverable_input(&malformed).is_none());
}

#[test]
fn restoring_a_record_does_not_silently_replace_another_draft_or_pending_message() {
    let first = Uuid::from_u128(10);
    let other = Uuid::from_u128(11);
    assert!(draft_can_be_restored("", "saved", None, first));
    assert!(draft_can_be_restored("saved", "saved", Some(first), first));
    assert!(!draft_can_be_restored("new text", "saved", None, first));
    assert!(!draft_can_be_restored("", "saved", Some(other), first));
    assert!(!draft_can_be_restored("saved", "saved", Some(other), first));
}

#[test]
fn saved_tool_permissions_default_off_and_round_trip_without_escalation() {
    let old: SavedLaunchOptions =
        serde_json::from_str(r#"{"permission_policy":"Inherit","model":null}"#).unwrap();
    assert!(old.local_tools.is_none());
    let saved = SavedLaunchOptions {
        permission_policy: PermissionPolicy::ReadOnly,
        permission_ceiling: None,
        model: None,
        selected_skills: Vec::new(),
        local_tools: Some(LocalToolPermissions {
            allow_spawn: true,
            allow_message: false,
        }),
    };
    let restored: SavedLaunchOptions =
        serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert_eq!(restored.permission_policy, PermissionPolicy::ReadOnly);
    assert_eq!(restored.local_tools, saved.local_tools);
}

fn manager_view(app: &mut warpui::App) -> ViewHandle<LocalCLITaskManagerView> {
    manager_view_with_records(app, Vec::new())
}

fn manager_view_with_records(
    app: &mut warpui::App,
    records: Vec<LocalCliTask>,
) -> ViewHandle<LocalCLITaskManagerView> {
    use warpui::AddSingletonModel;
    crate::test_util::terminal::initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| crate::workspace::ToastStack);
    app.add_singleton_model(|_| CLIAgentInstallModel::empty_for_test());
    app.add_singleton_model(|_| LocalCLITaskCoordinator::with_restored_for_test(records));
    app.add_window(
        warpui::platform::WindowStyle::NotStealFocus,
        LocalCLITaskManagerView::new,
    )
    .1
}

fn composer_snapshot(view: &LocalCLITaskManagerView, ctx: &AppContext) -> ComposerSubmission {
    ComposerSubmission {
        text: view.prompt.as_ref(ctx).buffer_text(ctx),
        revision: view.prompt.as_ref(ctx).buffer_revision(ctx),
        attachments_revision: view.managed_input.attachments.revision,
        input_generation: view.input_generation,
    }
}

#[test]
fn managed_composer_ack_does_not_clear_a_reedited_identical_draft_or_new_attachments() {
    warpui::App::test((), |mut app| async move {
        let manager = manager_view(&mut app);
        manager.update(&mut app, |view, ctx| {
            view.selected_task = Some("task".into());
            view.prompt.update(ctx, |editor, ctx| {
                editor.set_buffer_text("中文\nEnglish", ctx)
            });
            let submitted = composer_snapshot(view, ctx);
            view.prompt.update(ctx, |editor, ctx| {
                editor.set_buffer_text("临时编辑", ctx);
                editor.set_buffer_text("中文\nEnglish", ctx);
            });
            view.managed_input
                .attachments
                .skills
                .push(SkillReference::Path(
                    warp_util::local_or_remote_path::LocalOrRemotePath::Local(PathBuf::from(
                        "/new/SKILL.md",
                    )),
                ));
            view.managed_input.attachments.revision = Uuid::new_v4();
            view.acknowledge_composer("task", &submitted, ctx);
            assert_eq!(view.prompt.as_ref(ctx).buffer_text(ctx), "中文\nEnglish");
            assert_eq!(view.managed_input.attachments.skills.len(), 1);
        });
    });
}

#[test]
fn managed_composer_ack_clears_only_the_original_unedited_input() {
    warpui::App::test((), |mut app| async move {
        let manager = manager_view(&mut app);
        manager.update(&mut app, |view, ctx| {
            view.selected_task = Some("task".into());
            view.prompt.update(ctx, |editor, ctx| {
                editor.set_buffer_text("完整中文\nEnglish 🧪", ctx)
            });
            let submitted = composer_snapshot(view, ctx);
            view.acknowledge_composer("task", &submitted, ctx);
            assert!(view.prompt.as_ref(ctx).buffer_text(ctx).is_empty());
            assert!(view.managed_input.attachments.images.is_empty());
        });
    });
}

#[test]
fn managed_review_import_retains_literal_multiline_and_rejects_previous_task_callback() {
    warpui::App::test((), |mut app| async move {
        let manager = manager_view(&mut app);
        manager.update(&mut app, |view, ctx| {
            let key = view.draft_key();
            let generation = view.input_generation;
            let review = "review 中文\n`$(literal)` <tag> 🧪";
            view.append_context(&key, generation, review.into(), ctx)
                .unwrap();
            assert_eq!(view.prompt.as_ref(ctx).buffer_text(ctx), review);
            view.select_task(Some("another-task".into()), ctx);
            assert!(
                view.append_context(&key, generation, "stale".into(), ctx)
                    .is_err()
            );
            assert!(view.prompt.as_ref(ctx).buffer_text(ctx).is_empty());
            view.select_task(None, ctx);
            assert_eq!(view.prompt.as_ref(ctx).buffer_text(ctx), review);
        });
    });
}

#[test]
fn managed_conversion_failure_preserves_text_and_the_entire_attachment_draft() {
    use std::time::Duration;
    use warpui::r#async::Timer;
    warpui::App::test((), |mut app| async move {
        let manager = manager_view(&mut app);
        manager.update(&mut app, |view, ctx| {
            view.prompt.update(ctx, |editor, ctx| {
                editor.set_buffer_text("keep 中文\nEnglish", ctx)
            });
            view.managed_input
                .attachments
                .images
                .push(crate::ai::agent::ImageContext {
                    data: "malformed base64!".into(),
                    mime_type: "image/png".into(),
                    file_name: "bad.png".into(),
                    is_figma: false,
                });
            view.managed_input.attachments.revision = Uuid::new_v4();
            view.start(false, ctx).unwrap();
            assert!(view.managed_input.preparing.is_some());
        });
        Timer::after(Duration::from_millis(100)).await;
        manager.read(&app, |view, ctx| {
            assert_eq!(
                view.prompt.as_ref(ctx).buffer_text(ctx),
                "keep 中文\nEnglish"
            );
            assert_eq!(view.managed_input.attachments.images.len(), 1);
            assert!(view.pending_inputs.is_empty());
            assert!(view.selected_task.is_none());
            assert!(view.managed_input.preparing.is_none());
            assert!(view.error.is_some());
        });
    });
}

#[test]
fn managed_typed_history_remains_recoverable_without_automatic_resend() {
    let mut stored = message(LocalCliMessageState::Sent);
    let input = vec![
        InputContent::Text("中文\nEnglish".into()),
        InputContent::LocalImage(PathBuf::from("/scope/local-cli-attachments/image.png")),
        InputContent::Skill {
            name: "review".into(),
            path: PathBuf::from("/project/.agents/skills/review/SKILL.md"),
        },
    ];
    stored.body = serde_json::to_string(&RuntimeAction::Submit {
        input: input.clone(),
    })
    .unwrap();
    assert_eq!(recoverable_parts(&stored), Some(input));
    // 混合内容以完整 JSON 提供只读预览，不能把图片和技能从文本预览中静默丢掉。
    assert!(input_text(&stored).is_none());
    stored.state = LocalCliMessageState::Acknowledged;
    assert!(recoverable_parts(&stored).is_none());
}

#[test]
fn managed_history_restore_and_discard_keep_later_edits() {
    use std::time::Duration;
    use warpui::r#async::Timer;
    warpui::App::test((), |mut app| async move {
        let manager = manager_view(&mut app);
        manager.update(&mut app, |view, ctx| {
            view.selected_task = Some("task".into());
            view.restore_saved_composer(
                "task",
                1,
                Uuid::new_v4(),
                vec![InputContent::Text("原稿\nEnglish".into())],
                ctx,
            )
            .unwrap();
        });
        Timer::after(Duration::from_millis(100)).await;
        manager.update(&mut app, |view, ctx| {
            assert_eq!(view.prompt.as_ref(ctx).buffer_text(ctx), "原稿\nEnglish");
            assert!(
                view.pending_inputs
                    .get("task")
                    .is_some_and(|pending| pending.phase == InputPhase::Uncertain)
            );
            view.prompt
                .update(ctx, |editor, ctx| editor.set_buffer_text("新稿 🧪", ctx));
            view.handle_action(&TaskManagerAction::DiscardDraft, ctx);
            assert_eq!(view.prompt.as_ref(ctx).buffer_text(ctx), "新稿 🧪");
            assert!(view.pending_inputs.is_empty());
        });
    });
}

#[test]
fn parent_child_message_bodies_are_read_only_even_if_they_look_like_user_input() {
    let mut record = message(LocalCliMessageState::Sent);
    record.sender_task_id = "child".into();
    record.subject = "child-result".into();
    assert_eq!(message_body_text(&record), record.body);
    assert!(recoverable_parts(&record).is_none());
    record.subject = "user_input".into();
    assert_eq!(message_body_text(&record), "中文 🧪\nEnglish\n$(literal)");
    assert!(recoverable_parts(&record).is_some());
}

#[test]
fn recorded_receipts_do_not_turn_attempted_delivery_or_application_history_into_native_ack() {
    let mut record = message(LocalCliMessageState::Sent);
    record.receipt_kind = Some(LocalCliReceiptKind::NativeProtocol);
    let attempted = message_receipt_name(&record);
    record.state = LocalCliMessageState::Acknowledged;
    let native = message_receipt_name(&record);
    assert_ne!(attempted, native);
    record.receipt_kind = Some(LocalCliReceiptKind::ApplicationHistory);
    let history = message_receipt_name(&record);
    assert_ne!(history, native);
    record.receipt_kind = None;
    let missing = message_receipt_name(&record);
    record.receipt_kind = Some(LocalCliReceiptKind::Unknown);
    assert_eq!(missing, message_receipt_name(&record));
    assert_ne!(missing, native);
    assert_ne!(missing, history);
}

#[test]
fn history_selection_is_read_only_and_copies_the_complete_unicode_result() {
    warpui::App::test((), |mut app| async move {
        let current = task("history-task");
        let mut older = current.clone();
        older.generation = 2;
        let complete = format!("{}\nFINAL 中文 🧪", "多行English 🧪\n".repeat(4000));
        older.result = Some(complete.clone());
        let manager = manager_view_with_records(&mut app, vec![current.clone()]);
        manager.update(&mut app, |view, ctx| {
            view.select_task(Some(current.task_id.clone()), ctx);
            view.prompt.update(ctx, |editor, ctx| {
                editor.set_buffer_text("尚未提交的草稿", ctx)
            });
            let input_generation = view.input_generation;
            let input_revision = view.prompt.as_ref(ctx).buffer_revision(ctx);
            view.set_result_history_records(vec![older, current.clone()], ctx);
            let token = view.result_history.token;
            view.handle_action(
                &TaskManagerAction::SelectResultGeneration {
                    task_id: current.task_id.clone(),
                    active_generation: current.generation,
                    result_generation: 2,
                    token,
                },
                ctx,
            );
            assert_eq!(view.result_history.selected, Some(2));
            assert_eq!(view.selected_record(ctx).unwrap(), current);
            assert_eq!(view.input_generation, input_generation);
            assert_eq!(view.prompt.as_ref(ctx).buffer_revision(ctx), input_revision);
            view.handle_action(
                &TaskManagerAction::CopyHistoricalResult {
                    task_id: current.task_id.clone(),
                    active_generation: current.generation,
                    result_generation: 2,
                    token,
                },
                ctx,
            );
            assert_eq!(ctx.clipboard().read().plain_text, complete);
            assert!(view.pending_inputs.is_empty());
            assert!(view.pending_controls.is_empty());
        });
    });
}

#[test]
fn stale_history_copy_cannot_copy_another_task_or_an_unconfirmed_result() {
    warpui::App::test((), |mut app| async move {
        let first = task("one");
        let second = task("two");
        let manager = manager_view_with_records(&mut app, vec![first.clone(), second.clone()]);
        manager.update(&mut app, |view, ctx| {
            view.select_task(Some(first.task_id.clone()), ctx);
            view.set_result_history_records(vec![first.clone()], ctx);
            let stale = view.result_history.token;
            view.select_task(Some(second.task_id.clone()), ctx);
            let mut no_result = second.clone();
            no_result.result = None;
            view.set_result_history_records(vec![no_result], ctx);
            ctx.clipboard()
                .write(ClipboardContent::plain_text("keep clipboard".to_owned()));
            assert!(
                view.copy_historical_result(
                    &first.task_id,
                    first.generation,
                    first.generation,
                    stale,
                    ctx
                )
                .is_err()
            );
            assert!(
                view.copy_historical_result(
                    &second.task_id,
                    second.generation,
                    second.generation,
                    view.result_history.token,
                    ctx
                )
                .is_err()
            );
            assert_eq!(ctx.clipboard().read().plain_text, "keep clipboard");
            assert_eq!(view.selected_record(ctx).unwrap(), second);
        });
    });
}

#[test]
fn same_generation_committed_failure_refreshes_history_and_message_records_once() {
    warpui::App::test((), |mut app| async move {
        let mut running = task("same-generation");
        running.state = LocalCliTaskState::Running;
        running.result = None;
        let manager = manager_view_with_records(&mut app, vec![running.clone()]);
        let (history_token, message_token) = manager.update(&mut app, |view, ctx| {
            view.select_task(Some(running.task_id.clone()), ctx);
            view.observed_committed_task = Some(running.clone());
            view.set_result_history_records(vec![running.clone()], ctx);
            (view.result_history.token, view.message_load_token)
        });
        let mut failed = running;
        failed.revision += 1;
        failed.state = LocalCliTaskState::Failed;
        failed.result = Some("同代权限拒绝后的最终结果".into());
        app.update(|ctx| {
            LocalCLITaskCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                *coordinator =
                    LocalCLITaskCoordinator::with_restored_for_test(vec![failed.clone()]);
                ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
            });
        });
        manager.update(&mut app, |view, ctx| {
            assert_ne!(view.result_history.token, history_token);
            assert_ne!(view.message_load_token, message_token);
            assert_eq!(view.observed_committed_task.as_ref(), Some(&failed));
            let history_token = view.result_history.token;
            let message_token = view.message_load_token;
            view.handle_coordinator_event(&LocalCLITaskCoordinatorEvent::Changed, ctx);
            assert_eq!(view.result_history.token, history_token);
            assert_eq!(view.message_load_token, message_token);
        });
    });
}

#[test]
fn committed_message_notifications_refresh_both_endpoints_and_skip_unrelated_tasks() {
    warpui::App::test((), |mut app| async move {
        let sender = task("message-sender");
        let mut recipient = task("message-recipient");
        recipient.parent_task_id = Some(sender.task_id.clone());
        let unrelated = task("unrelated");
        let manager = manager_view_with_records(
            &mut app,
            vec![sender.clone(), recipient.clone(), unrelated.clone()],
        );
        manager.update(&mut app, |view, ctx| {
            for selected in [&sender, &recipient, &unrelated] {
                view.select_task(Some(selected.task_id.clone()), ctx);
                view.observed_committed_task = Some(selected.clone());
                let history_token = view.result_history.token;
                let message_token = view.message_load_token;
                view.handle_coordinator_event(
                    &LocalCLITaskCoordinatorEvent::MessagesChanged {
                        sender_task_id: sender.task_id.clone(),
                        recipient_task_id: recipient.task_id.clone(),
                    },
                    ctx,
                );
                assert_eq!(
                    view.message_load_token != message_token,
                    selected.task_id != unrelated.task_id
                );
                assert_eq!(view.result_history.token, history_token);
            }

            view.select_task(Some(recipient.task_id.clone()), ctx);
            view.observed_committed_task = Some(recipient.clone());
            let message_token = view.message_load_token;
            let history_token = view.result_history.token;
            let mut result = message(LocalCliMessageState::Queued);
            result.sender_task_id = sender.task_id.clone();
            result.recipient_task_id = recipient.task_id.clone();
            result.subject = "child-result".into();
            view.handle_coordinator_event(
                &LocalCLITaskCoordinatorEvent::ResultReady {
                    task: sender.clone(),
                    message: result,
                },
                ctx,
            );
            assert_ne!(view.message_load_token, message_token);
            assert_eq!(view.result_history.token, history_token);
            let message_token = view.message_load_token;
            view.handle_coordinator_event(
                &LocalCLITaskCoordinatorEvent::Runtime {
                    task: recipient.clone(),
                    event: RuntimeEvent {
                        generation: Uuid::new_v4(),
                        native_session_id: Some("native-session".into()),
                        kind: RuntimeEventKind::TextDelta {
                            turn_id: "turn".into(),
                            item_id: "item".into(),
                            text: "流式输出不重读消息".into(),
                        },
                    },
                },
                ctx,
            );
            assert_eq!(view.message_load_token, message_token);
            assert_eq!(view.result_history.token, history_token);

            // 父子任务首次消息尚未出现在列表时，早到回执和取消仍需读取已提交记录。
            assert!(view.saved_messages.is_empty());
            for kind in [
                RuntimeEventKind::MessageAccepted {
                    message_id: Uuid::new_v4(),
                    turn_id: None,
                },
                RuntimeEventKind::CommandDispatched {
                    message_id: Uuid::new_v4(),
                    turn_id: None,
                },
                RuntimeEventKind::RequestFailed {
                    message_id: Uuid::new_v4(),
                    message: "父任务消息失败".into(),
                },
                RuntimeEventKind::TurnFinished {
                    turn_id: "turn".into(),
                    outcome: super::super::TurnOutcome::Cancelled,
                    output: String::new(),
                },
                RuntimeEventKind::Disconnected {
                    reason: "父任务连接结束".into(),
                },
                RuntimeEventKind::LocalToolCancelled {
                    turn_id: "turn".into(),
                    call_id: "call".into(),
                },
            ] {
                let message_token = view.message_load_token;
                view.handle_coordinator_event(
                    &LocalCLITaskCoordinatorEvent::Runtime {
                        task: sender.clone(),
                        event: RuntimeEvent {
                            generation: Uuid::new_v4(),
                            native_session_id: Some("native-session".into()),
                            kind,
                        },
                    },
                    ctx,
                );
                assert_ne!(view.message_load_token, message_token);
                assert_eq!(view.result_history.token, history_token);
            }

            view.select_task(Some(sender.task_id.clone()), ctx);
            view.observed_committed_task = Some(sender);
            let message_token = view.message_load_token;
            view.handle_coordinator_event(
                &LocalCLITaskCoordinatorEvent::Runtime {
                    task: recipient,
                    event: RuntimeEvent {
                        generation: Uuid::new_v4(),
                        native_session_id: Some("native-session".into()),
                        kind: RuntimeEventKind::MessageAccepted {
                            message_id: Uuid::new_v4(),
                            turn_id: None,
                        },
                    },
                },
                ctx,
            );
            assert_ne!(view.message_load_token, message_token);
        });
    });
}

#[test]
fn recipient_receipts_refresh_the_senders_saved_message_without_fabricating_ack() {
    warpui::App::test((), |mut app| async move {
        let sender = task("message-sender");
        let recipient = task("message-recipient");
        let manager = manager_view_with_records(&mut app, vec![sender.clone(), recipient.clone()]);
        manager.update(&mut app, |view, ctx| {
            view.select_task(Some(sender.task_id.clone()), ctx);
            view.observed_committed_task = Some(sender.clone());
            view.prompt.update(ctx, |editor, ctx| {
                editor.set_buffer_text("另一个尚未提交的草稿", ctx)
            });
            let mut saved = message(LocalCliMessageState::Sent);
            saved.sender_task_id = sender.task_id;
            saved.recipient_task_id = recipient.task_id.clone();
            saved.subject = "parent-instruction".into();
            let message_id = Uuid::parse_str(&saved.message_id).unwrap();
            view.saved_messages = vec![saved.clone()];
            let history_token = view.result_history.token;
            for kind in [
                RuntimeEventKind::CommandDispatched {
                    message_id,
                    turn_id: Some("turn".into()),
                },
                RuntimeEventKind::MessageAccepted {
                    message_id,
                    turn_id: Some("turn".into()),
                },
                RuntimeEventKind::RequestFailed {
                    message_id,
                    message: "回执失败".into(),
                },
            ] {
                let message_token = view.message_load_token;
                view.handle_coordinator_event(
                    &LocalCLITaskCoordinatorEvent::Runtime {
                        task: recipient.clone(),
                        event: RuntimeEvent {
                            generation: Uuid::new_v4(),
                            native_session_id: Some("native-session".into()),
                            kind,
                        },
                    },
                    ctx,
                );
                assert_ne!(view.message_load_token, message_token);
                assert_eq!(view.result_history.token, history_token);
                // 真实数据库读取完成前保留旧记录；任何事件都不能直接伪造确认状态。
                assert_eq!(view.saved_messages, vec![saved.clone()]);
                assert_eq!(
                    view.prompt.as_ref(ctx).buffer_text(ctx),
                    "另一个尚未提交的草稿"
                );
            }
            let message_token = view.message_load_token;
            view.handle_coordinator_event(
                &LocalCLITaskCoordinatorEvent::Runtime {
                    task: recipient,
                    event: RuntimeEvent {
                        generation: Uuid::new_v4(),
                        native_session_id: Some("native-session".into()),
                        kind: RuntimeEventKind::MessageAccepted {
                            message_id: Uuid::new_v4(),
                            turn_id: None,
                        },
                    },
                },
                ctx,
            );
            assert_eq!(view.message_load_token, message_token);
        });
    });
}

#[test]
fn explicit_refresh_discovers_skills_in_a_new_project_without_opening_a_terminal() {
    use crate::ai::skills::SkillManager;
    use repo_metadata::RepoMetadataModel;
    use std::time::Duration;
    use warp_util::local_or_remote_path::LocalOrRemotePath;
    use warpui::r#async::Timer;

    warpui::App::test((), |mut app| async move {
        let project = tempfile::tempdir().unwrap();
        let skill = project.path().join(".agents/skills/fresh-project/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(
            &skill,
            "---\nname: fresh-project\ndescription: 新目录技能发现回归\n---\n\n保留中文与 English。\n",
        )
        .unwrap();
        let canonical_project = dunce::canonicalize(project.path()).unwrap();
        let mut input_directories = vec![project.path().to_path_buf()];
        #[cfg(unix)]
        {
            // Linux 也覆盖符号链接输入，不依赖 macOS 临时目录自带的 /var 别名。
            let alias = project.path().join("project-alias");
            std::os::unix::fs::symlink(&canonical_project, &alias).unwrap();
            input_directories.push(alias);
        }
        let manager = manager_view(&mut app);
        for input_directory in input_directories {
            manager.update(&mut app, |view, ctx| {
                view.directory.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&input_directory.to_string_lossy(), ctx)
                });
            });
            manager.update(&mut app, |view, ctx| {
                assert!(view.managed_input.skill_index_directory.is_none());
                view.handle_action(&TaskManagerAction::Refresh, ctx);
            });
            let mut found = false;
            for _ in 0..100 {
                Timer::after(Duration::from_millis(30)).await;
                found = manager.update(&mut app, |view, ctx| {
                    view.managed_input.available_skills
                        && SkillManager::as_ref(ctx)
                            .get_skills_for_working_directory(
                                Some(&LocalOrRemotePath::Local(canonical_project.clone())),
                                ctx,
                            )
                            .iter()
                            .any(|skill| skill.name == "fresh-project")
                });
                if found {
                    break;
                }
            }
            manager.update(&mut app, |view, ctx| {
                let indexed = RepoMetadataModel::as_ref(ctx)
                    .find_repository_for_path(&input_directory, ctx);
                assert!(
                    found && indexed.is_some(),
                    "明确刷新必须经过真实项目索引和 SkillWatcher 发布技能：error={:?}, pending={}, indexed={indexed:?}",
                    view.error,
                    view.managed_input.skill_index_token.is_some(),
                );
                assert_eq!(
                    view.directory.as_ref(ctx).buffer_text(ctx),
                    input_directory.to_string_lossy(),
                    "发现技能不得改写用户输入的启动目录"
                );
                assert_eq!(
                    view.managed_input.skill_index_directory,
                    Some((input_directory.to_string_lossy().into_owned(), canonical_project.clone()))
                );
            });
        }
    });
}

#[test]
fn project_skill_refresh_rejects_a_directory_changed_before_the_callback() {
    use repo_metadata::RepoMetadataModel;
    use std::time::Duration;
    use warpui::r#async::Timer;

    warpui::App::test((), |mut app| async move {
        let previous = tempfile::tempdir().unwrap();
        let current = tempfile::tempdir().unwrap();
        let manager = manager_view(&mut app);
        manager.update(&mut app, |view, ctx| {
            view.directory.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&previous.path().to_string_lossy(), ctx)
            });
        });
        manager.update(&mut app, |view, ctx| {
            view.handle_action(&TaskManagerAction::Refresh, ctx);
            view.directory.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&current.path().to_string_lossy(), ctx)
            });
        });
        Timer::after(Duration::from_millis(150)).await;
        manager.update(&mut app, |view, ctx| {
            assert_eq!(
                view.directory.as_ref(ctx).buffer_text(ctx),
                current.path().to_string_lossy()
            );
            assert!(
                RepoMetadataModel::as_ref(ctx)
                    .find_repository_for_path(previous.path(), ctx)
                    .is_none()
            );
            assert!(
                RepoMetadataModel::as_ref(ctx)
                    .find_repository_for_path(current.path(), ctx)
                    .is_none()
            );
            assert!(view.managed_input.skill_index_token.is_none());
            assert!(view.managed_input.skill_index_directory.is_none());
        });
    });
}
