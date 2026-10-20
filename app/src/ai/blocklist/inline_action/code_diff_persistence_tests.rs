use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use ai::diff_validation::{DiffDelta, ParsedDiff, V4AHunk};
use warp_files::{FileModel, FileModelEvent};
use warpui::platform::WindowStyle;
use warpui::{App, ReadView};

use super::*;
use crate::ai::agent::FileEdit;
use crate::ai::blocklist::SessionContext;
use crate::ai::blocklist::action_model::{FileReadResult, apply_edits};
use crate::auth::AuthState;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::vim_registers::VimRegisters;

fn add_review(app: &mut App, path: &Path, diff: DiffType) -> ViewHandle<CodeDiffView> {
    add_review_diffs(
        app,
        vec![FileDiff::new(
            "original\n".to_owned(),
            path.to_str().unwrap().to_owned(),
            diff,
        )],
    )
}

fn add_review_diffs(app: &mut App, diffs: Vec<FileDiff>) -> ViewHandle<CodeDiffView> {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(|_| ToastStack);
    let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let mut view = CodeDiffView::new_passive(
            &"file-edit".to_owned().into(),
            None,
            AIIdentifiers::default(),
            RequestFileEditsFormatKind::V4A,
            false,
            None,
            ctx,
        );
        view.set_candidate_diffs(diffs, ctx);
        view
    });
    view
}

#[test]
#[cfg(unix)]
fn accept_symlink_delete_rejects_without_changing_link_or_referent() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let referent = dir.path().join("referent.txt");
        let link = dir.path().join("link.txt");
        fs::write(&referent, "original\n").unwrap();
        std::os::unix::fs::symlink(&referent, &link).unwrap();
        let view = add_review(&mut app, &link, DiffType::deletion(1));
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
        assert_eq!(fs::read_link(&link).unwrap(), referent);
        assert_eq!(fs::read_to_string(&referent).unwrap(), "original\n");
        assert!(
            view.update(&mut app, |view, ctx| view.revert_changes(ctx))
                .unwrap()
                .await
                .is_err()
        );
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Accepted
        )));
    });
}

#[test]
#[cfg(unix)]
fn accept_relative_symlink_move_across_directories_rejects_before_writing() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let referent = dir.path().join("referent.txt");
        let link = dir.path().join("link.txt");
        let destination = dir.path().join("other/moved-link.txt");
        fs::write(&referent, "original\n").unwrap();
        std::os::unix::fs::symlink("referent.txt", &link).unwrap();
        let view = add_review(
            &mut app,
            &link,
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                Some(destination.to_str().unwrap().to_owned()),
            ),
        );
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
        assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("referent.txt"));
        assert_eq!(fs::read_to_string(&referent).unwrap(), "original\n");
        assert!(!destination.exists());
    });
}

#[test]
fn accept_hard_link_collision_fails_before_writing_or_reporting_deletion() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("alias.txt");
        fs::write(&source, "original\n").unwrap();
        fs::hard_link(&source, &destination).unwrap();
        let view = add_review(
            &mut app,
            &source,
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                Some(destination.to_str().unwrap().to_owned()),
            ),
        );

        let result = app
            .update(|ctx| view.downgrade().accept_and_save(ctx))
            .await;

        assert!(matches!(
            result,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original\n");
    });
}

#[test]
fn failed_reverse_move_keeps_accepted_state_and_can_be_retried() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "original\n").unwrap();
        let view = add_review(
            &mut app,
            &source,
            DiffType::update(Vec::new(), Some(destination.to_str().unwrap().to_owned())),
        );
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        fs::remove_file(&destination).unwrap();

        let completion = view
            .update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap();
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Accepted
        )));
        assert!(completion.await.is_err());
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Accepted
        )));
        assert!(!source.exists());
        assert!(!destination.exists());

        fs::write(&destination, "original\n").unwrap();
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Reverted
        )));
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert!(!destination.exists());
    });
}

async fn prepare_move(app: &App, source: &Path, target: &Path) -> Vec<FileDiff> {
    let diffs = apply_edits(
        vec![FileEdit::Edit(ParsedDiff::V4AEdit {
            file: Some(source.to_str().unwrap().to_owned()),
            move_to: Some(target.to_str().unwrap().to_owned()),
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: String::new(),
                old: "original".to_owned(),
                new: "updated".to_owned(),
                post_context: String::new(),
            }],
        })],
        &SessionContext::new_for_test(),
        &AIIdentifiers::default(),
        app.background_executor(),
        Arc::new(AuthState::new_for_test()),
        false,
        |path| async move { FileReadResult::from(fs::read_to_string(path)) },
    )
    .await
    .unwrap();
    diffs
        .into_iter()
        .map(|diff| {
            let mut file = FileDiff::new(diff.original_content, diff.file_name, diff.diff_type);
            file.rename_target_original_content = diff.rename_target_original_content;
            file
        })
        .collect()
}

#[test]
fn prepared_move_over_existing_target_restores_both_original_contents() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        fs::write(&source, "original\n").unwrap();
        fs::write(&target, "old target\n").unwrap();
        let diffs = prepare_move(&app, &source, &target).await;
        let view = add_review_diffs(&mut app, diffs);
        let result = app
            .update(|ctx| view.downgrade().accept_and_save(ctx))
            .await;
        let RequestFileEditsResult::Success {
            updated_files,
            deleted_files,
            ..
        } = result
        else {
            panic!("expected move success");
        };
        assert_eq!(deleted_files, vec![source.to_str().unwrap().to_owned()]);
        assert_eq!(
            updated_files[0].file_context.file_name,
            target.to_str().unwrap()
        );
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "updated\n");
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "old target\n");
    });
}

#[test]
fn prepared_move_to_hard_link_alias_leaves_both_entries_unchanged() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("alias.txt");
        fs::write(&source, "original\n").unwrap();
        fs::hard_link(&source, &target).unwrap();
        let diffs = prepare_move(&app, &source, &target).await;
        let view = add_review_diffs(&mut app, diffs);
        let result = app
            .update(|ctx| view.downgrade().accept_and_save(ctx))
            .await;
        assert!(matches!(
            result,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
    });
}

#[test]
fn prepared_same_path_move_reports_only_an_update() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("./source.txt");
        fs::write(&source, "original\n").unwrap();
        let diffs = prepare_move(&app, &source, &target).await;
        let view = add_review_diffs(&mut app, diffs);
        let result = app
            .update(|ctx| view.downgrade().accept_and_save(ctx))
            .await;
        let RequestFileEditsResult::Success { deleted_files, .. } = result else {
            panic!("expected update success");
        };
        assert!(deleted_files.is_empty());
        assert_eq!(fs::read_to_string(&source).unwrap(), "updated\n");
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
    });
}

#[test]
#[cfg(unix)]
fn prepared_symlink_move_rejects_without_changing_either_link_identity() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source_referent = dir.path().join("source-referent.txt");
        let source = dir.path().join("source-link.txt");
        let target_referent = dir.path().join("target-referent.txt");
        let target = dir.path().join("target-link.txt");
        fs::write(&source_referent, "original\n").unwrap();
        fs::write(&target_referent, "old target\n").unwrap();
        std::os::unix::fs::symlink(&source_referent, &source).unwrap();
        std::os::unix::fs::symlink(&target_referent, &target).unwrap();
        let diffs = prepare_move(&app, &source, &target).await;
        let view = add_review_diffs(&mut app, diffs);
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
        assert_eq!(fs::read_link(&source).unwrap(), source_referent);
        assert_eq!(fs::read_link(&target).unwrap(), target_referent);
        assert_eq!(fs::read_to_string(&source_referent).unwrap(), "original\n");
        assert_eq!(
            fs::read_to_string(&target_referent).unwrap(),
            "old target\n"
        );
    });
}

#[test]
fn failed_target_restore_retries_without_repeating_the_reverse_move() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        fs::write(&source, "original\n").unwrap();
        fs::write(&target, "old target\n").unwrap();
        let diffs = prepare_move(&app, &source, &target).await;
        let view = add_review_diffs(&mut app, diffs);
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        let block_restore = Rc::new(Cell::new(true));
        let source_for_event = source.clone();
        let target_for_event = target.clone();
        app.update(|ctx| {
            let files = FileModel::handle(ctx);
            ctx.subscribe_to_model(&files, move |_, event, _| {
                if matches!(event, FileModelEvent::FileSaved { .. })
                    && source_for_event.exists()
                    && !target_for_event.exists()
                    && block_restore.replace(false)
                {
                    fs::create_dir(&target_for_event).unwrap();
                }
            });
        });
        let result = view
            .update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await;
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert!(target.is_dir());
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Accepted
        )));
        fs::remove_dir(&target).unwrap();
        let concurrent = dir.path().join("concurrent.txt");
        fs::write(&concurrent, "do not replace\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&concurrent, &target).unwrap();
            assert!(
                view.update(&mut app, |view, ctx| view.revert_changes(ctx))
                    .unwrap()
                    .await
                    .is_err()
            );
            assert_eq!(fs::read_link(&target).unwrap(), concurrent);
            assert_eq!(fs::read_to_string(&concurrent).unwrap(), "do not replace\n");
            fs::remove_file(&target).unwrap();
        }
        fs::hard_link(&concurrent, &target).unwrap();
        assert!(
            view.update(&mut app, |view, ctx| view.revert_changes(ctx))
                .unwrap()
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(&concurrent).unwrap(), "do not replace\n");
        fs::remove_file(&target).unwrap();
        fs::write(&target, "new user file\n").unwrap();
        assert!(
            view.update(&mut app, |view, ctx| view.revert_changes(ctx))
                .unwrap()
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new user file\n");
        fs::remove_file(&target).unwrap();
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "old target\n");
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Reverted
        )));
    });
}

#[test]
fn repeated_accept_without_edits_does_not_block_undo() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        fs::write(&source, "original\n").unwrap();
        let view = add_review(
            &mut app,
            &source,
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                None,
            ),
        );
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Reverted
        )));
    });
}

#[test]
fn accepting_during_undo_does_not_restart_file_operations() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        fs::write(&source, "original\n").unwrap();
        let view = add_review(&mut app, &source, DiffType::deletion(1));
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        let undo = view
            .update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap();
        assert!(
            view.update(&mut app, |view, ctx| view
                .try_accept_action_with_selection(AcceptSelection::Only, ctx))
                .is_err()
        );
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
        undo.await.unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Reverted
        )));
    });
}

#[test]
#[cfg(unix)]
fn deletion_undo_restores_private_and_executable_permissions() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o600, 0o755] {
        App::test((), |mut app| async move {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("source.sh");
            fs::write(&source, "original\n").unwrap();
            fs::set_permissions(&source, fs::Permissions::from_mode(mode)).unwrap();
            let view = add_review(&mut app, &source, DiffType::deletion(1));
            assert!(matches!(
                app.update(|ctx| view.downgrade().accept_and_save(ctx))
                    .await,
                RequestFileEditsResult::Success { .. }
            ));
            assert!(!source.exists());
            view.update(&mut app, |view, ctx| view.revert_changes(ctx))
                .unwrap()
                .await
                .unwrap();
            assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
            assert_eq!(
                fs::metadata(&source).unwrap().permissions().mode() & 0o777,
                mode
            );
        });
    }
}

#[test]
#[cfg(unix)]
fn replaced_move_target_undo_preserves_each_files_permissions() {
    use std::os::unix::fs::PermissionsExt;
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.sh");
        let target = dir.path().join("target.txt");
        fs::write(&source, "original\n").unwrap();
        fs::write(&target, "old target\n").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let diffs = prepare_move(&app, &source, &target).await;
        let view = add_review_diffs(&mut app, diffs);
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "old target\n");
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    });
}

#[test]
fn hard_link_deletion_rejects_without_unlinking_either_name() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let alias = dir.path().join("alias.txt");
        fs::write(&source, "original\n").unwrap();
        fs::hard_link(&source, &alias).unwrap();
        let view = add_review(&mut app, &source, DiffType::deletion(1));
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
        assert!(source.exists() && alias.exists());
        assert_eq!(fs::read_to_string(&alias).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
    });
}

#[test]
fn move_undo_preserves_new_file_at_original_path_and_remains_retryable() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        let user_file = dir.path().join("user-file.txt");
        fs::write(&source, "original\n").unwrap();
        let diffs = prepare_move(&app, &source, &target).await;
        let view = add_review_diffs(&mut app, diffs);
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        fs::write(&source, "new user file\n").unwrap();
        assert!(
            view.update(&mut app, |view, ctx| view.revert_changes(ctx))
                .unwrap()
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(&source).unwrap(), "new user file\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "updated\n");
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Accepted
        )));
        fs::rename(&source, &user_file).unwrap();
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&user_file).unwrap(), "new user file\n");
        assert!(!target.exists());
    });
}

#[test]
fn prepared_move_rejects_deleted_created_or_modified_target_before_writing_source() {
    for (expected, actual) in [
        (Some("old target\n"), None),
        (None, Some("new target\n")),
        (Some("old target\n"), Some("changed target\n")),
    ] {
        App::test((), |mut app| async move {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("source.txt");
            let target = dir.path().join("target.txt");
            fs::write(&source, "original\n").unwrap();
            if let Some(expected) = expected {
                fs::write(&target, expected).unwrap();
            }
            let diffs = prepare_move(&app, &source, &target).await;
            let view = add_review_diffs(&mut app, diffs);
            if let Some(actual) = actual {
                fs::write(&target, actual).unwrap();
            } else {
                fs::remove_file(&target).unwrap();
            }
            assert!(matches!(
                app.update(|ctx| view.downgrade().accept_and_save(ctx))
                    .await,
                RequestFileEditsResult::DiffApplicationFailed { .. }
            ));
            assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
            assert_eq!(fs::read_to_string(&target).ok().as_deref(), actual);
            if let Some(expected) = expected {
                fs::write(&target, expected).unwrap();
            } else {
                fs::remove_file(&target).unwrap();
            }
            assert!(matches!(
                app.update(|ctx| view.downgrade().accept_and_save(ctx))
                    .await,
                RequestFileEditsResult::Success { .. }
            ));
            assert_eq!(fs::read_to_string(&target).unwrap(), "updated\n");
            view.update(&mut app, |view, ctx| view.revert_changes(ctx))
                .unwrap()
                .await
                .unwrap();
            assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
            assert_eq!(fs::read_to_string(&target).ok().as_deref(), expected);
        });
    }
}

#[test]
fn rewind_during_save_is_an_error_and_can_retry_after_save_completes() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saving.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_review(&mut app, &path, DiffType::deletion(1));
        let save = app.update(|ctx| view.downgrade().accept_and_save(ctx));
        let revert = view
            .update(&mut app, |view, ctx| view.revert_changes(ctx))
            .expect("保存进行中必须返回失败，不能被上层当作无需撤销");
        assert!(revert.await.is_err());
        assert!(matches!(save.await, RequestFileEditsResult::Success { .. }));
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
    });
}

#[test]
fn duplicate_rewind_cannot_report_success_before_first_restore_completes() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("restoring.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_review(&mut app, &path, DiffType::deletion(1));
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        let first = view
            .update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap();
        let duplicate = view
            .update(&mut app, |view, ctx| view.revert_changes(ctx))
            .expect("恢复进行中不能作为无需撤销返回");
        assert!(duplicate.await.is_err());
        first.await.unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
    });
}

#[test]
fn rewind_one_card_partial_restore_retries_only_unrestored_files() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked.txt");
        let restored = dir.path().join("restored.txt");
        for path in [&blocked, &restored] {
            fs::write(path, "original\n").unwrap();
        }
        let view = add_review_diffs(
            &mut app,
            [&blocked, &restored]
                .into_iter()
                .map(|path| {
                    FileDiff::new(
                        "original\n".to_owned(),
                        path.to_str().unwrap().to_owned(),
                        DiffType::deletion(1),
                    )
                })
                .collect(),
        );
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        fs::write(&blocked, "external blocker\n").unwrap();
        assert!(
            view.update(&mut app, |view, ctx| view.revert_changes(ctx))
                .unwrap()
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(&blocked).unwrap(), "external blocker\n");
        assert_eq!(fs::read_to_string(&restored).unwrap(), "original\n");
        assert!(view.read(&app, |view, _| matches!(
            view.state(),
            CodeDiffState::Accepted
        )));

        // 同一张卡片中已恢复的文件也不能在重试时覆盖后来写入的内容。
        fs::write(&restored, "manual change after restore\n").unwrap();
        fs::remove_file(&blocked).unwrap();
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&blocked).unwrap(), "original\n");
        assert_eq!(
            fs::read_to_string(&restored).unwrap(),
            "manual change after restore\n"
        );
        assert!(view.read(&app, |view, _| matches!(
            view.state(),
            CodeDiffState::Reverted
        )));
    });
}
