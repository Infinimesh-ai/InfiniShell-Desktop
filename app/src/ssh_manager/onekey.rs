//! OneKey 凭据加载:从 SSH Manager 持久化层 + Keychain/DPAPI/Linux Keyring
//! 读出所有已保存的 server 凭据,供 `TerminalView` 在检测到 PTY 密码提示时
//! 弹出选择菜单。
//!
//! ## 注意
//!
//! - 内部调用 `warp_ssh_manager::with_conn`(同步 Mutex + SQLite)和
//!   `KeychainSecretStore::get`(同步 OS API),**不可以**在 UI 主线程直接
//!   同步调用——server 一多就会卡顿。调用方需走 `tokio::task::spawn_blocking`。
//! - secret 全程用 `Zeroizing<String>` 持有,丢弃时自动清零。

use anyhow::Result;
use warp_ssh_manager::{
    AuthType, KeychainSecretStore, NodeKind, OneKeyCredentialKind as StoredOneKeyCredentialKind,
    ResolvedSshAuth, SecretKind, SshRepository, SshSecretStore, SshServerInfo,
};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneKeyCredentialKind {
    Password,
    Passphrase,
}

pub struct OneKeyCredential {
    pub label: String,
    pub subtitle: String,
    pub secret: Zeroizing<String>,
    pub kind: OneKeyCredentialKind,
}

pub fn load_saved_ssh_credentials() -> Result<Vec<OneKeyCredential>> {
    let store = KeychainSecretStore;
    load_saved_ssh_credentials_with_store(&store)
}

#[derive(Clone, Debug)]
struct SavedSshPasswordLookup {
    host: String,
    port: u16,
    username: String,
    secret_lookup_id: String,
    secret_kind: SecretKind,
}

impl SavedSshPasswordLookup {
    fn from_resolved_auth(server: &SshServerInfo, auth: ResolvedSshAuth) -> Option<Self> {
        if auth.auth_type != AuthType::Password {
            return None;
        }
        Some(Self {
            host: server.host.clone(),
            port: server.port,
            username: auth.username,
            secret_lookup_id: auth.secret_lookup_id,
            secret_kind: auth.secret_kind,
        })
    }
}

pub fn load_unique_matching_ssh_password(
    username: &str,
    host: &str,
    port: u16,
) -> Result<Option<Zeroizing<String>>> {
    let store = KeychainSecretStore;
    let lookups = load_saved_ssh_password_lookups()?;
    load_unique_matching_ssh_password_from_lookups(&lookups, username, host, port, &store)
}

fn load_saved_ssh_password_lookups() -> Result<Vec<SavedSshPasswordLookup>> {
    warp_ssh_manager::with_conn(|conn| {
        let nodes = SshRepository::list_nodes(conn)?;
        let mut lookups = Vec::new();
        for node in nodes {
            if node.kind != NodeKind::Server {
                continue;
            }
            let Some(server) = SshRepository::get_server(conn, &node.id)? else {
                continue;
            };
            let Ok(auth) = SshRepository::resolve_server_auth(conn, &server) else {
                continue;
            };
            if let Some(lookup) = SavedSshPasswordLookup::from_resolved_auth(&server, auth) {
                lookups.push(lookup);
            }
        }
        Ok(lookups)
    })
}

fn load_unique_matching_ssh_password_from_lookups(
    lookups: &[SavedSshPasswordLookup],
    username: &str,
    host: &str,
    port: u16,
    store: &dyn SshSecretStore,
) -> Result<Option<Zeroizing<String>>> {
    let mut matches = lookups.iter().filter(|lookup| {
        lookup.username == username && lookup.host.eq_ignore_ascii_case(host) && lookup.port == port
    });
    let Some(lookup) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Ok(None);
    }
    Ok(store
        .get(&lookup.secret_lookup_id, lookup.secret_kind)?
        .filter(|secret| !secret.is_empty()))
}

fn load_saved_ssh_credentials_with_store(
    store: &dyn SshSecretStore,
) -> Result<Vec<OneKeyCredential>> {
    warp_ssh_manager::with_conn(|conn| {
        let nodes = SshRepository::list_nodes(conn)?;
        let mut credentials = Vec::new();

        for credential in SshRepository::list_onekey_credentials(conn)? {
            let (secret_kind, kind) = match credential.kind {
                StoredOneKeyCredentialKind::Password => {
                    (SecretKind::OneKeyPassword, OneKeyCredentialKind::Password)
                }
                StoredOneKeyCredentialKind::Key => {
                    (SecretKind::Passphrase, OneKeyCredentialKind::Passphrase)
                }
            };
            let secret = match store.get(&credential.id, secret_kind) {
                Ok(Some(secret)) if !secret.is_empty() => secret,
                Ok(Some(_)) | Ok(None) => continue,
                Err(e) => {
                    log::warn!("onekey: failed to read shared ssh credential: {e}");
                    continue;
                }
            };
            let subtitle = match credential.kind {
                StoredOneKeyCredentialKind::Password => credential.username,
                StoredOneKeyCredentialKind::Key => {
                    let key_path = credential.key_path.as_deref().unwrap_or("key");
                    if credential.username.is_empty() {
                        key_path.to_string()
                    } else {
                        format!("{key_path} for {}", credential.username)
                    }
                }
            };
            credentials.push(OneKeyCredential {
                label: credential.label,
                subtitle,
                secret,
                kind,
            });
        }

        for node in nodes {
            if node.kind != NodeKind::Server {
                continue;
            }
            let Some(server) = SshRepository::get_server(conn, &node.id)? else {
                continue;
            };
            let kind = match server.auth_type {
                AuthType::Password => SecretKind::Password,
                AuthType::Key => SecretKind::Passphrase,
                AuthType::OneKey => continue,
            };
            let secret = match store.get(&node.id, kind) {
                Ok(Some(secret)) if !secret.is_empty() => secret,
                Ok(Some(_)) | Ok(None) => continue,
                Err(e) => {
                    log::warn!("onekey: failed to read saved ssh credential: {e}");
                    continue;
                }
            };
            let target = if server.username.is_empty() {
                format!("{}:{}", server.host, server.port)
            } else {
                format!("{}@{}:{}", server.username, server.host, server.port)
            };
            // kind 由 auth_type 推出,只能是 Password / Passphrase 两者,RootPassword
            // 不在 OneKey 本身的范围内(走独立的 su 弹窗确认流程)。
            let (subtitle, kind) = match server.auth_type {
                AuthType::Password => (target, OneKeyCredentialKind::Password),
                AuthType::Key => {
                    let key_path = server.key_path.as_deref().unwrap_or("key");
                    (
                        format!("{key_path} for {target}"),
                        OneKeyCredentialKind::Passphrase,
                    )
                }
                AuthType::OneKey => continue,
            };
            credentials.push(OneKeyCredential {
                label: node.name,
                subtitle,
                secret,
                kind,
            });
        }

        Ok(credentials)
    })
}

#[cfg(test)]
#[path = "onekey_tests.rs"]
mod tests;
