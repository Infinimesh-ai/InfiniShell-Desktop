use std::fs;
use std::path::Path;

use ai::diff_validation::DiffDelta;
use line_ending::LineEnding;
use warp_core::ui::appearance::Appearance;
use warp_editor::content::buffer::{EditOrigin, InitialBufferState};
use warp_editor::model::CoreEditorModel;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;
use crate::AuthStateProvider;
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::code::editor::view::CodeEditorRenderOptions;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::ActiveSession;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn add_diff(app: &mut App, path: &Path, base: &str, diff: DiffType) -> ViewHandle<InlineDiffView> {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(ObjectStoreModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(UserWorkspaces::default_mock);
    let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let editor = ctx.add_typed_action_view(|ctx| {
            let editor = CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight)
                    .lazy_layout(),
                ctx,
            );
            editor.reset(InitialBufferState::plain_text(base), ctx);
            // 固定夹具为 LF，避免空缓冲区的平台默认换行符影响测试。
            let buffer = editor.model.as_ref(ctx).buffer().clone();
            buffer.update(ctx, |buffer, _| {
                buffer.set_line_ending_mode(LineEnding::LF);
            });
            editor
        });
        let mut view = InlineDiffView::new(
            editor,
            base.to_owned(),
            Some(diff),
            None,
            Some(StandardizedPath::try_from_local(path).unwrap()),
            ctx,
        );
        view.register_file(&DiffSessionType::Local, ctx);
        view
    });
    view
}

#[test]
fn accept_deletes_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delete.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(&mut app, &path, "original\n", DiffType::deletion(1));

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert!(!path.exists());
    });
}

async fn restore(app: &mut App, view: &ViewHandle<InlineDiffView>) {
    view.update(app, |view, ctx| view.restore_and_save_diff(ctx))
        .unwrap()
        .await
        .unwrap();
}

#[test]
fn accept_move_to_same_normalized_path_writes_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        let target = dir.path().join("./source.txt");
        let view = add_diff(
            &mut app,
            &path,
            "",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..1,
                    insertion: "updated\n".to_owned(),
                }],
                Some(target.to_str().unwrap().to_owned()),
            ),
        );

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "updated\n");
    });
}

#[test]
fn accept_creates_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");
        let view = add_diff(
            &mut app,
            &path,
            "",
            DiffType::creation("created\n".to_owned()),
        );

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "created\n");
    });
}

#[test]
fn restore_deleted_file_recreates_original_content() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deleted.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(&mut app, &path, "original\n", DiffType::deletion(1));
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        restore(&mut app, &view).await;

        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
    });
}

#[test]
fn restore_moved_file_restores_original_path_and_content() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.txt");
        let destination = dir.path().join("new.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                Some(destination.to_str().unwrap().to_owned()),
            ),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        restore(&mut app, &view).await;

        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
        assert!(!destination.exists());
    });
}

#[test]
fn accept_deletes_initially_empty_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        fs::write(&path, "").unwrap();
        let view = add_diff(&mut app, &path, "", DiffType::deletion(0));

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert!(!path.exists());
    });
}

#[test]
fn accept_update_to_empty_keeps_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: String::new(),
                }],
                None,
            ),
        );

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    });
}

#[test]
fn accept_moves_file_with_updated_content() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.txt");
        let destination = dir.path().join("nested/new.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                Some(destination.to_str().unwrap().to_owned()),
            ),
        );

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "updated\n");
    });
}

#[test]
fn accept_propagates_delete_failure() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.txt");
        let view = add_diff(&mut app, &path, "", DiffType::deletion(0));

        let result = view
            .update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await;

        assert!(result.is_err());
        assert!(!path.exists());
    });
}

#[test]
fn overlapping_and_repeated_restores_do_not_repeat_a_move() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.txt");
        let destination = dir.path().join("new.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(Vec::new(), Some(destination.to_str().unwrap().to_owned())),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        let first = view
            .update(&mut app, |view, ctx| view.restore_and_save_diff(ctx))
            .unwrap();
        let overlapping = view
            .update(&mut app, |view, ctx| view.restore_and_save_diff(ctx))
            .unwrap();
        first.await.unwrap();
        overlapping.await.unwrap();
        assert!(!destination.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");

        fs::write(&path, "subsequent edit\n").unwrap();
        restore(&mut app, &view).await;
        restore(&mut app, &view).await;
        assert_eq!(fs::read_to_string(&path).unwrap(), "subsequent edit\n");
        assert!(!destination.exists());
    });
}

#[test]
fn accepted_move_saves_subsequent_edits_at_the_destination() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        fs::write(&source, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &source,
            "original\n",
            DiffType::update(Vec::new(), Some(target.to_str().unwrap().to_owned())),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        view.update(&mut app, |view, ctx| {
            view.editor.update(ctx, |editor, ctx| {
                let saved_version = editor.version(ctx);
                editor.model.update(ctx, |model, ctx| {
                    model.select_all(ctx);
                    model.insert("edited again\n", EditOrigin::UserTyped, ctx);
                });
                assert_eq!(editor.text(ctx).into_string(), "edited again\n");
                assert_ne!(editor.version(ctx), saved_version);
            });
        });
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "edited again\n");
    });
}

#[test]
fn accepted_update_saves_subsequent_edits() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(Vec::new(), None),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        view.update(&mut app, |view, ctx| {
            view.editor.update(ctx, |editor, ctx| {
                let saved_version = editor.version(ctx);
                editor.model.update(ctx, |model, ctx| {
                    model.select_all(ctx);
                    model.insert("edited again\n", EditOrigin::UserTyped, ctx);
                });
                assert_eq!(editor.text(ctx).into_string(), "edited again\n");
                assert_ne!(editor.version(ctx), saved_version);
            });
        });
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "edited again\n");
    });
}

#[test]
fn failed_native_move_can_restore_the_saved_source_content() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("directory");
        fs::write(&source, "original 中文𠮷\r\nsecond\nlast\r").unwrap();
        fs::create_dir(&target).unwrap();
        let view = add_diff(
            &mut app,
            &source,
            "original 中文𠮷\r\nsecond\nlast\r",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..4,
                    insertion: "updated\n".to_owned(),
                }],
                Some(target.to_str().unwrap().to_owned()),
            ),
        );
        // 在底层真实产生“已写源内容、移动失败”，验证 GUI 消费类型化部分失败回执。
        let operation = view.update(&mut app, |view, ctx| {
            let version = view.editor.as_ref(ctx).version(ctx);
            let operation = FileModel::handle(ctx)
                .update(ctx, |model, ctx| {
                    model.rename_and_save(
                        view.backing_file_id.unwrap(),
                        target.clone(),
                        "updated\n".to_owned(),
                        version,
                        ctx,
                    )
                })
                .unwrap();
            view.track_operation(operation, false, ctx)
        });
        let error = operation.await.unwrap_err();
        assert!(matches!(
            error.as_ref(),
            FileSaveError::RenameFailedAfterSave(_)
        ));
        assert_eq!(fs::read_to_string(&source).unwrap(), "updated\n");
        restore(&mut app, &view).await;
        assert_eq!(
            fs::read(&source).unwrap(),
            "original 中文𠮷\r\nsecond\nlast\r".as_bytes()
        );
        assert!(target.is_dir());
    });
}

#[test]
fn restore_deleted_file_preserves_crlf_bytes() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deleted.txt");
        fs::write(&path, "original 中文𠮷\r\nsecond\r\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original 中文𠮷\r\nsecond\r\n",
            DiffType::deletion(2),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        assert!(!path.exists());

        restore(&mut app, &view).await;

        assert_eq!(
            fs::read(&path).unwrap(),
            "original 中文𠮷\r\nsecond\r\n".as_bytes()
        );
    });
}

#[test]
fn restore_deleted_file_retries_with_original_mixed_line_endings() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deleted.txt");
        let retained_blocker = dir.path().join("retained-blocker.txt");
        fs::write(&path, "original 中文𠮷\r\nsecond\nlast\r").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original 中文𠮷\r\nsecond\nlast\r",
            DiffType::deletion(3),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        assert!(!path.exists());
        fs::write(&path, "blocker\r\n").unwrap();

        let result = view
            .update(&mut app, |view, ctx| view.restore_and_save_diff(ctx))
            .unwrap()
            .await;
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"blocker\r\n");
        fs::rename(&path, &retained_blocker).unwrap();
        restore(&mut app, &view).await;

        assert_eq!(
            fs::read(&path).unwrap(),
            "original 中文𠮷\r\nsecond\nlast\r".as_bytes()
        );
        assert_eq!(fs::read(&retained_blocker).unwrap(), b"blocker\r\n");
    });
}

#[test]
fn restore_updated_file_preserves_original_mixed_line_endings() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updated.txt");
        fs::write(&path, "original 中文𠮷\r\nsecond\nlast\r").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original 中文𠮷\r\nsecond\nlast\r",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..4,
                    insertion: "updated\n".to_owned(),
                }],
                None,
            ),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"updated\n");

        restore(&mut app, &view).await;

        assert_eq!(
            fs::read(&path).unwrap(),
            "original 中文𠮷\r\nsecond\nlast\r".as_bytes()
        );
    });
}

#[test]
fn restore_moved_file_preserves_original_source_and_target_bytes() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        fs::write(&source, "original 中文𠮷\r\nsecond\nlast\r").unwrap();
        fs::write(&target, "target\r\nlast\n").unwrap();
        let view = add_diff(
            &mut app,
            &source,
            "original 中文𠮷\r\nsecond\nlast\r",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..4,
                    insertion: "updated\n".to_owned(),
                }],
                Some(target.to_str().unwrap().to_owned()),
            ),
        );
        view.update(&mut app, |view, _| {
            view.set_rename_target_original_content(Some("target\r\nlast\n".to_owned()));
        });
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(&target).unwrap(), b"updated\n");

        restore(&mut app, &view).await;

        assert_eq!(
            fs::read(&source).unwrap(),
            "original 中文𠮷\r\nsecond\nlast\r".as_bytes()
        );
        assert_eq!(fs::read(&target).unwrap(), b"target\r\nlast\n");
    });
}

#[test]
fn failed_native_move_can_retry_after_the_destination_is_repaired() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("directory");
        fs::write(&source, "original\n").unwrap();
        fs::create_dir(&target).unwrap();
        let view = add_diff(
            &mut app,
            &source,
            "original\n",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                Some(target.to_str().unwrap().to_owned()),
            ),
        );
        assert!(
            view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
                .unwrap()
                .await
                .is_err()
        );
        fs::remove_dir(&target).unwrap();
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "updated\n");
    });
}
