//! models.dev 数据源接入。
//!
//! 在用户打开 Providers 设置页时,后台异步拉取 `https://models.dev/api.json`,
//! 缓存到 `${cache_dir}/models-dev.json`。下一次启动直接读缓存,
//! 缓存命中且未过 TTL(默认 24h) 不再发请求;过期/缺失时再去拉。
//!
//! 数据结构对齐 opencode 的 `provider/models.ts`:顶层是
//! `{ <provider_id>: Provider }`,Provider 含 `models: { <model_id>: Model }`。
//! catalog 有两个用途:
//! - 运行时根据 attachment / modalities 自动推断附件能力;
//! - API 新发现模型或用户保存新模型 ID 时自动补全元数据,并允许用户手动强制刷新。
//!   两种路径都只更新已配置模型,绝不借 catalog 追加模型。
//!
//! 没列出的字段一律走 `serde(default)` + `#[allow(dead_code)]` 容忍。
//!
//! 设计取舍:**同步缓存读、异步网络拉**。读侧给附件能力推断使用,
//! 拉侧后台 spawn,失败不弹错只 log;缓存读不到时由本地规则兜底。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, SystemTime};

use http_client::Client;
use serde::{Deserialize, Serialize};

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const CACHE_FILENAME: &str = "models-dev.json";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// `models.dev` 顶层数据 — provider_id → Provider。
pub type Catalog = BTreeMap<String, Provider>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Provider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// 上游 API base URL,例如 `https://api.deepseek.com/v1`。
    #[serde(default)]
    pub api: Option<String>,
    /// 该 provider 通常需要的环境变量名,例如 `["DEEPSEEK_API_KEY"]`。
    #[serde(default)]
    pub env: Vec<String>,
    /// 可用模型,key 为模型 id。
    #[serde(default)]
    pub models: BTreeMap<String, Model>,
    /// 文档 URL(部分 provider 有)。
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Model {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default = "default_true")]
    pub tool_call: bool,
    /// 是否支持文件附件(attachment 字段,与 modalities 互补:
    /// modalities 描述 native 多模态;attachment 涵盖 PDF / 通用文件附件协议)。
    #[serde(default)]
    pub attachment: bool,
    /// 输入 / 输出 modalities,典型值:`text` / `image` / `audio` / `video` / `pdf`。
    #[serde(default)]
    pub modalities: ModelModalities,
    /// 上下文窗口上限。
    #[serde(default)]
    pub limit: ModelLimit,
    /// "alpha" / "beta" / "deprecated" 标签。
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelModalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

impl ModelModalities {
    pub fn supports_input(&self, modality: &str) -> bool {
        self.input.iter().any(|m| m.eq_ignore_ascii_case(modality))
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelLimit {
    #[serde(default)]
    pub context: u32,
    #[serde(default)]
    pub output: u32,
}

// ── 进程内单例缓存 ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct State {
    /// 已加载的 catalog。`None` 表示从未加载成功。
    catalog: Option<Catalog>,
    /// 缓存最后修改时间(用于判断是否过期)。
    loaded_at: Option<SystemTime>,
}

fn state() -> &'static RwLock<State> {
    static S: OnceLock<RwLock<State>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(State::default()))
}

fn cache_path() -> PathBuf {
    let mut p = warp_core::paths::cache_dir();
    p.push(CACHE_FILENAME);
    p
}

/// 读取已加载的 catalog 副本。未成功加载过时返回 `None`。
pub fn cached() -> Option<Catalog> {
    state().read().ok().and_then(|s| s.catalog.clone())
}

/// 当前目录缓存的时间戳。用于把一次匹配固化为可离线使用的模型快照。
pub fn loaded_at_unix_seconds() -> u64 {
    state()
        .read()
        .ok()
        .and_then(|state| state.loaded_at)
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// 持久化快照是否已超过 models.dev 缓存 TTL。
pub fn snapshot_is_stale(updated_at_unix_seconds: u64) -> bool {
    if updated_at_unix_seconds == 0 {
        return true;
    }
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH + Duration::from_secs(updated_at_unix_seconds))
        .map(|age| age > CACHE_TTL)
        .unwrap_or(true)
}

/// 一个模型从 models.dev 抽出的能力快照,用于 BYOP UI / chat_stream 决策附件类型。
#[derive(Debug, Clone, Default)]
pub struct ModelCaps {
    pub vision: bool,
    pub pdf: bool,
    pub audio: bool,
    pub attachment: bool,
}

impl ModelCaps {
    pub fn from_model(m: &Model) -> Self {
        Self {
            vision: m.modalities.supports_input("image"),
            pdf: m.modalities.supports_input("pdf") || m.attachment,
            audio: m.modalities.supports_input("audio"),
            attachment: m.attachment,
        }
    }
}

/// 在已加载的 catalog 里按 model_id 查找,返回该模型在 models.dev 上声明的能力。
///
/// 优先用 `provider_id` 精确匹配 catalog provider key;miss 时仅在模型 ID 全局唯一时
/// 返回结果。重复 ID 不擅自选择某个供应商,避免给自定义网关套用错误能力。
pub fn lookup_caps(provider_id: &str, model_id: &str) -> Option<ModelCaps> {
    let s = state().read().ok()?;
    let catalog = s.catalog.as_ref()?;
    if let Some(p) = catalog.get(provider_id) {
        if let Some(m) = p.models.get(model_id) {
            return Some(ModelCaps::from_model(m));
        }
    }
    let mut matches = catalog.values().filter_map(|provider| {
        provider
            .models
            .get(model_id)
            .or_else(|| provider.models.values().find(|model| model.id == model_id))
    });
    let model = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(ModelCaps::from_model(model))
}

/// 把磁盘缓存读进内存(同步,非阻塞;只在 process 启动或 UI 第一次需要时调用)。
/// 如果磁盘缓存不存在或解析失败,返回 false,调用方应触发一次网络拉取。
pub fn load_from_disk() -> bool {
    let path = cache_path();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let mtime = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok());
    match serde_json::from_slice::<Catalog>(&bytes) {
        Ok(catalog) => {
            if let Ok(mut s) = state().write() {
                s.catalog = Some(catalog);
                s.loaded_at = mtime;
            }
            true
        }
        Err(e) => {
            log::warn!("[models.dev] 解析磁盘缓存失败 ({path:?}): {e}");
            false
        }
    }
}

/// 缓存是否过期 — 不存在或超过 TTL。
pub fn is_stale() -> bool {
    let s = match state().read() {
        Ok(s) => s,
        Err(_) => return true,
    };
    match s.loaded_at {
        Some(t) => SystemTime::now()
            .duration_since(t)
            .map(|d| d > CACHE_TTL)
            .unwrap_or(true),
        None => true,
    }
}

/// 异步拉取 models.dev 并写入磁盘缓存与内存缓存。
/// 失败仅 log,不向上 propagate(UI 调用方按 `cached()` 是否为 `Some` 决定显示)。
pub async fn fetch_and_cache(client: Client) -> Result<(), String> {
    let resp = client
        .get(MODELS_DEV_URL)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("HTTP 请求失败: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读响应体失败: {e}"))?;

    let catalog: Catalog =
        serde_json::from_slice(&bytes).map_err(|e| format!("JSON 解析失败: {e}"))?;

    // 写盘 — 失败不算致命,只 log。
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, &bytes) {
        log::warn!("[models.dev] 写磁盘缓存失败 ({path:?}): {e}");
    }

    if let Ok(mut s) = state().write() {
        s.catalog = Some(catalog);
        s.loaded_at = Some(SystemTime::now());
    }
    Ok(())
}

/// 把 models.dev 模型转换为独立的持久化目录快照。
pub fn into_catalog_metadata(
    provider_id: String,
    model_id: String,
    match_confidence: crate::settings::AgentProviderModelCatalogMatch,
    updated_at_unix_seconds: u64,
    model: &Model,
) -> crate::settings::AgentProviderModelCatalogMetadata {
    let caps = ModelCaps::from_model(model);
    crate::settings::AgentProviderModelCatalogMetadata {
        name: if model.name.is_empty() {
            model.id.clone()
        } else {
            model.name.clone()
        },
        context_window: model.limit.context,
        max_output_tokens: model.limit.output,
        reasoning: model.reasoning,
        tool_call: model.tool_call,
        image: caps.vision,
        pdf: caps.pdf,
        audio: caps.audio,
        provider_id,
        model_id,
        match_confidence,
        updated_at_unix_seconds,
        unmatched_in_latest_catalog: false,
    }
}
