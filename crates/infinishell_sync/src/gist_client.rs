//! Gist API 客户端
//!
// author: logic
// date: 2026-05-24

use std::time::Duration;

use reqwest::Client;
use serde_json::json;
use thiserror::Error;

use crate::types::{GistDetail, GistEntry, SyncPlatform};

const GIST_DESCRIPTION: &str = "INFINISHELL_CONFIG";
const GIST_FILENAME: &str = "infinishell_config.json";
const LEGACY_GIST_DESCRIPTION: &str = "ZAP_CONFIG";
const LEGACY_GIST_FILENAME: &str = "zap_config.json";
/// HTTP 整体请求超时（含 connect + read），避免网络挂起让 UI 永远卡在 Syncing
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// find_gist 翻页上限。100/page,上限 20 页 = 2000 个 gist 已远超任何正常用户需要;
/// 超过则提早返回 None,避免 API 分页 quirk 引起死循环 / 触发 rate limit
const FIND_GIST_MAX_PAGES: u32 = 20;

fn sync_file(detail: &serde_json::Value) -> Option<&serde_json::Value> {
    detail["files"]
        .get(GIST_FILENAME)
        .or_else(|| detail["files"].get(LEGACY_GIST_FILENAME))
}

/// Gist API 客户端错误
#[derive(Debug, Error)]
pub enum GistClientError {
    #[error("网络请求失败: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Gist 未找到")]
    NotFound,
    #[error("Token 未配置")]
    NoToken,
    #[error("API 错误: {status} {body}")]
    Api { status: u16, body: String },
}

/// Gist 操作 trait，支持真实客户端和测试 mock
pub trait GistOps: Send + Sync {
    /// 验证 Token 是否有效，返回用户名
    fn validate_token(
        &self,
        platform: SyncPlatform,
        token: String,
    ) -> impl std::future::Future<Output = Result<String, GistClientError>> + Send;

    /// 查找 InfiniShell 配置 Gist；同时识别 Zap 时期创建的旧 Gist。
    fn find_gist(
        &self,
        platform: SyncPlatform,
        token: String,
    ) -> impl std::future::Future<Output = Result<Option<String>, GistClientError>> + Send;

    /// 创建新 Gist
    fn create_gist(
        &self,
        platform: SyncPlatform,
        token: String,
        content: String,
    ) -> impl std::future::Future<Output = Result<String, GistClientError>> + Send;

    /// 更新已有 Gist
    fn update_gist(
        &self,
        platform: SyncPlatform,
        token: String,
        gist_id: String,
        content: String,
    ) -> impl std::future::Future<Output = Result<(), GistClientError>> + Send;

    /// 获取 Gist 文件内容
    fn get_gist_content(
        &self,
        platform: SyncPlatform,
        token: String,
        gist_id: String,
    ) -> impl std::future::Future<Output = Result<String, GistClientError>> + Send;
}

/// Gist API 客户端，支持 GitHub 和 Gitee
pub struct GistClient {
    client: Client,
}

impl GistClient {
    /// 创建新的 GistClient 实例。
    /// build 失败属于不可恢复的运行时错误（TLS backend 初始化失败等),宁可 panic
    /// 也不要静默回退到无 user-agent 的 Client::default() — GitHub 强制要求 UA。
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("InfiniShell-Terminal")
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("failed to build reqwest client for GistClient");
        Self { client }
    }

    /// 构建认证头，GitHub 用 Bearer，Gitee 用 token 前缀
    fn auth_header(platform: SyncPlatform, token: &str) -> String {
        match platform {
            SyncPlatform::GitHub => format!("Bearer {token}"),
            SyncPlatform::Gitee => format!("token {token}"),
        }
    }

    /// 验证 Token 是否有效，返回用户名
    pub async fn validate_token(
        &self,
        platform: SyncPlatform,
        token: &str,
    ) -> Result<String, GistClientError> {
        if token.is_empty() {
            return Err(GistClientError::NoToken);
        }
        let url = format!("{}/user", platform.base_url());
        let resp = self
            .client
            .get(&url)
            .header("Authorization", Self::auth_header(platform, token))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(GistClientError::Api {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }

        let user: serde_json::Value = resp.json().await?;
        // 真实成功响应必须含 login 字段;若不含说明响应不是预期的 GitHub/Gitee
        // /user(可能是 SSO 拦截页 / 代理伪造 200),不能误判为验证通过
        let login = user["login"].as_str().ok_or_else(|| GistClientError::Api {
            status: 200,
            body: "响应缺少 login 字段,Token 未真正通过验证".to_string(),
        })?;
        Ok(login.to_string())
    }

    /// 查找 InfiniShell 配置 Gist，返回其 ID。
    ///
    /// 若账号中同时存在新旧 Gist，始终使用新 Gist；仅存在旧 Gist 时复用旧 ID，
    /// 后续更新会把 description 和主文件名升级为 InfiniShell。
    pub async fn find_gist(
        &self,
        platform: SyncPlatform,
        token: &str,
    ) -> Result<Option<String>, GistClientError> {
        if token.is_empty() {
            return Err(GistClientError::NoToken);
        }
        let base_url = platform.base_url();

        let mut legacy_gist_id = None;
        for page in 1..=FIND_GIST_MAX_PAGES {
            let url = format!("{base_url}/gists?page={page}&per_page=100");
            let resp = self
                .client
                .get(&url)
                .header("Authorization", Self::auth_header(platform, token))
                .send()
                .await?;

            if !resp.status().is_success() {
                return Err(GistClientError::Api {
                    status: resp.status().as_u16(),
                    body: resp.text().await.unwrap_or_default(),
                });
            }

            let gists: Vec<GistEntry> = resp.json().await?;

            if gists.is_empty() {
                return Ok(legacy_gist_id);
            }

            if let Some(found) = gists
                .iter()
                .find(|g| g.description.as_deref() == Some(GIST_DESCRIPTION))
            {
                return Ok(Some(found.id.clone()));
            }

            if legacy_gist_id.is_none() {
                legacy_gist_id = gists
                    .iter()
                    .find(|g| g.description.as_deref() == Some(LEGACY_GIST_DESCRIPTION))
                    .map(|gist| gist.id.clone());
            }
        }

        // 超过 MAX_PAGES 仍未找到,视作不存在 — 上层会触发 create_gist
        log::warn!(
            "find_gist: 已翻 {FIND_GIST_MAX_PAGES} 页仍未找到 {GIST_DESCRIPTION},放弃以避免死循环 / rate limit"
        );
        Ok(legacy_gist_id)
    }

    /// 创建新 Gist
    pub async fn create_gist(
        &self,
        platform: SyncPlatform,
        token: &str,
        content: &str,
    ) -> Result<String, GistClientError> {
        if token.is_empty() {
            return Err(GistClientError::NoToken);
        }
        let url = format!("{}/gists", platform.base_url());
        let body = json!({
            "description": GIST_DESCRIPTION,
            "public": false,
            "files": {
                GIST_FILENAME: {
                    "content": content
                }
            }
        });
        let resp = self
            .client
            .post(&url)
            .header("Authorization", Self::auth_header(platform, token))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(GistClientError::Api {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }

        let detail: GistDetail = resp.json().await?;
        Ok(detail.id)
    }

    /// 更新已有 Gist
    pub async fn update_gist(
        &self,
        platform: SyncPlatform,
        token: &str,
        gist_id: &str,
        content: &str,
    ) -> Result<(), GistClientError> {
        if token.is_empty() {
            return Err(GistClientError::NoToken);
        }
        let url = format!("{}/gists/{gist_id}", platform.base_url());
        let body = json!({
            "description": GIST_DESCRIPTION,
            "files": {
                GIST_FILENAME: {
                    "content": content
                }
            }
        });
        let resp = self
            .client
            .patch(&url)
            .header("Authorization", Self::auth_header(platform, token))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(GistClientError::Api {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }

        Ok(())
    }

    /// 获取 Gist 文件内容，自动处理截断
    pub async fn get_gist_content(
        &self,
        platform: SyncPlatform,
        token: &str,
        gist_id: &str,
    ) -> Result<String, GistClientError> {
        if token.is_empty() {
            return Err(GistClientError::NoToken);
        }
        let url = format!("{}/gists/{gist_id}", platform.base_url());
        let resp = self
            .client
            .get(&url)
            .header("Authorization", Self::auth_header(platform, token))
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(GistClientError::NotFound);
        }
        if !resp.status().is_success() {
            return Err(GistClientError::Api {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }

        let detail: serde_json::Value = resp.json().await?;
        let file_obj = sync_file(&detail).ok_or(GistClientError::NotFound)?;

        if file_obj["truncated"].as_bool() == Some(true) {
            let raw_url = file_obj["raw_url"]
                .as_str()
                .ok_or(GistClientError::NotFound)?;
            let raw_resp = self
                .client
                .get(raw_url)
                .header("Authorization", Self::auth_header(platform, token))
                .send()
                .await?;
            if !raw_resp.status().is_success() {
                return Err(GistClientError::Api {
                    status: raw_resp.status().as_u16(),
                    body: raw_resp.text().await.unwrap_or_default(),
                });
            }
            Ok(raw_resp.text().await?)
        } else {
            let content = file_obj["content"]
                .as_str()
                .ok_or(GistClientError::NotFound)?;
            Ok(content.to_string())
        }
    }
}

impl GistOps for GistClient {
    async fn validate_token(
        &self,
        platform: SyncPlatform,
        token: String,
    ) -> Result<String, GistClientError> {
        self.validate_token(platform, &token).await
    }

    async fn find_gist(
        &self,
        platform: SyncPlatform,
        token: String,
    ) -> Result<Option<String>, GistClientError> {
        self.find_gist(platform, &token).await
    }

    async fn create_gist(
        &self,
        platform: SyncPlatform,
        token: String,
        content: String,
    ) -> Result<String, GistClientError> {
        self.create_gist(platform, &token, &content).await
    }

    async fn update_gist(
        &self,
        platform: SyncPlatform,
        token: String,
        gist_id: String,
        content: String,
    ) -> Result<(), GistClientError> {
        self.update_gist(platform, &token, &gist_id, &content).await
    }

    async fn get_gist_content(
        &self,
        platform: SyncPlatform,
        token: String,
        gist_id: String,
    ) -> Result<String, GistClientError> {
        self.get_gist_content(platform, &token, &gist_id).await
    }
}

#[cfg(test)]
#[path = "gist_client_tests.rs"]
mod tests;
