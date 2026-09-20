use std::future::{Future, ready};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ai::skills::SKILL_PROVIDER_DEFINITIONS;
use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{Entity, ModelContext, ModelHandle};
use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};

use super::SkillWatcher;
use super::utils::read_skills_from_directories;

/// 输入面板持有此模型,离开本地 cwd 或销毁面板时结束对应监听。
pub(crate) struct LocalDirectorySkillWatcher {
    skill_watcher: ModelHandle<SkillWatcher>,
    filesystem_watcher: Option<ModelHandle<BulkFilesystemWatcher>>,
    directory: Option<PathBuf>,
    generation: u64,
}

impl LocalDirectorySkillWatcher {
    pub(crate) fn new(skill_watcher: ModelHandle<SkillWatcher>) -> Self {
        Self {
            skill_watcher,
            filesystem_watcher: None,
            directory: None,
            generation: 0,
        }
    }

    pub(crate) fn set_directory(
        &mut self,
        directory: Option<&LocalOrRemotePath>,
        ctx: &mut ModelContext<Self>,
    ) {
        let directory = directory
            .and_then(LocalOrRemotePath::to_local_path)
            .and_then(|path| dunce::canonicalize(path).ok())
            .filter(|path| path.is_dir());
        if self.directory == directory {
            return;
        }
        self.generation += 1;
        if let Some(old_directory) = self.directory.take()
            && let Some(watcher) = &self.filesystem_watcher
        {
            // 已入队注销不依赖 future 被轮询;旧扫描仍由 generation 拒绝。
            std::mem::drop(
                watcher.update(ctx, |watcher, _| watcher.unregister_path(&old_directory)),
            );
        }
        self.directory = directory;
        let Some(directory) = self.directory.clone() else {
            self.filesystem_watcher = None;
            return;
        };
        let watcher = self.filesystem_watcher.get_or_insert_with(|| {
            let watcher = ctx.add_model(|ctx| {
                if cfg!(test) {
                    BulkFilesystemWatcher::new_for_test()
                } else {
                    BulkFilesystemWatcher::new(Duration::from_millis(250), ctx)
                }
            });
            ctx.subscribe_to_model(&watcher, |me, _, event, ctx| {
                me.handle_event(event, ctx);
            });
            watcher
        });
        let filter_directory = directory.clone();
        let filter = Arc::new(move |path: &Path| is_skill_path(&filter_directory, path));
        // 递归监听仅进入固定 provider 路径;也接收尚未存在的 provider 目录创建。
        let registration = watcher.update(ctx, |watcher, _| {
            watcher.register_path(
                &directory,
                WatchFilter::with_filter(filter.clone(), filter),
                RecursiveMode::Recursive,
            )
        });
        self.refresh(registration, ctx);
    }

    fn handle_event(&mut self, event: &BulkFilesystemWatcherEvent, ctx: &mut ModelContext<Self>) {
        let Some(directory) = &self.directory else {
            return;
        };
        if event
            .added
            .iter()
            .chain(&event.modified)
            .chain(&event.deleted)
            .chain(event.moved.iter().flat_map(|(to, from)| [to, from]))
            .any(|path| is_skill_path(directory, path))
        {
            self.refresh(ready(Ok(())), ctx);
        }
    }

    fn refresh(
        &mut self,
        registration: impl Future<Output = anyhow::Result<()>> + Send + 'static,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(directory) = self.directory.clone() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let refresh_generation = self.skill_watcher.update(ctx, |watcher, _| {
            watcher.begin_local_directory_refresh(&directory)
        });
        let scan_directory = directory.clone();
        ctx.spawn(
            async move {
                // 注册失败时仍完成当前读取;注册层已记录失败,不阻塞技能菜单。
                let _ = registration.await;
                read_skills_from_directories(
                    SKILL_PROVIDER_DEFINITIONS
                        .iter()
                        .map(|provider| scan_directory.join(&provider.skills_path)),
                )
            },
            move |me, skills, ctx| {
                if me.generation != generation || me.directory.as_ref() != Some(&directory) {
                    return;
                }
                me.skill_watcher.update(ctx, |watcher, ctx| {
                    watcher.finish_local_directory_refresh(
                        &directory,
                        refresh_generation,
                        skills,
                        ctx,
                    );
                });
            },
        );
    }
}

/// provider 祖先必须可进入,普通项目目录不能被递归遍历。
fn is_skill_path(directory: &Path, path: &Path) -> bool {
    path.starts_with(directory)
        && SKILL_PROVIDER_DEFINITIONS.iter().any(|provider| {
            let root = directory.join(&provider.skills_path);
            path.starts_with(&root) || root.starts_with(path)
        })
}

impl Entity for LocalDirectorySkillWatcher {
    type Event = ();
}

#[cfg(test)]
#[path = "local_directory_tests.rs"]
mod tests;
