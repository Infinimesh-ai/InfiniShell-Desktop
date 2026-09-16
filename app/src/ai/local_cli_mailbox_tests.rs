use std::cell::Cell;
use std::sync::mpsc;

use futures::executor::block_on;
use futures::future::{AbortHandle, Abortable};

use super::*;
use crate::persistence::local_cli_tasks::{
    LocalCliPersistenceRequest, checkpoint_task, enqueue_task_result, load_messages, load_tasks,
};
use crate::persistence::model::{LocalCliReceiptKind, LocalCliTask, LocalCliTaskState};

fn task(id: &str, parent: Option<&str>) -> LocalCliTask {
    LocalCliTask {
        version: 1,
        task_id: id.to_owned(),
        parent_task_id: parent.map(str::to_owned),
        parent_generation: parent.map(|_| 1),
        harness: "codex".to_owned(),
        working_directory: "/project".to_owned(),
        config_json: "{}".to_owned(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    }
}

fn message() -> LocalCliMessage {
    LocalCliMessage {
        version: 1,
        message_id: Uuid::new_v4().to_string(),
        sender_task_id: "parent".to_owned(),
        recipient_task_id: "child".to_owned(),
        sender_generation: 1,
        recipient_generation: 1,
        subject: "追加指令".to_owned(),
        body: "检查中文与 English\n不要执行 `$()` 中的文本。".to_owned(),
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    }
}

fn with_writer(test: impl FnOnce(&SyncSender<ModelEvent>)) {
    let directory = tempfile::TempDir::new().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    for task in [task("parent", None), task("child", Some("parent"))] {
        block_on(checkpoint_task(&writer.sender, task, None).unwrap())
            .unwrap()
            .unwrap();
    }
    test(&writer.sender);
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn local_mailbox_does_not_redispatch_after_native_acknowledgement() {
    with_writer(|sender| {
        let message = message();
        let result = block_on(dispatch_once(sender, message.clone(), || async {
            let stored = load_messages(sender, "child".to_owned(), 1)?
                .await
                .map_err(|error| error.to_string())??;
            assert_eq!(stored[0].state, LocalCliMessageState::Sent);
            Ok(())
        }))
        .unwrap();
        assert_eq!(result, LocalCliMessageState::Sent);
        let accepted = RuntimeEventKind::MessageAccepted {
            message_id: message.message_id.parse().unwrap(),
            turn_id: Some("turn-1".to_owned()),
        };
        assert_eq!(
            block_on(acknowledge_runtime_message(sender, "child", 1, &accepted)).unwrap(),
            Some(LocalCliMessageState::Acknowledged)
        );
        assert_eq!(
            block_on(dispatch_once(sender, message, || async {
                panic!("重复消息不能再次调用 CLI")
            }))
            .unwrap(),
            LocalCliMessageState::Acknowledged
        );
    });
}

#[test]
fn local_mailbox_rejects_before_dispatch_when_writer_is_unavailable() {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    drop(receiver);
    assert!(
        block_on(dispatch_once(&sender, message(), || async {
            panic!("未持久化消息不能调用 CLI")
        }))
        .is_err()
    );
}

#[test]
fn local_mailbox_keeps_unconfirmed_dispatch_without_automatic_retry() {
    with_writer(|sender| {
        let message = message();
        assert!(
            block_on(dispatch_once(sender, message.clone(), || async {
                Err("连接已关闭".to_owned())
            }))
            .is_err()
        );
        assert_eq!(
            block_on(dispatch_once(sender, message, || async {
                panic!("失败消息不能自动重投")
            }))
            .unwrap(),
            LocalCliMessageState::Sent
        );
    });
}

#[test]
fn local_mailbox_ignores_non_mailbox_commands_and_rejects_wrong_run_receipts() {
    with_writer(|sender| {
        let message = message();
        block_on(dispatch_once(sender, message.clone(), || async { Ok(()) })).unwrap();
        let accepted = RuntimeEventKind::MessageAccepted {
            message_id: message.message_id.parse().unwrap(),
            turn_id: None,
        };
        assert_eq!(
            block_on(acknowledge_runtime_message(sender, "child", 2, &accepted)).unwrap(),
            None
        );
        let dispatched = RuntimeEventKind::CommandDispatched {
            message_id: message.message_id.parse().unwrap(),
            turn_id: None,
        };
        assert_eq!(
            block_on(acknowledge_runtime_message(sender, "child", 1, &dispatched)).unwrap(),
            None
        );
        let stored = block_on(load_messages(sender, "child".to_owned(), 1).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(stored[0].state, LocalCliMessageState::Sent);
    });
}

#[test]
fn local_result_dispatch_commits_one_claim_and_keeps_lost_reply_unconfirmed() {
    with_writer(|sender| {
        let mut child = task("child", Some("parent"));
        child.revision = 1;
        child.state = LocalCliTaskState::Cancelled;
        block_on(checkpoint_task(sender, child, Some(1)).unwrap())
            .unwrap()
            .unwrap();
        let result = block_on(enqueue_task_result(sender, "child".to_owned(), 1).unwrap())
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            block_on(dispatch_prepared_result_once(
                sender,
                result.clone(),
                || async {
                    let saved = load_messages(sender, "parent".to_owned(), 1)?
                        .await
                        .map_err(|error| error.to_string())??;
                    assert_eq!(saved[0].state, LocalCliMessageState::Sent);
                    Err("发送后回复通道关闭".to_owned())
                }
            ))
            .is_err()
        );
        block_on(dispatch_prepared_result_once(
            sender,
            result.clone(),
            || async { panic!("已经领取过的结果不能再次派发") },
        ))
        .unwrap();
        assert_eq!(
            block_on(load_messages(sender, "parent".to_owned(), 1).unwrap())
                .unwrap()
                .unwrap()[0]
                .state,
            LocalCliMessageState::Sent
        );
        let rejected = RuntimeEventKind::RequestFailed {
            message_id: result.message_id.parse().unwrap(),
            message: "原生拒绝".to_owned(),
        };
        assert_eq!(
            block_on(acknowledge_runtime_message(sender, "parent", 1, &rejected)).unwrap(),
            Some(LocalCliMessageState::Failed)
        );
    });
}

#[test]
fn native_ack_for_a_sent_message_survives_recipient_turn_completion() {
    let mut observed = None;
    with_writer(|sender| {
        observed = Some(block_on(async {
            let message = message();
            dispatch_once(sender, message.clone(), || async { Ok(()) })
                .await
                .unwrap();
            let mut child = task("child", Some("parent"));
            child.revision = 1;
            child.state = LocalCliTaskState::Completed;
            child.native_session_id = Some("native-child".into());
            child.result = Some("已提交的真实结果".into());
            child.terminal_evidence = Some("turn/completed:turn-1".into());
            checkpoint_task(sender, child, Some(1))
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let result = acknowledge_runtime_message(
                sender,
                "child",
                1,
                &RuntimeEventKind::MessageAccepted {
                    message_id: message.message_id.parse().unwrap(),
                    turn_id: Some("turn-1".into()),
                },
            )
            .await;
            let saved = load_messages(sender, "child".into(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            (result, saved[0].clone())
        }));
    });
    let (result, saved) = observed.unwrap();
    assert_eq!(result.unwrap(), Some(LocalCliMessageState::Acknowledged));
    assert_eq!(saved.state, LocalCliMessageState::Acknowledged);
    assert_eq!(
        saved.receipt_kind,
        Some(LocalCliReceiptKind::NativeProtocol)
    );
}

#[test]
fn native_ack_from_the_current_child_survives_the_sender_advancing_generation() {
    let mut observed = None;
    with_writer(|sender| {
        observed = Some(block_on(async {
            let message = message();
            dispatch_once(sender, message.clone(), || async { Ok(()) })
                .await
                .unwrap();
            let mut parent = task("parent", None);
            parent.revision = 1;
            parent.state = LocalCliTaskState::Disconnected;
            checkpoint_task(sender, parent, Some(1))
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let mut parent = task("parent", None);
            parent.generation = 2;
            checkpoint_task(sender, parent, Some(1))
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let result = acknowledge_runtime_message(
                sender,
                "child",
                1,
                &RuntimeEventKind::MessageAccepted {
                    message_id: message.message_id.parse().unwrap(),
                    turn_id: Some("still-current-child-turn".into()),
                },
            )
            .await;
            let saved = load_messages(sender, "child".into(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            (result, saved[0].clone())
        }));
    });
    let (result, saved) = observed.unwrap();
    assert_eq!(result.unwrap(), Some(LocalCliMessageState::Acknowledged));
    assert_eq!(saved.sender_generation, 1);
    assert_eq!(saved.recipient_generation, 1);
    assert_eq!(saved.state, LocalCliMessageState::Acknowledged);
}

#[test]
fn cancelling_while_sqlite_commit_ack_is_pending_prevents_the_dispatch_after_commit() {
    with_writer(|sender| {
        let (proxy, requests) = mpsc::sync_channel(2);
        let (committed, committed_receiver) = futures::channel::oneshot::channel();
        let (release, release_receiver) = mpsc::channel();
        let writer = sender.clone();
        let proxy_thread = std::thread::spawn(move || {
            let mut committed = Some(committed);
            while let Ok(event) = requests.recv() {
                match event {
                    ModelEvent::LocalCliPersistence(
                        LocalCliPersistenceRequest::EnqueueMessage {
                            message,
                            completion,
                        },
                    ) => {
                        let outcome = block_on(enqueue_message(&writer, message).unwrap()).unwrap();
                        // 真正的 SQLite 提交已经发生，暂停的仅是发给调用方的确认。
                        committed.take().unwrap().send(()).unwrap();
                        release_receiver.recv().unwrap();
                        let _ = completion.send(outcome);
                    }
                    event => writer.send(event).unwrap(),
                }
            }
        });
        let dispatched = Cell::new(false);
        let (abort, registration) = AbortHandle::new_pair();
        let result = block_on(async {
            let pending = Abortable::new(
                dispatch_once(&proxy, message(), || async {
                    dispatched.set(true);
                    Ok(())
                }),
                registration,
            );
            let cancel = async {
                committed_receiver.await.unwrap();
                abort.abort();
                release.send(()).unwrap();
            };
            futures::join!(pending, cancel).0
        });
        drop(proxy);
        proxy_thread.join().unwrap();
        let saved = block_on(load_messages(sender, "child".into(), 1).unwrap())
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        assert!(!dispatched.get(), "被取消的工具不能在提交确认后继续发送");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].state, LocalCliMessageState::Queued);
        assert!(saved[0].receipt_kind.is_none());
    });
}

#[test]
fn late_explicit_native_rejection_updates_only_the_sent_message_not_the_task_result() {
    let mut observed = None;
    with_writer(|sender| {
        observed = Some(block_on(async {
            let message = message();
            dispatch_once(sender, message.clone(), || async { Ok(()) })
                .await
                .unwrap();
            let mut child = task("child", Some("parent"));
            child.revision = 1;
            child.state = LocalCliTaskState::Completed;
            child.native_session_id = Some("native-child".into());
            child.result = Some("原生完成结果".into());
            child.terminal_evidence = Some("turn/completed:turn-1".into());
            checkpoint_task(sender, child, Some(1))
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let outcome = acknowledge_runtime_message(
                sender,
                "child",
                1,
                &RuntimeEventKind::RequestFailed {
                    message_id: message.message_id.parse().unwrap(),
                    message: "The turn was already completed".into(),
                },
            )
            .await;
            let tasks = load_tasks(sender, false).unwrap().await.unwrap().unwrap();
            (
                outcome,
                tasks
                    .into_iter()
                    .find(|task| task.task_id == "child")
                    .unwrap(),
            )
        }));
    });
    let (outcome, task) = observed.unwrap();
    assert_eq!(outcome.unwrap(), Some(LocalCliMessageState::Failed));
    assert_eq!(task.state, LocalCliTaskState::Completed);
    assert_eq!(task.result.as_deref(), Some("原生完成结果"));
    assert_eq!(task.revision, 1);
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[path = "local_cli_mailbox_restart_tests.rs"]
mod restart;
