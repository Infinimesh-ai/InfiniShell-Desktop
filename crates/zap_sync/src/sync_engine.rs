//! 同步引擎
//!
// author: logic
// date: 2026-05-24

use crate::gist_client::{GistClient, GistOps};
use crate::types::*;
use chrono::Utc;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplyDataOutcome {
    pub local_changed: bool,
    pub needs_upload: bool,
}

impl ApplyDataOutcome {
    fn merge(&mut self, other: Self) {
        self.local_changed |= other.local_changed;
        self.needs_upload |= other.needs_upload;
    }
}

/// 数据提供者 trait，各业务模块实现此 trait 接入同步
pub trait SyncDataProvider: Send + Sync {
    /// 数据所属的 section key（如 "ssh"）
    fn section_key(&self) -> &str;

    /// 收集本地数据，返回该 section 的 JSON Value
    fn collect_data(&self, token: &str) -> Result<serde_json::Value, SyncEngineError>;

    /// 将云端数据应用到本地
    fn apply_data(
        &self,
        token: &str,
        data: &serde_json::Value,
    ) -> Result<ApplyDataOutcome, SyncEngineError>;

    /// 同版本或上传前只归并可安全独立合并的数据。默认 provider 无需处理。
    fn reconcile_data(
        &self,
        _token: &str,
        _data: &serde_json::Value,
    ) -> Result<ApplyDataOutcome, SyncEngineError> {
        Ok(ApplyDataOutcome::default())
    }
}

/// 同步引擎，负责上传/下载同步数据到 Gist
pub struct SyncEngine<C: GistOps> {
    client: C,
}

impl SyncEngine<GistClient> {
    /// 创建新的 SyncEngine 实例（使用真实 GistClient）
    pub fn new() -> Self {
        Self {
            client: GistClient::new(),
        }
    }
}

impl<C: GistOps> SyncEngine<C> {
    /// 使用自定义 GistOps 实现创建引擎
    pub fn with_client(client: C) -> Self {
        Self { client }
    }

    /// 上传数据到指定平台
    pub async fn upload(
        &self,
        platform: SyncPlatform,
        token: &str,
        providers: &[&dyn SyncDataProvider],
        version_store: &dyn SyncVersionStore,
    ) -> Result<SyncResult, SyncEngineError> {
        let local_version = tokio::task::block_in_place(|| version_store.get_sync_version())?;
        let token_owned = token.to_string();

        if let Some(gist_id) = self
            .client
            .find_gist(platform, token_owned.clone())
            .await
            .map_err(|e| SyncEngineError::Gist(e.to_string()))?
        {
            let remote_content = self
                .client
                .get_gist_content(platform, token_owned.clone(), gist_id.clone())
                .await
                .map_err(|e| SyncEngineError::Gist(e.to_string()))?;
            let remote_data: SyncData = serde_json::from_str(&remote_content)
                .map_err(|e| SyncEngineError::Serialization(e.to_string()))?;

            // 远端版本更高时仍保留现有冲突流程；其余路径先让 provider 归并。
            if remote_data.version > local_version {
                return Ok(SyncResult::Conflict {
                    local_version,
                    remote_version: remote_data.version,
                });
            }

            let outcome = reconcile_provider_sections(token, providers, &remote_data)?;
            if remote_data.version == local_version {
                if !outcome.needs_upload {
                    if outcome.local_changed {
                        tokio::task::block_in_place(|| {
                            version_store
                                .update_sync_meta(&Utc::now().to_rfc3339(), platform.to_db_str())
                        })?;
                        return Ok(SyncResult::Success {
                            version: local_version,
                            platform,
                        });
                    }
                    return Ok(SyncResult::AlreadyUpToDate {
                        version: local_version,
                    });
                }
            }

            let upload_version = if remote_data.version == local_version {
                next_sync_version(local_version)?
            } else {
                local_version
            };
            let content = collect_sync_content(token, providers, upload_version)?;
            self.client
                .update_gist(platform, token_owned, gist_id, content)
                .await
                .map_err(|e| SyncEngineError::Gist(e.to_string()))?;

            tokio::task::block_in_place(|| {
                version_store.commit_sync_version(local_version, upload_version)
            })?;
            tokio::task::block_in_place(|| {
                version_store.update_sync_meta(&Utc::now().to_rfc3339(), platform.to_db_str())
            })?;
            return Ok(SyncResult::Success {
                version: upload_version,
                platform,
            });
        } else {
            let content = collect_sync_content(token, providers, local_version)?;
            self.client
                .create_gist(platform, token_owned, content)
                .await
                .map_err(|e| SyncEngineError::Gist(e.to_string()))?;
        }

        tokio::task::block_in_place(|| {
            version_store.commit_sync_version(local_version, local_version)
        })?;
        tokio::task::block_in_place(|| {
            version_store.update_sync_meta(&Utc::now().to_rfc3339(), platform.to_db_str())
        })?;
        Ok(SyncResult::Success {
            version: local_version,
            platform,
        })
    }

    /// 从指定平台下载数据
    pub async fn download(
        &self,
        platform: SyncPlatform,
        token: &str,
        providers: &[&dyn SyncDataProvider],
        version_store: &dyn SyncVersionStore,
    ) -> Result<SyncResult, SyncEngineError> {
        let token_owned = token.to_string();

        let gist_id = self
            .client
            .find_gist(platform, token_owned.clone())
            .await
            .map_err(|e| SyncEngineError::Gist(e.to_string()))?
            .ok_or_else(|| SyncEngineError::Gist("Gist 未找到".to_string()))?;

        let remote_content = self
            .client
            .get_gist_content(platform, token_owned.clone(), gist_id.clone())
            .await
            .map_err(|e| SyncEngineError::Gist(e.to_string()))?;
        let remote_data: SyncData = serde_json::from_str(&remote_content)
            .map_err(|e| SyncEngineError::Serialization(e.to_string()))?;

        let local_version = tokio::task::block_in_place(|| version_store.get_sync_version())?;

        if remote_data.version < local_version {
            return Ok(SyncResult::AlreadyUpToDate {
                version: remote_data.version,
            });
        }

        let outcome = if remote_data.version == local_version {
            reconcile_provider_sections(token, providers, &remote_data)?
        } else {
            apply_provider_sections(token, providers, &remote_data)?
        };

        if remote_data.version == local_version && !outcome.local_changed && !outcome.needs_upload {
            return Ok(SyncResult::AlreadyUpToDate {
                version: remote_data.version,
            });
        }

        let final_version = if outcome.needs_upload {
            let version = next_sync_version(std::cmp::max(local_version, remote_data.version))?;
            let content = collect_sync_content(token, providers, version)?;
            self.client
                .update_gist(platform, token_owned, gist_id, content)
                .await
                .map_err(|e| SyncEngineError::Gist(e.to_string()))?;
            version
        } else {
            remote_data.version
        };

        tokio::task::block_in_place(|| {
            version_store.commit_sync_version(local_version, final_version)
        })?;
        tokio::task::block_in_place(|| {
            version_store.update_sync_meta(&Utc::now().to_rfc3339(), platform.to_db_str())
        })?;

        Ok(SyncResult::Success {
            version: final_version,
            platform,
        })
    }

    /// 强制上传，忽略远程版本冲突；远端写成功后才推进本地版本。
    pub async fn force_upload(
        &self,
        platform: SyncPlatform,
        token: &str,
        providers: &[&dyn SyncDataProvider],
        version_store: &dyn SyncVersionStore,
    ) -> Result<SyncResult, SyncEngineError> {
        let local_version = tokio::task::block_in_place(|| version_store.get_sync_version())?;
        let token_owned = token.to_string();

        // 查找已有 Gist
        let gist_id = self
            .client
            .find_gist(platform, token_owned.clone())
            .await
            .map_err(|e| SyncEngineError::Gist(e.to_string()))?;

        // 确定远程版本号
        let remote_version = if let Some(ref gid) = gist_id {
            let remote_content = self
                .client
                .get_gist_content(platform, token_owned.clone(), gid.clone())
                .await
                .map_err(|e| SyncEngineError::Gist(e.to_string()))?;
            let remote_data: SyncData = serde_json::from_str(&remote_content)
                .map_err(|e| SyncEngineError::Serialization(e.to_string()))?;
            reconcile_provider_sections(token, providers, &remote_data)?;
            Some(remote_data.version)
        } else {
            None
        };

        let new_version =
            next_sync_version(std::cmp::max(local_version, remote_version.unwrap_or(0)))?;
        let content = collect_sync_content(token, providers, new_version)?;

        let upload_result = if let Some(gid) = gist_id {
            self.client
                .update_gist(platform, token_owned, gid, content)
                .await
        } else {
            self.client
                .create_gist(platform, token_owned, content)
                .await
                .map(|_| ())
        };

        if let Err(e) = upload_result {
            return Err(SyncEngineError::Gist(e.to_string()));
        }

        tokio::task::block_in_place(|| {
            version_store.commit_sync_version(local_version, new_version)
        })?;
        tokio::task::block_in_place(|| {
            version_store.update_sync_meta(&Utc::now().to_rfc3339(), platform.to_db_str())
        })?;

        Ok(SyncResult::Success {
            version: new_version,
            platform,
        })
    }

    /// 获取本地版本号
    pub fn get_local_version(version_store: &dyn SyncVersionStore) -> Result<i64, SyncEngineError> {
        version_store.get_sync_version()
    }
}

fn next_sync_version(version: i64) -> Result<i64, SyncEngineError> {
    version
        .checked_add(1)
        .ok_or_else(|| SyncEngineError::VersionStore(format!("sync version overflow at {version}")))
}

fn collect_sync_content(
    token: &str,
    providers: &[&dyn SyncDataProvider],
    version: i64,
) -> Result<String, SyncEngineError> {
    let mut sections = serde_json::Map::new();
    for provider in providers {
        let data = tokio::task::block_in_place(|| provider.collect_data(token))?;
        sections.insert(provider.section_key().to_string(), data);
    }
    serde_json::to_string_pretty(&SyncData {
        version,
        synced_at: Utc::now().to_rfc3339(),
        sections,
    })
    .map_err(|e| SyncEngineError::Serialization(e.to_string()))
}

fn reconcile_provider_sections(
    token: &str,
    providers: &[&dyn SyncDataProvider],
    remote_data: &SyncData,
) -> Result<ApplyDataOutcome, SyncEngineError> {
    let mut outcome = ApplyDataOutcome::default();
    for provider in providers {
        if let Some(section_data) = remote_data.sections.get(provider.section_key()) {
            outcome.merge(tokio::task::block_in_place(|| {
                provider.reconcile_data(token, section_data)
            })?);
        } else {
            outcome.needs_upload = true;
        }
    }
    Ok(outcome)
}

fn apply_provider_sections(
    token: &str,
    providers: &[&dyn SyncDataProvider],
    remote_data: &SyncData,
) -> Result<ApplyDataOutcome, SyncEngineError> {
    let mut outcome = ApplyDataOutcome::default();
    for provider in providers {
        if let Some(section_data) = remote_data.sections.get(provider.section_key()) {
            outcome.merge(tokio::task::block_in_place(|| {
                provider.apply_data(token, section_data)
            })?);
        } else {
            outcome.needs_upload = true;
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GistClientError;
    use std::sync::{Arc, Mutex};

    struct MockVersionStore {
        version: Arc<Mutex<i64>>,
    }

    impl MockVersionStore {
        fn new(v: i64) -> Self {
            Self {
                version: Arc::new(Mutex::new(v)),
            }
        }

        fn version_handle(&self) -> Arc<Mutex<i64>> {
            self.version.clone()
        }
    }

    impl SyncVersionStore for MockVersionStore {
        fn get_sync_version(&self) -> Result<i64, SyncEngineError> {
            Ok(*self.version.lock().unwrap())
        }
        fn set_sync_version(&self, version: i64) -> Result<(), SyncEngineError> {
            *self.version.lock().unwrap() = version;
            Ok(())
        }
        fn commit_sync_version(
            &self,
            expected_version: i64,
            synced_version: i64,
        ) -> Result<i64, SyncEngineError> {
            let mut current_version = self.version.lock().unwrap();
            let committed_version = if *current_version == expected_version {
                synced_version
            } else {
                next_sync_version((*current_version).max(synced_version))?
            };
            *current_version = committed_version;
            Ok(committed_version)
        }
        fn update_sync_meta(&self, _time: &str, _platform: &str) -> Result<(), SyncEngineError> {
            Ok(())
        }
    }

    #[test]
    fn test_get_local_version() {
        let store = MockVersionStore::new(42);
        let result = SyncEngine::<MockGistOps>::get_local_version(&store).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_get_local_version_default() {
        let store = MockVersionStore::new(0);
        let result = SyncEngine::<MockGistOps>::get_local_version(&store).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_mock_version_store_set() {
        let store = MockVersionStore::new(1);
        store.set_sync_version(99).unwrap();
        assert_eq!(store.get_sync_version().unwrap(), 99);
    }

    struct MockGistOps {
        find_result: Mutex<Option<String>>,
        content: String,
        create_called: Mutex<bool>,
        update_called: Mutex<bool>,
        written_content: Mutex<Option<String>>,
        fail_create: bool,
        fail_update: bool,
        mutate_version_during_write: Option<Arc<Mutex<i64>>>,
        /// 记录最近一次调用经过的 platform,便于测试断言 platform-specific 路径
        last_platform: Mutex<Option<SyncPlatform>>,
    }

    impl MockGistOps {
        fn new(find_result: Option<String>, content: &str) -> Self {
            Self {
                find_result: Mutex::new(find_result),
                content: content.to_string(),
                create_called: Mutex::new(false),
                update_called: Mutex::new(false),
                written_content: Mutex::new(None),
                fail_create: false,
                fail_update: false,
                mutate_version_during_write: None,
                last_platform: Mutex::new(None),
            }
        }

        fn with_create_failure(mut self) -> Self {
            self.fail_create = true;
            self
        }

        fn with_update_failure(mut self) -> Self {
            self.fail_update = true;
            self
        }

        fn with_concurrent_version_write(mut self, version: Arc<Mutex<i64>>) -> Self {
            self.mutate_version_during_write = Some(version);
            self
        }

        fn simulate_concurrent_version_write(&self) {
            if let Some(version) = &self.mutate_version_during_write {
                *version.lock().unwrap() += 1;
            }
        }
    }

    impl GistOps for MockGistOps {
        async fn validate_token(
            &self,
            platform: SyncPlatform,
            _token: String,
        ) -> Result<String, GistClientError> {
            *self.last_platform.lock().unwrap() = Some(platform);
            Ok("testuser".to_string())
        }
        async fn find_gist(
            &self,
            platform: SyncPlatform,
            _token: String,
        ) -> Result<Option<String>, GistClientError> {
            *self.last_platform.lock().unwrap() = Some(platform);
            Ok(self.find_result.lock().unwrap().clone())
        }
        async fn create_gist(
            &self,
            platform: SyncPlatform,
            _token: String,
            content: String,
        ) -> Result<String, GistClientError> {
            *self.last_platform.lock().unwrap() = Some(platform);
            *self.create_called.lock().unwrap() = true;
            *self.written_content.lock().unwrap() = Some(content);
            self.simulate_concurrent_version_write();
            if self.fail_create {
                return Err(GistClientError::Api {
                    status: 500,
                    body: "create failed".to_string(),
                });
            }
            Ok("new_gist_id".to_string())
        }
        async fn update_gist(
            &self,
            platform: SyncPlatform,
            _token: String,
            _gist_id: String,
            content: String,
        ) -> Result<(), GistClientError> {
            *self.last_platform.lock().unwrap() = Some(platform);
            *self.update_called.lock().unwrap() = true;
            *self.written_content.lock().unwrap() = Some(content);
            self.simulate_concurrent_version_write();
            if self.fail_update {
                return Err(GistClientError::Api {
                    status: 500,
                    body: "update failed".to_string(),
                });
            }
            Ok(())
        }
        async fn get_gist_content(
            &self,
            platform: SyncPlatform,
            _token: String,
            _gist_id: String,
        ) -> Result<String, GistClientError> {
            *self.last_platform.lock().unwrap() = Some(platform);
            Ok(self.content.clone())
        }
    }

    struct MockProvider;

    impl SyncDataProvider for MockProvider {
        fn section_key(&self) -> &str {
            "ssh"
        }
        fn collect_data(&self, _token: &str) -> Result<serde_json::Value, SyncEngineError> {
            Ok(serde_json::json!({"nodes": []}))
        }
        fn apply_data(
            &self,
            _token: &str,
            _data: &serde_json::Value,
        ) -> Result<ApplyDataOutcome, SyncEngineError> {
            Ok(ApplyDataOutcome::default())
        }
    }

    struct OutcomeProvider {
        data: Mutex<serde_json::Value>,
        reconcile_outcome: ApplyDataOutcome,
        apply_outcome: ApplyDataOutcome,
        reconcile_called: Mutex<bool>,
        apply_called: Mutex<bool>,
    }

    impl OutcomeProvider {
        fn new(reconcile_outcome: ApplyDataOutcome, apply_outcome: ApplyDataOutcome) -> Self {
            Self {
                data: Mutex::new(serde_json::json!({"local": true})),
                reconcile_outcome,
                apply_outcome,
                reconcile_called: Mutex::new(false),
                apply_called: Mutex::new(false),
            }
        }

        fn mark_merged(&self) {
            *self.data.lock().unwrap() = serde_json::json!({"merged": true});
        }
    }

    impl SyncDataProvider for OutcomeProvider {
        fn section_key(&self) -> &str {
            "ssh"
        }

        fn collect_data(&self, _token: &str) -> Result<serde_json::Value, SyncEngineError> {
            Ok(self.data.lock().unwrap().clone())
        }

        fn apply_data(
            &self,
            _token: &str,
            _data: &serde_json::Value,
        ) -> Result<ApplyDataOutcome, SyncEngineError> {
            *self.apply_called.lock().unwrap() = true;
            self.mark_merged();
            Ok(self.apply_outcome)
        }

        fn reconcile_data(
            &self,
            _token: &str,
            _data: &serde_json::Value,
        ) -> Result<ApplyDataOutcome, SyncEngineError> {
            *self.reconcile_called.lock().unwrap() = true;
            self.mark_merged();
            Ok(self.reconcile_outcome)
        }
    }

    fn make_sync_data_json(version: i64) -> String {
        let data = SyncData {
            version,
            synced_at: "2026-01-01T00:00:00Z".to_string(),
            sections: {
                let mut m = serde_json::Map::new();
                m.insert("ssh".to_string(), serde_json::json!({"nodes": []}));
                m
            },
        };
        serde_json::to_string_pretty(&data).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upload_creates_new_gist() {
        do_test_upload_creates_new_gist(SyncPlatform::GitHub).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upload_creates_new_gist_gitee() {
        do_test_upload_creates_new_gist(SyncPlatform::Gitee).await;
    }

    async fn do_test_upload_creates_new_gist(platform: SyncPlatform) {
        let mock = MockGistOps::new(None, "");
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(1);
        let result = engine
            .upload(platform, "token", &[&provider], &store)
            .await
            .unwrap();
        assert!(matches!(result, SyncResult::Success { version: 1, .. }));
        assert!(*engine.client.create_called.lock().unwrap());
        // 断言 platform 真的传到 GistOps,避免 mock 吞参数
        assert_eq!(*engine.client.last_platform.lock().unwrap(), Some(platform));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upload_updates_existing_gist() {
        do_test_upload_updates_existing_gist(SyncPlatform::GitHub).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upload_updates_existing_gist_gitee() {
        do_test_upload_updates_existing_gist(SyncPlatform::Gitee).await;
    }

    async fn do_test_upload_updates_existing_gist(platform: SyncPlatform) {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(0));
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(1);
        let result = engine
            .upload(platform, "token", &[&provider], &store)
            .await
            .unwrap();
        assert!(matches!(result, SyncResult::Success { version: 1, .. }));
        assert!(*engine.client.update_called.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upload_detects_conflict() {
        do_test_upload_detects_conflict(SyncPlatform::GitHub).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upload_detects_conflict_gitee() {
        do_test_upload_detects_conflict(SyncPlatform::Gitee).await;
    }

    async fn do_test_upload_detects_conflict(platform: SyncPlatform) {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(5));
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(1);
        let result = engine
            .upload(platform, "token", &[&provider], &store)
            .await
            .unwrap();
        assert!(matches!(
            result,
            SyncResult::Conflict {
                local_version: 1,
                remote_version: 5
            }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_already_up_to_date() {
        do_test_download_already_up_to_date(SyncPlatform::GitHub).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_already_up_to_date_gitee() {
        do_test_download_already_up_to_date(SyncPlatform::Gitee).await;
    }

    async fn do_test_download_already_up_to_date(platform: SyncPlatform) {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(1));
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(5);
        let result = engine
            .download(platform, "token", &[&provider], &store)
            .await
            .unwrap();
        assert!(matches!(result, SyncResult::AlreadyUpToDate { version: 1 }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_succeeds() {
        do_test_download_succeeds(SyncPlatform::GitHub).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_succeeds_gitee() {
        do_test_download_succeeds(SyncPlatform::Gitee).await;
    }

    async fn do_test_download_succeeds(platform: SyncPlatform) {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(10));
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(1);
        let result = engine
            .download(platform, "token", &[&provider], &store)
            .await
            .unwrap();
        assert!(matches!(result, SyncResult::Success { version: 10, .. }));
        assert_eq!(store.get_sync_version().unwrap(), 10);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_gist_not_found() {
        do_test_download_gist_not_found(SyncPlatform::GitHub).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_gist_not_found_gitee() {
        do_test_download_gist_not_found(SyncPlatform::Gitee).await;
    }

    async fn do_test_download_gist_not_found(platform: SyncPlatform) {
        let mock = MockGistOps::new(None, "");
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(1);
        let result = engine
            .download(platform, "token", &[&provider], &store)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upload_equal_version_is_no_op() {
        // 本地与远程同版本时,上传应当返回 AlreadyUpToDate 而非 Conflict
        // (修复:此前会陷入虚假冲突循环,见 PR #161 review)
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(3));
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(3);
        let result = engine
            .upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();
        assert!(matches!(result, SyncResult::AlreadyUpToDate { version: 3 }));
        // 远程版本未变,不应触发任何写操作
        assert!(!*engine.client.update_called.lock().unwrap());
        assert!(!*engine.client.create_called.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_force_upload_new_gist() {
        let mock = MockGistOps::new(None, "");
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(1);
        let result = engine
            .force_upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();
        assert!(matches!(result, SyncResult::Success { version: 2, .. }));
        assert_eq!(store.get_sync_version().unwrap(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_force_upload_existing_gist_max_version() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(5));
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(3);
        let result = engine
            .force_upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();
        assert!(matches!(result, SyncResult::Success { version: 6, .. }));
        assert_eq!(store.get_sync_version().unwrap(), 6);
    }

    fn written_sync_data(engine: &SyncEngine<MockGistOps>) -> SyncData {
        let content = engine
            .client
            .written_content
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        serde_json::from_str(&content).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn equal_version_upload_reconciles_and_uploads_merged_data() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(3));
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: true,
            },
            ApplyDataOutcome::default(),
        );
        let store = MockVersionStore::new(3);

        let result = engine
            .upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();

        assert!(matches!(result, SyncResult::Success { version: 4, .. }));
        assert_eq!(store.get_sync_version().unwrap(), 4);
        assert!(*provider.reconcile_called.lock().unwrap());
        let uploaded = written_sync_data(&engine);
        assert_eq!(uploaded.version, 4);
        assert_eq!(
            uploaded.sections["ssh"],
            serde_json::json!({"merged": true})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn equal_version_download_reconciles_and_uploads_merged_data() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(3));
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: true,
            },
            ApplyDataOutcome::default(),
        );
        let store = MockVersionStore::new(3);

        let result = engine
            .download(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();

        assert!(matches!(result, SyncResult::Success { version: 4, .. }));
        assert_eq!(store.get_sync_version().unwrap(), 4);
        assert!(*provider.reconcile_called.lock().unwrap());
        assert!(*engine.client.update_called.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn newer_remote_download_uploads_local_memory_winners() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(5));
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome::default(),
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: true,
            },
        );
        let store = MockVersionStore::new(1);

        let result = engine
            .download(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();

        assert!(matches!(result, SyncResult::Success { version: 6, .. }));
        assert_eq!(store.get_sync_version().unwrap(), 6);
        assert!(*provider.apply_called.lock().unwrap());
        assert_eq!(written_sync_data(&engine).version, 6);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn newer_remote_download_without_local_winner_does_not_upload() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(5));
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome::default(),
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: false,
            },
        );
        let store = MockVersionStore::new(1);

        let result = engine
            .download(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();

        assert!(matches!(result, SyncResult::Success { version: 5, .. }));
        assert_eq!(store.get_sync_version().unwrap(), 5);
        assert!(!*engine.client.update_called.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upload_to_older_remote_reconciles_before_collecting() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(2));
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: false,
            },
            ApplyDataOutcome::default(),
        );
        let store = MockVersionStore::new(3);

        let result = engine
            .upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();

        assert!(matches!(result, SyncResult::Success { version: 3, .. }));
        assert!(*provider.reconcile_called.lock().unwrap());
        let uploaded = written_sync_data(&engine);
        assert_eq!(uploaded.version, 3);
        assert_eq!(
            uploaded.sections["ssh"],
            serde_json::json!({"merged": true})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn force_upload_reconciles_remote_data_before_collecting() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(5));
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: true,
            },
            ApplyDataOutcome::default(),
        );
        let store = MockVersionStore::new(3);

        let result = engine
            .force_upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();

        assert!(matches!(result, SyncResult::Success { version: 6, .. }));
        assert!(*provider.reconcile_called.lock().unwrap());
        assert_eq!(
            written_sync_data(&engine).sections["ssh"],
            serde_json::json!({"merged": true})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_reconciled_upload_does_not_advance_local_version() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(3))
            .with_update_failure();
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: true,
            },
            ApplyDataOutcome::default(),
        );
        let store = MockVersionStore::new(3);

        let result = engine
            .upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await;

        assert!(matches!(result, Err(SyncEngineError::Gist(_))));
        assert_eq!(store.get_sync_version().unwrap(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn successful_upload_preserves_concurrent_local_write_as_pending() {
        let store = MockVersionStore::new(3);
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(3))
            .with_concurrent_version_write(store.version_handle());
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome {
                local_changed: false,
                needs_upload: true,
            },
            ApplyDataOutcome::default(),
        );

        let result = engine
            .upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await
            .unwrap();

        assert!(matches!(result, SyncResult::Success { version: 4, .. }));
        assert_eq!(written_sync_data(&engine).version, 4);
        assert_eq!(store.get_sync_version().unwrap(), 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_newer_remote_upload_does_not_advance_local_version() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(5))
            .with_update_failure();
        let engine = SyncEngine::with_client(mock);
        let provider = OutcomeProvider::new(
            ApplyDataOutcome::default(),
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: true,
            },
        );
        let store = MockVersionStore::new(1);

        let result = engine
            .download(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await;

        assert!(matches!(result, Err(SyncEngineError::Gist(_))));
        assert_eq!(store.get_sync_version().unwrap(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_force_upload_does_not_advance_local_version() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(5))
            .with_update_failure();
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(3);

        let result = engine
            .force_upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await;

        assert!(matches!(result, Err(SyncEngineError::Gist(_))));
        assert_eq!(store.get_sync_version().unwrap(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_create_gist_does_not_advance_local_version() {
        let mock = MockGistOps::new(None, "").with_create_failure();
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(3);

        let result = engine
            .upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await;

        assert!(matches!(result, Err(SyncEngineError::Gist(_))));
        assert_eq!(store.get_sync_version().unwrap(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn force_upload_rejects_sync_version_overflow() {
        let mock = MockGistOps::new(Some("gist123".to_string()), &make_sync_data_json(i64::MAX));
        let engine = SyncEngine::with_client(mock);
        let provider = MockProvider;
        let store = MockVersionStore::new(3);

        let result = engine
            .force_upload(SyncPlatform::GitHub, "token", &[&provider], &store)
            .await;

        assert!(matches!(result, Err(SyncEngineError::VersionStore(_))));
        assert_eq!(store.get_sync_version().unwrap(), 3);
        assert!(!*engine.client.update_called.lock().unwrap());
    }
}
