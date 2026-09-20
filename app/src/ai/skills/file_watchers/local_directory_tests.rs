use std::fs;

use ai::skills::SkillProvider;
use async_channel::Receiver;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::{DirectoryWatcher, RepoMetadataModel};
use tempfile::TempDir;
use warp_util::host_id::HostId;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::*;
use crate::ai::skills::skill_manager::SkillWatcherEvent;

fn write_skill(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(format!(".grok/skills/{name}/SKILL.md"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!("---\nname: {name}\ndescription: 本地技能验收\n---\n正文\n"),
    )
    .unwrap();
    path
}

async fn receive_replacement(rx: &Receiver<SkillWatcherEvent>, directory: &Path) {
    let SkillWatcherEvent::SkillsDeleted { paths } = rx.recv().await.unwrap() else {
        panic!("必须先替换当前目录的旧快照");
    };
    assert!(paths.contains(&LocalOrRemotePath::Local(directory.join(".grok/skills"))));
    assert!(
        paths
            .iter()
            .all(|path| path.to_local_path().unwrap().starts_with(directory))
    );
}

#[test]
fn local_directory_filter_excludes_project_tree_and_accepts_new_provider_ancestors() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    assert!(is_skill_path(root, root));
    assert!(is_skill_path(root, &root.join(".grok")));
    assert!(is_skill_path(root, &root.join(".grok/skills")));
    assert!(is_skill_path(
        root,
        &root.join(".grok/skills/later/SKILL.md")
    ));
    assert!(!is_skill_path(root, &root.join("src")));
    assert!(!is_skill_path(root, &root.join(".grok/settings.json")));
    assert!(!is_skill_path(
        root,
        &root.join("subproject/.grok/skills/later/SKILL.md")
    ));
    assert!(!is_skill_path(root, root.parent().unwrap()));
}

#[test]
fn non_git_directory_discovers_and_refreshes_skills_without_repository_metadata() {
    let temp = TempDir::new().unwrap();
    let directory = dunce::canonicalize(temp.path()).unwrap();
    let nested = directory.join("subproject");
    write_skill(&nested, "must-not-scan");
    assert!(!directory.join(".git").exists());
    let (tx, rx) = async_channel::unbounded();
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let shared = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));
        let watcher = app.add_model(|_| LocalDirectorySkillWatcher::new(shared));
        watcher.update(&mut app, |watcher, ctx| {
            watcher.set_directory(Some(&LocalOrRemotePath::Local(directory.clone())), ctx);
        });
        receive_replacement(&rx, &directory).await;
        assert!(rx.try_recv().is_err());

        // 初次监听时 provider 不存在;创建事件到达后读取真实文件。
        let path = write_skill(&directory, "visible");
        watcher.update(&mut app, |watcher, ctx| {
            watcher.handle_event(
                &BulkFilesystemWatcherEvent {
                    added: [directory.join(".grok")].into(),
                    ..Default::default()
                },
                ctx,
            );
        });
        receive_replacement(&rx, &directory).await;
        let SkillWatcherEvent::SkillsAdded { skills } = rx.recv().await.unwrap() else {
            panic!("新建 provider 必须触发技能发现");
        };
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "visible");
        assert_eq!(skills[0].provider, SkillProvider::Grok);
        assert_eq!(skills[0].path, LocalOrRemotePath::Local(path.clone()));

        fs::remove_file(&path).unwrap();
        watcher.update(&mut app, |watcher, ctx| {
            watcher.handle_event(
                &BulkFilesystemWatcherEvent {
                    deleted: [path].into(),
                    ..Default::default()
                },
                ctx,
            );
        });
        receive_replacement(&rx, &directory).await;
        assert!(rx.try_recv().is_err());

        let remote = LocalOrRemotePath::Remote(RemotePath::new(
            HostId::new("remote-test".into()),
            StandardizedPath::try_new(directory.to_str().unwrap()).unwrap(),
        ));
        watcher.update(&mut app, |watcher, ctx| {
            watcher.set_directory(Some(&remote), ctx);
            assert!(watcher.directory.is_none());
            assert!(watcher.filesystem_watcher.is_none());
            watcher.handle_event(
                &BulkFilesystemWatcherEvent {
                    added: [directory.join(".grok")].into(),
                    ..Default::default()
                },
                ctx,
            );
        });
        assert!(rx.try_recv().is_err());
    });
}

#[test]
#[cfg(unix)]
fn symlink_cwd_reads_canonical_skills_and_reuses_the_same_watch() {
    let temp = TempDir::new().unwrap();
    let directory = temp.path().join("actual");
    let path = write_skill(&directory, "via-alias");
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&directory, &alias).unwrap();
    let canonical = dunce::canonicalize(&directory).unwrap();
    let expected_path = dunce::canonicalize(path).unwrap();
    let (tx, rx) = async_channel::unbounded();
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let shared = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));
        let watcher = app.add_model(|_| LocalDirectorySkillWatcher::new(shared));
        watcher.update(&mut app, |watcher, ctx| {
            watcher.set_directory(Some(&LocalOrRemotePath::Local(alias)), ctx);
            assert_eq!(watcher.directory.as_ref(), Some(&canonical));
        });
        receive_replacement(&rx, &canonical).await;
        let SkillWatcherEvent::SkillsAdded { skills } = rx.recv().await.unwrap() else {
            panic!("符号链接 cwd 必须能发现真实目录技能");
        };
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].path, LocalOrRemotePath::Local(expected_path));
        watcher.update(&mut app, |watcher, ctx| {
            let generation = watcher.generation;
            watcher.set_directory(Some(&LocalOrRemotePath::Local(canonical)), ctx);
            assert_eq!(watcher.generation, generation);
        });
        assert!(rx.try_recv().is_err());
    });
}

#[test]
fn stale_local_directory_snapshot_cannot_replace_a_newer_scan() {
    let temp = TempDir::new().unwrap();
    let directory = dunce::canonicalize(temp.path()).unwrap();
    let (tx, rx) = async_channel::unbounded();
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let shared = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));
        shared.update(&mut app, |watcher, ctx| {
            let old = watcher.begin_local_directory_refresh(&directory);
            let current = watcher.begin_local_directory_refresh(&directory);
            watcher.finish_local_directory_refresh(&directory, old, Vec::new(), ctx);
            assert!(rx.try_recv().is_err());
            watcher.finish_local_directory_refresh(&directory, current, Vec::new(), ctx);
        });
        receive_replacement(&rx, &directory).await;
        assert!(rx.try_recv().is_err());
    });
}
