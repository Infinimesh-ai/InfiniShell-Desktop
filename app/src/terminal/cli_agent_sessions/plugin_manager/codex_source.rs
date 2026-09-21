//! Codex 固定完整来源与限定配置事务；原生安装在暂存 HOME 完成，不改用户信任设置。

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use std::{fs, io};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use tempfile::{NamedTempFile, TempDir};
use toml_edit::{DocumentMut, Item};

use super::PluginInstallError;
use super::notification_patch::{self, PatchKind, VerifiedRuntime};

const MARKETPLACE: &str = "codex-warp";
const COMMIT: &str = "31ce59d9011cfb1d78f265649a228dac5de58d76";
const METADATA: &str =
    include_str!("../../../../assets/bundled/cli-agent-plugins/codex/SOURCE_METADATA.json");
const PREVIOUS_METADATA: &str = include_str!(
    "../../../../assets/bundled/cli-agent-plugins/codex/revisions/rev3/SOURCE_METADATA.json"
);
const REV4_METADATA: &str = include_str!(
    "../../../../assets/bundled/cli-agent-plugins/codex/revisions/rev4/SOURCE_METADATA.json"
);

#[derive(Deserialize)]
struct SourceMetadata {
    upstream_commit: String,
    directory: String,
    files: BTreeMap<String, SourceFile>,
}

#[derive(Deserialize)]
struct SourceFile {
    upstream_sha256: String,
    sha256: String,
    mode: u32,
}

static BUNDLE: LazyLock<SourceMetadata> =
    LazyLock::new(|| serde_json::from_str(METADATA).expect("随附 Codex 完整来源元数据必须有效"));
static PREVIOUS_BUNDLE: LazyLock<SourceMetadata> = LazyLock::new(|| {
    serde_json::from_str(PREVIOUS_METADATA).expect("随附 Codex rev3 完整来源元数据必须有效")
});
static REV4_BUNDLE: LazyLock<SourceMetadata> = LazyLock::new(|| {
    serde_json::from_str(REV4_METADATA).expect("随附 Codex rev4 完整来源元数据必须有效")
});
static CURRENT: LazyLock<Mutex<BTreeMap<PathBuf, (Instant, bool)>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn invalid() -> io::Error {
    io::Error::other("Codex 来源、配置或缓存变化；未覆盖未知状态，请重新核对并重试")
}

fn failure(error: impl std::fmt::Display, log: &str) -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-patch-failed"),
        log: format!("{log}\n{error}"),
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn source_parent(home: &Path) -> PathBuf {
    home.join("plugins/infinishell-sources")
        .join(&BUNDLE.directory)
}

fn source_path(home: &Path) -> PathBuf {
    source_parent(home).join("source")
}

fn plain_path(path: &Path) -> io::Result<()> {
    // 检查全部已存在的路径分量，不能只检查最终文件而漏过父目录链接。
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(invalid()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn relative_file(root: &Path, name: &str) -> io::Result<PathBuf> {
    if Path::new(name)
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid());
    }
    let path = root.join(name);
    plain_path(&path)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() > 1_048_576 {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(invalid());
        }
    }
    Ok(path)
}

fn tree(root: &Path, git_snapshot: bool) -> io::Result<BTreeMap<String, String>> {
    plain_path(root)?;
    let mut pending = vec![root.to_owned()];
    let mut result = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid())?
                .to_str()
                .ok_or_else(invalid)?
                .replace('\\', "/");
            if entry.file_type()?.is_symlink() {
                return Err(invalid());
            }
            if git_snapshot
                && matches!(
                    relative.as_str(),
                    ".git" | ".codex-marketplace-install.json"
                )
            {
                continue;
            }
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                if result.len() >= 128 {
                    return Err(invalid());
                }
                result.insert(
                    relative.clone(),
                    digest(&fs::read(relative_file(root, &relative)?)?),
                );
            }
        }
    }
    Ok(result)
}

fn cache_snapshot(root: &Path) -> io::Result<BTreeMap<String, String>> {
    let mut files = tree(root, false)?;
    for (relative, fingerprint) in &mut files {
        let permissions = fs::metadata(relative_file(root, relative)?)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fingerprint.push_str(&format!(":{:o}", permissions.mode() & 0o777));
        }
        #[cfg(not(unix))]
        fingerprint.push_str(&format!(":readonly={}", permissions.readonly()));
    }
    Ok(files)
}

fn expected_tree(prefix: &str, upstream: bool) -> BTreeMap<String, String> {
    revision_tree(&BUNDLE, prefix, upstream)
}

fn revision_tree(
    revision: &SourceMetadata,
    prefix: &str,
    upstream: bool,
) -> BTreeMap<String, String> {
    revision
        .files
        .iter()
        .filter_map(|(name, file)| {
            name.strip_prefix(prefix).map(|name| {
                (
                    name.to_owned(),
                    if upstream {
                        file.upstream_sha256.clone()
                    } else {
                        file.sha256.clone()
                    },
                )
            })
        })
        .collect()
}

fn verify_owned(home: &Path) -> io::Result<()> {
    verify_revision(home, &BUNDLE, METADATA)
}

fn verify_revision(home: &Path, revision: &SourceMetadata, metadata: &str) -> io::Result<()> {
    let parent = home
        .join("plugins/infinishell-sources")
        .join(&revision.directory);
    if fs::read(relative_file(&parent, "SOURCE_METADATA.json")?)? != metadata.as_bytes()
        || tree(&parent.join("source"), false)? != revision_tree(revision, "", false)
    {
        return Err(invalid());
    }
    verify_revision_modes(&parent.join("source"), "", revision)?;
    Ok(())
}

fn verify_modes(root: &Path, prefix: &str) -> io::Result<()> {
    verify_revision_modes(root, prefix, &BUNDLE)
}

fn verify_revision_modes(root: &Path, prefix: &str, revision: &SourceMetadata) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for (name, expected) in &revision.files {
            if let Some(relative) = name.strip_prefix(prefix)
                && fs::metadata(relative_file(root, relative)?)?
                    .permissions()
                    .mode()
                    & 0o777
                    != expected.mode
            {
                return Err(invalid());
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (root, prefix, revision);
    Ok(())
}

/// 只接收 rev3/rev4 的完整已部署树；不能将任意新旧脚本组合当作可迁移版本。
pub(super) fn is_previous_notification_cache(root: &Path) -> bool {
    let Ok(actual) = tree(root, false) else {
        return false;
    };
    [&*REV4_BUNDLE, &*PREVIOUS_BUNDLE]
        .into_iter()
        .any(|revision| {
            actual == revision_tree(revision, "plugins/warp/", false)
                && verify_revision_modes(root, "plugins/warp/", revision).is_ok()
        })
}

fn validate_config_shape(document: &DocumentMut) -> io::Result<()> {
    for parent in ["marketplaces", "plugins"] {
        if document
            .get(parent)
            .is_some_and(|value| value.as_table_like().is_none())
        {
            return Err(invalid());
        }
    }
    if marketplace(document).is_some_and(|value| value.as_table_like().is_none()) {
        return Err(invalid());
    }
    for key in ["warp@codex-warp", "orchestration@codex-warp"] {
        if let Some(plugin) = document.get("plugins").and_then(|plugins| plugins.get(key)) {
            if plugin.as_table_like().is_none()
                || plugin
                    .get("enabled")
                    .is_some_and(|enabled| enabled.as_bool().is_none())
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn read_config(home: &Path) -> io::Result<(Option<Vec<u8>>, DocumentMut)> {
    let path = home.join("config.toml");
    plain_path(&path)?;
    match fs::read(&path) {
        Ok(bytes) => {
            relative_file(home, "config.toml")?;
            let document = std::str::from_utf8(&bytes)
                .map_err(|_| invalid())?
                .parse::<DocumentMut>()
                .map_err(|_| invalid())?;
            validate_config_shape(&document)?;
            Ok((Some(bytes), document))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((None, DocumentMut::new())),
        Err(error) => Err(error),
    }
}

fn marketplace(document: &DocumentMut) -> Option<&Item> {
    document
        .get("marketplaces")
        .and_then(|table| table.get(MARKETPLACE))
}

fn owned_entry(document: &DocumentMut, home: &Path) -> bool {
    owned_revision_entry(document, home, &BUNDLE)
}

fn owned_revision_entry(document: &DocumentMut, home: &Path, revision: &SourceMetadata) -> bool {
    let source = home
        .join("plugins/infinishell-sources")
        .join(&revision.directory)
        .join("source");
    marketplace(document).is_some_and(|entry| {
        entry.get("source_type").and_then(Item::as_str) == Some("local")
            && entry.get("source").and_then(Item::as_str).map(Path::new) == Some(source.as_path())
    })
}

fn canonical_entry(entry: &Item) -> bool {
    entry.get("source_type").and_then(Item::as_str) == Some("git")
        && matches!(
            entry.get("source").and_then(Item::as_str),
            Some(
                "https://github.com/warpdotdev/codex-warp.git"
                    | "https://github.com/warpdotdev/codex-warp"
                    | "warpdotdev/codex-warp"
            )
        )
        && entry
            .get("ref")
            .is_none_or(|value| value.as_str() == Some(COMMIT))
        && entry
            .get("last_revision")
            .is_none_or(|value| value.as_str() == Some(COMMIT))
        && entry.get("sparse_paths").is_none()
}

/// 只豁免本应用固定目录与完整清单一致的来源，不能放行任意本地 override。
pub(super) fn has_custom_source(home: &Path) -> bool {
    let canonical = home.canonicalize().unwrap_or_else(|_| home.to_owned());
    let home = canonical.as_path();
    match read_config(home) {
        Ok((_, document)) => match marketplace(&document) {
            None => false,
            Some(entry) if canonical_entry(entry) => false,
            Some(_) if owned_revision_entry(&document, home, &PREVIOUS_BUNDLE) => {
                verify_revision(home, &PREVIOUS_BUNDLE, PREVIOUS_METADATA).is_err()
            }
            Some(_) if owned_revision_entry(&document, home, &REV4_BUNDLE) => {
                verify_revision(home, &REV4_BUNDLE, REV4_METADATA).is_err()
            }
            Some(_) => !owned_entry(&document, home) || !is_current(home),
        },
        Err(_) => true,
    }
}

pub(super) fn invalidate(home: &Path) {
    CURRENT
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(home);
    notification_patch::invalidate(home, PatchKind::Codex);
}

/// 渲染端短期复用完整验证结果；安装预检始终直接重读，不依赖此缓存。
pub(super) fn is_current(home: &Path) -> bool {
    let canonical = home.canonicalize().unwrap_or_else(|_| home.to_owned());
    let home = canonical.as_path();
    let mut cache = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((when, value)) = cache.get(home)
        && when.elapsed() < Duration::from_secs(1)
    {
        return *value;
    }
    let value = read_config(home).is_ok_and(|(_, document)| owned_entry(&document, home))
        && verify_owned(home).is_ok();
    if cache.len() > 32 {
        cache.clear();
    }
    cache.insert(home.to_owned(), (Instant::now(), value));
    value
}

fn validate_existing(home: &Path, document: &DocumentMut) -> io::Result<()> {
    match marketplace(document) {
        None => {}
        Some(entry) if canonical_entry(entry) => {
            let original = home.join(".tmp/marketplaces/codex-warp");
            if original.exists() {
                if tree(&original, true)? != expected_tree("", true) {
                    return Err(invalid());
                }
                verify_modes(&original, "")?;
            }
        }
        Some(_) if owned_entry(document, home) => verify_owned(home)?,
        Some(_) if owned_revision_entry(document, home, &PREVIOUS_BUNDLE) => {
            verify_revision(home, &PREVIOUS_BUNDLE, PREVIOUS_METADATA)?;
        }
        Some(_) if owned_revision_entry(document, home, &REV4_BUNDLE) => {
            verify_revision(home, &REV4_BUNDLE, REV4_METADATA)?;
        }
        Some(_) => return Err(invalid()),
    }
    notification_patch::preflight(home, PatchKind::Codex).map_err(|_| invalid())?;
    let orchestration = home.join("plugins/cache/codex-warp/orchestration");
    if orchestration.exists() {
        let children = fs::read_dir(&orchestration)?.collect::<io::Result<Vec<_>>>()?;
        if children.len() != 1
            || children[0].file_name() != "0.4.0"
            || tree(&children[0].path(), false)? != expected_tree("plugins/orchestration/", false)
        {
            return Err(invalid());
        }
        verify_modes(&children[0].path(), "plugins/orchestration/")?;
    }
    Ok(())
}

fn materialize(home: &Path) -> io::Result<PathBuf> {
    let parent = source_parent(home);
    plain_path(&parent)?;
    if parent.exists() {
        verify_owned(home)?;
        return Ok(parent.join("source"));
    }
    let container = parent.parent().ok_or_else(invalid)?;
    fs::create_dir_all(container)?;
    let temporary = TempDir::new_in(container)?;
    let source = temporary.path().join("source");
    for (name, contents) in FILES {
        let expected = BUNDLE.files.get(*name).ok_or_else(invalid)?;
        if digest(contents) != expected.sha256 || BUNDLE.upstream_commit != COMMIT {
            return Err(invalid());
        }
        let path = source.join(name);
        fs::create_dir_all(path.parent().ok_or_else(invalid)?)?;
        fs::write(&path, contents)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(expected.mode))?;
        }
        #[cfg(not(unix))]
        let _ = expected.mode;
    }
    fs::write(temporary.path().join("SOURCE_METADATA.json"), METADATA)?;
    if tree(&source, false)? != expected_tree("", false) {
        return Err(invalid());
    }
    // 只发布新的固定版本目录；已存在的未知内容永不覆盖。
    fs::rename(temporary.path(), &parent)?;
    verify_owned(home)?;
    Ok(parent.join("source"))
}

#[derive(Clone)]
struct Scope {
    marketplace: Item,
    enabled: Item,
}

impl Scope {
    fn read(document: &DocumentMut, key: &str) -> Self {
        Self {
            marketplace: marketplace(document).cloned().unwrap_or(Item::None),
            enabled: document
                .get("plugins")
                .and_then(|plugins| plugins.get(key))
                .and_then(|plugin| plugin.get("enabled"))
                .cloned()
                .unwrap_or(Item::None),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        let signatures = (
            item_signature(&self.marketplace),
            item_signature(&other.marketplace),
            item_signature(&self.enabled),
            item_signature(&other.enabled),
        );
        if let (Ok(marketplace), Ok(other_marketplace), Ok(enabled), Ok(other_enabled)) = signatures
        {
            marketplace == other_marketplace && enabled == other_enabled
        } else {
            false
        }
    }

    fn apply(&self, document: &mut DocumentMut, key: &str) {
        if self.marketplace.is_none() {
            if let Some(table) = document
                .get_mut("marketplaces")
                .and_then(Item::as_table_like_mut)
            {
                table.remove(MARKETPLACE);
            }
        } else {
            if document.get("marketplaces").is_none() {
                document["marketplaces"] = Item::Table(toml_edit::Table::new());
            }
            let mut replacement = self.marketplace.clone();
            // 合法用户配置可用内联父表；其中不能插入普通 Table 项目。
            if document
                .get("marketplaces")
                .is_some_and(|parent| parent.is_inline_table())
            {
                replacement.make_value();
            }
            document["marketplaces"][MARKETPLACE] = replacement;
        }
        if self.enabled.is_none() {
            if let Some(plugin) = document
                .get_mut("plugins")
                .and_then(|plugins| plugins.get_mut(key))
                .and_then(Item::as_table_like_mut)
            {
                plugin.remove("enabled");
            }
        } else {
            document["plugins"][key]["enabled"] = self.enabled.clone();
        }
    }
}

fn item_signature(item: &Item) -> io::Result<Option<toml::Value>> {
    if item.is_none() {
        return Ok(None);
    }
    // 原生重读会添加空格或将表改为内联表示；比较带类型的值，不比较格式化字节。
    let mut wrapper = DocumentMut::new();
    wrapper["value"] = item.clone();
    let parsed = toml::from_str::<toml::Value>(&wrapper.to_string()).map_err(|_| invalid())?;
    parsed.get("value").cloned().map(Some).ok_or_else(invalid)
}

fn write_scoped(home: &Path, key: &str, expected: &Scope, next: &Scope) -> io::Result<()> {
    let (bytes, mut document) = read_config(home)?;
    if !expected.matches(&Scope::read(&document, key)) {
        return Err(invalid());
    }
    next.apply(&mut document, key);
    let path = home.join("config.toml");
    let mut temporary = NamedTempFile::new_in(home)?;
    if path.exists() {
        temporary
            .as_file()
            .set_permissions(fs::metadata(&path)?.permissions())?;
    }
    temporary.write_all(document.to_string().as_bytes())?;
    temporary.as_file().sync_all()?;
    // 这是读后比较加原子替换，不是跨进程原子 CAS；未知变化一律保留并返回失败。
    if read_config(home)?.0 != bytes {
        return Err(invalid());
    }
    temporary.persist(path).map_err(|error| error.error)?;
    // 返回后由事务标记已写入再校验，避免写后读取失败被误当作从未写入而跳过回退。
    Ok(())
}

fn cache_root(home: &Path, name: &str) -> PathBuf {
    home.join("plugins/cache/codex-warp").join(name)
}

/// 暂存调用原生安装；迁入时只替换已确认缓存和受影响配置键，保留原始 Git 快照。
pub(super) async fn install(
    home: &Path,
    name: &str,
    runtime: &VerifiedRuntime,
    log: &mut String,
) -> Result<(), PluginInstallError> {
    invalidate(home);
    if !matches!(name, "warp" | "orchestration") {
        return Err(notification_patch::unsupported());
    }
    fs::create_dir_all(home)?;
    let home = home.canonicalize()?;
    // 普通拒绝路径不能为了取锁而新增文件；中断事务则必须先恢复再检查半成品缓存。
    if !home.join("plugins/infinishell-transactions").try_exists()? {
        let (_, document) = read_config(&home).map_err(|error| failure(error, log))?;
        if Scope::read(&document, &format!("{name}@codex-warp"))
            .enabled
            .as_bool()
            == Some(false)
        {
            return Err(PluginInstallError {
                message: crate::t!("cli-agent-plugin-disabled"),
                log: log.clone(),
            });
        }
        validate_existing(&home, &document).map_err(|_| notification_patch::modified())?;
    }
    let _publication_lock = publication_lock(&home).map_err(|error| failure(error, log))?;
    recover_transactions(&home).map_err(|error| failure(error, log))?;
    let key = format!("{name}@codex-warp");
    let (_, document) = read_config(&home).map_err(|error| failure(error, log))?;
    let original = Scope::read(&document, &key);
    if original.enabled.as_bool() == Some(false) {
        return Err(PluginInstallError {
            message: crate::t!("cli-agent-plugin-disabled"),
            log: log.clone(),
        });
    }
    if !original.enabled.is_none() && original.enabled.as_bool().is_none() {
        return Err(notification_patch::modified());
    }
    validate_existing(&home, &document).map_err(|_| notification_patch::modified())?;
    if owned_entry(&document, &home)
        && original.enabled.as_bool() == Some(true)
        && verify_installed_cache(&cache_root(&home, name), name).is_ok()
    {
        // 恢复完成或已经精确安装时不再重复调用原生 marketplace/add。
        return Ok(());
    }
    let preserve_previous_cache =
        name == "warp" && is_previous_notification_cache(&cache_root(&home, name).join("0.4.0"));
    let source = materialize(&home).map_err(|error| failure(error, log))?;
    let transactions = home.join("plugins/infinishell-transactions");
    plain_path(&transactions)?;
    fs::create_dir_all(&transactions)?;
    let transaction = TempDir::new_in(&transactions)?;
    let stage = transaction.path().join("codex");
    fs::create_dir(&stage)?;
    let staged_runtime = runtime.with_home(&stage);
    let source_arg = source.to_str().ok_or_else(notification_patch::modified)?;
    staged_runtime
        .run(&["plugin", "marketplace", "add", source_arg], log)
        .await?;
    staged_runtime.run(&["plugin", "add", &key], log).await?;
    let (_, staged_config) = read_config(&stage).map_err(|error| failure(error, log))?;
    let installed = Scope::read(&staged_config, &key);
    let expected_entry = installed
        .marketplace
        .as_table_like()
        .ok_or_else(notification_patch::modified)?;
    if !owned_entry(&staged_config, &home)
        || installed.enabled.as_bool() != Some(true)
        || expected_entry
            .iter()
            .any(|(key, _)| !matches!(key, "last_updated" | "source_type" | "source"))
    {
        return Err(notification_patch::modified());
    }
    let staged_cache = cache_root(&stage, name);
    let expected = expected_tree(&format!("plugins/{name}/"), false);
    if tree(&staged_cache.join("0.4.0"), false).map_err(|error| failure(error, log))? != expected {
        return Err(notification_patch::modified());
    }
    verify_modes(&staged_cache.join("0.4.0"), &format!("plugins/{name}/"))
        .map_err(|error| failure(error, log))?;
    // 真正修改前将事务目录持久化；任何恢复失败都不能由析构删除唯一旧缓存。
    let transaction = transaction.keep();
    commit_install(
        &home,
        name,
        &original,
        &installed,
        &staged_cache,
        &transaction,
        |_, _| Ok(()),
    )
    .map_err(|error| {
        failure(
            format!("{error}; 事务资料保留于 {}", transaction.display()),
            log,
        )
    })?;
    if preserve_previous_cache {
        // 受控旧版升级成功仍保留旧缓存和事务记录，旧不可变来源也不会被删除。
        log.push_str(&format!("旧版恢复资料保留于 {}\n", transaction.display()));
    } else {
        fs::remove_dir_all(transaction)?;
    }
    invalidate(&home);
    Ok(())
}

fn commit_install(
    home: &Path,
    name: &str,
    original: &Scope,
    installed: &Scope,
    staged_cache: &Path,
    transaction: &Path,
    mut before_step: impl FnMut(usize, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let key = format!("{name}@codex-warp");
    let target = cache_root(home, name);
    plain_path(&target)?;
    let parent = target.parent().ok_or_else(invalid)?;
    fs::create_dir_all(parent)?;
    let old_tree = if target.exists() {
        Some(cache_snapshot(&target)?)
    } else {
        None
    };
    let new_tree = cache_snapshot(staged_cache)?;
    let backup = transaction.join("previous-cache");
    let journal = InstallJournal {
        version: 1,
        phase: "prepared".to_owned(),
        plugin: key.clone(),
        source: source_path(home),
        source_metadata_sha256: digest(METADATA.as_bytes()),
        original_cache: old_tree.clone(),
        installed_cache: new_tree.clone(),
        original_scope: scope_document(original, &key),
        installed_scope: scope_document(installed, &key),
        staged_cache: staged_cache
            .strip_prefix(transaction)
            .map_err(|_| invalid())?
            .to_owned(),
    };
    write_journal(transaction, &journal)?;
    before_step(0, home)?;
    validate_existing(home, &read_config(home)?.1)?;
    let current = Scope::read(&read_config(home)?.1, &key);
    if current.enabled.as_bool() == Some(false)
        || !original.matches(&current)
        || target
            .exists()
            .then(|| cache_snapshot(&target))
            .transpose()?
            != old_tree
    {
        return Err(invalid());
    }
    if old_tree.is_some() {
        fs::rename(&target, &backup)?;
        sync_directory(parent)?;
        sync_directory(transaction)?;
    }
    before_step(3, home)?;
    let mut cache_replaced = false;
    let mut config_replaced = false;
    let outcome = (|| {
        fs::rename(staged_cache, &target)?;
        cache_replaced = true;
        sync_directory(parent)?;
        sync_directory(staged_cache.parent().ok_or_else(invalid)?)?;
        before_step(4, home)?;
        record_phase(transaction, "cache_written")?;
        before_step(1, home)?;
        // 原生暂存结束后再次检查禁用及来源；绝不沿用开始安装时的启用判断。
        write_scoped(home, &key, original, installed)?;
        config_replaced = true;
        sync_directory(home)?;
        before_step(5, home)?;
        record_phase(transaction, "configuration_written")?;
        before_step(2, home)?;
        if cache_snapshot(&target)? != new_tree
            || verify_owned(home).is_err()
            || !installed.matches(&Scope::read(&read_config(home)?.1, &key))
        {
            return Err(invalid());
        }
        verify_modes(&target.join("0.4.0"), &format!("plugins/{name}/"))?;
        record_phase(transaction, "verified")?;
        before_step(6, home)?;
        Ok(())
    })();
    if let Err(error) = outcome {
        let mut recovery_failed = false;
        if config_replaced
            && write_scoped(home, &key, installed, original)
                .and_then(|()| sync_directory(home))
                .is_err()
        {
            recovery_failed = true;
        }
        if cache_replaced {
            if cache_snapshot(&target).ok().as_ref() == Some(&new_tree) {
                if fs::remove_dir_all(&target)
                    .and_then(|()| sync_directory(parent))
                    .is_err()
                {
                    recovery_failed = true;
                }
            } else {
                recovery_failed = true;
            }
        }
        if backup.exists() && (!target.exists()) {
            if fs::rename(&backup, &target)
                .and_then(|()| sync_directory(parent))
                .and_then(|()| sync_directory(transaction))
                .is_err()
            {
                recovery_failed = true;
            }
        } else if backup.exists() {
            recovery_failed = true;
        }
        if recovery_failed {
            let _ = record_phase(transaction, "needs_review");
            return Err(io::Error::other(format!(
                "{error}; 未覆盖并发变化，旧缓存及阶段资料保留于 {}",
                transaction.display()
            )));
        }
        let _ = record_phase(transaction, "rolled_back");
        return Err(error);
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InstallJournal {
    version: u8,
    phase: String,
    plugin: String,
    source: PathBuf,
    source_metadata_sha256: String,
    original_cache: Option<BTreeMap<String, String>>,
    installed_cache: BTreeMap<String, String>,
    original_scope: String,
    installed_scope: String,
    staged_cache: PathBuf,
}

fn publication_lock(home: &Path) -> io::Result<fs::File> {
    let path = home.join("plugins/infinishell-codex-publication.lock");
    plain_path(&path)?;
    fs::create_dir_all(path.parent().ok_or_else(invalid)?)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&path)?;
    relative_file(
        path.parent().ok_or_else(invalid)?,
        "infinishell-codex-publication.lock",
    )?;
    file.try_lock().map_err(io::Error::other)?;
    // 句柄随进程退出释放，不用 PID 或文件年龄猜测另一个安装是否仍活跃。
    Ok(file)
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    // Windows 的 std 文件接口没有可移植目录 fsync；不声称断电后的目录持久化保证。
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn scope_document(scope: &Scope, key: &str) -> String {
    let mut document = DocumentMut::new();
    scope.apply(&mut document, key);
    document.to_string()
}

fn journal_scope(document: &str, key: &str) -> io::Result<Scope> {
    let document = document.parse::<DocumentMut>().map_err(|_| invalid())?;
    validate_config_shape(&document)?;
    Ok(Scope::read(&document, key))
}

fn write_journal(transaction: &Path, journal: &InstallJournal) -> io::Result<()> {
    plain_path(transaction)?;
    let mut file = NamedTempFile::new_in(transaction)?;
    file.write_all(&serde_json::to_vec_pretty(journal)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(transaction.join("state.json"))
        .map_err(|error| error.error)?;
    sync_directory(transaction)?;
    sync_directory(transaction.parent().ok_or_else(invalid)?)
}

fn snapshot_if_present(path: &Path) -> io::Result<Option<BTreeMap<String, String>>> {
    plain_path(path)?;
    path.try_exists()?.then(|| cache_snapshot(path)).transpose()
}

fn verify_installed_cache(root: &Path, name: &str) -> io::Result<()> {
    let expected = expected_tree(&format!("plugins/{name}/"), false)
        .into_iter()
        .map(|(path, sha)| (format!("0.4.0/{path}"), sha))
        .collect();
    if tree(root, false)? != expected {
        return Err(invalid());
    }
    verify_modes(&root.join("0.4.0"), &format!("plugins/{name}/"))
}

fn recover_transactions(home: &Path) -> io::Result<()> {
    let transactions = home.join("plugins/infinishell-transactions");
    plain_path(&transactions)?;
    if !transactions.try_exists()? {
        return Ok(());
    }
    for entry in fs::read_dir(&transactions)? {
        let transaction = entry?.path();
        plain_path(&transaction)?;
        if !transaction.is_dir() {
            return Err(invalid());
        }
        let path = transaction.join("state.json");
        plain_path(&path)?;
        if !path.try_exists()? {
            // 尚未写 journal 的暂存安装没有修改实际缓存或配置，不推断其安装成功。
            continue;
        }
        let bytes = fs::read(relative_file(&transaction, "state.json")?)?;
        let state: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if state.get("version").is_none() {
            let keys = [
                "phase",
                "plugin",
                "source",
                "original_cache",
                "installed_cache",
                "original_marketplace",
                "original_enabled",
            ];
            if matches!(state["phase"].as_str(), Some("verified" | "rolled_back"))
                && state.as_object().is_some_and(|object| {
                    object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
                })
                && matches!(
                    state["plugin"].as_str(),
                    Some("warp@codex-warp" | "orchestration@codex-warp")
                )
            {
                // 已完成的旧版 rev3 归档只保留，不用诊断字符串反向改写用户配置。
                continue;
            }
            return Err(invalid());
        }
        let journal: InstallJournal = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if journal.version != 1
            || !matches!(
                journal.plugin.as_str(),
                "warp@codex-warp" | "orchestration@codex-warp"
            )
        {
            return Err(invalid());
        }
        if matches!(journal.phase.as_str(), "verified" | "rolled_back") {
            continue;
        }
        recover_transaction(home, &transaction, &journal)?;
    }
    invalidate(home);
    Ok(())
}

fn recover_transaction(
    home: &Path,
    transaction: &Path,
    journal: &InstallJournal,
) -> io::Result<()> {
    if journal.version != 1
        || !matches!(
            journal.plugin.as_str(),
            "warp@codex-warp" | "orchestration@codex-warp"
        )
        || !matches!(
            journal.phase.as_str(),
            "prepared" | "cache_written" | "configuration_written" | "needs_review"
        )
        || journal.source != source_path(home)
        || journal.source_metadata_sha256 != digest(METADATA.as_bytes())
        || journal.staged_cache.as_os_str().is_empty()
        || journal
            .staged_cache
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid());
    }
    verify_owned(home)?;
    let key = journal.plugin.clone();
    let name = key.strip_suffix("@codex-warp").ok_or_else(invalid)?;
    let original = journal_scope(&journal.original_scope, &key)?;
    let installed = journal_scope(&journal.installed_scope, &key)?;
    let installed_document = journal
        .installed_scope
        .parse::<DocumentMut>()
        .map_err(|_| invalid())?;
    if !owned_entry(&installed_document, home)
        || installed.enabled.as_bool() != Some(true)
        || installed.marketplace.as_table_like().is_none_or(|entry| {
            entry
                .iter()
                .any(|(key, _)| !matches!(key, "last_updated" | "source_type" | "source"))
        })
    {
        return Err(invalid());
    }
    let target = cache_root(home, name);
    let backup = transaction.join("previous-cache");
    let staged = transaction.join(&journal.staged_cache);
    if staged == backup || staged.starts_with(&backup) {
        return Err(invalid());
    }
    let target_tree = snapshot_if_present(&target)?;
    let backup_tree = snapshot_if_present(&backup)?;
    let staged_tree = snapshot_if_present(&staged)?;
    let current = Scope::read(&read_config(home)?.1, &key);
    // 先核对全部边界再做任何修改；未知缓存、作用域或唯一旧备份都不能覆盖。
    if backup_tree.is_some() && backup_tree != journal.original_cache {
        return Err(invalid());
    }
    if staged_tree.is_some() && staged_tree.as_ref() != Some(&journal.installed_cache) {
        return Err(invalid());
    }
    if installed.matches(&current) && target_tree.as_ref() == Some(&journal.installed_cache) {
        if backup_tree != journal.original_cache {
            return Err(invalid());
        }
        verify_installed_cache(&target, name)?;
        return record_phase(transaction, "verified");
    }
    if !original.matches(&current) || current.enabled.as_bool() == Some(false) {
        return Err(invalid());
    }
    if staged_tree.is_none() && target_tree.as_ref() != Some(&journal.installed_cache) {
        // 普通错误回退也可能被强杀；只恢复已经还原配置且没有未知字节的旧缓存。
        if target_tree == journal.original_cache && backup_tree.is_none() {
            return record_phase(transaction, "rolled_back");
        }
        if target_tree.is_none() && backup_tree.is_some() && backup_tree == journal.original_cache {
            fs::rename(&backup, &target)?;
            sync_directory(target.parent().ok_or_else(invalid)?)?;
            sync_directory(transaction)?;
            return record_phase(transaction, "rolled_back");
        }
        return Err(invalid());
    }
    if target_tree.as_ref() == Some(&journal.installed_cache) && staged_tree.is_none() {
        if backup_tree != journal.original_cache {
            return Err(invalid());
        }
        verify_installed_cache(&target, name)?;
    } else {
        if staged_tree.as_ref() != Some(&journal.installed_cache)
            || (target_tree.is_some()
                && (target_tree != journal.original_cache || backup_tree.is_some()))
            || (target_tree.is_none() && backup_tree != journal.original_cache)
        {
            return Err(invalid());
        }
        verify_installed_cache(&staged, name)?;
        let parent = target.parent().ok_or_else(invalid)?;
        fs::create_dir_all(parent)?;
        if target_tree.is_some() {
            fs::rename(&target, &backup)?;
            sync_directory(parent)?;
            sync_directory(transaction)?;
        }
        fs::rename(&staged, &target)?;
        sync_directory(parent)?;
        sync_directory(staged.parent().ok_or_else(invalid)?)?;
    }
    if snapshot_if_present(&backup)? != journal.original_cache
        || snapshot_if_present(&target)?.as_ref() != Some(&journal.installed_cache)
    {
        return Err(invalid());
    }
    record_phase(transaction, "cache_written")?;
    write_scoped(home, &key, &original, &installed)?;
    sync_directory(home)?;
    record_phase(transaction, "configuration_written")?;
    if snapshot_if_present(&target)?.as_ref() != Some(&journal.installed_cache)
        || !installed.matches(&Scope::read(&read_config(home)?.1, &key))
    {
        return Err(invalid());
    }
    record_phase(transaction, "verified")
}

fn record_phase(transaction: &Path, phase: &str) -> io::Result<()> {
    let path = transaction.join("state.json");
    let mut state: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    state["phase"] = json!(phase);
    let mut temporary = NamedTempFile::new_in(transaction)?;
    temporary.write_all(&serde_json::to_vec_pretty(&state)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_directory(transaction)
}

#[cfg(test)]
#[path = "codex_source_tests.rs"]
mod tests;

static FILES: &[(&str, &[u8])] = &[
    (
        ".agents/plugins/marketplace.json",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/.agents/plugins/marketplace.json"
        ),
    ),
    (
        ".github/workflows/test.yml",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/.github/workflows/test.yml"
        ),
    ),
    (
        "LICENSE",
        include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/LICENSE"),
    ),
    (
        "README.md",
        include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/README.md"),
    ),
    (
        "plugins/orchestration/.codex-plugin/plugin.json",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/.codex-plugin/plugin.json"
        ),
    ),
    (
        "plugins/orchestration/hooks/hooks.json",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/hooks/hooks.json"
        ),
    ),
    (
        "plugins/orchestration/scripts/drain-mailbox.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/scripts/drain-mailbox.sh"
        ),
    ),
    (
        "plugins/orchestration/scripts/on-post-tool-use.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/scripts/on-post-tool-use.sh"
        ),
    ),
    (
        "plugins/orchestration/scripts/on-prompt-submit.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/scripts/on-prompt-submit.sh"
        ),
    ),
    (
        "plugins/orchestration/scripts/on-session-end.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/scripts/on-session-end.sh"
        ),
    ),
    (
        "plugins/orchestration/scripts/on-session-start.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/scripts/on-session-start.sh"
        ),
    ),
    (
        "plugins/orchestration/scripts/on-stop.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/scripts/on-stop.sh"
        ),
    ),
    (
        "plugins/orchestration/scripts/oz-parent-common.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/scripts/oz-parent-common.sh"
        ),
    ),
    (
        "plugins/orchestration/scripts/oz-parent-listener.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/scripts/oz-parent-listener.sh"
        ),
    ),
    (
        "plugins/orchestration/skills/factory-files/SKILL.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/factory-files/SKILL.md"
        ),
    ),
    (
        "plugins/orchestration/skills/factory-files/references/examples.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/factory-files/references/examples.md"
        ),
    ),
    (
        "plugins/orchestration/skills/factory-files/references/scorers.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/factory-files/references/scorers.md"
        ),
    ),
    (
        "plugins/orchestration/skills/factory-files/references/validation.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/factory-files/references/validation.md"
        ),
    ),
    (
        "plugins/orchestration/skills/factory-files/scripts/validate_factory_files.py",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/factory-files/scripts/validate_factory_files.py"
        ),
    ),
    (
        "plugins/orchestration/skills/oz-child-agent-orchestration/SKILL.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/oz-child-agent-orchestration/SKILL.md"
        ),
    ),
    (
        "plugins/orchestration/skills/oz-finish-task/SKILL.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/oz-finish-task/SKILL.md"
        ),
    ),
    (
        "plugins/orchestration/skills/oz-notify-user/SKILL.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/oz-notify-user/SKILL.md"
        ),
    ),
    (
        "plugins/orchestration/skills/oz-report-pr/SKILL.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/oz-report-pr/SKILL.md"
        ),
    ),
    (
        "plugins/orchestration/skills/oz-upload-file/SKILL.md",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/skills/oz-upload-file/SKILL.md"
        ),
    ),
    (
        "plugins/orchestration/tests/test-factory-files.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/orchestration/tests/test-factory-files.sh"
        ),
    ),
    (
        "plugins/warp/.codex-plugin/plugin.json",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/.codex-plugin/plugin.json"
        ),
    ),
    (
        "plugins/warp/hooks/hooks.json",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/hooks/hooks.json"
        ),
    ),
    (
        "plugins/warp/scripts/build-payload.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/build-payload.sh"
        ),
    ),
    (
        "plugins/warp/scripts/on-permission-request.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-permission-request.sh"
        ),
    ),
    (
        "plugins/warp/scripts/on-post-tool-use.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-post-tool-use.sh"
        ),
    ),
    (
        "plugins/warp/scripts/on-prompt-submit.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-prompt-submit.sh"
        ),
    ),
    (
        "plugins/warp/scripts/on-session-start.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-session-start.sh"
        ),
    ),
    (
        "plugins/warp/scripts/on-stop.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-stop.sh"
        ),
    ),
    (
        "plugins/warp/scripts/should-use-structured.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/should-use-structured.sh"
        ),
    ),
    (
        "plugins/warp/scripts/warp-notify.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/warp-notify.sh"
        ),
    ),
    (
        "tests/test-hooks.sh",
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/source/tests/test-hooks.sh"
        ),
    ),
];
