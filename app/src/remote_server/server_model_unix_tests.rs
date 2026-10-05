use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_io::Timer;
use instant::Instant;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use warp_core::SessionId;
use warpui::r#async::SpawnedFutureHandle;

use super::*;
use crate::remote_server::proto::{
    Abort, ClientMessage, RunCommandRequest, SessionBootstrapped, notification,
    session_scoped_request,
};
use crate::terminal::model::session::command_executor::{CommandExecutor, LocalCommandExecutor};

struct CommandCleanup {
    request: SpawnedFutureHandle,
    executor: Arc<LocalCommandExecutor>,
}

impl Drop for CommandCleanup {
    fn drop(&mut self) {
        // 失败时也只取消本测试 executor 持有的命令，不按进程名称或外部 PID 清理。
        self.request.abort();
        self.executor.cancel_active_commands();
    }
}

async fn wait_for_command_pid(path: &Path) -> Pid {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(contents) = fs::read_to_string(path)
            && let Ok(pid) = contents.parse()
        {
            return Pid::from_raw(pid);
        }
        assert!(Instant::now() < deadline, "等待测试命令写入 PID 超时");
        Timer::after(Duration::from_millis(10)).await;
    }
}

async fn wait_for_process_exit(pid: Pid) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match kill(pid, None) {
            Err(nix::errno::Errno::ESRCH) => return,
            Ok(()) | Err(nix::errno::Errno::EPERM) => {}
            Err(error) => panic!("检查测试命令进程 {pid} 失败：{error}"),
        }
        assert!(Instant::now() < deadline, "等待测试命令进程 {pid} 退出超时");
        Timer::after(Duration::from_millis(10)).await;
    }
}

#[test]
fn aborting_remote_run_command_kills_process_and_clears_in_progress_request() {
    App::test((), |mut app| async move {
        let temp_dir = tempfile::tempdir().expect("创建临时目录");
        let ready_file = temp_dir.path().join("ready");
        let session_id = SessionId::from(42u64);
        let connection_id = uuid::Uuid::new_v4();
        let request_id = RequestId::new();
        let model = test_model_handle(&mut app);

        let _command_cleanup = model.update(&mut app, |model, ctx| {
            model.handle_message(
                connection_id,
                ClientMessage::notification(notification::Message::SessionBootstrapped(
                    SessionBootstrapped {
                        session_id: session_id.as_u64(),
                        shell_type: "bash".to_string(),
                        shell_path: Some("/bin/bash".to_string()),
                        ..Default::default()
                    },
                )),
                ctx,
            );
            model.handle_message(
                connection_id,
                ClientMessage::session_scoped(
                    request_id.to_string(),
                    session_scoped_request::Message::RunCommand(RunCommandRequest {
                        // 通过真实 LocalCommandExecutor 和 crates/command 启动，exec 避免额外后代。
                        command: "printf %s \"$$\" > \"$READY_FILE\"; exec /bin/sleep 60"
                            .to_string(),
                        working_directory: None,
                        environment_variables: HashMap::from([(
                            "READY_FILE".to_string(),
                            ready_file.to_string_lossy().into_owned(),
                        )]),
                        session_id: session_id.as_u64(),
                    }),
                ),
                ctx,
            );
            CommandCleanup {
                request: model
                    .in_progress
                    .get(&request_id)
                    .expect("请求应处于执行中")
                    .clone(),
                executor: model
                    .executors
                    .get(&session_id)
                    .expect("会话 executor 应已创建")
                    .clone(),
            }
        });

        let command_pid = wait_for_command_pid(&ready_file).await;
        model.update(&mut app, |model, ctx| {
            model.handle_message(
                connection_id,
                ClientMessage::notification(notification::Message::Abort(Abort {
                    request_id_to_abort: request_id.to_string(),
                })),
                ctx,
            );
        });

        wait_for_process_exit(command_pid).await;
        assert!(model.read(&app, |model, _| model.in_progress.is_empty()));
    });
}
