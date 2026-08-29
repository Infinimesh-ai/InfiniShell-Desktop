use std::collections::HashMap;
use std::sync::Mutex;

use warp_ssh_manager::{SecretKind, SshSecretStore, SshSecretStoreError};
use zeroize::Zeroizing;

use super::*;

#[derive(Default)]
struct MockSecretStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl MockSecretStore {
    fn insert(&self, id: &str, kind: SecretKind, secret: &str) {
        self.secrets
            .lock()
            .unwrap()
            .insert(secret_key(id, kind), secret.to_string());
    }
}

fn secret_key(id: &str, kind: SecretKind) -> String {
    let kind = match kind {
        SecretKind::Password => "password",
        SecretKind::Passphrase => "passphrase",
        SecretKind::RootPassword => "root_password",
        SecretKind::OneKeyPassword => "onekey_password",
    };
    format!("{id}:{kind}")
}

impl SshSecretStore for MockSecretStore {
    fn set(
        &self,
        node_id: &str,
        kind: SecretKind,
        secret: &str,
    ) -> Result<(), SshSecretStoreError> {
        self.insert(node_id, kind, secret);
        Ok(())
    }

    fn get(
        &self,
        node_id: &str,
        kind: SecretKind,
    ) -> Result<Option<Zeroizing<String>>, SshSecretStoreError> {
        Ok(self
            .secrets
            .lock()
            .unwrap()
            .get(&secret_key(node_id, kind))
            .cloned()
            .map(Zeroizing::new))
    }

    fn delete(&self, node_id: &str, kind: SecretKind) -> Result<(), SshSecretStoreError> {
        self.secrets
            .lock()
            .unwrap()
            .remove(&secret_key(node_id, kind));
        Ok(())
    }
}

fn lookup(
    id: &str,
    username: &str,
    host: &str,
    port: u16,
    kind: SecretKind,
) -> SavedSshPasswordLookup {
    SavedSshPasswordLookup {
        host: host.to_string(),
        port,
        username: username.to_string(),
        secret_lookup_id: id.to_string(),
        secret_kind: kind,
    }
}

#[test]
fn unique_destination_returns_saved_password() {
    let store = MockSecretStore::default();
    store.insert("server-1", SecretKind::Password, "secret");
    let lookups = vec![lookup(
        "server-1",
        "root",
        "192.0.2.10",
        22,
        SecretKind::Password,
    )];

    let password =
        load_unique_matching_ssh_password_from_lookups(&lookups, "root", "192.0.2.10", 22, &store)
            .unwrap();

    assert_eq!(
        password.as_ref().map(|secret| secret.as_str()),
        Some("secret")
    );
}

#[test]
fn matching_requires_exact_username() {
    let store = MockSecretStore::default();
    store.insert("server-1", SecretKind::Password, "secret");
    let lookups = vec![lookup(
        "server-1",
        "admin",
        "192.0.2.10",
        22,
        SecretKind::Password,
    )];

    let password =
        load_unique_matching_ssh_password_from_lookups(&lookups, "root", "192.0.2.10", 22, &store)
            .unwrap();

    assert!(password.is_none());
}

#[test]
fn matching_requires_exact_port() {
    let store = MockSecretStore::default();
    store.insert("server-1", SecretKind::Password, "secret");
    let lookups = vec![lookup(
        "server-1",
        "root",
        "192.0.2.10",
        2222,
        SecretKind::Password,
    )];

    let password =
        load_unique_matching_ssh_password_from_lookups(&lookups, "root", "192.0.2.10", 22, &store)
            .unwrap();

    assert!(password.is_none());
}

#[test]
fn duplicate_destinations_do_not_auto_fill() {
    let store = MockSecretStore::default();
    store.insert("server-1", SecretKind::Password, "first");
    store.insert("server-2", SecretKind::Password, "second");
    let lookups = vec![
        lookup("server-1", "root", "192.0.2.10", 22, SecretKind::Password),
        lookup("server-2", "root", "192.0.2.10", 22, SecretKind::Password),
    ];

    let password =
        load_unique_matching_ssh_password_from_lookups(&lookups, "root", "192.0.2.10", 22, &store)
            .unwrap();

    assert!(password.is_none());
}

#[test]
fn shared_onekey_password_uses_credential_lookup() {
    let store = MockSecretStore::default();
    store.insert("credential-1", SecretKind::OneKeyPassword, "shared-secret");
    let server = SshServerInfo {
        node_id: "server-1".to_string(),
        host: "192.0.2.10".to_string(),
        port: 22,
        username: String::new(),
        auth_type: AuthType::OneKey,
        key_path: None,
        credential_id: Some("credential-1".to_string()),
        startup_command: None,
        notes: None,
        last_connected_at: None,
    };
    let auth = ResolvedSshAuth {
        username: "root".to_string(),
        auth_type: AuthType::Password,
        key_path: None,
        secret_lookup_id: "credential-1".to_string(),
        secret_kind: SecretKind::OneKeyPassword,
    };
    let lookups = vec![SavedSshPasswordLookup::from_resolved_auth(&server, auth).unwrap()];

    let password =
        load_unique_matching_ssh_password_from_lookups(&lookups, "root", "192.0.2.10", 22, &store)
            .unwrap();

    assert_eq!(
        password.as_ref().map(|secret| secret.as_str()),
        Some("shared-secret")
    );
}

#[test]
fn empty_password_is_not_auto_filled() {
    let store = MockSecretStore::default();
    store.insert("server-1", SecretKind::Password, "");
    let lookups = vec![lookup(
        "server-1",
        "root",
        "192.0.2.10",
        22,
        SecretKind::Password,
    )];

    let password =
        load_unique_matching_ssh_password_from_lookups(&lookups, "root", "192.0.2.10", 22, &store)
            .unwrap();

    assert!(password.is_none());
}
