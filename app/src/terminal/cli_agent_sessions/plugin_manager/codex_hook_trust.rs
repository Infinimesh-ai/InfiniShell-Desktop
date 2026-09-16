//! 只读核对原生 Hook 信任状态；绝不代替用户写入 trusted_hash 或启用开关。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};
use std::{fs, io};

use serde::Deserialize;
use toml_edit::DocumentMut;

use super::NativeAuthorizationStatus;
use super::notification_patch::{self, PatchKind};

#[derive(Deserialize)]
struct NativeContract {
    hooks: Vec<NativeHook>,
}

#[derive(Deserialize)]
struct NativeHook {
    key: String,
    #[serde(rename = "currentHash")]
    current_hash: String,
}

const CONTRACT: &str =
    include_str!("../../../../assets/bundled/cli-agent-plugins/codex/NATIVE_HOOK_TRUST.json");
static HOOKS: LazyLock<NativeContract> =
    LazyLock::new(|| serde_json::from_str(CONTRACT).expect("随附原生 Hook 信任契约必须有效"));

type FileStamp = (PathBuf, u64, Option<SystemTime>);
struct CachedStatus {
    checked_at: Instant,
    stamps: Vec<FileStamp>,
    status: NativeAuthorizationStatus,
}
static CACHE: LazyLock<Mutex<HashMap<PathBuf, CachedStatus>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn invalidate(home: &Path) {
    CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(home);
}

/// 渲染只读短期缓存；缓存到期比较元数据，文件未变时不再读取完整插件或配置。
pub(super) fn status(home: &Path) -> NativeAuthorizationStatus {
    let previous = {
        let cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(cached) = cache.get(home) {
            if cached.checked_at.elapsed() < Duration::from_secs(1) {
                return cached.status;
            }
            Some((cached.stamps.clone(), cached.status))
        } else {
            None
        }
    };
    let stamps = file_stamps(home).ok();
    let status = if let Some(stamps) = &stamps
        && let Some((previous_stamps, previous_status)) = &previous
        && previous_stamps == stamps
    {
        *previous_status
    } else if stamps.is_none() {
        NativeAuthorizationStatus::Unknown
    } else {
        evaluate(home)
    };
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    if cache.len() > 32 {
        cache.clear();
    }
    cache.insert(
        home.to_owned(),
        CachedStatus {
            checked_at: Instant::now(),
            stamps: stamps.unwrap_or_default(),
            status,
        },
    );
    status
}

fn file_stamps(home: &Path) -> io::Result<Vec<FileStamp>> {
    let mut pending = vec![home.join("plugins/cache/codex-warp/warp")];
    let mut stamps = Vec::new();
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || stamps.len() > 128 {
            return Err(io::Error::other("插件信任检查路径不受控"));
        }
        stamps.push((path.clone(), metadata.len(), metadata.modified().ok()));
        if metadata.is_dir() {
            for child in fs::read_dir(&path)? {
                pending.push(child?.path());
            }
        }
    }
    let config = home.join("config.toml");
    match fs::symlink_metadata(&config) {
        Ok(metadata) if metadata.is_file() => {
            stamps.push((config, metadata.len(), metadata.modified().ok()));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            stamps.push((config, 0, None));
        }
        Ok(_) => return Err(io::Error::other("原生信任配置不是普通文件")),
        Err(error) => return Err(error),
    }
    stamps.sort();
    Ok(stamps)
}

fn evaluate(home: &Path) -> NativeAuthorizationStatus {
    if !notification_patch::full_tree_is_applied(home, PatchKind::Codex) {
        return NativeAuthorizationStatus::Unknown;
    }
    let config = match fs::read_to_string(home.join("config.toml")) {
        Ok(contents) => match contents.parse::<DocumentMut>() {
            Ok(config) => config,
            Err(_) => return NativeAuthorizationStatus::Unknown,
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return NativeAuthorizationStatus::Required;
        }
        Err(_) => return NativeAuthorizationStatus::Unknown,
    };
    configured_status(&config)
}

fn configured_status(config: &DocumentMut) -> NativeAuthorizationStatus {
    let states = config.get("hooks").and_then(|hooks| hooks.get("state"));
    for expected in &HOOKS.hooks {
        let state = states.and_then(|states| states.get(&expected.key));
        if state
            .and_then(|state| state.get("enabled"))
            .is_some_and(|enabled| enabled.as_bool().is_none())
        {
            return NativeAuthorizationStatus::Unknown;
        }
        if state
            .and_then(|state| state.get("enabled"))
            .and_then(|enabled| enabled.as_bool())
            == Some(false)
            || state
                .and_then(|state| state.get("trusted_hash"))
                .and_then(|hash| hash.as_str())
                != Some(expected.current_hash.as_str())
        {
            return NativeAuthorizationStatus::Required;
        }
    }
    // 这里只证明磁盘配置匹配固定契约；版本兼容与当前会话通知分别验证，不依赖安装进程的 PATH 缓存。
    NativeAuthorizationStatus::Configured
}

#[cfg(test)]
#[path = "codex_hook_trust_tests.rs"]
mod tests;
