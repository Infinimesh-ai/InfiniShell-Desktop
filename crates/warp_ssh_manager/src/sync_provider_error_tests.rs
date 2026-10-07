use std::error::Error;
use std::sync::{Arc, Mutex};

use super::*;

#[derive(Clone)]
struct FailingSecretStore {
    values: Arc<Mutex<Vec<(String, SecretKind, String)>>>,
    fail_read: Option<String>,
    fail_write: Option<String>,
}

impl FailingSecretStore {
    fn new(fail_read: Option<&str>, fail_write: Option<&str>) -> Self {
        Self {
            values: Arc::new(Mutex::new(vec![
                (
                    "srv-1".to_string(),
                    SecretKind::Password,
                    "old-1".to_string(),
                ),
                (
                    "srv-2".to_string(),
                    SecretKind::Password,
                    "old-2".to_string(),
                ),
            ])),
            fail_read: fail_read.map(str::to_string),
            fail_write: fail_write.map(str::to_string),
        }
    }

    fn password(&self, node_id: &str) -> String {
        self.values
            .lock()
            .unwrap()
            .iter()
            .find(|(owner, kind, _)| owner == node_id && *kind == SecretKind::Password)
            .unwrap()
            .2
            .clone()
    }
}

impl SshSecretStore for FailingSecretStore {
    fn set(
        &self,
        node_id: &str,
        kind: SecretKind,
        secret: &str,
    ) -> Result<(), SshSecretStoreError> {
        if self.fail_write.as_deref() == Some(node_id) {
            return Err(SshSecretStoreError::NoBackend);
        }
        let mut values = self.values.lock().unwrap();
        if let Some((_, _, value)) = values
            .iter_mut()
            .find(|(owner, slot, _)| owner == node_id && *slot == kind)
        {
            *value = secret.to_string();
        } else {
            values.push((node_id.to_string(), kind, secret.to_string()));
        }
        Ok(())
    }

    fn get(
        &self,
        node_id: &str,
        kind: SecretKind,
    ) -> Result<Option<Zeroizing<String>>, SshSecretStoreError> {
        if self.fail_read.as_deref() == Some(node_id) {
            return Err(SshSecretStoreError::NoBackend);
        }
        Ok(self
            .values
            .lock()
            .unwrap()
            .iter()
            .find(|(owner, slot, _)| owner == node_id && *slot == kind)
            .map(|(_, _, value)| Zeroizing::new(value.clone())))
    }

    fn delete(&self, node_id: &str, kind: SecretKind) -> Result<(), SshSecretStoreError> {
        self.values
            .lock()
            .unwrap()
            .retain(|(owner, slot, _)| owner != node_id || *slot != kind);
        Ok(())
    }
}

fn setup_database() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    crate::db::set_database_path(dir.path().join("ssh.sqlite3"));
    with_conn(|conn| {
        crate::repository::run_test_migrations(conn);
        Ok(())
    })
    .unwrap();
    dir
}

fn downloaded_data(invalid_kind: bool) -> serde_json::Value {
    let mut data = SshSyncData::default();
    for (node_id, password) in [("srv-1", "new-1"), ("srv-2", "new-2")] {
        data.nodes.push(SyncNode {
            id: node_id.to_string(),
            parent_id: None,
            kind: if invalid_kind { "invalid" } else { "server" }.to_string(),
            name: node_id.to_string(),
            sort_order: 0,
            is_collapsed: false,
        });
        data.servers.push(SyncServer {
            node_id: node_id.to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_type: "password".to_string(),
            key_path: None,
            startup_command: None,
            notes: None,
            credential_id: None,
            password_encrypted: Some(crypto::encrypt("token", password).unwrap()),
            passphrase_encrypted: None,
            root_password_encrypted: None,
        });
    }
    serde_json::to_value(data).unwrap()
}

#[test]
fn read_secret_keeps_operation_and_original_source() {
    let store = FailingSecretStore::new(Some("srv-1"), None);
    let error = read_secret(&store, "srv-1", SecretKind::Password).unwrap_err();
    let SyncEngineError::Provider(error) = error else {
        panic!("读取凭据错误必须保留 provider 类型");
    };
    let error = error.downcast_ref::<SshSyncProviderError>().unwrap();
    let SshSyncProviderError::ReadSecret {
        node_id,
        kind,
        source,
    } = error
    else {
        panic!("读取凭据错误不能退化为字符串");
    };
    assert_eq!(node_id, "srv-1");
    assert_eq!(*kind, SecretKind::Password);
    assert!(matches!(source, SshSecretStoreError::NoBackend));
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<SshSecretStoreError>()
            .is_some()
    );
}

#[test]
fn failed_previous_secret_read_retains_type_and_rolls_back_changes() {
    let _dir = setup_database();
    let store = FailingSecretStore::new(Some("srv-2"), None);
    let provider = SshSyncProvider::with_secret_store(Box::new(store.clone()));
    let error = provider
        .apply_data("token", &downloaded_data(false))
        .unwrap_err();
    let SyncEngineError::Provider(error) = error else {
        panic!("读取旧凭据失败必须保留 provider 类型");
    };
    let Some(SshSyncProviderError::ReadPreviousSecret {
        node_id,
        rolled_back,
        source,
        ..
    }) = error.downcast_ref()
    else {
        panic!("读取旧凭据失败必须保留操作与回滚数");
    };
    assert_eq!(node_id, "srv-2");
    assert_eq!(*rolled_back, 1);
    assert!(matches!(source, SshSecretStoreError::NoBackend));
    assert_eq!(store.password("srv-1"), "old-1");
    assert!(
        with_conn(|conn| Ok(SshRepository::list_nodes(conn)?))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn failed_secret_write_retains_type_and_rolls_back_changes() {
    let _dir = setup_database();
    let store = FailingSecretStore::new(None, Some("srv-2"));
    let provider = SshSyncProvider::with_secret_store(Box::new(store.clone()));
    let error = provider
        .apply_data("token", &downloaded_data(false))
        .unwrap_err();
    let SyncEngineError::Provider(error) = error else {
        panic!("写凭据失败必须保留 provider 类型");
    };
    let Some(SshSyncProviderError::WriteSecret {
        node_id, source, ..
    }) = error.downcast_ref()
    else {
        panic!("写凭据失败必须保留操作与原始来源");
    };
    assert_eq!(node_id, "srv-2");
    assert!(matches!(source, SshSecretStoreError::NoBackend));
    assert_eq!(store.password("srv-1"), "old-1");
    assert_eq!(store.password("srv-2"), "old-2");
    assert!(
        with_conn(|conn| Ok(SshRepository::list_nodes(conn)?))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn failed_database_write_retains_rollback_count_and_restores_credentials() {
    let _dir = setup_database();
    let store = FailingSecretStore::new(None, None);
    let provider = SshSyncProvider::with_secret_store(Box::new(store.clone()));
    let error = provider
        .apply_data("token", &downloaded_data(true))
        .unwrap_err();
    let SyncEngineError::Provider(error) = error else {
        panic!("数据库写入失败必须保留 provider 类型");
    };
    let Some(SshSyncProviderError::WriteDatabase {
        rolled_back,
        source,
    }) = error.downcast_ref()
    else {
        panic!("数据库写入失败必须保留回滚数与诊断来源");
    };
    assert_eq!(*rolled_back, 2);
    assert!(!source.to_string().is_empty());
    assert_eq!(store.password("srv-1"), "old-1");
    assert_eq!(store.password("srv-2"), "old-2");
    assert!(
        with_conn(|conn| Ok(SshRepository::list_nodes(conn)?))
            .unwrap()
            .is_empty()
    );
}
