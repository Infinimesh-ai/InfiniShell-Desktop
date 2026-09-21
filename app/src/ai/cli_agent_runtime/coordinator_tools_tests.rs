use futures::executor::block_on;

use super::*;

fn task(id: &str, parent: Option<&str>) -> LocalCliTask {
    LocalCliTask {
        version: 1,
        task_id: id.into(),
        parent_task_id: parent.map(str::to_owned),
        parent_generation: parent.map(|_| 1),
        harness: "codex".into(),
        working_directory: std::env::temp_dir().to_string_lossy().into(),
        config_json: "{}".into(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    }
}

fn message(source: &LocalCliTask, target: &LocalCliTask) -> LocalCliMessage {
    LocalCliMessage {
        version: 1,
        message_id: Uuid::new_v4().to_string(),
        sender_task_id: source.task_id.clone(),
        recipient_task_id: target.task_id.clone(),
        sender_generation: source.generation,
        recipient_generation: target.generation,
        subject: "progress".into(),
        body: "进度 中文\nEnglish 🧪".into(),
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    }
}

#[test]
fn spawn_limit_counts_active_children_and_rejects_missing_or_cyclic_ancestors() {
    let root = task("root", None);
    let mut records = vec![root.clone()];
    records.extend((0..8).map(|n| task(&format!("child-{n}"), Some("root"))));
    assert!(validate_spawn_scope(&root, &records, 1).is_err());
    records[1].state = LocalCliTaskState::Completed;
    assert!(validate_spawn_scope(&root, &records, 1).is_ok());
    assert!(validate_spawn_scope(&root, &records, 0).is_err());
    let orphan = task("orphan", Some("missing"));
    assert!(validate_spawn_scope(&orphan, &records, 1).is_err());
    let first = task("first", Some("second"));
    let second = task("second", Some("first"));
    assert!(validate_spawn_scope(&first, &[first.clone(), second], 1).is_err());
}

#[test]
fn stable_child_and_receipt_ids_are_separate_and_reproducible() {
    let call = Uuid::new_v4();
    assert_eq!(derived_id(call, "child-0"), derived_id(call, "child-0"));
    assert_ne!(derived_id(call, "child-0"), derived_id(call, "child-1"));
    assert_ne!(derived_id(call, "child-0"), derived_id(call, "result"));
    assert_ne!(
        derived_id(call, "child-0"),
        derived_id(Uuid::new_v4(), "child-0")
    );
}

#[test]
fn cancelled_call_cannot_pass_the_side_effect_gate_even_while_its_turn_is_active() {
    let token = Uuid::new_v4();
    let mut task = task("parent", None);
    task.state = LocalCliTaskState::Running;
    let (commands, _receiver) = mpsc::channel(1);
    let mut model = LocalCLITaskCoordinator::new(None);
    model.entries.insert(
        task.task_id.clone(),
        ManagedTaskEntry {
            token,
            commands,
            pending_tool_calls: HashSet::from(["call".to_owned()]),
            snapshot: ManagedTaskSnapshot {
                task,
                ready: true,
                connected: true,
                active_turn_id: Some("turn".into()),
                approvals: Vec::new(),
                output: String::new(),
                error: None,
            },
        },
    );
    assert!(check_current(&model, "parent", token, 1, "turn", "call").is_ok());
    model
        .entries
        .get_mut("parent")
        .unwrap()
        .pending_tool_calls
        .remove("call");
    assert!(check_current(&model, "parent", token, 1, "turn", "call").is_err());
    assert!(model.entries["parent"].snapshot.connected);
}

#[test]
fn queued_managed_message_is_claimed_after_online_receiver_reserves_capacity() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    let mut parent = task("parent", None);
    parent.config_json =
        json!({"local_tools":{"allow_spawn":false,"allow_message":true}}).to_string();
    let child = task("child", Some("parent"));
    let offered = message(&parent, &child);
    let runtime_generation = Uuid::new_v4();
    let (commands, mut receiver) = tokio::sync::mpsc::channel(1);
    let endpoint = ManagedTaskEndpoint {
        task_id: child.task_id.clone(),
        generation: child.generation,
        harness: Harness::Codex,
        runtime_generation,
        active_turn_id: Some("active-turn".into()),
        commands,
    };
    block_on(async {
        checkpoint_task(&writer.sender, parent.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        checkpoint_task(&writer.sender, child.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        enqueue_message(&writer.sender, offered.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let delivery = dispatch_managed_message(
            &writer.sender,
            endpoint,
            parent.clone(),
            child.clone(),
            offered.clone(),
        );
        let receive = async {
            let request = receiver.recv().await.unwrap();
            assert_eq!(request.message_id.to_string(), offered.message_id);
            assert_eq!(request.expected_generation, child.generation);
            assert!(request.from_mailbox);
            assert!(matches!(
                request.action,
                RuntimeAction::Steer { ref expected_turn_id, ref input }
                    if expected_turn_id == "active-turn"
                        && input == &[InputContent::Text(
                            "Subject: progress\n\n进度 中文\nEnglish 🧪".into()
                        )]
            ));
            request.reply.send(Ok(())).unwrap();
        };
        let (state, ()) = futures::join!(delivery, receive);
        assert_eq!(state.unwrap(), LocalCliMessageState::Sent);
        let stored = load_messages(&writer.sender, child.task_id.clone(), child.generation)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].state, LocalCliMessageState::Sent);
        assert_eq!(stored[0].receipt_kind, None);
        assert_eq!(
            crate::ai::local_cli_mailbox::acknowledge_runtime_message(
                &writer.sender,
                &child.task_id,
                child.generation,
                &RuntimeEventKind::MessageAccepted {
                    message_id: Uuid::parse_str(&offered.message_id).unwrap(),
                    turn_id: Some("native-turn".into()),
                },
            )
            .await
            .unwrap(),
            Some(LocalCliMessageState::Acknowledged)
        );
        let acknowledged = load_messages(&writer.sender, child.task_id, child.generation)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(acknowledged[0].state, LocalCliMessageState::Acknowledged);
        assert_eq!(
            acknowledged[0].receipt_kind,
            Some(LocalCliReceiptKind::NativeProtocol)
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn unavailable_managed_receiver_leaves_message_queued_for_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    let mut parent = task("parent", None);
    parent.config_json =
        json!({"local_tools":{"allow_spawn":false,"allow_message":true}}).to_string();
    let child = task("child", Some("parent"));
    let offered = message(&parent, &child);
    let (commands, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    let endpoint = ManagedTaskEndpoint {
        task_id: child.task_id.clone(),
        generation: child.generation,
        harness: Harness::Claude,
        runtime_generation: Uuid::new_v4(),
        active_turn_id: None,
        commands,
    };
    block_on(async {
        checkpoint_task(&writer.sender, parent.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        checkpoint_task(&writer.sender, child.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert!(
            dispatch_managed_message(
                &writer.sender,
                endpoint,
                parent,
                child.clone(),
                offered.clone(),
            )
            .await
            .is_err()
        );
        let stored = load_messages(&writer.sender, child.task_id, child.generation)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, [offered]);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn failed_managed_transport_keeps_claimed_message_sent_and_unconfirmed() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    let mut parent = task("parent", None);
    parent.config_json =
        json!({"local_tools":{"allow_spawn":false,"allow_message":true}}).to_string();
    let child = task("child", Some("parent"));
    let offered = message(&parent, &child);
    let (commands, mut receiver) = tokio::sync::mpsc::channel(1);
    let endpoint = ManagedTaskEndpoint {
        task_id: child.task_id.clone(),
        generation: child.generation,
        harness: Harness::Grok,
        runtime_generation: Uuid::new_v4(),
        active_turn_id: Some("active-turn".into()),
        commands,
    };
    block_on(async {
        checkpoint_task(&writer.sender, parent.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        checkpoint_task(&writer.sender, child.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let delivery = dispatch_managed_message(
            &writer.sender,
            endpoint,
            parent,
            child.clone(),
            offered.clone(),
        );
        let reject = async {
            let request = receiver.recv().await.unwrap();
            request.reply.send(Err("协议写入失败".into())).unwrap();
        };
        let (result, ()) = futures::join!(delivery, reject);
        assert!(result.is_err());
        let stored = load_messages(&writer.sender, child.task_id, child.generation)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored[0].state, LocalCliMessageState::Sent);
        assert_eq!(stored[0].receipt_kind, None);
        assert_eq!(stored[0].message_id, offered.message_id);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn inspection_keeps_unconfirmed_prior_generation_messages_and_bounds_utf8() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    block_on(async {
        let mut current = task("root", None);
        checkpoint_task(&writer.sender, current.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let mut input = input_message(
            &current,
            Uuid::new_v4(),
            &RuntimeAction::Submit {
                input: vec![InputContent::Text("中文🧪".repeat(1000))],
            },
        )
        .unwrap();
        enqueue_message(&writer.sender, input.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        set_record_sent(&writer.sender, &input).await.unwrap();
        input.state = LocalCliMessageState::Sent;
        current.revision = 1;
        current.state = LocalCliTaskState::Disconnected;
        checkpoint_task(&writer.sender, current.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        current = next_task_generation(&current).unwrap();
        checkpoint_task(&writer.sender, current.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let view = inspect(
            &writer.sender,
            &[current],
            &["root".into()],
            &ResultPage::default(),
        )
        .await
        .unwrap();
        let messages = view["tasks"][0]["messages"].as_array().unwrap();
        let recovered = messages
            .iter()
            .find(|message| message["message_id"] == input.message_id)
            .unwrap();
        assert_eq!(recovered["generation"], 1);
        assert_eq!(
            recovered["state"],
            serde_json::to_value(LocalCliMessageState::Sent).unwrap()
        );
        assert_eq!(recovered["body_truncated"], true);
        assert!(recovered["receipt_kind"].is_null());
        assert!(
            view["delivery_note"]
                .as_str()
                .unwrap()
                .contains("application_history")
        );
        assert!(recovered["body"].as_str().unwrap().len() <= 1024);
        assert_eq!(
            serde_json::from_slice::<Value>(&serde_json::to_vec(&view).unwrap()).unwrap(),
            view
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn historical_result_pages_recover_every_utf8_byte_without_changing_current_generation() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    block_on(async {
        let mut original = task("root", None);
        checkpoint_task(&writer.sender, original.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let full_result = "中文🧪\n完整历史结果\n".repeat(3000);
        original.state = LocalCliTaskState::Disconnected;
        original.revision = 1;
        original.result = Some(full_result.clone());
        checkpoint_task(&writer.sender, original.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let current = next_task_generation(&original).unwrap();
        checkpoint_task(&writer.sender, current.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let ids = vec!["root".to_owned()];
        let records = vec![current.clone()];
        let mut page = ResultPage {
            result_generation: Some(1),
            result_offset: 0,
        };
        let mut recovered = String::new();
        loop {
            let view = inspect(&writer.sender, &records, &ids, &page)
                .await
                .unwrap();
            let result = &view["tasks"][0];
            assert_eq!(result["generation"], 1);
            assert_eq!(result["current_generation"], 2);
            assert_eq!(result["result_available"], true);
            assert_eq!(result["result_total_bytes"], full_result.len());
            assert_eq!(result["result_offset"], recovered.len());
            let text = result["result"].as_str().unwrap();
            assert!(text.len() <= 8192);
            recovered.push_str(text);
            match result["result_next_offset"].as_u64() {
                Some(next) => {
                    assert!(next as usize > page.result_offset);
                    page.result_offset = next as usize;
                }
                None => {
                    assert_eq!(result["result_truncated"], false);
                    break;
                }
            }
        }
        assert_eq!(recovered, full_result);
        for invalid in [
            ResultPage {
                result_generation: Some(1),
                result_offset: 1,
            },
            ResultPage {
                result_generation: Some(1),
                result_offset: full_result.len() + 1,
            },
            ResultPage {
                result_generation: Some(3),
                result_offset: 0,
            },
        ] {
            assert!(
                inspect(&writer.sender, &records, &ids, &invalid)
                    .await
                    .is_err()
            );
        }
        let latest = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest, records);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn tool_reply_is_persisted_as_sent_before_transport_and_is_not_automatically_retried() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    block_on(async {
        let current = task("root", None);
        checkpoint_task(&writer.sender, current.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let receipt_id = Uuid::new_v4();
        let result = Ok(json!({"tasks":[]}));
        let receipt = tool_record(
            &current,
            receipt_id,
            "native_tool_result",
            serde_json::to_string(&result).unwrap(),
        );
        enqueue_message(&writer.sender, receipt)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let completion = ToolCompletion {
            generation: current.generation,
            turn_id: "turn".into(),
            call_id: "call".into(),
            receipt_id: Some(receipt_id),
            result,
        };
        assert_eq!(
            prepare_tool_reply(&writer.sender, &current, &completion)
                .await
                .unwrap(),
            receipt_id
        );
        assert!(
            prepare_tool_reply(&writer.sender, &current, &completion)
                .await
                .is_err()
        );
        let event = RuntimeEventKind::CommandDispatched {
            message_id: receipt_id,
            turn_id: Some("turn".into()),
        };
        assert_eq!(
            acknowledge_runtime_message(&writer.sender, "root", 1, &event)
                .await
                .unwrap(),
            None
        );
        let stored = load_messages(&writer.sender, "root".into(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored[0].state, LocalCliMessageState::Sent);
        assert!(stored[0].receipt_kind.is_none());
        assert_eq!(
            stored[0].body,
            serde_json::to_string(&completion.result).unwrap()
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn tool_error_requires_durable_receipt_and_cannot_bypass_database_failure() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    let current = task("root", None);
    let completion = ToolCompletion {
        generation: current.generation,
        turn_id: "turn".into(),
        call_id: "rejected-call".into(),
        receipt_id: None,
        result: Err("本地工具并发请求达到上限".into()),
    };
    block_on(async {
        checkpoint_task(&writer.sender, current.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let receipt_id = prepare_tool_reply(&writer.sender, &current, &completion)
            .await
            .unwrap();
        let stored = load_messages(&writer.sender, current.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].message_id, receipt_id.to_string());
        assert_eq!(stored[0].state, LocalCliMessageState::Sent);
        assert_eq!(stored[0].receipt_kind, None);
        assert_eq!(
            stored[0].body,
            serde_json::to_string(&completion.result).unwrap()
        );
        assert!(
            prepare_tool_reply(&writer.sender, &current, &completion)
                .await
                .is_err()
        );
        assert!(
            prepare_tool_reply(&writer.sender, &task("missing", None), &completion)
                .await
                .is_err()
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
    assert!(block_on(prepare_tool_reply(&writer.sender, &current, &completion)).is_err());
}
