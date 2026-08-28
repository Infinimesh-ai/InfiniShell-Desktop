//! Zap(本地化):harness 可用性模型。
//!
//! 上游这里由云端驱动:`ServerApiProvider` 拉取 `get_available_harnesses()`,
//! 并通过 GraphQL 的 `list_harness_auth_secrets` 拉取云端托管密钥,整条链路
//! 都被 `is_logged_in()` 门控。Zap 没有账号体系、也不接 warp 云端网关,所以
//! 这里改为**本地 CLI 驱动**:
//!
//! - harness 列表在本地静态枚举,`enabled` 取决于对应的第三方 CLI
//!   (`claude` / `codex` / `gemini` / `opencode`)是否在 PATH 中可执行,
//!   内置的 Oz(Zap Agent)恒为可用。
//! - auth secrets 的"拉取"变成纯本地判定:Zap 接的是
//!   `DisabledManagedSecretsClient`(见 `app/src/local_managed_secrets.rs`),
//!   云端托管密钥恒为空,本地 CLI agent 自己维护登录态(`claude login` 等),
//!   所以这里直接给出确定的空列表,不再有登录门控,也不再需要 GraphQL
//!   重试策略。写入路径(创建/删除)仍走本地 `ManagedSecretManager`。
//!
//! 对外 API 形状(公开方法名/签名/事件)与上游保持一致,调用点无需改动。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warp_managed_secrets::client::SecretOwner;
use warp_managed_secrets::{ManagedSecretManager, ManagedSecretOwner, ManagedSecretValue};
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::harness_display;
use crate::ai::local_harness_setup::local_harness_is_product_enabled;

/// Zap 暴露的 harness 全集(顺序即 UI 展示顺序)。`Harness::Unknown` 不进列表。
const KNOWN_HARNESSES: &[Harness] = &[
    Harness::Oz,
    Harness::Claude,
    Harness::Codex,
    Harness::Gemini,
    Harness::OpenCode,
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessModelInfo {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessAvailability {
    pub harness: Harness,
    pub display_name: String,
    pub enabled: bool,
    #[serde(default)]
    pub available_models: Vec<HarnessModelInfo>,
}

/// 本地 CLI 名称。`None` 表示该 harness 不依赖外部 CLI(内置 Oz)。
fn local_cli_command_for(harness: Harness) -> Option<&'static str> {
    match harness {
        // 内置 BYOP agent,不依赖任何外部可执行文件。
        Harness::Oz => None,
        Harness::Claude => Some("claude"),
        Harness::Codex => Some("codex"),
        Harness::Gemini => Some("gemini"),
        Harness::OpenCode => Some("opencode"),
        Harness::Unknown => None,
    }
}

fn local_cli_is_installed(command: &str) -> bool {
    #[cfg(not(target_family = "wasm"))]
    {
        crate::util::path::resolve_executable(command).is_some()
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = command;
        false
    }
}

/// 本地判定单个 harness 是否可用:先看产品开关(feature flag),再看本地 CLI。
fn harness_is_locally_enabled(harness: Harness) -> bool {
    if !local_harness_is_product_enabled(harness) {
        return false;
    }
    match local_cli_command_for(harness) {
        Some(command) => local_cli_is_installed(command),
        None => true,
    }
}

/// 基于本地环境重新计算 harness 列表。
///
/// 与上游不同,这里不带 `available_models`:模型目录只有云端 harness 服务
/// 才提供,本地 CLI 无法枚举。`models_for()` 因此恒返回 `None`,调用点会
/// 回落到各自的默认模型选择逻辑。
fn local_harness_availability() -> Vec<HarnessAvailability> {
    KNOWN_HARNESSES
        .iter()
        .copied()
        .map(|harness| HarnessAvailability {
            harness,
            display_name: harness_display::display_name(harness).to_string(),
            enabled: harness_is_locally_enabled(harness),
            available_models: vec![],
        })
        .collect()
}

#[derive(Debug, Clone)]
pub enum AuthSecretFetchState {
    NotFetched,
    Loading,
    Loaded(Vec<AuthSecretEntry>),
    Failed(#[allow(dead_code)] String),
}

#[derive(Debug, Clone)]
pub struct AuthSecretEntry {
    pub name: String,
    pub owner: SecretOwner,
}

pub enum HarnessAvailabilityEvent {
    Changed,
    AuthSecretsLoaded,
    /// Emitted when a lazy auth-secrets fetch fails. Subscribers should
    /// re-render so any "Loading…" placeholders can transition to an
    /// error state — without this signal the picker would otherwise be
    /// stuck on the loading placeholder until the next refetch.
    ///
    /// Zap:本地判定不会失败,这个事件目前不会被触发;保留变体是因为
    /// 十余处调用点都在 `match` 里穷举它,且未来接真正的本地密钥库时会用到。
    AuthSecretsFetchFailed,
    AuthSecretCreated {
        harness: Harness,
        name: String,
    },
    AuthSecretCreationFailed {
        error: String,
    },
    AuthSecretDeleted {
        harness: Harness,
        name: String,
        owner: SecretOwner,
    },
    AuthSecretDeletionFailed {
        harness: Harness,
        name: String,
        owner: SecretOwner,
        error: String,
    },
}

pub struct HarnessAvailabilityModel {
    harnesses: Vec<HarnessAvailability>,
    auth_secrets: HashMap<Harness, AuthSecretFetchState>,
}

impl HarnessAvailabilityModel {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        // 本地判定是同步的,直接在构造时算出来,不需要上游那套
        // "先塞默认值、等服务端响应再替换" 的缓存流程。
        Self {
            harnesses: local_harness_availability(),
            auth_secrets: HashMap::new(),
        }
    }

    pub fn available_harnesses(&self) -> &[HarnessAvailability] {
        &self.harnesses
    }

    pub fn display_name_for(&self, harness: Harness) -> String {
        self.harnesses
            .iter()
            .find(|h| h.harness == harness)
            .map(|h| h.display_name.clone())
            .unwrap_or_else(|| harness_display::display_name(harness))
    }

    /// Whether the harness selector should be shown (>1 known harness, including disabled).
    pub fn should_show_harness_selector(&self) -> bool {
        FeatureFlag::AgentHarness.is_enabled() && self.harnesses.len() > 1
    }

    /// Whether any harness is available at all (at least one enabled).
    pub fn has_any_enabled_harness(&self) -> bool {
        self.harnesses.iter().any(|h| h.enabled)
    }

    /// Whether a harness is both known and enabled.
    pub fn is_harness_enabled(&self, harness: Harness) -> bool {
        self.harnesses
            .iter()
            .any(|h| h.harness == harness && h.enabled)
    }

    pub fn models_for(&self, harness: Harness) -> Option<&[HarnessModelInfo]> {
        self.harnesses
            .iter()
            .find(|h| h.harness == harness)
            .map(|h| h.available_models.as_slice())
            .filter(|m| !m.is_empty())
    }

    pub fn auth_secrets_for(&self, harness: Harness) -> &AuthSecretFetchState {
        self.auth_secrets
            .get(&harness)
            .unwrap_or(&AuthSecretFetchState::NotFetched)
    }

    pub fn ensure_auth_secrets_fetched(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        // 这个方法是各个 harness 菜单打开时的统一入口。Zap 没有云端推送,
        // 借这个时机顺带重算一次本地 CLI 可用性,这样用户在会话中途装上
        // `claude`/`codex` 后不必重启就能看到 harness 变为可用。
        self.recompute_harnesses(ctx);

        match self.auth_secrets_for(harness) {
            AuthSecretFetchState::NotFetched | AuthSecretFetchState::Failed(_) => {
                self.fetch_auth_secrets(harness, ctx);
            }
            AuthSecretFetchState::Loading | AuthSecretFetchState::Loaded(_) => {}
        }
    }

    /// 解析该 harness 的托管密钥集合。
    ///
    /// Zap 下这是一次纯本地判定,永远得到空集合:云端托管密钥被剥离,
    /// `DisabledManagedSecretsClient::list_secrets()` 也恒返回空,本地 CLI
    /// agent 的凭据由 CLI 自身保管。给出确定的 `Loaded(vec![])`(而不是停在
    /// `NotFetched`/`Loading`)可以让 picker 立刻从 "Loading…" 占位切到
    /// "没有密钥 / 新建" 的正常态,也避免每次开菜单重复发起拉取。
    ///
    /// 后续若接入真正的本地密钥库,把这里换成一次 `ManagedSecretManager`
    /// 的 `list_secrets()` spawn 即可(写入路径见 `create_auth_secret`)。
    fn fetch_auth_secrets(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        self.auth_secrets
            .insert(harness, AuthSecretFetchState::Loaded(Vec::new()));
        ctx.emit(HarnessAvailabilityEvent::AuthSecretsLoaded);
    }

    pub fn invalidate_auth_secrets(&mut self, harness: Harness) {
        self.auth_secrets.remove(&harness);
    }

    pub fn create_auth_secret(
        &mut self,
        harness: Harness,
        name: String,
        value: ManagedSecretValue,
        owner: SecretOwner,
        ctx: &mut ModelContext<Self>,
    ) {
        let manager = ManagedSecretManager::handle(ctx);
        let create_future = manager.as_ref(ctx).create_secret(owner, name, value, None);
        ctx.spawn(create_future, move |me, result, ctx| match result {
            Ok(secret) => {
                let entry = AuthSecretEntry {
                    name: secret.name.clone(),
                    owner: secret_owner_from_managed_owner(&secret.owner),
                };
                match me.auth_secrets.get_mut(&harness) {
                    Some(AuthSecretFetchState::Loaded(entries)) => {
                        entries.push(entry);
                    }
                    _ => {
                        me.auth_secrets
                            .insert(harness, AuthSecretFetchState::Loaded(vec![entry]));
                    }
                }
                ctx.emit(HarnessAvailabilityEvent::AuthSecretCreated {
                    harness,
                    name: secret.name,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                report_error!(e.context("Failed to create harness auth secret"));
                ctx.emit(HarnessAvailabilityEvent::AuthSecretCreationFailed { error: msg });
            }
        });
    }

    pub fn delete_auth_secret(
        &mut self,
        harness: Harness,
        name: String,
        owner: SecretOwner,
        ctx: &mut ModelContext<Self>,
    ) {
        let manager = ManagedSecretManager::handle(ctx);
        let delete_future = manager
            .as_ref(ctx)
            .delete_secret(owner.clone(), name.clone());
        ctx.spawn(delete_future, move |me, result, ctx| match result {
            Ok(()) => {
                if let Some(AuthSecretFetchState::Loaded(entries)) =
                    me.auth_secrets.get_mut(&harness)
                {
                    remove_deleted_auth_secret_entry(entries, &name, &owner);
                }
                ctx.emit(HarnessAvailabilityEvent::AuthSecretDeleted {
                    harness,
                    name,
                    owner,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                report_error!(e.context("Failed to delete harness auth secret"));
                ctx.emit(HarnessAvailabilityEvent::AuthSecretDeletionFailed {
                    harness,
                    name,
                    owner,
                    error: msg,
                });
            }
        });
    }

    /// 重新计算 harness 可用性。
    ///
    /// 上游这里是一次云端请求,Zap 改为本地判定。签名保持 `&self` 不变
    /// (调用点可能在只读上下文里持有模型),所以通过一次空 spawn 拿到
    /// `&mut Self` 再落库。
    pub fn refresh(&self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(async {}, |me, (), ctx| me.recompute_harnesses(ctx));
    }

    fn recompute_harnesses(&mut self, ctx: &mut ModelContext<Self>) {
        let new_harnesses = local_harness_availability();
        if new_harnesses == self.harnesses {
            return;
        }
        self.harnesses = new_harnesses;
        // Invalidate cached auth secrets so the next menu open refetches.
        let stale: Vec<Harness> = self.auth_secrets.keys().copied().collect();
        for harness in stale {
            self.invalidate_auth_secrets(harness);
        }
        ctx.emit(HarnessAvailabilityEvent::Changed);
    }
}

fn secret_owner_from_managed_owner(owner: &ManagedSecretOwner) -> SecretOwner {
    match owner {
        ManagedSecretOwner::Team { uid } => SecretOwner::Team {
            team_uid: uid.clone(),
        },
        ManagedSecretOwner::User { .. } => SecretOwner::CurrentUser,
    }
}

fn remove_deleted_auth_secret_entry(
    entries: &mut Vec<AuthSecretEntry>,
    name: &str,
    owner: &SecretOwner,
) {
    entries.retain(|entry| entry.name.as_str() != name || &entry.owner != owner);
}

impl Entity for HarnessAvailabilityModel {
    type Event = HarnessAvailabilityEvent;
}

impl SingletonEntity for HarnessAvailabilityModel {}
