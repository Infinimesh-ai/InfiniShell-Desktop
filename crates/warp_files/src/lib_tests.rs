use std::io::ErrorKind;
use std::rc::Rc;

use async_channel::{Receiver, unbounded};
use warpui_core::r#async::block_on;
use warpui_core::{App, ModelHandle};

// lib_tests.rs
use super::*;

const WRITE_TEST_PATH: &str = "test_data/test_write/";

/// This enum is used so that we can pass the event through the async channel.
/// io::Error is not clonable, so we can't clone the FileModelEvent.
#[derive(Debug)]
enum TestFileModelEvent {
    FileLoaded {
        id: FileId,
        content: String,
        _version: ContentVersion,
    },
    FileSaved,
    FailedToLoad(Rc<FileLoadError>),
    FailedToSave,
}

impl From<&FileModelEvent> for TestFileModelEvent {
    fn from(event: &FileModelEvent) -> Self {
        match event {
            FileModelEvent::FileLoaded {
                id,
                content,
                version,
            } => TestFileModelEvent::FileLoaded {
                id: *id,
                content: content.clone(),
                _version: *version,
            },
            FileModelEvent::FileSaved { .. } => TestFileModelEvent::FileSaved,
            FileModelEvent::FailedToLoad {
                id: _id,
                error: err,
            } => TestFileModelEvent::FailedToLoad(Rc::clone(err)),
            FileModelEvent::FailedToSave { .. } => TestFileModelEvent::FailedToSave,
            FileModelEvent::FileUpdated { .. } => {
                // For now, we don't handle file updated events in tests
                // This could be extended to include a FileUpdated variant in TestFileModelEvent if needed
                TestFileModelEvent::FileLoaded {
                    id: event.file_id(),
                    content: String::new(),
                    _version: ContentVersion::new(),
                }
            }
        }
    }
}

/// Setup a Tokio channel that will forward any events from the FileModel to the receiver.
fn setup_event_channel(
    app: &mut App,
    files: &ModelHandle<FileModel>,
) -> Receiver<TestFileModelEvent> {
    let (sender, receiver) = unbounded();
    app.update(|ctx| {
        ctx.subscribe_to_model(files, move |_model, event, _ctx| {
            block_on(sender.send(TestFileModelEvent::from(event)))
                .expect("Could not send the result");
        });
    });
    receiver
}

#[test]
fn test_load() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        // Load the test file.
        files.update(app, |model, ctx| {
            model.open(Path::new("test_data/test_file.rs"), false, ctx);
        });

        // Check that the first event out is the file loaded event.
        let event = receiver.recv().await.expect("Could not receive the result");
        match event {
            TestFileModelEvent::FileLoaded { content, .. } => {
                assert_eq!(content.as_bytes(), TEST_FILE_CONTENT)
            }
            _ => panic!("Failed to load file"),
        }
    });
}

#[test]
fn test_save_uninitialized_file() {
    App::test((), |mut app| async move {
        let app = &mut app;

        let files = app.add_singleton_model(FileModel::new);
        let id = FileId::new();

        // This file has not been initialized with the model.  Make sure trying to save it fails immediately.
        files.update(app, |model, ctx| {
            let result = model.save(
                id,
                "This file doesn't exist".to_string(),
                ContentVersion::new(),
                ctx,
            );
            assert!(
                matches!(result, Err(FileSaveError::NoFilePath(file_id)) if file_id == id),
                "expected NoFilePath error"
            );
        });
    });
}

#[test]
fn test_save_file() {
    // Create the test write directory if it doesn't exist.
    std::fs::create_dir_all(WRITE_TEST_PATH).unwrap();

    // Write the test file content to a random file in the test write directory.
    let path = PathBuf::from(WRITE_TEST_PATH).join("test_save_file.rs");
    std::fs::write(&path, TEST_FILE_CONTENT).unwrap();

    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        // Open the newly created file.
        let path_clone = path.clone();
        files.update(app, |model, ctx| {
            model.open(&path_clone, false, ctx);
        });

        let file_id = match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FileLoaded { id, .. } => id,
            _ => panic!("Failed to load file"),
        };

        let old_version = files.read(app, |files, _ctx| files.version(file_id));
        let new_version = ContentVersion::new();

        // Save new content to the file.
        files.update(app, |model, ctx| {
            let result = model.save(file_id, "Overwrite content".to_string(), new_version, ctx);
            assert!(result.is_ok());
        });

        // Make sure that the file saved event was emitted.
        match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FileSaved => (),
            _ => panic!("Failed to save file"),
        }

        // Make sure the content on disk matches the content we saved.
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Overwrite content");

        // Make sure the version was updated.
        let model_version = files.read(app, |files, _ctx| files.version(file_id));
        assert_ne!(old_version, model_version);
        assert_eq!(Some(new_version), model_version);
    });
}

#[test]
fn test_load_missing_file() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        // Load a file that doesn't exist.
        files.update(app, |model, ctx| {
            model.open(Path::new("test_data/missing_file.rs"), false, ctx);
        });

        // Check that the first event out is the failed to load event.
        let event = receiver.recv().await.expect("Could not receive the result");
        match event {
            TestFileModelEvent::FailedToLoad(err) => {
                assert!(
                    matches!(
                        err.as_ref(),
                        FileLoadError::IOError(error) if error.kind() == ErrorKind::NotFound
                    ),
                    "缺失文件应返回 NotFound,实际={err:?}"
                );
            }
            _ => panic!("Failed to load file"),
        }
    });
}

#[test]
fn test_save_missing_directory() {
    // Create the test write directory if it doesn't exist.
    let directory = PathBuf::from(WRITE_TEST_PATH).join("missing-directory");
    std::fs::create_dir_all(&directory).unwrap();

    // Write the test file content to a random file in the test write directory.
    let path = directory.join("test_save_missing_directory.rs");
    std::fs::write(&path, TEST_FILE_CONTENT).unwrap();

    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        // Save a file to a directory that doesn't exist.
        let file_id = files.update(app, |model, ctx| model.open(&path, false, ctx));

        // Check that the first event out is the successful load.
        let event = receiver.recv().await.expect("Could not receive the result");
        match event {
            TestFileModelEvent::FileLoaded { content, .. } => {
                assert_eq!(content.as_bytes(), TEST_FILE_CONTENT)
            }
            event => panic!("Failed to load file {event:?}"),
        }

        // Delete the directory that the file is in.
        std::fs::remove_dir_all(directory).unwrap();

        // Save new content to the file.
        files.update(app, |model, ctx| {
            let result = model.save(
                file_id,
                "Overwrite content".to_string(),
                ContentVersion::new(),
                ctx,
            );
            assert!(result.is_ok());
        });

        // Now we expect the save to succeed because ensure_parent_directories will create the missing directory
        match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FileSaved => {
                // Make sure the content on disk matches the content we saved.
                let content = std::fs::read_to_string(&path).unwrap();
                assert_eq!(content, "Overwrite content");
            }
            event => panic!("Save should have succeeded but got event: {event:?}"),
        }
    });
}

static TEST_FILE_CONTENT: &[u8] = include_bytes!("../test_data/test_file.rs");

#[test]
fn remote_move_reports_disconnected_host_through_future_and_event() {
    App::test((), |mut app| async move {
        app.add_singleton_model(RemoteServerManager::new);
        let files = app.add_singleton_model(FileModel::new);
        let (tx, rx) = unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(&files, move |_, event, _| {
                if let FileModelEvent::FailedToSave { id, error } = event {
                    tx.try_send((*id, error.clone())).unwrap();
                }
            });
        });
        let file_id = files.update(&mut app, |model, _| {
            model.register_remote_file(
                HostId::new("disconnected-host".to_owned()),
                StandardizedPath::try_new("/repo/source.txt").unwrap(),
            )
        });
        let future = files
            .update(&mut app, |model, ctx| {
                model.rename_and_save(
                    file_id,
                    PathBuf::from("/repo/target.txt"),
                    "updated\n".to_owned(),
                    ContentVersion::new(),
                    ctx,
                )
            })
            .expect("remote move must dispatch without a local file path");

        let error = future.await.unwrap_err();
        assert!(matches!(&*error, FileSaveError::RemoteError(_)));
        let (event_id, event_error) = rx.recv().await.unwrap();
        assert_eq!(event_id, file_id);
        assert_eq!(error.to_string(), event_error.to_string());
    });
}

#[test]
#[cfg(unix)]
fn restore_prepares_private_permissions_and_leaves_no_file_after_invalid_snapshot() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("secret");
    assert!(
        restore_local_file(
            &target,
            "secret",
            FilePermissions {
                unix_mode: None,
                readonly: false
            }
        )
        .is_err()
    );
    assert!(!target.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    restore_local_file(
        &target,
        "secret",
        FilePermissions {
            unix_mode: Some(0o600),
            readonly: false,
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "secret");
    assert!(
        restore_local_file(
            &target,
            "replacement",
            FilePermissions {
                unix_mode: Some(0o755),
                readonly: false
            }
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "secret");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
#[cfg(windows)]
fn restore_preserves_readonly_and_cleans_its_temporary_file_after_collision() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("readonly.txt");
    let permissions = FilePermissions {
        unix_mode: None,
        readonly: true,
    };
    restore_local_file(&target, "original", permissions).unwrap();
    assert!(std::fs::metadata(&target).unwrap().permissions().readonly());
    assert!(restore_local_file(&target, "replacement", permissions).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    let mut writable = std::fs::metadata(&target).unwrap().permissions();
    writable.set_readonly(false);
    std::fs::set_permissions(&target, writable).unwrap();
}

#[test]
fn native_undo_move_rejects_existing_destination_and_preserves_file_identity() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("moved.txt");
    let target = dir.path().join("original.txt");
    std::fs::write(&source, "moved content").unwrap();
    std::fs::write(&target, "new user file").unwrap();
    let identity = same_file::Handle::from_path(&source).unwrap();
    assert!(rename_file_noreplace(&source, &target).is_err());
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "moved content");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new user file");
    std::fs::remove_file(&target).unwrap();
    rename_file_noreplace(&source, &target).unwrap();
    assert!(!source.exists());
    assert_eq!(same_file::Handle::from_path(&target).unwrap(), identity);
}
