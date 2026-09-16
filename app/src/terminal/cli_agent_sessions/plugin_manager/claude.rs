use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{env, fs, io};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

use super::notification_patch::{self, PatchKind, VerifiedRuntime};
use super::{
    CliAgentPluginManager, PluginInstallError, PluginInstructionStep, PluginInstructions,
    compare_versions, run_cli_command_logged,
};
use crate::terminal::model::session::LocalCommandExecutor;
use crate::terminal::shell::ShellType;

const PLUGIN_KEY: &str = "warp@claude-code-warp";
const PLATFORM_PLUGIN_KEY: &str = "oz-harness-support@claude-code-warp";

const MARKETPLACE_REPO: &str = "warpdotdev/claude-code-warp";
const MARKETPLACE_NAME: &str = "claude-code-warp";

// Keep in sync with the plugin version in warpdotdev/claude-code-warp.
// (See the Versioning section of that repo's README.)
const MINIMUM_PLUGIN_VERSION: &str = "2.2.0";
// Keep in sync with the oz-harness-support plugin version in warpdotdev/claude-code-warp.
const MINIMUM_PLATFORM_PLUGIN_VERSION: &str = "1.1.2";

pub(super) struct ClaudeCodePluginManager {
    executor: LocalCommandExecutor,
    path_env_var: Option<String>,
}

impl ClaudeCodePluginManager {
    pub(super) fn new(
        shell_path: Option<PathBuf>,
        shell_type: Option<ShellType>,
        path_env_var: Option<String>,
    ) -> Self {
        let shell_type = shell_type.unwrap_or(ShellType::Bash);
        Self {
            executor: LocalCommandExecutor::new(shell_path, shell_type),
            path_env_var,
        }
    }

    async fn notification_operation(&self) -> Result<(), PluginInstallError> {
        if self.is_disabled() {
            return Err(PluginInstallError {
                message: crate::t!("cli-agent-plugin-disabled"),
                log: String::new(),
            });
        }
        if self.has_local_marketplace_override() {
            return Err(notification_patch::modified());
        }
        let home = claude_home_dir()?;
        fs::create_dir_all(&home)?;
        let home = home.canonicalize()?;
        let mut log = String::new();
        let _publication_lock =
            lock_claude_publication(&home).map_err(|error| publication_error(error, &log))?;
        recover_claude_publication(&home).map_err(|error| publication_error(error, &log))?;
        let runtime = VerifiedRuntime::probe(
            PatchKind::Claude,
            &home,
            self.path_env_var.as_deref(),
            &mut log,
        )
        .await?;
        let current = notification_patch::preflight(&home, PatchKind::Claude)?;
        // 已处于受测版本时只补关联脚本，避免重新安装覆盖仍在使用的缓存。
        if !current {
            install_staged_claude(&home, &runtime, &mut log).await?;
        }
        // 原生 CLI 可能拒绝操作、留下旧版本或遇到禁用配置；不能仅凭退出码报告成功。
        if self.is_disabled() || !self.is_installed() {
            return Err(PluginInstallError {
                message: crate::t!("cli-agent-plugin-update-not-effective"),
                log,
            });
        }
        if current {
            notification_patch::apply(&home, PatchKind::Claude, &log)
        } else {
            // 暂存路径已经完成修补与发布校验，不再对活跃缓存执行第二次原地替换。
            Ok(())
        }
    }

    async fn run_logged(&self, args: &[&str], log: &mut String) -> Result<(), PluginInstallError> {
        let env_vars = self
            .path_env_var
            .as_deref()
            .map(|path| HashMap::from([("PATH".to_owned(), path.to_owned())]));
        run_cli_command_logged("claude", args, &self.executor, env_vars, log).await
    }
}

#[async_trait]
impl CliAgentPluginManager for ClaudeCodePluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        MINIMUM_PLUGIN_VERSION
    }

    fn can_auto_install(&self) -> bool {
        notification_patch::auto_install_supported()
            && !self.is_disabled()
            && !self.has_local_marketplace_override()
    }

    fn is_disabled(&self) -> bool {
        claude_home_dir()
            .ok()
            .is_some_and(|dir| check_plugin_disabled(&dir, PLUGIN_KEY))
    }

    fn is_installed(&self) -> bool {
        let Ok(claude_dir) = claude_home_dir() else {
            return false;
        };
        check_installed(&claude_dir)
    }

    fn is_platform_plugin_installed(&self) -> bool {
        let Ok(claude_dir) = claude_home_dir() else {
            return false;
        };
        check_platform_plugin_installed(&claude_dir)
    }

    fn platform_plugin_needs_update(&self) -> bool {
        let Ok(claude_dir) = claude_home_dir() else {
            return false;
        };
        match installed_platform_plugin_version(&claude_dir) {
            Some(v) => compare_versions(&v, MINIMUM_PLATFORM_PLUGIN_VERSION).is_lt(),
            // No version field means very old plugin.
            None => check_platform_plugin_installed(&claude_dir),
        }
    }

    fn has_local_marketplace_override(&self) -> bool {
        let Ok(claude_dir) = claude_home_dir() else {
            return false;
        };
        claude_code_marketplace_has_local_override(&claude_dir)
    }

    /// Runs `claude plugin` CLI commands via the session shell.
    async fn install(&self) -> Result<(), PluginInstallError> {
        self.notification_operation().await
    }

    async fn update(&self) -> Result<(), PluginInstallError> {
        self.notification_operation().await
    }

    fn install_success_message(&self) -> &'static str {
        crate::t_static!("cli-agent-plugin-claude-installed")
    }

    fn update_success_message(&self) -> &'static str {
        crate::t_static!("cli-agent-plugin-claude-updated")
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        if self.is_disabled() {
            &ENABLE_INSTRUCTIONS
        } else {
            &INSTALL_INSTRUCTIONS
        }
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &UPDATE_INSTRUCTIONS
    }

    fn remote_install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn needs_update(&self) -> bool {
        let Ok(claude_dir) = claude_home_dir() else {
            return false;
        };
        check_installed(&claude_dir)
            && !notification_patch::is_applied(&claude_dir, PatchKind::Claude)
    }

    async fn install_platform_plugin(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(
            &["plugin", "marketplace", "add", MARKETPLACE_REPO],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "install", PLATFORM_PLUGIN_KEY], &mut log)
            .await?;
        Ok(())
    }

    async fn update_platform_plugin(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(
            &["plugin", "marketplace", "add", MARKETPLACE_REPO],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "install", PLATFORM_PLUGIN_KEY], &mut log)
            .await?;

        let still_outdated = claude_home_dir()
            .ok()
            .and_then(|dir| installed_platform_plugin_version(&dir))
            .map(|v| compare_versions(&v, MINIMUM_PLATFORM_PLUGIN_VERSION).is_lt())
            .unwrap_or(true);
        if still_outdated {
            log.push_str("Post-update version check: platform plugin is still outdated\n");
            return Err(PluginInstallError {
                message: crate::t!("cli-agent-platform-plugin-update-not-effective"),
                log,
            });
        }
        Ok(())
    }
}

const CLAUDE_PUBLICATION_JOURNAL: &str = "plugins/infinishell-claude-publication.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClaudePublishedKey {
    path: Vec<String>,
    before: Option<Value>,
    after: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClaudePublishedDocument {
    file: String,
    keys: Vec<ClaudePublishedKey>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClaudePublication {
    version: u8,
    documents: Vec<ClaudePublishedDocument>,
}

fn publication_conflict() -> io::Error {
    io::Error::other("Claude 插件事务格式、受控路径或配置已发生变化；保留现场，未覆盖并发编辑")
}

fn publication_error(error: impl std::fmt::Display, log: &str) -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-patch-failed"),
        log: format!("{log}\n{error}"),
    }
}

fn plain_claude_path(path: &Path) -> io::Result<()> {
    for component in path.ancestors() {
        match fs::symlink_metadata(component) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(publication_conflict());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt as _;
                    if metadata.is_file() && metadata.nlink() != 1 {
                        return Err(publication_conflict());
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn lock_claude_publication(home: &Path) -> io::Result<fs::File> {
    let path = home.join("plugins/infinishell-claude-publication.lock");
    plain_claude_path(&path)?;
    fs::create_dir_all(path.parent().ok_or_else(publication_conflict)?)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&path)?;
    plain_claude_path(&path)?;
    file.try_lock().map_err(io::Error::other)?;
    // 锁随本进程的文件句柄释放；不使用 PID 或等待时长推断旧安装进程已退出。
    Ok(file)
}

fn read_claude_document(home: &Path, file: &str) -> io::Result<(Option<Vec<u8>>, Value)> {
    let path = home.join(file);
    plain_claude_path(&path)?;
    match fs::read(&path) {
        Ok(bytes) => {
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(publication_conflict());
            }
            let value: Value = serde_json::from_slice(&bytes)?;
            if !value.is_object() {
                return Err(publication_conflict());
            }
            Ok((Some(bytes), value))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((None, json!({}))),
        Err(error) => Err(error),
    }
}

fn scoped_value<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, name| value.get(name))
}

fn set_scoped_value(value: &mut Value, path: &[String], next: &Option<Value>) -> io::Result<()> {
    let (last, parents) = path.split_last().ok_or_else(publication_conflict)?;
    let mut target = value;
    for parent in parents {
        let object = target.as_object_mut().ok_or_else(publication_conflict)?;
        target = object.entry(parent).or_insert_with(|| json!({}));
    }
    let object = target.as_object_mut().ok_or_else(publication_conflict)?;
    match next {
        Some(next) => {
            object.insert(last.clone(), next.clone());
        }
        None => {
            object.remove(last);
        }
    }
    Ok(())
}

fn write_claude_document(
    home: &Path,
    change: &ClaudePublishedDocument,
    forward: bool,
) -> io::Result<()> {
    let (bytes, mut value) = read_claude_document(home, &change.file)?;
    for key in &change.keys {
        let (expected, next) = if forward {
            (&key.before, &key.after)
        } else {
            (&key.after, &key.before)
        };
        if scoped_value(&value, &key.path) != expected.as_ref() {
            return Err(publication_conflict());
        }
        set_scoped_value(&mut value, &key.path, next)?;
    }
    let path = home.join(&change.file);
    let mut temporary = NamedTempFile::new_in(path.parent().ok_or_else(publication_conflict)?)?;
    if bytes.is_some() {
        temporary
            .as_file()
            .set_permissions(fs::metadata(&path)?.permissions())?;
    }
    temporary.write_all(&serde_json::to_vec_pretty(&value)?)?;
    temporary.as_file().sync_all()?;
    // 外部 CLI 不遵守本应用的锁；检测到任何写入即停止，不能声称这是跨进程原子 CAS。
    if read_claude_document(home, &change.file)?.0 != bytes {
        return Err(publication_conflict());
    }
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn recover_claude_publication(home: &Path) -> io::Result<()> {
    let path = home.join(CLAUDE_PUBLICATION_JOURNAL);
    plain_claude_path(&path)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) if bytes.len() <= 4 * 1024 * 1024 => bytes,
        Ok(_) => return Err(publication_conflict()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let publication: ClaudePublication = serde_json::from_slice(&bytes)?;
    let allowed = [
        (
            "plugins/known_marketplaces.json",
            vec![vec![MARKETPLACE_NAME]],
        ),
        (
            "settings.json",
            vec![
                vec!["extraKnownMarketplaces", MARKETPLACE_NAME],
                vec!["enabledPlugins", PLUGIN_KEY],
            ],
        ),
        (
            "plugins/installed_plugins.json",
            vec![vec!["version"], vec!["plugins", PLUGIN_KEY]],
        ),
    ];
    if publication.version != 1 || publication.documents.len() != allowed.len() {
        return Err(publication_conflict());
    }
    for (document, (file, paths)) in publication.documents.iter().zip(allowed) {
        if document.file != file
            || document.keys.len() != paths.len()
            || document
                .keys
                .iter()
                .zip(paths)
                .any(|(key, path)| key.path != path)
        {
            return Err(publication_conflict());
        }
    }
    let official = json!({"source":"github","repo":MARKETPLACE_REPO});
    for key in [
        &publication.documents[0].keys[0],
        &publication.documents[1].keys[0],
    ] {
        if key.after.as_ref().and_then(|value| value.get("source")) != Some(&official)
            || key
                .before
                .as_ref()
                .is_some_and(|value| value.get("source") != Some(&official))
        {
            return Err(publication_conflict());
        }
    }
    let enabled = &publication.documents[1].keys[1];
    let version = &publication.documents[2].keys[0];
    if enabled.after != Some(json!(true))
        || enabled.before.as_ref().is_some_and(|value| value != true)
        || version.after != Some(json!(2))
        || version.before.as_ref().is_some_and(|value| value != 2)
    {
        return Err(publication_conflict());
    }
    let registry = &publication.documents[2].keys[1];
    for (entry, restoring) in [(&registry.before, true), (&registry.after, false)] {
        let Some(entry) = entry else {
            if !restoring {
                return Err(publication_conflict());
            }
            continue;
        };
        let entries = entry.as_array().ok_or_else(publication_conflict)?;
        if entries.len() != 1 || entries[0]["scope"] != "user" {
            return Err(publication_conflict());
        }
        let version = entries[0]["version"]
            .as_str()
            .ok_or_else(publication_conflict)?;
        if (!restoring && version != "2.2.0")
            || !matches!(version, "2.1.0" | "2.2.0")
            || entries[0]["installPath"]
                != json!(
                    home.join("plugins/cache/claude-code-warp/warp")
                        .join(version)
                )
        {
            return Err(publication_conflict());
        }
    }
    // 先检查全部字段，再恢复已发布的字段；冲突或用户禁用不会被旧前像覆盖。
    let mut written = Vec::new();
    for document in &publication.documents {
        let value = read_claude_document(home, &document.file)?.1;
        let before = document
            .keys
            .iter()
            .all(|key| scoped_value(&value, &key.path) == key.before.as_ref());
        let after = document
            .keys
            .iter()
            .all(|key| scoped_value(&value, &key.path) == key.after.as_ref());
        if !before && !after {
            return Err(publication_conflict());
        }
        written.push(!before && after);
    }
    for (document, written) in publication.documents.iter().zip(written).rev() {
        if written {
            write_claude_document(home, document, false)?;
        }
    }
    // 缓存和原生暂存目录故意保留：仍存活的旧原生命令只能继续写自己的随机目录。
    fs::remove_file(path)
}

async fn install_staged_claude(
    home: &Path,
    runtime: &VerifiedRuntime,
    log: &mut String,
) -> Result<(), PluginInstallError> {
    let original = [
        read_claude_document(home, "plugins/known_marketplaces.json")?.1,
        read_claude_document(home, "settings.json")?.1,
        read_claude_document(home, "plugins/installed_plugins.json")?.1,
    ];
    let transactions = home.join("plugins/infinishell-claude-staging");
    plain_claude_path(&transactions)?;
    fs::create_dir_all(&transactions)?;
    // 在派生原生进程前持久化目录；取消 future 或父进程退出不触发目录析构清理。
    let transaction = tempfile::tempdir_in(&transactions)?.keep();
    let stage = transaction.join("native");
    fs::create_dir(&stage)?;
    let staged_runtime = runtime.with_home(&stage);
    staged_runtime
        .run(&["plugin", "marketplace", "add", MARKETPLACE_REPO], log)
        .await?;
    staged_runtime
        .run(&["plugin", "install", PLUGIN_KEY], log)
        .await?;
    notification_patch::apply(&stage, PatchKind::Claude, log)?;
    publish_staged_claude(home, &stage, &original, |_, _| Ok(())).map_err(|error| {
        publication_error(
            format!("{error}; 暂存资料保留于 {}", transaction.display()),
            log,
        )
    })
}

fn publish_staged_claude(
    home: &Path,
    stage: &Path,
    original: &[Value; 3],
    mut before_step: impl FnMut(usize, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let source_cache = stage.join("plugins/cache/claude-code-warp/warp/2.2.0");
    plain_claude_path(&source_cache)?;
    notification_patch::verify_staged_claude_cache(&source_cache)?;
    let mut staged = [
        read_claude_document(stage, "plugins/known_marketplaces.json")?.1,
        read_claude_document(stage, "settings.json")?.1,
        read_claude_document(stage, "plugins/installed_plugins.json")?.1,
    ];
    let official = json!({"source":"github","repo":MARKETPLACE_REPO});
    if staged[0][MARKETPLACE_NAME]["source"] != official
        || staged[1]["extraKnownMarketplaces"][MARKETPLACE_NAME]["source"] != official
        || staged[1]["enabledPlugins"][PLUGIN_KEY] != true
        || staged[2]["version"] != 2
        || original[2]
            .get("version")
            .is_some_and(|version| version != 2)
        || staged[2]["plugins"][PLUGIN_KEY]
            .as_array()
            .is_none_or(|entries| {
                entries.len() != 1
                    || entries[0]["scope"] != "user"
                    || entries[0]["version"] != MINIMUM_PLUGIN_VERSION
            })
    {
        return Err(publication_conflict());
    }
    let target = home.join("plugins/cache/claude-code-warp/warp/2.2.0");
    plain_claude_path(&target)?;
    staged[2]["plugins"][PLUGIN_KEY][0]["installPath"] = json!(target);
    if original[0].get(MARKETPLACE_NAME).is_some() {
        if original[0][MARKETPLACE_NAME]["source"] != official {
            return Err(publication_conflict());
        }
        staged[0][MARKETPLACE_NAME] = original[0][MARKETPLACE_NAME].clone();
    } else {
        // 原生 clone 留在本次持久暂存目录；发布失败不留下占住固定路径的半安装来源。
        let marketplace = stage.join("plugins/marketplaces/claude-code-warp");
        if staged[0][MARKETPLACE_NAME]["installLocation"] != json!(marketplace) {
            return Err(publication_conflict());
        }
        plain_claude_path(&marketplace)?;
        if !fs::symlink_metadata(&marketplace)?.file_type().is_dir() {
            return Err(publication_conflict());
        }
    }
    if let Some(source) = original[1]
        .get("extraKnownMarketplaces")
        .and_then(|sources| sources.get(MARKETPLACE_NAME))
    {
        if source.get("source") != Some(&official) {
            return Err(publication_conflict());
        }
        staged[1]["extraKnownMarketplaces"][MARKETPLACE_NAME] = source.clone();
    }
    if original[1]
        .get("enabledPlugins")
        .and_then(|plugins| plugins.get(PLUGIN_KEY))
        .is_some_and(|enabled| enabled != true)
    {
        return Err(publication_conflict());
    }
    let specifications = [
        (
            "plugins/known_marketplaces.json",
            vec![vec![MARKETPLACE_NAME]],
        ),
        (
            "settings.json",
            vec![
                vec!["extraKnownMarketplaces", MARKETPLACE_NAME],
                vec!["enabledPlugins", PLUGIN_KEY],
            ],
        ),
        (
            "plugins/installed_plugins.json",
            vec![vec!["version"], vec!["plugins", PLUGIN_KEY]],
        ),
    ];
    let publication = ClaudePublication {
        version: 1,
        documents: specifications
            .into_iter()
            .zip(original.iter().zip(&staged))
            .map(|((file, paths), (before, after))| ClaudePublishedDocument {
                file: file.to_owned(),
                keys: paths
                    .into_iter()
                    .map(|path| {
                        let path = path.into_iter().map(str::to_owned).collect::<Vec<_>>();
                        ClaudePublishedKey {
                            before: scoped_value(before, &path).cloned(),
                            after: scoped_value(after, &path).cloned(),
                            path,
                        }
                    })
                    .collect(),
            })
            .collect(),
    };
    let journal = home.join(CLAUDE_PUBLICATION_JOURNAL);
    plain_claude_path(&journal)?;
    let mut pending = NamedTempFile::new_in(journal.parent().ok_or_else(publication_conflict)?)?;
    pending.write_all(&serde_json::to_vec_pretty(&publication)?)?;
    pending.as_file().sync_all()?;
    pending
        .persist_noclobber(&journal)
        .map_err(|error| error.error)?;
    let outcome = (|| {
        before_step(0, home)?;
        if target.exists() {
            notification_patch::verify_staged_claude_cache(&target)?;
        } else {
            fs::create_dir_all(target.parent().ok_or_else(publication_conflict)?)?;
            fs::rename(source_cache, &target)?;
        }
        notification_patch::verify_staged_claude_cache(&target)?;
        for (index, document) in publication.documents.iter().enumerate() {
            before_step(index + 1, home)?;
            write_claude_document(home, document, true)?;
        }
        before_step(4, home)?;
        if !notification_patch::preflight(home, PatchKind::Claude).map_err(io::Error::other)? {
            return Err(publication_conflict());
        }
        for document in &publication.documents {
            let value = read_claude_document(home, &document.file)?.1;
            if document
                .keys
                .iter()
                .any(|key| scoped_value(&value, &key.path) != key.after.as_ref())
            {
                return Err(publication_conflict());
            }
        }
        notification_patch::verify_staged_claude_cache(&target)?;
        if check_plugin_disabled(home, PLUGIN_KEY) {
            return Err(publication_conflict());
        }
        fs::remove_file(&journal)
    })();
    if let Err(error) = outcome {
        recover_claude_publication(home)?;
        return Err(error);
    }
    Ok(())
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: crate::t_static!("cli-agent-plugin-claude-install-title"),
    subtitle: crate::t_static!("cli-agent-plugin-claude-install-subtitle"),
    steps: vec![
        PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-claude-add-marketplace-step"),
            command: "claude plugin marketplace add warpdotdev/claude-code-warp",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-install-warp-plugin-step"),
            command: "claude plugin install warp@claude-code-warp",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: vec![
        crate::t_static!("cli-agent-plugin-claude-restart-note"),
        crate::t_static!("cli-agent-plugin-claude-known-issues-note"),
        crate::t_static!("cli-agent-plugin-patch-manual-note"),
    ],
});

static UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: crate::t_static!("cli-agent-plugin-claude-update-title"),
    subtitle: crate::t_static!("cli-agent-plugin-run-following-commands"),
    steps: vec![
        PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-refresh-marketplace-step"),
            command: "claude plugin marketplace update claude-code-warp",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-install-latest-version-step"),
            command: "claude plugin update warp@claude-code-warp",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: vec![
        crate::t_static!("cli-agent-plugin-claude-restart-update-note"),
        crate::t_static!("cli-agent-plugin-patch-manual-note"),
    ],
});

fn check_installed(claude_dir: &Path) -> bool {
    check_plugin_installed(claude_dir, PLUGIN_KEY)
}

fn check_platform_plugin_installed(claude_dir: &Path) -> bool {
    check_plugin_installed(claude_dir, PLATFORM_PLUGIN_KEY)
}

fn check_plugin_installed(claude_dir: &Path, plugin_key: &str) -> bool {
    if check_plugin_disabled(claude_dir, plugin_key) {
        return false;
    }
    let plugins_path = claude_dir.join("plugins").join("installed_plugins.json");
    let Ok(contents) = fs::read_to_string(plugins_path) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&contents) else {
        return false;
    };
    parsed
        .get("plugins")
        .and_then(|p| p.get(plugin_key))
        .and_then(|v| v.as_array())
        .map(|arr| !arr.is_empty())
        .unwrap_or(false)
}

fn check_plugin_disabled(claude_dir: &Path, plugin_key: &str) -> bool {
    fs::read_to_string(claude_dir.join("settings.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .and_then(|settings| settings.get("enabledPlugins")?.get(plugin_key)?.as_bool())
        == Some(false)
}

static ENABLE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: crate::t_static!("cli-agent-plugin-claude-install-title"),
    subtitle: crate::t_static!("cli-agent-plugin-disabled"),
    steps: vec![PluginInstructionStep {
        description: crate::t_static!("cli-agent-plugin-enable-step"),
        command: "claude plugin enable warp@claude-code-warp",
        executable: true,
        link: None,
    }],
    post_install_notes: vec![crate::t_static!(
        "cli-agent-plugin-installed-restart-session"
    )],
});

/// Reads the installed version string for the Zap plugin, if present.
#[cfg(test)]
fn installed_version(claude_dir: &Path) -> Option<String> {
    installed_plugin_version(claude_dir, PLUGIN_KEY)
}

/// Reads the installed version string for the Oz platform plugin, if present.
fn installed_platform_plugin_version(claude_dir: &Path) -> Option<String> {
    installed_plugin_version(claude_dir, PLATFORM_PLUGIN_KEY)
}

fn installed_plugin_version(claude_dir: &Path, plugin_key: &str) -> Option<String> {
    let plugins_path = claude_dir.join("plugins").join("installed_plugins.json");
    let contents = fs::read_to_string(plugins_path).ok()?;
    let parsed: Value = serde_json::from_str(&contents).ok()?;
    parsed
        .get("plugins")?
        .get(plugin_key)?
        .as_array()?
        .first()?
        .get("version")?
        .as_str()
        .map(|s| s.to_owned())
}

fn claude_code_marketplace_has_local_override(claude_dir: &Path) -> bool {
    let settings_path = claude_dir.join("settings.json");
    let Ok(contents) = fs::read_to_string(settings_path) else {
        return false;
    };
    let Ok(settings) = serde_json::from_str::<Value>(&contents) else {
        return false;
    };

    settings
        .get("extraKnownMarketplaces")
        .and_then(|marketplaces| marketplaces.get(MARKETPLACE_NAME))
        .map(marketplace_entry_has_local_path)
        .unwrap_or(false)
}

fn marketplace_entry_has_local_path(entry: &Value) -> bool {
    let Some(source) = entry.get("source") else {
        return false;
    };
    match source {
        Value::Object(source) => {
            let source_kind = source.get("source").and_then(Value::as_str);
            let path = source.get("path").and_then(Value::as_str);
            source_kind == Some("directory") && path.map(is_local_marketplace_path).unwrap_or(false)
        }
        Value::String(source) => is_local_marketplace_path(source),
        _ => false,
    }
}

fn is_local_marketplace_path(source: &str) -> bool {
    source.starts_with('/')
        || source.starts_with("~/")
        || source.starts_with("./")
        || source.starts_with("../")
        || source.starts_with("file://")
}

/// Resolves the dir the Claude CLI reads/writes its state from.
///
/// Honors `CLAUDE_CONFIG_DIR` (respected by the Claude CLI, and set by the Oz
/// worker to a per-task dir), falling back to `~/.claude`. Must match where
/// `claude plugin install` writes, else install/verify checks read the wrong dir.
fn claude_home_dir() -> io::Result<PathBuf> {
    if let Ok(dir) = env::var("CLAUDE_CONFIG_DIR")
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    dirs::home_dir()
        .map(|home| home.join(".claude"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not determine home directory",
            )
        })
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
