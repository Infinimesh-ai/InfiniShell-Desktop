//! SSH 数据同步提供者，实现 SyncDataProvider trait
//!
// author: logic
// date: 2026-05-26

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use diesel::connection::{Connection, SimpleConnection};
use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl};
use serde::{Deserialize, Serialize};
use zap_sync::{ApplyDataOutcome, SyncDataProvider, SyncEngineError, SyncVersionStore, crypto};
use zeroize::Zeroizing;

use crate::db::with_conn;
use crate::memory::{MAX_MEMORY_CHARS, MachineMemory, MachineMemoryRepository};
use crate::repository::{SshRepository, SyncMetaRepository};
use crate::secrets::{KeychainSecretStore, SecretKind, SshSecretStore};
use crate::types::{NodeKind, OneKeyCredentialKind, SshRoute};

/// keychain 三种凭据 kind,用于 collect/apply/orphan-cleanup 时统一遍历
const ALL_SECRET_KINDS: [SecretKind; 4] = [
    SecretKind::Password,
    SecretKind::Passphrase,
    SecretKind::RootPassword,
    SecretKind::OneKeyPassword,
];

/// SSH 同步用的节点数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub name: String,
    pub sort_order: i32,
    pub is_collapsed: bool,
}

/// SSH 同步用的服务器数据（含加密密码）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncServer {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    pub key_path: Option<String>,
    pub startup_command: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub credential_id: Option<String>,
    pub password_encrypted: Option<String>,
    pub passphrase_encrypted: Option<String>,
    pub root_password_encrypted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOneKeyCredential {
    pub id: String,
    pub label: String,
    pub username: String,
    #[serde(default = "default_onekey_kind")]
    pub kind: String,
    #[serde(default)]
    pub key_path: Option<String>,
    pub password_encrypted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMachineMemory {
    pub machine_key: String,
    #[serde(default)]
    pub content_encrypted: Option<String>,
    #[serde(default)]
    pub hostname_alias: Option<String>,
    #[serde(default)]
    pub ssh_node_id: Option<String>,
    #[serde(default)]
    pub last_review_at: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub deleted_at: Option<String>,
}

/// SSH 同步数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SshSyncData {
    pub nodes: Vec<SyncNode>,
    pub servers: Vec<SyncServer>,
    #[serde(default)]
    pub onekey_credentials: Vec<SyncOneKeyCredential>,
    #[serde(default)]
    pub machine_memories: Vec<SyncMachineMemory>,
    /// 仅同步路径结构；该类型没有密码、socket、私钥内容或 agent 信息。
    /// `None` 表示载荷来自尚不认识保存路径的旧客户端，应用时必须保留本地路径；
    /// `Some([])` 才表示新客户端明确清空路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routes: Option<Vec<SshRoute>>,
}

/// SSH 数据同步提供者
pub struct SshSyncProvider {
    secret_store: Box<dyn SshSecretStore>,
}

impl SshSyncProvider {
    /// 创建新的 SshSyncProvider 实例
    pub fn new() -> Self {
        Self {
            secret_store: Box::new(KeychainSecretStore::default()),
        }
    }

    /// 测试注入自定义凭据存储,绕开 OS keychain。
    #[cfg(test)]
    pub(crate) fn with_secret_store(secret_store: Box<dyn SshSecretStore>) -> Self {
        Self { secret_store }
    }
}

impl SyncDataProvider for SshSyncProvider {
    fn section_key(&self) -> &str {
        "ssh"
    }

    fn collect_data(&self, token: &str) -> Result<serde_json::Value, SyncEngineError> {
        let nodes = with_conn(|conn| Ok(SshRepository::list_nodes(conn)?))
            .map_err(|e| SyncEngineError::Provider(e.to_string()))?;

        let mut sync_nodes = Vec::new();
        let mut sync_servers = Vec::new();
        let mut sync_onekey_credentials = Vec::new();

        let machine_memories =
            with_conn(|conn| Ok(MachineMemoryRepository::list_all_for_sync(conn)?))
                .map_err(|e| SyncEngineError::Provider(e.to_string()))?;
        let sync_machine_memories = encrypt_machine_memories(token, &machine_memories)?;
        let routes = with_conn(|conn| Ok(SshRepository::list_routes(conn)?))
            .map_err(|e| SyncEngineError::Provider(e.to_string()))?;

        let onekey_credentials =
            with_conn(|conn| Ok(SshRepository::list_onekey_credentials(conn)?))
                .map_err(|e| SyncEngineError::Provider(e.to_string()))?;
        for credential in onekey_credentials {
            let secret_kind = onekey_secret_kind(credential.kind);
            let password = read_secret(self.secret_store.as_ref(), &credential.id, secret_kind)?;
            sync_onekey_credentials.push(SyncOneKeyCredential {
                id: credential.id,
                label: credential.label,
                username: credential.username,
                kind: credential.kind.as_db_str().to_string(),
                key_path: credential.key_path,
                password_encrypted: encrypt_optional(token, password.as_deref())?,
            });
        }

        for node in &nodes {
            sync_nodes.push(SyncNode {
                id: node.id.clone(),
                parent_id: node.parent_id.clone(),
                kind: node.kind.as_db_str().to_string(),
                name: node.name.clone(),
                sort_order: node.sort_order,
                is_collapsed: node.is_collapsed,
            });

            if node.kind == NodeKind::Server {
                let server_result =
                    with_conn(|conn| Ok(SshRepository::get_server(conn, &node.id)?))
                        .map_err(|e| SyncEngineError::Provider(e.to_string()))?;
                if let Some(server) = server_result {
                    // 区分 keychain 错误与"用户没设密码":
                    // - Ok(Some) = 有密码,加密上传
                    // - Ok(None) = 用户确实没设,字段写 None
                    // - Err = 中止整次上传,避免把瞬时 keychain 故障序列化为
                    //   "无密码"覆盖其他设备的真实密码(PR #161 review #5)
                    let password =
                        read_secret(self.secret_store.as_ref(), &node.id, SecretKind::Password)?;
                    let passphrase =
                        read_secret(self.secret_store.as_ref(), &node.id, SecretKind::Passphrase)?;
                    let root_password = read_secret(
                        self.secret_store.as_ref(),
                        &node.id,
                        SecretKind::RootPassword,
                    )?;

                    sync_servers.push(SyncServer {
                        node_id: server.node_id.clone(),
                        host: server.host.clone(),
                        port: server.port,
                        username: server.username.clone(),
                        auth_type: server.auth_type.as_db_str().to_string(),
                        key_path: server.key_path.clone(),
                        startup_command: server.startup_command.clone(),
                        notes: server.notes.clone(),
                        credential_id: server.credential_id.clone(),
                        password_encrypted: encrypt_optional(token, password.as_deref())?,
                        passphrase_encrypted: encrypt_optional(token, passphrase.as_deref())?,
                        root_password_encrypted: encrypt_optional(token, root_password.as_deref())?,
                    });
                }
            }
        }

        let data = SshSyncData {
            nodes: sync_nodes,
            servers: sync_servers,
            onekey_credentials: sync_onekey_credentials,
            machine_memories: sync_machine_memories,
            routes: Some(routes),
        };

        serde_json::to_value(&data)
            .map_err(|e: serde_json::Error| SyncEngineError::Serialization(e.to_string()))
    }

    fn apply_data(
        &self,
        token: &str,
        data: &serde_json::Value,
    ) -> Result<ApplyDataOutcome, SyncEngineError> {
        let ssh_data: SshSyncData = serde_json::from_value(data.clone())
            .map_err(|e: serde_json::Error| SyncEngineError::Serialization(e.to_string()))?;

        // 先解密全部 memory，确保坏密文不会触发后续 keychain 或数据库写入。
        // 本地快照的读取与归并推迟到阶段 2 的写回事务内(见 merge_and_persist_memories),
        // 在此处提前读会留下竞态窗口:归并期间 Agent 的 update_machine_memory 写入
        // 会被旧快照算出的归并结果覆盖。
        let remote_memories = decrypt_machine_memories(token, &ssh_data.machine_memories)?;

        // ---- 阶段 0 ---- 全部解密 + 收集 explicit-clear 列表
        // pending_secrets: 远程明确给了密文 → 需要写入 keychain
        // explicit_clears: 远程明确给了 None → 需要 delete keychain(用户在其他设备清空了密码,
        //                  不清理会导致本地继续用旧密码,违反用户意图;PR #161 七轮 review)
        struct PendingSecret {
            node_id: String,
            kind: SecretKind,
            value: String,
        }
        let mut pending_secrets: Vec<PendingSecret> = Vec::new();
        let mut explicit_clears: Vec<(String, SecretKind)> = Vec::new();
        for server in &ssh_data.servers {
            for (kind, enc) in [
                (SecretKind::Password, &server.password_encrypted),
                (SecretKind::Passphrase, &server.passphrase_encrypted),
                (SecretKind::RootPassword, &server.root_password_encrypted),
            ] {
                match enc {
                    Some(enc) => {
                        let value = crypto::decrypt(token, enc)
                            .map_err(|e| SyncEngineError::Crypto(e.to_string()))?;
                        pending_secrets.push(PendingSecret {
                            node_id: server.node_id.clone(),
                            kind,
                            value,
                        });
                    }
                    None => {
                        explicit_clears.push((server.node_id.clone(), kind));
                    }
                }
            }
        }
        for credential in &ssh_data.onekey_credentials {
            let secret_kind = onekey_secret_kind(
                OneKeyCredentialKind::parse(&credential.kind)
                    .unwrap_or(OneKeyCredentialKind::Password),
            );
            match &credential.password_encrypted {
                Some(enc) => {
                    let value = crypto::decrypt(token, enc)
                        .map_err(|e| SyncEngineError::Crypto(e.to_string()))?;
                    pending_secrets.push(PendingSecret {
                        node_id: credential.id.clone(),
                        kind: secret_kind,
                        value,
                    });
                }
                None => {
                    explicit_clears.push((credential.id.clone(), secret_kind));
                }
            }
        }

        // ---- 阶段 0.5 ---- 拓扑排序节点,父节点先于子节点;orphan(parent 不在数据集中)
        // 视作根节点插入,避免 SQLite FK 违规整事务回滚
        let sorted_nodes = topologically_sort_nodes(&ssh_data.nodes);
        let synced_node_ids = sorted_nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();

        // ---- 阶段 0.6 ---- 收集本地原有 keychain owner id,供后续 orphan keychain 清理
        let mut existing_secret_owner_ids: Vec<String> = with_conn(|conn| {
            Ok(persistence::schema::ssh_nodes::table
                .select(persistence::schema::ssh_nodes::id)
                .load::<String>(conn)?)
        })
        .map_err(|e| SyncEngineError::Provider(e.to_string()))?;
        let existing_credential_ids: Vec<String> = with_conn(|conn| {
            Ok(persistence::schema::ssh_onekey_credentials::table
                .select(persistence::schema::ssh_onekey_credentials::id)
                .load::<String>(conn)?)
        })
        .map_err(|e| SyncEngineError::Provider(e.to_string()))?;
        existing_secret_owner_ids.extend(existing_credential_ids);

        // ---- 阶段 1 ---- 先写 keychain。任一失败 → 立即中止,不动 DB。
        // 跟踪 (node_id, kind, prior_value) 列表,DB 阶段失败时:
        // - prior_value=Some(v) → restore 回旧值(避免覆盖了用户既有密码)
        // - prior_value=None    → delete(避免污染)
        // 真正的"原子回滚"以 secret_store.set 的幂等覆盖语义为基础(PR #161 三轮 review)
        let mut written_secrets: Vec<WrittenSecret> = Vec::new();
        for s in &pending_secrets {
            // 写入前快照原值,以便后续 rollback 可以真正恢复旧值。
            // 真实的 keychain 错误中止整个流程,但 NoBackend(headless Linux 等)按"无旧值"处理。
            // 此设计与 collect_data 的 read_secret 一致 — 同样的环境约束。
            let prior_value = match self.secret_store.get(&s.node_id, s.kind) {
                // store.get 已经返回 Option<Zeroizing<String>>,直接用,保留零化语义
                Ok(opt) => opt,
                Err(e) => {
                    // 与 read_secret 同等严格:keychain 任何错误都中止,避免无法 rollback
                    rollback_keychain_writes(self.secret_store.as_ref(), &written_secrets);
                    return Err(SyncEngineError::Provider(format!(
                        "读取 keychain 旧值失败 ({}, {:?}): {e}。已回滚 {} 项,请确认密钥库可用后重试下载",
                        s.node_id,
                        s.kind,
                        written_secrets.len()
                    )));
                }
            };
            if let Err(e) = self.secret_store.set(&s.node_id, s.kind, &s.value) {
                rollback_keychain_writes(self.secret_store.as_ref(), &written_secrets);
                return Err(SyncEngineError::Provider(format!(
                    "写入 keychain 失败 ({}, {:?}): {e},请检查密钥库权限后重试下载",
                    s.node_id, s.kind
                )));
            }
            written_secrets.push(WrittenSecret {
                node_id: s.node_id.clone(),
                kind: s.kind,
                prior_value,
            });
        }

        // ---- 阶段 2 ---- DB 事务:DELETE + 按拓扑顺序 INSERT
        let db_result = with_conn(|conn| {
            conn.transaction::<ApplyDataOutcome, anyhow::Error, _>(|conn| {
                let (preserved_route_targets, preserved_hop_nodes) =
                    if ssh_data.routes.is_none() {
                        (
                            persistence::schema::ssh_routes::table
                                .select((
                                    persistence::schema::ssh_routes::id,
                                    persistence::schema::ssh_routes::target_node_id,
                                ))
                                .load::<(String, Option<String>)>(conn)?,
                            persistence::schema::ssh_route_hops::table
                                .select((
                                    persistence::schema::ssh_route_hops::route_id,
                                    persistence::schema::ssh_route_hops::position,
                                    persistence::schema::ssh_route_hops::node_id,
                                ))
                                .load::<(String, i32, Option<String>)>(conn)?,
                        )
                    } else {
                        (Vec::new(), Vec::new())
                    };
                if ssh_data.routes.is_some() {
                    conn.batch_execute("DELETE FROM ssh_route_hops; DELETE FROM ssh_routes;")?;
                }
                conn.batch_execute(
                    "DELETE FROM ssh_servers; DELETE FROM ssh_nodes; DELETE FROM ssh_onekey_credentials;",
                )?;

                for credential in &ssh_data.onekey_credentials {
                    diesel::insert_into(persistence::schema::ssh_onekey_credentials::table)
                        .values(persistence::model::NewSshOneKeyCredential {
                            id: &credential.id,
                            label: &credential.label,
                            username: &credential.username,
                            kind: OneKeyCredentialKind::parse(&credential.kind)
                                .unwrap_or(OneKeyCredentialKind::Password)
                                .as_db_str(),
                            key_path: credential.key_path.as_deref(),
                        })
                        .execute(conn)?;
                }

                for node in &sorted_nodes {
                    let kind = NodeKind::parse(&node.kind)
                        .ok_or_else(|| anyhow::anyhow!("无效的 kind: {}", node.kind))?;
                    diesel::insert_into(persistence::schema::ssh_nodes::table)
                        .values(persistence::model::NewSshNode {
                            id: &node.id,
                            parent_id: node.parent_id.as_deref(),
                            kind: kind.as_db_str(),
                            name: &node.name,
                            sort_order: node.sort_order,
                        })
                        .execute(conn)?;
                    if node.is_collapsed {
                        SshRepository::set_collapsed(conn, &node.id, true)?;
                    }
                }

                for server in &ssh_data.servers {
                    diesel::insert_into(persistence::schema::ssh_servers::table)
                        .values(persistence::model::NewSshServer {
                            node_id: &server.node_id,
                            host: &server.host,
                            port: server.port as i32,
                            username: &server.username,
                            auth_type: &server.auth_type,
                            key_path: server.key_path.as_deref(),
                            startup_command: server.startup_command.as_deref(),
                            notes: server.notes.as_deref(),
                            credential_id: server.credential_id.as_deref(),
                        })
                        .execute(conn)?;
                }
                // 旧客户端载荷不包含 routes。重建 ssh_nodes 时 SQLite 会按外键规则
                // 暂时把保存路径的 node 引用置空；对本次载荷中仍存在的 node 恢复
                // 引用，避免一次旧客户端同步让第一跳丢失 SSH Manager 凭据关联。
                for (route_id, node_id) in preserved_route_targets {
                    if let Some(node_id) = node_id.filter(|id| synced_node_ids.contains(id.as_str()))
                    {
                        diesel::update(persistence::schema::ssh_routes::table.find(route_id))
                            .set(persistence::schema::ssh_routes::target_node_id.eq(node_id))
                            .execute(conn)?;
                    }
                }
                for (route_id, position, node_id) in preserved_hop_nodes {
                    if let Some(node_id) = node_id.filter(|id| synced_node_ids.contains(id.as_str()))
                    {
                        diesel::update(
                            persistence::schema::ssh_route_hops::table
                                .filter(
                                    persistence::schema::ssh_route_hops::route_id.eq(route_id),
                                )
                                .filter(
                                    persistence::schema::ssh_route_hops::position.eq(position),
                                ),
                        )
                        .set(persistence::schema::ssh_route_hops::node_id.eq(node_id))
                        .execute(conn)?;
                    }
                }
                for route in ssh_data.routes.iter().flatten() {
                    if route.name.trim().is_empty() || route.hops.is_empty() || route.hops.len() > 8
                    {
                        return Err(anyhow::anyhow!("同步数据包含无效的 SSH 路径"));
                    }
                    let target_node_id = route
                        .target_node_id
                        .as_deref()
                        .filter(|node_id| synced_node_ids.contains(node_id));
                    diesel::insert_into(persistence::schema::ssh_routes::table)
                        .values((
                            persistence::schema::ssh_routes::id.eq(&route.id),
                            persistence::schema::ssh_routes::name.eq(&route.name),
                            persistence::schema::ssh_routes::target_node_id.eq(target_node_id),
                            persistence::schema::ssh_routes::created_at.eq(route.created_at),
                            persistence::schema::ssh_routes::updated_at.eq(route.updated_at),
                            persistence::schema::ssh_routes::last_connected_at
                                .eq(route.last_connected_at),
                        ))
                        .execute(conn)?;
                    for (position, hop) in route.hops.iter().enumerate() {
                        if hop.port == Some(0)
                            || hop.target_alias.trim().is_empty()
                            || hop.target_alias.trim() != hop.target_alias
                            || hop.target_alias.starts_with('-')
                            || hop.target_alias.len() > 255
                            || hop
                                .target_alias
                                .chars()
                                .any(|ch| ch.is_control() || ch.is_whitespace())
                        {
                            return Err(anyhow::anyhow!("同步数据包含无效的 SSH 跳点"));
                        }
                        let node_id = hop
                            .node_id
                            .as_deref()
                            .filter(|node_id| synced_node_ids.contains(node_id));
                        diesel::insert_into(persistence::schema::ssh_route_hops::table)
                            .values(persistence::model::NewSshRouteHop {
                                route_id: &route.id,
                                position: position as i32,
                                node_id,
                                target_alias: &hop.target_alias,
                                port: hop.port.map(i32::from),
                                execution_scope: "previous_hop",
                            })
                            .execute(conn)?;
                    }
                }
                // 快照读取 + LWW 归并 + 写回在同一事务内完成,
                // 阶段 1(keychain)期间落地的本地 memory 写入也会被归并进来。
                merge_and_persist_memories(conn, &remote_memories)
            })
        });
        let memory_outcome = match db_result {
            Ok(outcome) => outcome,
            Err(e) => {
                // DB 失败 → 回滚刚写入的 keychain,避免长期残留指向不存在 node 的密钥
                let rolled = written_secrets.len();
                rollback_keychain_writes(self.secret_store.as_ref(), &written_secrets);
                return Err(SyncEngineError::Provider(format!(
                    "DB 写入失败 ({e});已回滚 {rolled} 项 keychain 写入"
                )));
            }
        };

        // ---- 阶段 3a ---- 清理 explicit-clear:节点仍存在但远程把对应 *_encrypted 设为 None
        // 用户在其他设备清空了某项密码 → 必须 delete 本地 keychain,否则 connect 时会继续用旧密码,
        // 违背用户清除意图(PR #161 七轮 review)
        for (node_id, kind) in &explicit_clears {
            if let Err(e) = self.secret_store.delete(node_id, *kind) {
                log::warn!(
                    "清理 explicit-clear keychain 项失败 {node_id}/{:?}: {e}",
                    kind
                );
            }
        }

        // ---- 阶段 3b ---- 清理 orphan keychain:本地原有但远程已删除的 owner id 对应的密码,
        // 必须显式 delete,否则同 UUID 节点重新出现时会读到陈旧密码 (PR #161 review #4)
        let mut new_secret_owner_ids: HashSet<&str> =
            ssh_data.nodes.iter().map(|n| n.id.as_str()).collect();
        new_secret_owner_ids.extend(
            ssh_data
                .onekey_credentials
                .iter()
                .map(|credential| credential.id.as_str()),
        );
        for old_id in &existing_secret_owner_ids {
            if new_secret_owner_ids.contains(old_id.as_str()) {
                continue;
            }
            for kind in ALL_SECRET_KINDS {
                if let Err(e) = self.secret_store.delete(old_id, kind) {
                    log::warn!("清理 orphan keychain 项失败 {old_id}/{:?}: {e}", kind);
                }
            }
        }

        Ok(memory_outcome)
    }

    fn reconcile_data(
        &self,
        token: &str,
        data: &serde_json::Value,
    ) -> Result<ApplyDataOutcome, SyncEngineError> {
        let ssh_data: SshSyncData = serde_json::from_value(data.clone())
            .map_err(|e: serde_json::Error| SyncEngineError::Serialization(e.to_string()))?;
        let remote_memories = decrypt_machine_memories(token, &ssh_data.machine_memories)?;
        // 快照读取与写回必须在同一次 with_conn(持全局锁)+ 同一事务内,
        // 否则归并期间的本地写会被旧快照覆盖(见 merge_and_persist_memories)。
        with_conn(|conn| {
            conn.transaction::<ApplyDataOutcome, anyhow::Error, _>(|conn| {
                merge_and_persist_memories(conn, &remote_memories)
            })
        })
        .map_err(|e| SyncEngineError::Provider(e.to_string()))
    }
}

/// 在同一事务内完成:读取本地 memory 快照 → LWW 归并 → 写回。
///
/// 快照与写回必须原子。若先在单独一次 `with_conn` 里读快照(db.rs 的全局锁
/// 按调用持有,不跨读写),后台同步归并期间 Agent 的 `update_machine_memory`
/// 写入(更新 updated_at 并 bump sync_version)会被旧快照算出的归并结果覆盖,
/// updated_at 更旧的数据反而胜出,造成真实数据丢失;commit_sync_version 的
/// pending 机制只保住版本号,重新同步时上传的仍是被覆盖后的旧数据
/// (specs/ssh-machine-memory/TECH.md Task 6)。
fn merge_and_persist_memories(
    conn: &mut diesel::sqlite::SqliteConnection,
    remote_memories: &[MachineMemory],
) -> Result<ApplyDataOutcome, anyhow::Error> {
    let local_memories = MachineMemoryRepository::list_all_for_sync(conn)?;
    let (merged_memories, outcome) = merge_machine_memories(&local_memories, remote_memories);
    // 归并结果与本地一致时跳过写回,避免无意义的行覆盖。
    if outcome.local_changed {
        for memory in &merged_memories {
            MachineMemoryRepository::upsert_from_sync(conn, memory)?;
        }
    }
    Ok(outcome)
}

fn encrypt_machine_memories(
    token: &str,
    memories: &[MachineMemory],
) -> Result<Vec<SyncMachineMemory>, SyncEngineError> {
    let mut encrypted = memories
        .iter()
        .map(|memory| {
            let content_encrypted = if memory.deleted_at.is_some() {
                None
            } else {
                Some(
                    crypto::encrypt(token, &memory.content)
                        .map_err(|e| SyncEngineError::Crypto(e.to_string()))?,
                )
            };
            Ok(SyncMachineMemory {
                machine_key: memory.machine_key.clone(),
                content_encrypted,
                hostname_alias: memory.hostname_alias.clone(),
                ssh_node_id: memory.ssh_node_id.clone(),
                last_review_at: memory.last_review_at.map(|at| at.to_rfc3339()),
                updated_at: memory.updated_at.to_rfc3339(),
                deleted_at: memory.deleted_at.map(|at| at.to_rfc3339()),
            })
        })
        .collect::<Result<Vec<_>, SyncEngineError>>()?;
    encrypted.sort_by(|a, b| a.machine_key.cmp(&b.machine_key));
    Ok(encrypted)
}

fn decrypt_machine_memories(
    token: &str,
    memories: &[SyncMachineMemory],
) -> Result<Vec<MachineMemory>, SyncEngineError> {
    let mut seen = HashSet::new();
    let mut decrypted = Vec::with_capacity(memories.len());
    for memory in memories {
        if !seen.insert(memory.machine_key.as_str()) {
            return Err(SyncEngineError::Serialization(format!(
                "duplicate machine memory key: {}",
                memory.machine_key
            )));
        }

        let last_review_at = memory
            .last_review_at
            .as_deref()
            .map(|value| parse_memory_timestamp("last_review_at", &memory.machine_key, value))
            .transpose()?;
        let updated_at =
            parse_memory_timestamp("updated_at", &memory.machine_key, &memory.updated_at)?;
        let deleted_at = memory
            .deleted_at
            .as_deref()
            .map(|value| parse_memory_timestamp("deleted_at", &memory.machine_key, value))
            .transpose()?;

        let mut content = match &memory.content_encrypted {
            Some(content) => crypto::decrypt(token, content)
                .map_err(|e| SyncEngineError::Crypto(e.to_string()))?,
            None if deleted_at.is_some() => String::new(),
            None => {
                return Err(SyncEngineError::Serialization(format!(
                    "active machine memory is missing encrypted content: {}",
                    memory.machine_key
                )));
            }
        };
        if deleted_at.is_some() {
            content.clear();
        } else if content.chars().count() > MAX_MEMORY_CHARS {
            return Err(SyncEngineError::Serialization(format!(
                "machine memory exceeds {MAX_MEMORY_CHARS} characters: {}",
                memory.machine_key
            )));
        }

        decrypted.push(MachineMemory {
            machine_key: memory.machine_key.clone(),
            content,
            hostname_alias: memory.hostname_alias.clone(),
            ssh_node_id: memory.ssh_node_id.clone(),
            last_review_at,
            updated_at,
            deleted_at,
        });
    }
    decrypted.sort_by(|a, b| a.machine_key.cmp(&b.machine_key));
    Ok(decrypted)
}

fn parse_memory_timestamp(
    column: &'static str,
    machine_key: &str,
    value: &str,
) -> Result<DateTime<Utc>, SyncEngineError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| {
            SyncEngineError::Serialization(format!(
                "invalid {column} for machine memory {machine_key}: {value}"
            ))
        })
}

fn merge_machine_memories(
    local: &[MachineMemory],
    remote: &[MachineMemory],
) -> (Vec<MachineMemory>, ApplyDataOutcome) {
    let local_by_key = local
        .iter()
        .cloned()
        .map(|memory| (memory.machine_key.clone(), memory))
        .collect::<BTreeMap<_, _>>();
    let remote_by_key = remote
        .iter()
        .cloned()
        .map(|memory| (memory.machine_key.clone(), memory))
        .collect::<BTreeMap<_, _>>();
    let mut merged = local_by_key.clone();
    for (machine_key, remote_memory) in &remote_by_key {
        match merged.get(machine_key) {
            Some(local_memory) => {
                if compare_machine_memories(remote_memory, local_memory).is_gt() {
                    merged.insert(machine_key.clone(), remote_memory.clone());
                }
            }
            None => {
                merged.insert(machine_key.clone(), remote_memory.clone());
            }
        }
    }

    let outcome = ApplyDataOutcome {
        local_changed: merged != local_by_key,
        needs_upload: merged != remote_by_key,
    };
    (merged.into_values().collect(), outcome)
}

fn compare_machine_memories(a: &MachineMemory, b: &MachineMemory) -> Ordering {
    a.updated_at.cmp(&b.updated_at).then_with(|| {
        a.deleted_at
            .is_some()
            .cmp(&b.deleted_at.is_some())
            .then_with(|| a.deleted_at.cmp(&b.deleted_at))
            .then_with(|| a.hostname_alias.cmp(&b.hostname_alias))
            .then_with(|| a.ssh_node_id.cmp(&b.ssh_node_id))
            .then_with(|| a.last_review_at.cmp(&b.last_review_at))
            .then_with(|| a.content.cmp(&b.content))
    })
}

/// apply_data Phase 1 已写入的 keychain 条目记录,带原值快照用于真正回滚。
/// `prior_value` 用 `Zeroizing<String>` 持有,保证回滚链上明文密码 drop 时被零化。
struct WrittenSecret {
    node_id: String,
    kind: SecretKind,
    prior_value: Option<Zeroizing<String>>,
}

/// 真正的"回滚":对每个已被覆盖的条目:
/// - prior_value=Some → 写回旧值,避免用户既有密码被吞
/// - prior_value=None → delete,避免 orphan
/// 任何步骤失败仅 log,不阻塞调用方(尽力而为)。
fn rollback_keychain_writes<S: SshSecretStore + ?Sized>(store: &S, written: &[WrittenSecret]) {
    for entry in written {
        let res = match &entry.prior_value {
            Some(v) => store.set(&entry.node_id, entry.kind, v.as_str()),
            None => store.delete(&entry.node_id, entry.kind),
        };
        if let Err(e) = res {
            log::warn!(
                "回滚 keychain 写入失败 {}/{:?}: {e}(secret 可能保持新值或成为 orphan)",
                entry.node_id,
                entry.kind
            );
        }
    }
}

/// 读取 keychain 凭据。
/// - `Ok(Some)` = 有密码,加密上传
/// - `Ok(None)` = 用户没设密码(合法状态),字段写 None
/// - `Err` = keychain 故障 (NoBackend / Locked / 权限拒绝)
///
/// 注意:对 NoBackend 不做 fallback。上层 keyring crate 把锁定的 keychain 和
/// 完全无 backend 都映射成 NoBackend,无法可靠区分(keyring 3.6 documented 行为)。
/// 把 NoBackend 当成 Ok(None) 会让"锁定" 这种瞬时故障静默丢密码 → 云端被清空,
/// 重装后无法恢复(KDF/格式仍是待优化项)。
/// headless Linux / CI 用户若全程无密码,upload 不会触发此函数;一旦遇到 Err,
/// 错误信息明确指引用户解锁/启用 keychain。
fn read_secret(
    store: &dyn SshSecretStore,
    node_id: &str,
    kind: SecretKind,
) -> Result<Option<String>, SyncEngineError> {
    match store.get(node_id, kind) {
        Ok(opt) => Ok(opt.map(|z| z.to_string())),
        Err(e) => Err(SyncEngineError::Provider(format!(
            "读取 keychain 失败 ({node_id}, {kind:?}): {e}。\
             keychain 可能被锁定或当前环境无 backend(headless Linux / WSL 等)。\
             请解锁 keychain 或启用 secret-service / Credential Manager 后重试上传。\
             若该服务器确实不需要密码同步,可在 SSH 管理器中清除该字段。"
        ))),
    }
}

fn encrypt_optional(token: &str, value: Option<&str>) -> Result<Option<String>, SyncEngineError> {
    match value {
        None => Ok(None),
        // 空字符串视为"无密码",不上传(与既往行为兼容,避免空字符串密文污染)
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => Ok(Some(
            crypto::encrypt(token, s).map_err(|e| SyncEngineError::Crypto(e.to_string()))?,
        )),
    }
}

fn default_onekey_kind() -> String {
    OneKeyCredentialKind::Password.as_db_str().to_string()
}

fn onekey_secret_kind(kind: OneKeyCredentialKind) -> SecretKind {
    match kind {
        OneKeyCredentialKind::Password => SecretKind::OneKeyPassword,
        OneKeyCredentialKind::Key => SecretKind::Passphrase,
    }
}

/// BFS 拓扑排序:父节点先于子节点。parent_id 引用数据集外节点的孤儿节点,
/// 视作根节点附加到末尾,parent_id 清空,避免 SQLite FK 约束失败让整个 download 回滚。
fn topologically_sort_nodes(nodes: &[SyncNode]) -> Vec<SyncNode> {
    use std::collections::HashMap;
    let mut by_parent: HashMap<Option<&str>, Vec<&SyncNode>> = HashMap::new();
    for n in nodes {
        by_parent.entry(n.parent_id.as_deref()).or_default().push(n);
    }

    let mut result: Vec<SyncNode> = Vec::with_capacity(nodes.len());
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<&SyncNode> = VecDeque::new();
    if let Some(roots) = by_parent.get(&None) {
        for r in roots {
            queue.push_back(*r);
        }
    }
    while let Some(node) = queue.pop_front() {
        if !seen.insert(node.id.clone()) {
            continue;
        }
        result.push(node.clone());
        if let Some(children) = by_parent.get(&Some(node.id.as_str())) {
            for c in children {
                queue.push_back(*c);
            }
        }
    }

    // 剩余节点要么是 orphan(parent_id 指向数据集外),要么属于一个循环。
    // 两种都把 parent_id 清空降级为根插入(可恢复且无数据丢失),并显式日志告警,
    // 让用户能在日志中看到数据被结构化重置。
    for n in nodes {
        if !seen.contains(&n.id) {
            if has_cycle_membership(n, nodes) {
                log::warn!(
                    "apply_data: 节点 {} 处于循环引用中(parent_id {:?}),已降级为根节点",
                    n.id,
                    n.parent_id
                );
            } else {
                log::warn!(
                    "apply_data: 节点 {} 的 parent_id {:?} 在数据集中不存在,作为根节点插入",
                    n.id,
                    n.parent_id
                );
            }
            let mut orphan = n.clone();
            orphan.parent_id = None;
            result.push(orphan);
        }
    }

    result
}

/// 判断节点 `start` 是否在循环中(从它出发沿 parent_id 链最终回到自身或环上)。
/// 用于区分日志中的 "orphan" vs "cycle";限制最大遍历步数防止指数复杂度。
fn has_cycle_membership(start: &SyncNode, all: &[SyncNode]) -> bool {
    let by_id: std::collections::HashMap<&str, &SyncNode> =
        all.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut current = start;
    let mut visited: HashSet<&str> = HashSet::new();
    let max_steps = all.len() + 1;
    for _ in 0..max_steps {
        let Some(pid) = current.parent_id.as_deref() else {
            return false;
        };
        if !visited.insert(current.id.as_str()) {
            // 走过同一节点 → 循环
            return true;
        }
        match by_id.get(pid) {
            Some(parent) => current = parent,
            None => return false, // parent 在数据集外 → orphan,不是 cycle
        }
    }
    // 超过 max_steps 还没结束 → 一定有环
    true
}

/// 数据库同步版本存储适配器
pub struct DbVersionStore;

impl SyncVersionStore for DbVersionStore {
    fn get_sync_version(&self) -> Result<i64, SyncEngineError> {
        with_conn(|c| Ok(SyncMetaRepository::get_sync_version(c)?))
            .map_err(|e| SyncEngineError::VersionStore(e.to_string()))
    }

    fn set_sync_version(&self, version: i64) -> Result<(), SyncEngineError> {
        with_conn(|c| Ok(SyncMetaRepository::set_sync_version(c, version)?))
            .map_err(|e| SyncEngineError::VersionStore(e.to_string()))
    }

    fn commit_sync_version(
        &self,
        expected_version: i64,
        synced_version: i64,
    ) -> Result<i64, SyncEngineError> {
        with_conn(|c| {
            Ok(SyncMetaRepository::commit_sync_version(
                c,
                expected_version,
                synced_version,
            )?)
        })
        .map_err(|e| SyncEngineError::VersionStore(e.to_string()))
    }

    fn update_sync_meta(&self, time: &str, platform: &str) -> Result<(), SyncEngineError> {
        with_conn(|c| Ok(SyncMetaRepository::update_sync_meta(c, time, platform)?))
            .map_err(|e| SyncEngineError::VersionStore(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_key() {
        let provider = SshSyncProvider::new();
        assert_eq!(provider.section_key(), "ssh");
    }

    #[test]
    fn test_sync_node_serialization_roundtrip() {
        let node = SyncNode {
            id: "n1".to_string(),
            parent_id: Some("p1".to_string()),
            kind: "folder".to_string(),
            name: "Prod".to_string(),
            sort_order: 0,
            is_collapsed: true,
        };
        let json = serde_json::to_string(&node).unwrap();
        let parsed: SyncNode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "n1");
        assert_eq!(parsed.parent_id, Some("p1".to_string()));
        assert_eq!(parsed.kind, "folder");
        assert_eq!(parsed.name, "Prod");
        assert_eq!(parsed.sort_order, 0);
        assert!(parsed.is_collapsed);
    }

    #[test]
    fn test_sync_server_serialization_with_secrets() {
        let server = SyncServer {
            node_id: "s1".to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_type: "password".to_string(),
            key_path: Some("/key".to_string()),
            startup_command: None,
            notes: Some("test".to_string()),
            credential_id: None,
            password_encrypted: Some("enc123".to_string()),
            passphrase_encrypted: None,
            root_password_encrypted: Some("enc456".to_string()),
        };
        let json = serde_json::to_string(&server).unwrap();
        let parsed: SyncServer = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.node_id, "s1");
        assert_eq!(parsed.port, 22);
        assert_eq!(parsed.password_encrypted, Some("enc123".to_string()));
        assert_eq!(parsed.passphrase_encrypted, None);
        assert_eq!(parsed.root_password_encrypted, Some("enc456".to_string()));
    }

    #[test]
    fn test_sync_server_no_secrets() {
        let server = SyncServer {
            node_id: "s2".to_string(),
            host: "host".to_string(),
            port: 2222,
            username: "admin".to_string(),
            auth_type: "key".to_string(),
            key_path: None,
            startup_command: None,
            notes: None,
            credential_id: None,
            password_encrypted: None,
            passphrase_encrypted: None,
            root_password_encrypted: None,
        };
        let json = serde_json::to_string(&server).unwrap();
        let parsed: SyncServer = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.password_encrypted, None);
        assert_eq!(parsed.passphrase_encrypted, None);
        assert_eq!(parsed.root_password_encrypted, None);
    }

    #[test]
    fn test_ssh_sync_data_roundtrip() {
        let data = SshSyncData {
            nodes: vec![SyncNode {
                id: "n1".to_string(),
                parent_id: None,
                kind: "folder".to_string(),
                name: "Root".to_string(),
                sort_order: 0,
                is_collapsed: false,
            }],
            servers: vec![SyncServer {
                node_id: "s1".to_string(),
                host: "h".to_string(),
                port: 22,
                username: "u".to_string(),
                auth_type: "password".to_string(),
                key_path: None,
                startup_command: None,
                notes: None,
                credential_id: None,
                password_encrypted: Some("enc".to_string()),
                passphrase_encrypted: None,
                root_password_encrypted: None,
            }],
            onekey_credentials: Vec::new(),
            machine_memories: Vec::new(),
            routes: Some(Vec::new()),
        };
        let json = serde_json::to_string(&data).unwrap();
        let parsed: SshSyncData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.nodes.len(), 1);
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.nodes[0].id, "n1");
        assert_eq!(
            parsed.servers[0].password_encrypted,
            Some("enc".to_string())
        );
        assert_eq!(parsed.routes, Some(Vec::new()));
    }

    #[test]
    fn test_ssh_sync_data_deserializes_legacy_payload_without_onekey_fields() {
        let json = r#"{
            "nodes": [
                {
                    "id": "s1",
                    "parent_id": null,
                    "kind": "server",
                    "name": "legacy",
                    "sort_order": 0,
                    "is_collapsed": false
                }
            ],
            "servers": [
                {
                    "node_id": "s1",
                    "host": "example.com",
                    "port": 22,
                    "username": "root",
                    "auth_type": "password",
                    "key_path": null,
                    "startup_command": null,
                    "notes": null,
                    "password_encrypted": null,
                    "passphrase_encrypted": null,
                    "root_password_encrypted": null
                }
            ]
        }"#;

        let parsed: SshSyncData = serde_json::from_str(json).unwrap();

        assert!(parsed.onekey_credentials.is_empty());
        assert!(parsed.machine_memories.is_empty());
        assert!(parsed.routes.is_none());
        assert_eq!(parsed.servers[0].credential_id, None);
    }

    #[test]
    fn test_onekey_credential_serialization_roundtrip() {
        let data = SshSyncData {
            nodes: Vec::new(),
            servers: Vec::new(),
            onekey_credentials: vec![SyncOneKeyCredential {
                id: "cred-1".to_string(),
                label: "prod-root".to_string(),
                username: "root".to_string(),
                kind: "key".to_string(),
                key_path: Some("/home/root/.ssh/id_ed25519".to_string()),
                password_encrypted: Some("enc".to_string()),
            }],
            machine_memories: Vec::new(),
            routes: Some(Vec::new()),
        };

        let json = serde_json::to_string(&data).unwrap();
        let parsed: SshSyncData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.onekey_credentials.len(), 1);
        assert_eq!(parsed.onekey_credentials[0].id, "cred-1");
        assert_eq!(parsed.onekey_credentials[0].label, "prod-root");
        assert_eq!(parsed.onekey_credentials[0].username, "root");
        assert_eq!(parsed.onekey_credentials[0].kind, "key");
        assert_eq!(
            parsed.onekey_credentials[0].key_path.as_deref(),
            Some("/home/root/.ssh/id_ed25519")
        );
        assert_eq!(
            parsed.onekey_credentials[0].password_encrypted,
            Some("enc".to_string())
        );
    }

    #[test]
    fn test_onekey_credential_deserializes_legacy_payload_as_password() {
        let json = r#"{
            "id": "cred-1",
            "label": "prod-root",
            "username": "root",
            "password_encrypted": null
        }"#;

        let parsed: SyncOneKeyCredential = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.kind, "password");
        assert_eq!(parsed.key_path, None);
    }

    #[test]
    fn test_onekey_key_credentials_use_passphrase_secret_slot() {
        assert_eq!(
            onekey_secret_kind(OneKeyCredentialKind::Password),
            SecretKind::OneKeyPassword
        );
        assert_eq!(
            onekey_secret_kind(OneKeyCredentialKind::Key),
            SecretKind::Passphrase
        );
    }

    #[test]
    fn test_ssh_sync_data_default_empty() {
        let data = SshSyncData::default();
        assert!(data.nodes.is_empty());
        assert!(data.servers.is_empty());
        assert!(data.machine_memories.is_empty());
    }

    #[test]
    fn test_sync_node_null_parent() {
        let node = SyncNode {
            id: "root".to_string(),
            parent_id: None,
            kind: "folder".to_string(),
            name: "R".to_string(),
            sort_order: 0,
            is_collapsed: false,
        };
        let json = serde_json::to_string(&node).unwrap();
        assert!(
            json.contains("\"parent_id\":null"),
            "parent_id=None 应序列化为 null"
        );
        let parsed: SyncNode = serde_json::from_str(&json).unwrap();
        assert!(parsed.parent_id.is_none());
    }

    fn memory(machine_key: &str, content: &str, updated_at: i64, deleted: bool) -> MachineMemory {
        let updated_at = DateTime::<Utc>::from_timestamp(updated_at, 0).unwrap();
        MachineMemory {
            machine_key: machine_key.to_string(),
            content: if deleted {
                String::new()
            } else {
                content.to_string()
            },
            hostname_alias: None,
            ssh_node_id: None,
            last_review_at: None,
            updated_at,
            deleted_at: deleted.then_some(updated_at),
        }
    }

    #[test]
    fn machine_memory_content_is_encrypted_per_row_and_round_trips() {
        let memories = vec![
            memory("web-01:22", "plaintext-memory-marker", 10, false),
            memory("deleted:22", "", 11, true),
        ];

        let encrypted = encrypt_machine_memories("token", &memories).unwrap();
        let json = serde_json::to_string(&encrypted).unwrap();
        assert!(!json.contains("plaintext-memory-marker"));
        assert!(encrypted[0].content_encrypted.is_none());
        assert!(encrypted[1].content_encrypted.is_some());

        let mut expected = memories;
        expected.sort_by(|a, b| a.machine_key.cmp(&b.machine_key));
        assert_eq!(
            decrypt_machine_memories("token", &encrypted).unwrap(),
            expected
        );
    }

    #[test]
    fn invalid_or_missing_active_memory_ciphertext_is_rejected_before_apply() {
        let mut data = SshSyncData::default();
        data.machine_memories.push(SyncMachineMemory {
            machine_key: "web-01:22".to_string(),
            content_encrypted: Some("not-valid-ciphertext".to_string()),
            hostname_alias: None,
            ssh_node_id: None,
            last_review_at: None,
            updated_at: "2026-07-26T12:00:00Z".to_string(),
            deleted_at: None,
        });
        let error = SshSyncProvider::new()
            .apply_data("token", &serde_json::to_value(&data).unwrap())
            .unwrap_err();
        assert!(matches!(error, SyncEngineError::Crypto(_)));

        data.machine_memories[0].content_encrypted = None;
        let error = decrypt_machine_memories("token", &data.machine_memories).unwrap_err();
        assert!(matches!(error, SyncEngineError::Serialization(_)));
    }

    #[test]
    fn duplicate_machine_keys_are_rejected() {
        let encrypted = crypto::encrypt("token", "memory").unwrap();
        let memory = SyncMachineMemory {
            machine_key: "web-01:22".to_string(),
            content_encrypted: Some(encrypted),
            hostname_alias: None,
            ssh_node_id: None,
            last_review_at: None,
            updated_at: "2026-07-26T12:00:00Z".to_string(),
            deleted_at: None,
        };

        let error = decrypt_machine_memories("token", &[memory.clone(), memory]).unwrap_err();
        assert!(matches!(error, SyncEngineError::Serialization(_)));
    }

    #[test]
    fn memory_merge_unions_both_sides_and_uses_newer_timestamp() {
        let local = vec![
            memory("local-only:22", "local", 1, false),
            memory("shared:22", "old", 2, false),
        ];
        let remote = vec![
            memory("remote-only:22", "remote", 1, false),
            memory("shared:22", "new", 3, false),
        ];

        let (merged, outcome) = merge_machine_memories(&local, &remote);
        assert_eq!(
            merged
                .iter()
                .map(|memory| (memory.machine_key.as_str(), memory.content.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("local-only:22", "local"),
                ("remote-only:22", "remote"),
                ("shared:22", "new"),
            ]
        );
        assert_eq!(
            outcome,
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: true,
            }
        );
    }

    #[test]
    fn memory_merge_keeps_newer_local_row_for_upload() {
        let local = vec![memory("shared:22", "new local", 3, false)];
        let remote = vec![memory("shared:22", "old remote", 2, false)];

        let (merged, outcome) = merge_machine_memories(&local, &remote);
        assert_eq!(merged, local);
        assert_eq!(
            outcome,
            ApplyDataOutcome {
                local_changed: false,
                needs_upload: true,
            }
        );
    }

    #[test]
    fn equal_timestamp_conflicts_are_deterministic_and_tombstone_wins() {
        let alpha = memory("shared:22", "alpha", 3, false);
        let zeta = memory("shared:22", "zeta", 3, false);
        let tombstone = memory("shared:22", "", 3, true);

        let (first, _) = merge_machine_memories(&[alpha.clone()], &[zeta.clone()]);
        let (second, _) = merge_machine_memories(&[zeta], &[alpha]);
        assert_eq!(first, second);
        assert_eq!(first[0].content, "zeta");

        let (merged, _) = merge_machine_memories(&first, &[tombstone.clone()]);
        assert_eq!(merged, vec![tombstone]);
    }

    #[test]
    fn newer_active_memory_explicitly_revives_older_tombstone() {
        let tombstone = memory("shared:22", "", 2, true);
        let active = memory("shared:22", "restored", 3, false);

        let (merged, outcome) = merge_machine_memories(&[tombstone], &[active.clone()]);
        assert_eq!(merged, vec![active]);
        assert!(outcome.local_changed);
        assert!(!outcome.needs_upload);
    }

    #[test]
    fn merge_and_persist_keeps_newer_local_write_over_older_remote() {
        let mut conn = crate::repository::setup_in_memory();
        // Agent 已写入较新内容(updated_at = now,同时 bump sync_version)
        MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", "Agent 新内容").unwrap();

        // 远端持有较旧内容;快照在写回事务内读取,归并必须以当前 DB 状态为准
        let remote = vec![memory("web-01:22", "远端旧内容", 1, false)];
        let outcome = merge_and_persist_memories(&mut conn, &remote).unwrap();

        let stored = MachineMemoryRepository::get(&mut conn, "web-01:22")
            .unwrap()
            .unwrap();
        assert_eq!(stored.content, "Agent 新内容");
        assert_eq!(
            outcome,
            ApplyDataOutcome {
                local_changed: false,
                needs_upload: true,
            }
        );
    }

    #[test]
    fn merge_and_persist_applies_newer_remote_without_bumping_sync_version() {
        let mut conn = crate::repository::setup_in_memory();
        let remote = vec![memory("web-01:22", "远端内容", 100, false)];

        let outcome = merge_and_persist_memories(&mut conn, &remote).unwrap();

        let stored = MachineMemoryRepository::get(&mut conn, "web-01:22")
            .unwrap()
            .unwrap();
        assert_eq!(stored.content, "远端内容");
        assert_eq!(
            outcome,
            ApplyDataOutcome {
                local_changed: true,
                needs_upload: false,
            }
        );
        assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 0);
    }

    /// 归并期间注入本地写的 mock 凭据存储:apply_data 阶段 1 首次读取旧值时,
    /// 通过 with_conn 模拟 Agent 的 update_machine_memory 写入。该时点位于
    /// 旧实现"读快照 → 写回"的竞态窗口内(旧实现在阶段 0 之前就读了快照),
    /// 用于回归验证快照与写回的原子性。
    #[derive(Default)]
    struct InjectingSecretStore {
        injected: std::sync::atomic::AtomicBool,
    }

    impl SshSecretStore for InjectingSecretStore {
        fn set(
            &self,
            _node_id: &str,
            _kind: SecretKind,
            _secret: &str,
        ) -> Result<(), crate::secrets::SshSecretStoreError> {
            Ok(())
        }

        fn get(
            &self,
            _node_id: &str,
            _kind: SecretKind,
        ) -> Result<Option<Zeroizing<String>>, crate::secrets::SshSecretStoreError> {
            if !self
                .injected
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                with_conn(|conn| {
                    MachineMemoryRepository::upsert_content(
                        conn,
                        "web-01:22",
                        "Agent 归并期间写入",
                    )?;
                    Ok(())
                })
                .unwrap();
            }
            Ok(None)
        }

        fn delete(
            &self,
            _node_id: &str,
            _kind: SecretKind,
        ) -> Result<(), crate::secrets::SshSecretStoreError> {
            Ok(())
        }
    }

    /// 回归测试:同步归并期间落地的本地写不能被旧快照的归并结果覆盖。
    /// 使用真实的 db::with_conn 全局连接(nextest 每个测试独立进程,
    /// OnceLock 路径不会跨测试污染)。
    #[test]
    fn apply_data_preserves_local_write_that_lands_during_keychain_phase() {
        let dir = tempfile::tempdir().unwrap();
        crate::db::set_database_path(dir.path().join("ssh.sqlite3"));
        with_conn(|conn| {
            crate::repository::run_test_migrations(conn);
            Ok(())
        })
        .unwrap();

        let token = "token";
        let payload = SshSyncData {
            nodes: vec![SyncNode {
                id: "srv-1".to_string(),
                parent_id: None,
                kind: "server".to_string(),
                name: "web".to_string(),
                sort_order: 0,
                is_collapsed: false,
            }],
            // 带密码的服务器,确保阶段 1 会调用 secret_store.get 触发注入
            servers: vec![SyncServer {
                node_id: "srv-1".to_string(),
                host: "web-01".to_string(),
                port: 22,
                username: "root".to_string(),
                auth_type: "password".to_string(),
                key_path: None,
                startup_command: None,
                notes: None,
                credential_id: None,
                password_encrypted: Some(crypto::encrypt(token, "pw").unwrap()),
                passphrase_encrypted: None,
                root_password_encrypted: None,
            }],
            onekey_credentials: Vec::new(),
            machine_memories: vec![SyncMachineMemory {
                machine_key: "web-01:22".to_string(),
                content_encrypted: Some(crypto::encrypt(token, "远端旧内容").unwrap()),
                hostname_alias: None,
                ssh_node_id: None,
                last_review_at: None,
                updated_at: "2020-01-01T00:00:00Z".to_string(),
                deleted_at: None,
            }],
            routes: Some(Vec::new()),
        };

        let provider =
            SshSyncProvider::with_secret_store(Box::new(InjectingSecretStore::default()));
        let outcome = provider
            .apply_data(token, &serde_json::to_value(&payload).unwrap())
            .unwrap();

        // Agent 注入的写 updated_at 更新,LWW 归并必须让它胜出并回传远端;
        // 旧实现会用注入前的空快照归并出远端旧内容并覆盖之。
        let stored = with_conn(|conn| Ok(MachineMemoryRepository::get(conn, "web-01:22")?))
            .unwrap()
            .unwrap();
        assert_eq!(stored.content, "Agent 归并期间写入");
        assert!(outcome.needs_upload);
        assert!(!outcome.local_changed);
    }
}
