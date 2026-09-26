use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::process::Child;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use command::Stdio;
use command::blocking::Command;
use futures::channel::oneshot;
use futures::future::{Either, select};
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use warpui::r#async::Timer;
use warpui::{App, ViewHandle};

use super::*;
use crate::ai::blocklist::{PendingAttachment, PendingFile};
use crate::features::FeatureFlag;
use crate::persistence::{ModelEvent as PersistenceEvent, WriterHandles};
use crate::terminal::Event;
use crate::terminal::cli_agent_sessions::event::parse_event;
use crate::terminal::cli_agent_sessions::grok_leader_input::GrokLeaderInputError;
use crate::terminal::cli_agent_sessions::listener::CLIAgentSessionListener;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::workspace::{ToastStack, ToastStackEvent};

struct OwnedPtyFixture {
    master: File,
    shell: Child,
}

impl OwnedPtyFixture {
    fn new() -> Self {
        let pty = nix::pty::openpty(None, None).unwrap();
        let (master, slave) =
            unsafe { (File::from_raw_fd(pty.master), File::from_raw_fd(pty.slave)) };
        for file in [&master, &slave] {
            fcntl(file.as_raw_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).unwrap();
        }
        // shell 只执行阻塞的 read 内建命令；不加载用户环境、不派生后代或启动 Grok。
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "read -r infinishell_pty_fixture"])
            .env_clear()
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // 与真实 PTY 派生一致；这里只调用 fork 后安全的系统调用，spawn 会等待 exec 成功。
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Self {
            master,
            shell: command.spawn().unwrap(),
        }
    }
}

impl Drop for OwnedPtyFixture {
    fn drop(&mut self) {
        // 包括断言失败路径，精确回收本次直接子进程并等待退出，避免遗留 shell 或僵尸。
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }
}

fn owned_terminal(app: &mut App) -> (ViewHandle<TerminalView>, OwnedPtyFixture) {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| ToastStack);
    let terminal = add_window_with_terminal(app, None);
    let pty = OwnedPtyFixture::new();
    // 使用实际持有该控制终端的 shell；测试不连接原生 socket 或读取认证材料。
    let identity = LocalPtyIdentity::capture(pty.shell.id(), &pty.master).unwrap();
    terminal.update(app, |view, ctx| {
        let snapshot = {
            let mut model = view.model.lock();
            model.simulate_long_running_block("grok", "");
            model.set_local_pty_identity(Some(identity));
            let block = model.block_list().active_block();
            LaunchSnapshot {
                pty: identity,
                session: block.session_id().unwrap(),
                block: block.id().clone(),
                cwd: "/private/tmp".into(),
                shell: ShellType::Zsh,
            }
        };
        let owner = GrokTerminalOwner {
            version: 1,
            launch_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            binding_id: Uuid::new_v4(),
            input_revision: Uuid::new_v4(),
            permission_revision: Uuid::new_v4(),
            cli_version: "1.0.41".into(),
            model_id: "grok-4.7".into(),
            permission_mode: "default".into(),
        };
        let task = LocalCliTask {
            version: 1,
            task_id: Uuid::new_v4().to_string(),
            parent_task_id: None,
            parent_generation: None,
            harness: "grok".into(),
            working_directory: "/private/tmp".into(),
            config_json: json!({
                "execution_kind":"grok_owned_terminal",
                "grok_terminal":owner,
                "launch_manifest":"/private/tmp/unused-grok-test-launch.json",
                "launch_sha256":"unused"
            })
            .to_string(),
            native_session_id: Some(owner.session_id.to_string()),
            generation: 1,
            revision: 0,
            state: LocalCliTaskState::Running,
            result: None,
            terminal_evidence: None,
        };
        let lease = GrokOwnedInputLease::new(
            owner.binding_id,
            owner.input_revision,
            owner.permission_revision,
        )
        .unwrap();
        let view_id = view.view_id;
        let dispatcher = view.model_events_handle.clone();
        let listener = ctx.add_model(|ctx| {
            CLIAgentSessionListener::new(view_id, CLIAgent::Grok, &dispatcher, ctx)
        });
        let session_id = owner.session_id.to_string();
        let event = parse_event(
            Some("warp://cli-agent"),
            &json!({
                "v":1,"agent":"grok","event":"session_start",
                "session_id":session_id,"event_id":"owned-test-start",
                "cwd":"/private/tmp","permission_mode":"default","plugin_version":"0.1.5"
            })
            .to_string(),
        )
        .unwrap();
        CLIAgentSessionsModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_listener(
                view_id,
                CLIAgent::Grok,
                Some("/private/tmp".into()),
                None,
                Some(session_id),
                Some("0.1.5".into()),
                None,
                false,
                listener,
                ctx,
            );
            model.update_from_event(view_id, &event, ctx);
        });
        view.grok_owned_input = Some(OwnedInput {
            task,
            owner,
            snapshot,
            worker: None,
            lease: Some(lease),
            last_attempt: None,
            sending: false,
            invalidated: false,
            unconfirmed_drafts: HashMap::new(),
            binding: false,
            launch_command: None,
        });
    });
    (terminal, pty)
}

async fn received_event(receiver: oneshot::Receiver<()>) {
    match select(receiver, Box::pin(Timer::after(Duration::from_secs(5)))).await {
        Either::Left((result, _)) => result.unwrap(),
        Either::Right((_, _)) => panic!("owned 输入测试未收到预期事件"),
    }
}

#[test]
fn closed_owned_grok_context_restores_draft_without_writing_to_pty() {
    App::test((), |mut app| async move {
        let _review = FeatureFlag::HoaCodeReview.override_enabled(true);
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let (terminal, _pty) = owned_terminal(&mut app);
        let review = crate::ai::agent::AgentReviewCommentBatch {
            comments: Vec::new(),
            diff_set: Default::default(),
        };
        let review_prompt = crate::terminal::cli_agent::build_review_prompt(&review);
        let writes = Arc::new(AtomicUsize::new(0));
        let observed = writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if matches!(event, Event::WriteBytesToPty { .. }) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            });
        });
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, ctx| {
                model.close_input(view.view_id, false, ctx);
                model.set_draft(view.view_id, "保留原草稿".into());
            });
            assert!(matches!(
                view.try_send_text_to_cli_agent_or_rich_input("\n中文路径/file.rs".into(), ctx),
                Some(super::super::CliAgentRouting::RichInput)
            ));
            // 同一事件轮的第二次投递也必须排在草稿恢复后，不能覆盖或丢失任一段。
            view.send_review_to_cli_agent_or_rich_input(&review, ctx)
                .unwrap();
        });
        terminal.read(&app, |view, ctx| {
            assert!(view.is_cli_agent_rich_input_open(ctx));
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                format!("保留原草稿\n中文路径/file.rs{review_prompt}")
            );
        });
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn cancelled_owned_grok_context_callback_does_not_append_to_new_generation() {
    App::test((), |mut app| async move {
        let _review = FeatureFlag::HoaCodeReview.override_enabled(true);
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let (terminal, _pty) = owned_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, ctx| {
                model.close_input(view.view_id, false, ctx);
                model.set_draft(view.view_id, "只保留草稿".into());
            });
            assert!(
                view.try_send_text_to_cli_agent_or_rich_input("过期上下文".into(), ctx)
                    .is_some()
            );
            // 在打开事件和追加动作处理前取消；旧动作不能写入新的输入代次。
            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, ctx| {
                model.observe_ctrl_c_write(view.view_id, ctx);
            });
        });
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "只保留草稿");
        });
    });
}

#[test]
fn invalidated_owned_grok_context_keeps_saved_draft_closed() {
    App::test((), |mut app| async move {
        let _review = FeatureFlag::HoaCodeReview.override_enabled(true);
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let (terminal, _pty) = owned_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, ctx| {
                model.close_input(view.view_id, false, ctx);
                model.set_draft(view.view_id, "保留原草稿".into());
            });
            view.grok_owned_input.as_mut().unwrap().invalidated = true;
            assert!(
                view.try_send_text_to_cli_agent_or_rich_input("未送达上下文".into(), ctx)
                    .is_none()
            );
            assert!(!view.is_cli_agent_rich_input_open(ctx));
            assert_eq!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .draft_text
                    .as_deref(),
                Some("保留原草稿")
            );
        });
    });
}

#[test]
fn forwarded_ctrl_c_revokes_pending_owned_input_even_without_cancel_observation() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::CtrlCCancelsThirdPartyHarness.override_enabled(false);
        let (terminal, _pty) = owned_terminal(&mut app);
        let lease = terminal.read(&app, |view, _| {
            view.grok_owned_input
                .as_ref()
                .unwrap()
                .lease
                .clone()
                .unwrap()
        });
        let worker_lease = lease.clone();
        let (sent, received) = oneshot::channel();
        let mut sent = Some(sent);
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    assert_eq!(&**bytes, &[0x03]);
                    assert!(matches!(
                        worker_lease.claim_write(),
                        Err(GrokLeaderInputError::StaleBinding)
                    ));
                    sent.take().unwrap().send(()).unwrap();
                }
            });
        });
        terminal.update(&mut app, |view, ctx| view.write_to_pty(vec![0x03], ctx));
        received_event(received).await;
        assert!(lease.revoked());
    });
}

#[test]
fn forwarded_ctrl_c_keeps_claimed_input_able_to_receive_its_ack() {
    App::test((), |mut app| async move {
        let (terminal, _pty) = owned_terminal(&mut app);
        let lease = terminal.read(&app, |view, _| {
            view.grok_owned_input
                .as_ref()
                .unwrap()
                .lease
                .clone()
                .unwrap()
        });
        lease.claim_write().unwrap();
        let worker_lease = lease.clone();
        let (sent, received) = oneshot::channel();
        let mut sent = Some(sent);
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    assert_eq!(&**bytes, &[0x03]);
                    assert!(!worker_lease.revoked());
                    sent.take().unwrap().send(()).unwrap();
                }
            });
        });
        terminal.update(&mut app, |view, ctx| view.write_to_pty(vec![0x03], ctx));
        received_event(received).await;
        // 保留收据资格不等于允许第二次写入。
        assert!(matches!(
            lease.claim_write(),
            Err(GrokLeaderInputError::StaleBinding)
        ));
    });
}

#[test]
fn ordinary_pty_bytes_do_not_cancel_pending_owned_input() {
    App::test((), |mut app| async move {
        let (terminal, _pty) = owned_terminal(&mut app);
        let lease = terminal.read(&app, |view, _| {
            view.grok_owned_input
                .as_ref()
                .unwrap()
                .lease
                .clone()
                .unwrap()
        });
        terminal.update(&mut app, |view, ctx| view.write_to_pty(vec![b'c'], ctx));
        assert!(lease.claim_write().is_ok());
    });
}

#[test]
fn failed_second_attachment_keeps_first_unknown_draft_blocked() {
    App::test((), |mut app| async move {
        let (terminal, _pty) = owned_terminal(&mut app);
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let (sender, receiver) = mpsc::sync_channel(64);
        // 本场景必须在附件检查阶段停止，任何 CLI 持久化请求都属于越过预期边界。
        let writer = std::thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                if matches!(event, PersistenceEvent::Terminate) {
                    break;
                }
                if matches!(event, PersistenceEvent::LocalCliPersistence(_)) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        app.add_singleton_model(|_| {
            PersistenceWriter::new(Some(WriterHandles {
                handle: writer,
                sender,
            }))
        });
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("不存在的附件.txt");
        let first = Uuid::new_v4();
        let (rejected, rejection) = oneshot::channel();
        let mut rejected = Some(rejected);
        app.update(|ctx| {
            ctx.subscribe_to_model(&ToastStack::handle(ctx), move |_, event, _| {
                if let ToastStackEvent::AddEphemeralToast { .. } = event {
                    if let Some(rejected) = rejected.take() {
                        rejected.send(()).unwrap();
                    }
                }
            });
        });
        let second = terminal.update(&mut app, |view, ctx| {
            let owned = view.grok_owned_input.as_mut().unwrap();
            owned
                .unconfirmed_drafts
                .insert(first, ("未知输入 A".into(), Vec::new(), Vec::new()));
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("输入 B", ctx)
            });
            view.ai_context_model.update(ctx, |model, ctx| {
                model.append_pending_attachments(
                    vec![PendingAttachment::File(PendingFile {
                        file_name: "不存在的附件.txt".into(),
                        file_path: missing,
                        mime_type: "text/plain".into(),
                    })],
                    ctx,
                );
            });
            view.submit_owned_grok_input("输入 B".into(), ctx);
            let owned = view.grok_owned_input.as_ref().unwrap();
            assert!(owned.sending);
            assert_eq!(owned.unconfirmed_drafts.len(), 2);
            owned.owner.input_revision
        });
        received_event(rejection).await;
        terminal.update(&mut app, |view, ctx| {
            let owned = view.grok_owned_input.as_ref().unwrap();
            assert!(!owned.sending);
            assert_eq!(
                owned.unconfirmed_drafts.get(&first),
                Some(&("未知输入 A".into(), Vec::new(), Vec::new()))
            );
            assert!(!owned.unconfirmed_drafts.contains_key(&second));
            assert_eq!(owned.task.revision, 0);
            view.ai_context_model
                .update(ctx, |model, ctx| model.clear_pending_attachments(ctx));
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("未知输入 A", ctx)
            });
            view.submit_owned_grok_input("未知输入 A".into(), ctx);
            let owned = view.grok_owned_input.as_ref().unwrap();
            assert!(!owned.sending);
            assert_eq!(owned.owner.input_revision, second);
            assert_eq!(owned.unconfirmed_drafts.len(), 1);
            assert_eq!(owned.task.revision, 0);
        });
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    });
}
