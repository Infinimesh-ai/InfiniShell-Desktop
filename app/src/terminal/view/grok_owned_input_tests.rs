use std::cell::RefCell;
use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::process::Child;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use command::Stdio;
use command::blocking::Command;
use futures::channel::oneshot;
use futures::future::{Either, select};
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use warpui::r#async::Timer;
use warpui::geometry::vector::vec2f;
use warpui::{App, EntityIdSet, Presenter, TypedActionView, ViewHandle, WindowInvalidation};

use super::*;
use crate::ai::blocklist::{BlocklistAIContextEvent, PendingAttachment, PendingFile};
use crate::ai::llms::{LLMInfo, LLMPreferences};
use crate::features::FeatureFlag;
use crate::persistence::{ModelEvent as PersistenceEvent, WriterHandles};
use crate::terminal::Event;
use crate::terminal::cli_agent_sessions::CLIAgentInputEntrypoint;
use crate::terminal::cli_agent_sessions::event::parse_event;
use crate::terminal::cli_agent_sessions::grok_leader_input::GrokLeaderInputError;
use crate::terminal::cli_agent_sessions::listener::CLIAgentSessionListener;
use crate::terminal::input::InputAction;
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
            recovery_blocks_input: Arc::new(AtomicBool::new(false)),
            unconfirmed_drafts: HashMap::new(),
            binding: false,
            launch_command: None,
        });
    });
    (terminal, pty)
}

#[test]
fn new_block_in_same_pty_and_session_revokes_old_owned_input() {
    App::test((), |mut app| async move {
        let (terminal, _pty) = owned_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let (old_block, session) = {
                let owned = view.grok_owned_input.as_mut().unwrap();
                owned.launch_command = Some("grok".into());
                (owned.snapshot.block.clone(), owned.snapshot.session)
            };
            let lease = view
                .grok_owned_input
                .as_ref()
                .unwrap()
                .lease
                .clone()
                .unwrap();
            {
                let mut model = view.model.lock();
                assert!(view.is_owned_grok_command(&model));
                model.finish_block();
                model.simulate_long_running_block("grok", "");
                model
                    .block_list_mut()
                    .active_block_mut()
                    .set_session_id(session);
                let block = model.block_list().active_block();
                assert_ne!(block.id(), &old_block);
                assert_eq!(block.session_id(), Some(session));
                assert!(!view.is_owned_grok_command(&model));
            }
            assert!(!view.owned_grok_identity_matches());
            view.invalidate_owned_grok_if_changed(ctx);
            assert!(lease.revoked());
        });
    });
}

#[test]
fn saved_result_only_updates_matching_owned_task() {
    App::test((), |mut app| async move {
        let (terminal, _pty) = owned_terminal(&mut app);
        terminal.read(&app, |view, _| {
            let owned = view.grok_owned_input.as_ref().unwrap();
            let mut saved = owned.task.clone();
            saved.revision += 1;
            saved.result = Some("verified".into());
            assert!(owned.accepts_saved_result(&saved));

            let mut stale = saved.clone();
            stale.task_id = Uuid::new_v4().to_string();
            assert!(!owned.accepts_saved_result(&stale));
            stale = saved.clone();
            stale.generation += 1;
            assert!(!owned.accepts_saved_result(&stale));
            stale = saved.clone();
            stale.revision = owned.task.revision - 1;
            assert!(!owned.accepts_saved_result(&stale));
            stale = saved.clone();
            stale.native_session_id = Some(Uuid::new_v4().to_string());
            assert!(!owned.accepts_saved_result(&stale));
            stale = saved.clone();
            let mut config: serde_json::Value = serde_json::from_str(&stale.config_json).unwrap();
            config["launch_sha256"] = json!("other-launch");
            stale.config_json = config.to_string();
            assert!(!owned.accepts_saved_result(&stale));
            for field in [
                "launch_id",
                "binding_id",
                "input_revision",
                "permission_revision",
            ] {
                let mut config: serde_json::Value =
                    serde_json::from_str(&saved.config_json).unwrap();
                config["grok_terminal"][field] = json!(Uuid::new_v4());
                stale = saved.clone();
                stale.config_json = config.to_string();
                assert!(!owned.accepts_saved_result(&stale), "{field}");
            }
        });
    });
}

#[test]
fn owned_grok_clipboard_and_drop_attach_in_order_without_pty_write() {
    App::test((), |mut app| async move {
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let _images = FeatureFlag::ImageAsContext.override_enabled(true);
        let (terminal, _pty) = owned_terminal(&mut app);
        let writes = Arc::new(AtomicUsize::new(0));
        let observed = writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if matches!(event, Event::WriteBytesToPty { .. }) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            });
        });
        let png = include_bytes!("../../editor/view/figma_utils/non-figma-export.png");
        let (first, first_attached) = oneshot::channel();
        let mut first = Some(first);
        let context = terminal.read(&app, |view, _| view.ai_context_model.clone());
        app.update(|ctx| {
            ctx.subscribe_to_model(&context, move |model, event, ctx| {
                if matches!(event, BlocklistAIContextEvent::UpdatedPendingContext { .. })
                    && model.as_ref(ctx).pending_images().len() == 1
                {
                    if let Some(first) = first.take() {
                        first.send(()).unwrap();
                    }
                }
            });
        });
        terminal.update(&mut app, |view, ctx| {
            ctx.clipboard().write(warpui::clipboard::ClipboardContent {
                images: Some(vec![warpui::clipboard::ImageData {
                    data: png.to_vec(),
                    mime_type: "image/png".into(),
                    filename: Some("clip.png".into()),
                }]),
                ..Default::default()
            });
            assert!(view.paste_clipboard_image_to_cli_agent(ctx));
            assert!(view.is_cli_agent_rich_input_open(ctx));
        });
        received_event(first_attached).await;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("drop.png");
        std::fs::write(&path, png).unwrap();
        let (second, second_attached) = oneshot::channel();
        let mut second = Some(second);
        app.update(|ctx| {
            ctx.subscribe_to_model(&context, move |model, event, ctx| {
                if matches!(event, BlocklistAIContextEvent::UpdatedPendingContext { .. })
                    && model.as_ref(ctx).pending_images().len() == 2
                {
                    if let Some(second) = second.take() {
                        second.send(()).unwrap();
                    }
                }
            });
        });
        terminal.update(&mut app, |view, ctx| {
            view.paste_dropped_images_to_cli_agent(vec![path.to_string_lossy().into_owned()], ctx);
        });
        received_event(second_attached).await;
        terminal.read(&app, |view, ctx| {
            let images = view.ai_context_model.as_ref(ctx).pending_images();
            assert_eq!(images.len(), 2);
            assert_eq!(images[0].file_name, "clip.png");
            assert_eq!(images[1].file_name, "drop.png");
        });
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn owned_grok_open_composer_routes_image_drops_without_pty_write() {
    App::test((), |mut app| async move {
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let _images = FeatureFlag::ImageAsContext.override_enabled(true);
        let (terminal, _pty) = owned_terminal(&mut app);
        let writes = Arc::new(AtomicUsize::new(0));
        let observed = writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if matches!(event, Event::WriteBytesToPty { .. }) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            });
        });
        let root = tempfile::tempdir().unwrap();
        let png = include_bytes!("../../editor/view/figma_utils/non-figma-export.png");
        let first = root.path().join("first.png");
        let second = root.path().join("second.png");
        let other = root.path().join("other.txt");
        std::fs::write(&first, png).unwrap();
        std::fs::write(&second, png).unwrap();
        std::fs::write(&other, b"not an image").unwrap();
        let context = terminal.read(&app, |view, _| view.ai_context_model.clone());
        let (attached, received) = oneshot::channel();
        let mut attached = Some(attached);
        app.update(|ctx| {
            ctx.subscribe_to_model(&context, move |model, event, ctx| {
                if matches!(event, BlocklistAIContextEvent::UpdatedPendingContext { .. })
                    && model.as_ref(ctx).pending_images().len() == 2
                {
                    if let Some(attached) = attached.take() {
                        attached.send(()).unwrap();
                    }
                }
            });
        });
        terminal.update(&mut app, |view, ctx| {
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.is_cli_agent_rich_input_open(ctx));
            view.drag_and_drop_files(
                &[
                    first.to_string_lossy().into_owned(),
                    second.to_string_lossy().into_owned(),
                ],
                ctx,
            );
        });
        received_event(received).await;
        terminal.read(&app, |view, ctx| {
            let images = view.ai_context_model.as_ref(ctx).pending_images();
            assert_eq!(images.len(), 2);
            assert_eq!(images[0].file_name, "first.png");
            assert_eq!(images[1].file_name, "second.png");
        });
        terminal.update(&mut app, |view, ctx| {
            view.drag_and_drop_files(
                &[
                    first.to_string_lossy().into_owned(),
                    other.to_string_lossy().into_owned(),
                ],
                ctx,
            );
            view.drag_and_drop_files(&[other.to_string_lossy().into_owned()], ctx);
        });
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 2);
        });
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        terminal.update(&mut app, |view, ctx| {
            view.grok_owned_input.as_mut().unwrap().invalidated = true;
            view.drag_and_drop_files(&[first.to_string_lossy().into_owned()], ctx);
        });
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn owned_grok_file_drop_event_reaches_batch_guard_before_editor() {
    App::test((), |mut app| async move {
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let _images = FeatureFlag::ImageAsContext.override_enabled(true);
        let (terminal, _pty) = owned_terminal(&mut app);
        let window_id = app.window_ids()[0];
        let writes = Arc::new(AtomicUsize::new(0));
        let observed = writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if matches!(event, Event::WriteBytesToPty { .. }) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            });
        });
        let root = tempfile::tempdir().unwrap();
        let png = include_bytes!("../../editor/view/figma_utils/non-figma-export.png");
        let first = root.path().join("first.png");
        let second = root.path().join("second.png");
        let other = root.path().join("other.txt");
        std::fs::write(&first, png).unwrap();
        std::fs::write(&second, png).unwrap();
        std::fs::write(&other, b"not an image").unwrap();

        let editor_position_id = terminal.update(&mut app, |view, ctx| {
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            let editor = view.input.as_ref(ctx).editor().clone();
            ctx.focus(&editor);
            assert!(view.is_cli_agent_rich_input_open(ctx));
            view.input.as_ref(ctx).editor_save_position_id()
        });
        terminal.read(&app, |view, ctx| {
            assert!(view.input.as_ref(ctx).editor().is_focused(ctx));
        });

        let (mixed_reported, mixed_notification) = oneshot::channel();
        let (unavailable_reported, unavailable_notification) = oneshot::channel();
        let mut mixed_reported = Some(mixed_reported);
        let mut unavailable_reported = Some(unavailable_reported);
        let non_image = crate::t!("cli-agent-input-non-image-drop-unavailable");
        let unavailable = crate::t!("cli-agent-grok-owned-input-unavailable");
        app.update(|ctx| {
            ctx.subscribe_to_model(&ToastStack::handle(ctx), move |_, event, _| {
                if let ToastStackEvent::AddEphemeralToast { toast, .. } = event {
                    if toast.main_text() == non_image
                        && let Some(reported) = mixed_reported.take()
                    {
                        reported.send(()).unwrap();
                    }
                    if toast.main_text() == unavailable
                        && let Some(reported) = unavailable_reported.take()
                    {
                        reported.send(()).unwrap();
                    }
                }
            });
        });

        let mut updated = EntityIdSet::default();
        for view_id in app.update(|ctx| ctx.view_ids_for_window(window_id)) {
            updated.insert(view_id);
        }
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        app.update(|ctx| {
            presenter.borrow_mut().invalidate(
                WindowInvalidation {
                    updated,
                    ..Default::default()
                },
                ctx,
            );
            presenter
                .borrow_mut()
                .build_scene(vec2f(1024., 768.), 1., None, ctx);
        });
        let editor_bounds = presenter
            .borrow()
            .position_cache()
            .get_position(&editor_position_id)
            .unwrap();
        let location = editor_bounds.origin() + vec2f(8., 8.);
        assert!(editor_bounds.contains_point(location));
        let original_draft =
            terminal.read(&app, |view, ctx| view.input.as_ref(ctx).buffer_text(ctx));

        let drop_files = |app: &mut App, paths: Vec<String>| {
            app.update(|ctx| {
                ctx.simulate_window_event(
                    warpui::Event::DragAndDropFiles { paths, location },
                    window_id,
                    presenter.clone(),
                );
            });
        };
        drop_files(
            &mut app,
            vec![
                first.to_string_lossy().into_owned(),
                other.to_string_lossy().into_owned(),
            ],
        );
        received_event(mixed_notification).await;
        terminal.read(&app, |view, ctx| {
            assert!(
                view.ai_context_model
                    .as_ref(ctx)
                    .pending_images()
                    .is_empty()
            );
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), original_draft);
        });

        let context = terminal.read(&app, |view, _| view.ai_context_model.clone());
        let (attached, received) = oneshot::channel();
        let mut attached = Some(attached);
        app.update(|ctx| {
            ctx.subscribe_to_model(&context, move |model, event, ctx| {
                if matches!(event, BlocklistAIContextEvent::UpdatedPendingContext { .. })
                    && model.as_ref(ctx).pending_images().len() == 1
                {
                    if let Some(attached) = attached.take() {
                        attached.send(()).unwrap();
                    }
                }
            });
        });
        drop_files(&mut app, vec![first.to_string_lossy().into_owned()]);
        received_event(received).await;
        terminal.read(&app, |view, ctx| {
            let images = view.ai_context_model.as_ref(ctx).pending_images();
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].file_name, "first.png");
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), original_draft);
        });

        terminal.update(&mut app, |view, _| {
            view.grok_owned_input.as_mut().unwrap().invalidated = true;
        });
        drop_files(&mut app, vec![second.to_string_lossy().into_owned()]);
        received_event(unavailable_notification).await;
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 1);
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), original_draft);
        });
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    });
}

async fn received_event(receiver: oneshot::Receiver<()>) {
    match select(receiver, Box::pin(Timer::after(Duration::from_secs(5)))).await {
        Either::Left((result, _)) => result.unwrap(),
        Either::Right((_, _)) => panic!("owned 输入测试未收到预期事件"),
    }
}

#[test]
fn owned_grok_image_picker_ignores_builtin_text_only_model() {
    App::test((), |mut app| async move {
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let _images = FeatureFlag::ImageAsContext.override_enabled(true);
        let (terminal, _pty) = owned_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
                preferences.add_agent_mode_model_for_test(LLMInfo::new_for_test("text-only"));
                preferences.set_agent_mode_llm_override(view.view_id, "text-only".into(), ctx);
            });
            assert!(!LLMPreferences::as_ref(ctx).vision_supported(ctx, Some(view.view_id)));
            assert!(view.owned_grok_identity_matches());
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        });
        terminal.update(&mut app, |view, ctx| {
            let generation = CLIAgentSessionsModel::as_ref(ctx)
                .input_generation(view.view_id)
                .unwrap();
            view.input.update(ctx, |input, ctx| {
                input.handle_action(&InputAction::SelectCLIAttachment { generation }, ctx);
                let options = &input.editor().as_ref(ctx).image_context_options;
                assert!(options.is_enabled());
                assert!(!options.is_unsupported_model());
                // 文件选择与异步图片处理之间的刷新同样不能重新套用内置模型门禁。
                input.set_is_processing_attached_images(true, ctx);
                assert!(
                    !input
                        .editor()
                        .as_ref(ctx)
                        .image_context_options
                        .is_enabled()
                );
                input.set_is_processing_attached_images(false, ctx);
                assert!(
                    input
                        .editor()
                        .as_ref(ctx)
                        .image_context_options
                        .is_enabled()
                );
            });
        });
    });
}

#[test]
fn closing_owned_grok_rich_input_restores_builtin_image_gate() {
    App::test((), |mut app| async move {
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let _images = FeatureFlag::ImageAsContext.override_enabled(true);
        let (terminal, _pty) = owned_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
                preferences.add_agent_mode_model_for_test(LLMInfo::new_for_test("text-only"));
                preferences.set_agent_mode_llm_override(view.view_id, "text-only".into(), ctx);
            });
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        });
        terminal.read(&app, |view, ctx| {
            assert!(
                view.input
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .image_context_options
                    .is_enabled()
            );
        });
        terminal.update(&mut app, |view, ctx| {
            view.close_cli_agent_rich_input_and_disable_auto_toggle(ctx);
        });
        terminal.read(&app, |view, ctx| {
            let options = &view
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .image_context_options;
            assert!(!options.is_enabled());
        });
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.set_input_mode_agent(true, ctx);
            });
        });
        terminal.read(&app, |view, ctx| {
            let options = &view
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .image_context_options;
            assert!(options.is_unsupported_model());
        });
    });
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
fn current_turn_recovery_preserves_draft_until_result_cas_finishes() {
    App::test((), |mut app| async move {
        let (terminal, _pty) = owned_terminal(&mut app);
        let (reported, notification) = oneshot::channel();
        let mut reported = Some(reported);
        let unavailable = crate::t!("cli-agent-grok-owned-input-unavailable");
        app.update(|ctx| {
            ctx.subscribe_to_model(&ToastStack::handle(ctx), move |_, event, _| {
                if let ToastStackEvent::AddEphemeralToast { toast, .. } = event
                    && toast.main_text() == unavailable
                    && let Some(reported) = reported.take()
                {
                    reported.send(()).unwrap();
                }
            });
        });
        terminal.update(&mut app, |view, ctx| {
            let owned = view.grok_owned_input.as_mut().unwrap();
            let original_task = owned.task.clone();
            let original_owner = owned.owner.clone();
            owned.recovery_blocks_input.store(true, Ordering::SeqCst);
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("补查期间保留草稿", ctx)
            });
            view.submit_owned_grok_input("补查期间保留草稿".into(), ctx);
            let owned = view.grok_owned_input.as_ref().unwrap();
            assert!(!owned.sending);
            assert_eq!(owned.task, original_task);
            assert_eq!(owned.owner, original_owner);
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "补查期间保留草稿");
            owned.recovery_blocks_input.store(false, Ordering::SeqCst);
        });
        received_event(notification).await;
    });
}

#[test]
fn lost_owned_grok_session_reports_unavailable_and_keeps_draft() {
    App::test((), |mut app| async move {
        let (terminal, _pty) = owned_terminal(&mut app);
        let (reported, notification) = oneshot::channel();
        let mut reported = Some(reported);
        let unavailable = crate::t!("cli-agent-grok-owned-input-unavailable");
        let writes = Arc::new(AtomicUsize::new(0));
        let observed = writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&ToastStack::handle(ctx), move |_, event, _| {
                if let ToastStackEvent::AddEphemeralToast { toast, .. } = event
                    && toast.main_text() == unavailable
                {
                    if let Some(reported) = reported.take() {
                        reported.send(()).unwrap();
                    }
                }
            });
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if matches!(event, Event::WriteBytesToPty { .. }) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            });
        });
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx)
                .update(ctx, |model, ctx| model.remove_session(view.view_id, ctx));
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("会话消失后保留草稿", ctx)
            });
            view.submit_owned_grok_input("会话消失后保留草稿".into(), ctx);
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "会话消失后保留草稿"
            );
            assert!(!view.grok_owned_input.as_ref().unwrap().sending);
        });
        received_event(notification).await;
        assert_eq!(writes.load(Ordering::SeqCst), 0);
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
