use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;
use std::time::Duration;

use warpui::r#async::Timer;
use warpui::platform::WindowStyle;
use warpui::{AddSingletonModel, App, TypedActionView, ViewHandle};

use super::*;
use crate::editor::{AttachedImage, EditorAction, ImageContextOptions};
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::workspace::ToastStack;

fn new_editor(app: &mut App, generation: Uuid) -> ViewHandle<EditorView> {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| ToastStack);
    app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let mut editor = EditorView::new(Default::default(), ctx);
        editor.set_external_image_input_generation(generation, ctx);
        editor.update_image_context_options(
            ImageContextOptions::Enabled {
                unsupported_model: false,
                is_processing_attached_images: false,
                num_images_attached: 0,
                num_images_in_conversation: 0,
            },
            ctx,
        );
        editor
    })
    .1
}

fn png(name: &str) -> AttachedImage {
    let mut data = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut data, image::ImageFormat::Png)
        .unwrap();
    AttachedImage {
        data: data.into_inner(),
        mime_type: "image/png".into(),
        file_name: name.into(),
    }
}

fn collect_images(
    app: &mut App,
    editor: &ViewHandle<EditorView>,
) -> Rc<RefCell<Vec<(Uuid, Vec<String>)>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(editor, move |_, event, _| {
            if let Event::ImagesProcessed { generation, images } = event {
                received.borrow_mut().push((
                    *generation,
                    images
                        .0
                        .iter()
                        .map(|image| image.file_name.clone())
                        .collect(),
                ));
            }
        });
    });
    events
}

#[test]
fn external_editor_processes_real_disk_image_and_keeps_the_original_generation() {
    App::test((), |mut app| async move {
        let generation = Uuid::new_v4();
        let editor = new_editor(&mut app, generation);
        let events = collect_images(&mut app, &editor);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("中文 picture.png");
        std::fs::write(&path, png("unused").data).unwrap();
        editor.update(&mut app, |editor, ctx| {
            editor.read_and_process_images_async(1, vec![path.to_string_lossy().into_owned()], ctx);
            assert!(editor.image_input.pending.is_some());
        });
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(
            *events.borrow(),
            vec![(generation, vec!["中文 picture.png".into()])]
        );
        editor.read(&app, |editor, _| {
            assert!(editor.image_input.pending.is_none())
        });
    });
}

#[test]
fn cancelled_image_processing_cannot_append_to_a_replacement_draft() {
    App::test((), |mut app| async move {
        let generation = Uuid::new_v4();
        let replacement = Uuid::new_v4();
        let editor = new_editor(&mut app, generation);
        let events = collect_images(&mut app, &editor);
        editor.update(&mut app, |editor, ctx| {
            editor.process_and_attach_images_as_ai_context(1, vec![png("old.png")], ctx);
            editor.set_external_image_input_generation(replacement, ctx);
            editor.process_and_attach_images_as_ai_context(1, vec![png("new.png")], ctx);
        });
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(
            *events.borrow(),
            vec![(replacement, vec!["new.png".into()])]
        );
        editor.read(&app, |editor, _| {
            assert!(editor.image_input.pending.is_none())
        });
    });
}

#[test]
fn stale_file_picker_actions_neither_start_reads_nor_insert_context() {
    App::test((), |mut app| async move {
        let editor = new_editor(&mut app, Uuid::new_v4());
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&editor, move |_, event, _| {
                if let Event::FilePathsSelected { file_paths, .. } = event {
                    captured.borrow_mut().extend(file_paths.clone());
                }
            });
        });
        editor.update(&mut app, |editor, ctx| {
            let scope = editor.image_input_scope(ctx);
            editor.set_external_image_input_generation(Uuid::new_v4(), ctx);
            editor.handle_action(
                &EditorAction::ReadAndProcessImagesAsync {
                    num_images_user_attached: 1,
                    file_paths: vec!["/missing/stale.png".into()],
                    scope,
                },
                ctx,
            );
            editor.handle_action(
                &EditorAction::ProcessNonImageFiles {
                    file_paths: vec!["/missing/stale.txt".into()],
                    scope,
                },
                ctx,
            );
            assert!(editor.image_input.pending.is_none());
            assert!(editor.buffer_text(ctx).is_empty());
        });
        Timer::after(Duration::from_millis(20)).await;
        assert!(events.borrow().is_empty());
    });
}

#[test]
fn an_invalid_image_rejects_the_whole_managed_batch() {
    App::test((), |mut app| async move {
        let editor = new_editor(&mut app, Uuid::new_v4());
        let events = collect_images(&mut app, &editor);
        editor.update(&mut app, |editor, ctx| {
            editor.process_and_attach_images_as_ai_context(
                2,
                vec![
                    png("valid.png"),
                    AttachedImage {
                        data: b"invalid image".to_vec(),
                        mime_type: "image/png".into(),
                        file_name: "invalid.png".into(),
                    },
                ],
                ctx,
            );
        });
        Timer::after(Duration::from_millis(100)).await;
        assert!(events.borrow().is_empty());
        editor.read(&app, |editor, _| {
            assert!(editor.image_input.pending.is_none())
        });
    });
}
