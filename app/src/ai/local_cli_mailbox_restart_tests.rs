//! 无模型的跨进程信箱故障注入；只验证生产 SQLite 与交付边界，不冒充原生 CLI 验收。

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::sync::mpsc::SyncSender;
use std::thread;
use std::time::{Duration, Instant};

use command::Stdio;
use command::blocking::Command;
use futures::executor::block_on;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use uuid::Uuid;

use super::{message, task};
use crate::ai::cli_agent_runtime::RuntimeEventKind;
use crate::ai::local_cli_mailbox::{
    acknowledge_runtime_message, dispatch_once, dispatch_prepared_result_once,
};
use crate::persistence::ModelEvent;
use crate::persistence::local_cli_tasks::{
    checkpoint_task, enqueue_task_result, load_messages, load_tasks,
};
use crate::persistence::model::{
    LocalCliMessage, LocalCliMessageState, LocalCliReceiptKind, LocalCliTask, LocalCliTaskState,
};

const ROOT_ENV: &str = "INFINISHELL_MAILBOX_RESTART_ROOT";
const TOKEN_ENV: &str = "INFINISHELL_MAILBOX_RESTART_TOKEN";
const COMMIT_TEST: &str = "ai::local_cli_mailbox::tests::restart::fixture_commit_sent_before_ack";
const VERIFY_TEST: &str = "ai::local_cli_mailbox::tests::restart::fixture_verify_after_restart";
const FIXTURE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Serialize, Deserialize)]
struct Input {
    token: Uuid,
    message: LocalCliMessage,
}

#[derive(Serialize, Deserialize)]
struct CommitPoint {
    token: Uuid,
    pid: u32,
    result: LocalCliMessage,
}

struct Fixture {
    child: Option<Child>,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Fixture {
    fn child(&mut self) -> &mut Child {
        self.child.as_mut().expect("夹具进程尚未回收")
    }

    fn diagnostics(&self) -> String {
        let stdout = fs::read_to_string(&self.stdout).unwrap_or_default();
        let stderr = fs::read_to_string(&self.stderr).unwrap_or_default();
        format!("stdout: {stdout}\nstderr: {stderr}")
    }

    fn kill_and_wait(&mut self) -> ExitStatus {
        self.child().kill().expect("必须由父测试终止仍存活的夹具");
        let status = self.child().wait().unwrap();
        // wait 之后清除句柄，不用已经回收的 PID 再执行清理。
        self.child.take();
        status
    }

    fn wait_success(&mut self) {
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        loop {
            if let Some(status) = self.child().try_wait().unwrap() {
                self.child.take();
                assert!(status.success(), "验证进程失败：{}", self.diagnostics());
                let output = fs::read_to_string(&self.stdout).unwrap();
                assert!(
                    output.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"),
                    "必须实际执行一个重启验证夹具：{output}"
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "重启验证超时：{}",
                self.diagnostics()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // 断言失败也只清理本测试拥有的进程；不按裸 PID 或进程名搜索。
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn write_record(path: &Path, value: &impl Serialize) {
    let mut temporary = NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    serde_json::to_writer(&mut temporary, value).unwrap();
    temporary.flush().unwrap();
    temporary.as_file().sync_all().unwrap();
    temporary.persist_noclobber(path).unwrap();
}

fn dispatch_marker(root: &Path, kind: &str, id: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("dispatches.log"))
        .unwrap();
    writeln!(file, "{kind}:{id}").unwrap();
    file.sync_all().unwrap();
}

fn fixture_input() -> (PathBuf, Input) {
    let root = PathBuf::from(env::var_os(ROOT_ENV).expect("内部夹具只能由父测试派生"))
        .canonicalize()
        .unwrap();
    assert_eq!(env::current_dir().unwrap().canonicalize().unwrap(), root);
    let input: Input = serde_json::from_slice(&fs::read(root.join("input.json")).unwrap()).unwrap();
    assert_eq!(env::var(TOKEN_ENV).unwrap(), input.token.to_string());
    (root, input)
}

fn start_fixture(name: &str, root: &Path, token: Uuid) -> Fixture {
    let stem = name.rsplit("::").next().unwrap();
    let stdout = root.join(format!("{stem}.stdout"));
    let stderr = root.join(format!("{stem}.stderr"));
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args([
            "--exact",
            name,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(root)
        .env_clear()
        .env(ROOT_ENV, root)
        .env(TOKEN_ENV, token.to_string())
        .stdin(Stdio::piped())
        .stdout(File::create(&stdout).unwrap())
        .stderr(File::create(&stderr).unwrap());
    for (key, value) in env::vars_os() {
        if [
            "PATH",
            "SYSTEMROOT",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "LANG",
            "LC_ALL",
        ]
        .contains(&key.to_string_lossy().to_ascii_uppercase().as_str())
        {
            command.env(key, value);
        }
    }
    for name in [
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "TMPDIR",
        "TMP",
        "TEMP",
    ] {
        let path = root.join("environment").join(name.to_ascii_lowercase());
        fs::create_dir_all(&path).unwrap();
        command.env(name, path);
    }
    Fixture {
        child: Some(command.spawn().unwrap()),
        stdout,
        stderr,
    }
}

fn checkpoint(sender: &SyncSender<ModelEvent>, task: LocalCliTask, expected: Option<i64>) {
    block_on(checkpoint_task(sender, task, expected).unwrap())
        .unwrap()
        .unwrap();
}

fn saved_messages(
    sender: &SyncSender<ModelEvent>,
    recipient: &str,
    generation: i64,
) -> Vec<LocalCliMessage> {
    block_on(load_messages(sender, recipient.into(), generation).unwrap())
        .unwrap()
        .unwrap()
}

#[test]
fn local_mailbox_process_restart_preserves_sent_and_result_claims() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let input = Input {
        token: Uuid::new_v4(),
        message: message(),
    };
    write_record(&root.join("input.json"), &input);
    let mut first = start_fixture(COMMIT_TEST, &root, input.token);
    let deadline = Instant::now() + FIXTURE_TIMEOUT;
    let ready = root.join("committed.json");
    while !ready.is_file() {
        if let Some(status) = first.child().try_wait().unwrap() {
            first.child.take();
            panic!(
                "提交夹具在停顿点前退出（{status}）：{}",
                first.diagnostics()
            );
        }
        assert!(
            Instant::now() < deadline,
            "等待真实 Sent 提交超时：{}",
            first.diagnostics()
        );
        thread::sleep(Duration::from_millis(20));
    }
    let committed: CommitPoint = serde_json::from_slice(&fs::read(&ready).unwrap()).unwrap();
    assert_eq!(committed.token, input.token);
    assert_eq!(committed.pid, first.child().id());
    assert!(first.child().try_wait().unwrap().is_none());
    assert!(!first.kill_and_wait().success());
    let original_dispatches = format!(
        "result:{}\nmessage:{}\n",
        committed.result.message_id, input.message.message_id
    );
    assert_eq!(
        fs::read_to_string(root.join("dispatches.log")).unwrap(),
        original_dispatches
    );

    let mut second = start_fixture(VERIFY_TEST, &root, input.token);
    second.wait_success();
    let verified: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("verified.json")).unwrap()).unwrap();
    assert_eq!(verified["token"], input.token.to_string());
    assert_eq!(verified["message_id"], input.message.message_id);
    assert_eq!(verified["result_id"], committed.result.message_id);
    assert_eq!(verified["old_message_state"], "sent");
    assert_eq!(verified["result_state"], "sent");
    assert_eq!(verified["new_message_state"], "acknowledged");
    let expected_dispatches = format!(
        "{original_dispatches}new-message:{}\n",
        verified["new_message_id"].as_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(root.join("dispatches.log")).unwrap(),
        expected_dispatches
    );
}

#[test]
#[ignore = "内部持久化夹具，只由 local_mailbox_process_restart 测试派生"]
fn fixture_commit_sent_before_ack() {
    let (root, input) = fixture_input();
    let database = root.join("tasks.sqlite");
    assert!(!database.exists());
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    for mut task in [
        task("parent", None),
        task("child", Some("parent")),
        task("result-child", Some("parent")),
    ] {
        task.working_directory = root.to_string_lossy().into_owned();
        checkpoint(&writer.sender, task, None);
    }
    let mut result_task = task("result-child", Some("parent"));
    result_task.working_directory = root.to_string_lossy().into_owned();
    result_task.revision = 1;
    result_task.state = LocalCliTaskState::Cancelled;
    result_task.result = Some("仅用于无模型故障注入的取消结果".into());
    checkpoint(&writer.sender, result_task, Some(1));
    let result = block_on(enqueue_task_result(&writer.sender, "result-child".into(), 1).unwrap())
        .unwrap()
        .unwrap()
        .unwrap();
    block_on(dispatch_prepared_result_once(
        &writer.sender,
        result.clone(),
        || async {
            dispatch_marker(&root, "result", &result.message_id);
            Ok(())
        },
    ))
    .unwrap();
    let saved_result = saved_messages(&writer.sender, "parent", 1);
    assert_eq!(saved_result.len(), 1);
    assert_eq!(saved_result[0].state, LocalCliMessageState::Sent);
    assert!(saved_result[0].receipt_kind.is_none());

    block_on(dispatch_once(
        &writer.sender,
        input.message.clone(),
        || async {
            // 生产 dispatch_once 已收到 Sent 事务提交确认，这里仍未产生任何 ACK。
            let stored = load_messages(&writer.sender, "child".into(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let mut expected = input.message.clone();
            expected.state = LocalCliMessageState::Sent;
            assert_eq!(stored, vec![expected]);
            dispatch_marker(&root, "message", &input.message.message_id);
            write_record(
                &root.join("committed.json"),
                &CommitPoint {
                    token: input.token,
                    pid: std::process::id(),
                    result: result.clone(),
                },
            );
            // 父测试只保持管道打开并强杀本进程；父意外退出时 EOF 防止夹具常驻。
            let mut byte = [0];
            std::io::stdin().read(&mut byte).unwrap();
            Err("父控制流关闭，未发生要求的强制终止".into())
        },
    ))
    .expect("此处不能正常返回；父测试必须在 ACK 前强杀进程");
}

#[test]
#[ignore = "内部重启验证夹具，只由 local_mailbox_process_restart 测试派生"]
fn fixture_verify_after_restart() {
    let (root, input) = fixture_input();
    let committed: CommitPoint =
        serde_json::from_slice(&fs::read(root.join("committed.json")).unwrap()).unwrap();
    assert_eq!(committed.token, input.token);
    let database = root.join("tasks.sqlite");
    assert!(database.is_file());
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let recovered = block_on(load_tasks(&writer.sender, true).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(recovered.len(), 3);
    let mut parent = recovered
        .iter()
        .find(|task| task.task_id == "parent")
        .unwrap()
        .clone();
    let child = recovered
        .iter()
        .find(|task| task.task_id == "child")
        .unwrap();
    assert_eq!(parent.state, LocalCliTaskState::Disconnected);
    assert_eq!(child.state, LocalCliTaskState::Disconnected);
    assert_eq!(child.generation, 1);
    let mut expected_original = input.message.clone();
    expected_original.state = LocalCliMessageState::Sent;
    assert_eq!(
        saved_messages(&writer.sender, "child", 1),
        vec![expected_original.clone()]
    );
    assert_eq!(
        block_on(dispatch_once(
            &writer.sender,
            input.message.clone(),
            || async { panic!("进程重启后，同一已发消息不能再次派发") }
        ))
        .unwrap(),
        LocalCliMessageState::Sent
    );

    // 恢复同代父任务后再重领结果，避免用“父已断线”掩盖唯一领取行为。
    parent.revision += 1;
    parent.state = LocalCliTaskState::Running;
    checkpoint(&writer.sender, parent, Some(1));
    let result = block_on(enqueue_task_result(&writer.sender, "result-child".into(), 1).unwrap())
        .unwrap()
        .unwrap()
        .unwrap();
    let mut expected_result = committed.result.clone();
    expected_result.state = LocalCliMessageState::Sent;
    assert_eq!(result, expected_result);
    block_on(dispatch_prepared_result_once(
        &writer.sender,
        committed.result.clone(),
        || async { panic!("进程重启后，已领取结果不能再次派发") },
    ))
    .unwrap();
    assert_eq!(
        saved_messages(&writer.sender, "parent", 1),
        vec![expected_result]
    );

    let mut next_child = (*child).clone();
    next_child.generation = 2;
    next_child.revision = 0;
    next_child.state = LocalCliTaskState::Queued;
    checkpoint(&writer.sender, next_child, Some(1));
    let mut next_message = input.message.clone();
    next_message.message_id = Uuid::new_v4().to_string();
    next_message.recipient_generation = 2;
    assert_eq!(
        block_on(dispatch_once(
            &writer.sender,
            next_message.clone(),
            || async {
                dispatch_marker(&root, "new-message", &next_message.message_id);
                Ok(())
            }
        ))
        .unwrap(),
        LocalCliMessageState::Sent
    );
    let current_ack = RuntimeEventKind::MessageAccepted {
        message_id: next_message.message_id.parse().unwrap(),
        turn_id: Some("fixture-turn-2".into()),
    };
    assert_eq!(
        block_on(acknowledge_runtime_message(
            &writer.sender,
            "child",
            1,
            &current_ack
        ))
        .unwrap(),
        None
    );
    let old_ack = RuntimeEventKind::MessageAccepted {
        message_id: input.message.message_id.parse().unwrap(),
        turn_id: Some("fixture-turn-1".into()),
    };
    assert_eq!(
        block_on(acknowledge_runtime_message(
            &writer.sender,
            "child",
            2,
            &old_ack
        ))
        .unwrap(),
        None
    );
    assert!(
        block_on(acknowledge_runtime_message(
            &writer.sender,
            "child",
            1,
            &old_ack
        ))
        .is_err()
    );
    let mut expected_next = next_message.clone();
    expected_next.state = LocalCliMessageState::Sent;
    assert_eq!(
        saved_messages(&writer.sender, "child", 2),
        vec![expected_next.clone()]
    );
    assert_eq!(
        saved_messages(&writer.sender, "child", 1),
        vec![expected_original.clone()]
    );
    assert_eq!(
        block_on(acknowledge_runtime_message(
            &writer.sender,
            "child",
            2,
            &current_ack
        ))
        .unwrap(),
        Some(LocalCliMessageState::Acknowledged)
    );
    expected_next.state = LocalCliMessageState::Acknowledged;
    expected_next.receipt_kind = Some(LocalCliReceiptKind::NativeProtocol);
    assert_eq!(
        saved_messages(&writer.sender, "child", 2),
        vec![expected_next]
    );
    assert_eq!(
        saved_messages(&writer.sender, "child", 1),
        vec![expected_original]
    );
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
    write_record(
        &root.join("verified.json"),
        &serde_json::json!({
            "token": input.token, "message_id": input.message.message_id,
            "result_id": result.message_id, "new_message_id": next_message.message_id,
            "old_message_state": "sent", "result_state": "sent", "new_message_state": "acknowledged",
        }),
    );
}
