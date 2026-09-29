#[cfg(windows)]
use std::fs::{File, OpenOptions};
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;

use futures::channel::oneshot;
use futures::future::{Either, select};
#[cfg(windows)]
use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
#[cfg(windows)]
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo};
#[cfg(windows)]
use windows::Win32::Security::{
    ACL, ACL_REVISION, DACL_SECURITY_INFORMATION, InitializeAcl, PSECURITY_DESCRIPTOR,
};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL, WRITE_DAC,
};

use super::*;
use crate::ai::blocklist::{PendingAttachment, PendingFile};
use crate::workspace::ToastStackEvent;

fn add_file(terminal: &ViewHandle<TerminalView>, app: &mut App, path: &Path) {
    terminal.update(app, |view, ctx| {
        view.ai_context_model.update(ctx, |model, ctx| {
            model.append_pending_attachments(
                vec![PendingAttachment::File(PendingFile {
                    file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
                    file_path: path.to_owned(),
                    mime_type: "text/plain".to_owned(),
                })],
                ctx,
            );
        });
    });
}

fn wait_for_submit(app: &mut App, terminal: &ViewHandle<TerminalView>) -> oneshot::Receiver<()> {
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    app.update(|ctx| {
        ctx.subscribe_to_view(terminal, move |_, event, _| {
            if let Event::WriteBytesToPty { bytes } = event {
                if &bytes[..] == b"\r" {
                    if let Some(sender) = sender.take() {
                        sender.send(()).unwrap();
                    }
                }
            }
        });
    });
    receiver
}

fn wait_for_rejection(app: &mut App) -> oneshot::Receiver<()> {
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    let stack = ToastStack::handle(app);
    app.update(|ctx| {
        ctx.subscribe_to_model(&stack, move |_, event, _| {
            if let ToastStackEvent::AddEphemeralToast { .. } = event {
                if let Some(sender) = sender.take() {
                    sender.send(()).unwrap();
                }
            }
        });
    });
    receiver
}

async fn await_submission_event(receiver: oneshot::Receiver<()>) {
    match select(receiver, Box::pin(Timer::after(Duration::from_secs(5)))).await {
        Either::Left((result, _)) => result.unwrap(),
        Either::Right((_, _)) => panic!("文件提交没有产生完成或拒绝事件"),
    }
}

#[cfg(windows)]
struct ReadDeniedFile {
    file: File,
    descriptor: PSECURITY_DESCRIPTOR,
    original_acl: *mut ACL,
    restored: bool,
}

#[cfg(windows)]
impl ReadDeniedFile {
    fn deny(path: &Path) -> Self {
        let file = OpenOptions::new()
            .access_mode((READ_CONTROL | WRITE_DAC).0)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
            .open(path)
            .unwrap();
        // 保留这个具备 WRITE_DAC 的句柄，测试结束后只恢复本轮私有文件的原 ACL。
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut original_acl = std::ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                HANDLE(file.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut original_acl),
                None,
                Some(&mut descriptor),
            )
        };
        assert_eq!(status.0, 0);

        let mut empty_acl = ACL::default();
        unsafe {
            InitializeAcl(
                &mut empty_acl,
                std::mem::size_of::<ACL>() as u32,
                ACL_REVISION,
            )
        }
        .unwrap();
        let denied = Self {
            file,
            descriptor,
            original_acl,
            restored: false,
        };
        let status = unsafe {
            SetSecurityInfo(
                HANDLE(denied.file.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&empty_acl),
                None,
            )
        };
        assert_eq!(status.0, 0);
        denied
    }

    fn restore(&mut self) {
        let status = unsafe {
            SetSecurityInfo(
                HANDLE(self.file.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(self.original_acl),
                None,
            )
        };
        assert_eq!(status.0, 0);
        self.restored = true;
    }
}

#[cfg(windows)]
impl Drop for ReadDeniedFile {
    fn drop(&mut self) {
        if !self.restored {
            let _ = unsafe {
                SetSecurityInfo(
                    HANDLE(self.file.as_raw_handle()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(self.original_acl),
                    None,
                )
            };
        }
        if !self.descriptor.0.is_null() {
            unsafe { LocalFree(Some(HLOCAL(self.descriptor.0))) };
        }
    }
}

#[test]
fn file_only_submission_delivers_exact_path_once_and_clears_accepted_cards() {
    for agent in [CLIAgent::Codex, CLIAgent::Claude] {
        App::test((), move |mut app| async move {
            let terminal = prepare_rich_cli_test(&mut app, agent);
            terminal.update(&mut app, |view, _| {
                view.model
                    .lock()
                    .set_mode(crate::terminal::model::ansi::Mode::BracketedPaste);
            });
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("中文 spaced file.txt");
            std::fs::write(&path, "private file bytes").unwrap();
            add_file(&terminal, &mut app, &path);
            let writes = collect_cli_test_writes(&mut app, &terminal);
            let submitted = wait_for_submit(&mut app, &terminal);

            submit_cli_test_input(&terminal, &mut app, "");
            submit_cli_test_input(&terminal, &mut app, "");
            await_submission_event(submitted).await;

            let delivered = String::from_utf8(writes.borrow().concat()).unwrap();
            // 原生输入约定为 JSON 路径数组，Windows 反斜杠必须保留转义边界。
            let expected_paths = serde_json::to_string(&[path.to_str().unwrap()]).unwrap();
            assert!(delivered.contains(&expected_paths));
            assert!(!delivered.contains("private file bytes"));
            assert_eq!(writes.borrow().len(), 2);
            terminal.read(&app, |view, ctx| {
                assert!(view.ai_context_model.as_ref(ctx).pending_files().is_empty());
                assert!(
                    CLIAgentSessionsModel::as_ref(ctx)
                        .session(view.view_id)
                        .unwrap()
                        .session_context
                        .query
                        .as_ref()
                        .unwrap()
                        .contains(&expected_paths)
                );
            });
        });
    }
}

#[test]
fn file_removed_after_card_creation_keeps_entire_composer_without_a_partial_pty_write() {
    for agent in [CLIAgent::Codex, CLIAgent::Claude] {
        App::test((), move |mut app| async move {
            let terminal = prepare_rich_cli_test(&mut app, agent);
            let root = tempfile::tempdir().unwrap();
            let first = root.path().join("first.txt");
            let removed = root.path().join("removed.txt");
            std::fs::write(&first, "first").unwrap();
            std::fs::write(&removed, "second").unwrap();
            add_file(&terminal, &mut app, &first);
            add_file(&terminal, &mut app, &removed);
            std::fs::remove_file(&removed).unwrap();
            let writes = collect_cli_test_writes(&mut app, &terminal);
            let rejected = wait_for_rejection(&mut app);

            submit_cli_test_input(&terminal, &mut app, "保留这份草稿");
            await_submission_event(rejected).await;

            assert!(writes.borrow().is_empty());
            terminal.read(&app, |view, ctx| {
                assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "保留这份草稿");
                let files = view.ai_context_model.as_ref(ctx).pending_files();
                assert_eq!(files.len(), 2);
                assert_eq!(files[0].file_path, first);
                assert_eq!(files[1].file_path, removed);
            });
        });
    }
}

#[test]
fn shell_mode_with_file_cards_keeps_bilingual_paths_and_draft_out_of_pty() {
    for agent in [CLIAgent::Codex, CLIAgent::Claude] {
        App::test((), move |mut app| async move {
            let terminal = prepare_rich_cli_test(&mut app, agent);
            let root = tempfile::tempdir().unwrap();
            let chinese = root.path().join("中文 空格.txt");
            let english = root.path().join("English spaced.txt");
            std::fs::write(&chinese, "first").unwrap();
            std::fs::write(&english, "second").unwrap();
            add_file(&terminal, &mut app, &chinese);
            add_file(&terminal, &mut app, &english);
            terminal.update(&mut app, |view, ctx| {
                view.input.update(ctx, |input, ctx| {
                    input.ai_input_model().update(ctx, |model, ctx| {
                        model.set_input_config(
                            InputConfig {
                                input_type: InputType::Shell,
                                is_locked: true,
                            },
                            false,
                            None,
                            ctx,
                        );
                    });
                });
            });
            let writes = collect_cli_test_writes(&mut app, &terminal);
            let rejected = wait_for_rejection(&mut app);

            submit_cli_test_input(&terminal, &mut app, "cat 中文 空格.txt English spaced.txt");
            await_submission_event(rejected).await;

            assert!(writes.borrow().is_empty());
            terminal.read(&app, |view, ctx| {
                let input = view.input.as_ref(ctx);
                assert_eq!(
                    input.buffer_text(ctx),
                    "cat 中文 空格.txt English spaced.txt"
                );
                assert!(!input.ai_input_model().as_ref(ctx).input_type().is_ai());
                assert!(input.ai_input_model().as_ref(ctx).is_input_type_locked());
                let files = view.ai_context_model.as_ref(ctx).pending_files();
                assert_eq!(files.len(), 2);
                assert_eq!(files[0].file_path, chinese);
                assert_eq!(files[1].file_path, english);
            });
        });
    }
}

#[cfg(unix)]
#[test]
fn unreadable_file_keeps_draft_and_cards_without_writing_to_pty() {
    use std::os::unix::fs::PermissionsExt;

    for agent in [CLIAgent::Codex, CLIAgent::Claude] {
        App::test((), move |mut app| async move {
            let terminal = prepare_rich_cli_test(&mut app, agent);
            let root = tempfile::tempdir().unwrap();
            let readable = root.path().join("readable.txt");
            let unreadable = root.path().join("unreadable.txt");
            std::fs::write(&readable, "first").unwrap();
            std::fs::write(&unreadable, "protected").unwrap();
            add_file(&terminal, &mut app, &readable);
            add_file(&terminal, &mut app, &unreadable);
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
            match std::fs::File::open(&unreadable) {
                Ok(_) => {
                    eprintln!("跳过权限拒绝用例：当前测试身份可读取 mode 000 文件");
                    return;
                }
                Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied),
            }
            let writes = collect_cli_test_writes(&mut app, &terminal);
            let rejected = wait_for_rejection(&mut app);

            submit_cli_test_input(&terminal, &mut app, "权限拒绝后保留草稿");
            await_submission_event(rejected).await;

            assert!(writes.borrow().is_empty());
            terminal.read(&app, |view, ctx| {
                assert_eq!(
                    view.input.as_ref(ctx).buffer_text(ctx),
                    "权限拒绝后保留草稿"
                );
                assert_eq!(view.ai_context_model.as_ref(ctx).pending_files().len(), 2);
            });
            assert_eq!(
                std::fs::metadata(&unreadable).unwrap().permissions().mode() & 0o777,
                0
            );
        });
    }
}

#[cfg(windows)]
#[test]
fn file_denied_after_card_creation_keeps_entire_composer_without_a_partial_pty_write() {
    for agent in [CLIAgent::Codex, CLIAgent::Claude] {
        App::test((), move |mut app| async move {
            let terminal = prepare_rich_cli_test(&mut app, agent);
            let root = tempfile::tempdir().unwrap();
            let readable = root.path().join("readable.txt");
            let denied_path = root.path().join("denied.txt");
            std::fs::write(&readable, "first").unwrap();
            std::fs::write(&denied_path, "protected").unwrap();
            add_file(&terminal, &mut app, &readable);
            add_file(&terminal, &mut app, &denied_path);
            let mut denied = ReadDeniedFile::deny(&denied_path);
            assert_eq!(
                std::fs::File::open(&denied_path).unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            let writes = collect_cli_test_writes(&mut app, &terminal);
            let rejected = wait_for_rejection(&mut app);

            submit_cli_test_input(&terminal, &mut app, "权限拒绝后保留草稿");
            await_submission_event(rejected).await;

            assert!(writes.borrow().is_empty());
            terminal.read(&app, |view, ctx| {
                assert_eq!(
                    view.input.as_ref(ctx).buffer_text(ctx),
                    "权限拒绝后保留草稿"
                );
                assert_eq!(view.ai_context_model.as_ref(ctx).pending_files().len(), 2);
            });
            denied.restore();
            assert_eq!(std::fs::read(&denied_path).unwrap(), b"protected");
        });
    }
}

#[test]
fn newly_added_file_and_reedited_draft_survive_an_earlier_submission() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Codex);
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first.txt");
        let second = root.path().join("second.txt");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        add_file(&terminal, &mut app, &first);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        let submitted = wait_for_submit(&mut app, &terminal);

        submit_cli_test_input(&terminal, &mut app, "第一轮");
        add_file(&terminal, &mut app, &second);
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("新草稿", ctx);
            });
        });
        await_submission_event(submitted).await;

        let delivered = String::from_utf8(writes.borrow().concat()).unwrap();
        let first_paths = serde_json::to_string(&[first.to_str().unwrap()]).unwrap();
        let second_path = serde_json::to_string(second.to_str().unwrap()).unwrap();
        assert!(delivered.contains(&first_paths));
        assert!(!delivered.contains(&second_path));
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "新草稿");
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_files().len(), 2);
        });
    });
}

#[test]
fn remote_cli_keeps_local_file_cards_and_draft_without_writing_paths() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Codex);
        terminal.update(&mut app, |view, ctx| {
            let mut session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap()
                .clone();
            session.remote_host = Some("user@remote".to_owned());
            session.input_state = CLIAgentInputState::Closed;
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(view.view_id, session, ctx);
            });
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        });
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("local-only.txt");
        std::fs::write(&path, "local").unwrap();
        add_file(&terminal, &mut app, &path);
        let writes = collect_cli_test_writes(&mut app, &terminal);

        submit_cli_test_input(&terminal, &mut app, "远端草稿");

        assert!(writes.borrow().is_empty());
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "远端草稿");
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_files().len(), 1);
        });
    });
}
