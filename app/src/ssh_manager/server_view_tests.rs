/// resolve_test_password 单元测试
/// author: logic
/// date: 2026/06/01
use super::*;
use pathfinder_geometry::vector::vec2f;
use std::collections::HashMap;
use std::sync::Mutex;
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, WindowInvalidation};

use crate::test_util::settings::initialize_settings_for_tests;
use crate::view_components::dropdown::DropdownAction;

/// 进程内 mock,绕开 OS keychain。支持错误注入,模拟 NoBackend / Keyring 错。
struct MockSecretStore {
    inner: Mutex<HashMap<String, String>>,
    get_err: Mutex<Option<SshSecretStoreError>>,
}

impl MockSecretStore {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            get_err: Mutex::new(None),
        }
    }

    fn with_secret(node: &str, kind: SecretKind, value: &str) -> Self {
        let s = Self::new();
        s.set(node, kind, value).unwrap();
        s
    }

    fn inject_get_error(&self, err: SshSecretStoreError) {
        *self.get_err.lock().unwrap() = Some(err);
    }
}

fn account_key(node_id: &str, kind: SecretKind) -> String {
    let suffix = match kind {
        SecretKind::Password => "password",
        SecretKind::Passphrase => "passphrase",
        SecretKind::RootPassword => "root_password",
        SecretKind::OneKeyPassword => "onekey_password",
    };
    format!("{node_id}:{suffix}")
}

impl SshSecretStore for MockSecretStore {
    fn set(
        &self,
        node_id: &str,
        kind: SecretKind,
        secret: &str,
    ) -> Result<(), SshSecretStoreError> {
        self.inner
            .lock()
            .unwrap()
            .insert(account_key(node_id, kind), secret.to_string());
        Ok(())
    }

    fn get(
        &self,
        node_id: &str,
        kind: SecretKind,
    ) -> Result<Option<Zeroizing<String>>, SshSecretStoreError> {
        if let Some(err) = self.get_err.lock().unwrap().take() {
            return Err(err);
        }
        Ok(self
            .inner
            .lock()
            .unwrap()
            .get(&account_key(node_id, kind))
            .cloned()
            .map(Zeroizing::new))
    }

    fn delete(&self, _node_id: &str, _kind: SecretKind) -> Result<(), SshSecretStoreError> {
        unimplemented!()
    }
}

#[test]
fn auth_toggle_includes_onekey_option() {
    crate::i18n::init(Some("en"));

    let options = auth_toggle_options();
    assert_eq!(
        options,
        [AuthType::Password, AuthType::Key, AuthType::OneKey]
    );
    assert_eq!(auth_toggle_label(AuthType::OneKey), "OneKey");
    assert_eq!(
        auth_toggle_action(AuthType::OneKey),
        SshServerAction::SetAuthOneKey
    );
}

#[test]
fn machine_memory_key_uses_normalized_server_host_and_port() {
    let server = SshServerInfo {
        node_id: "server-1".to_string(),
        host: " Admin@WEB-01.EXAMPLE.COM ".to_string(),
        port: 2202,
        username: "admin".to_string(),
        auth_type: AuthType::Password,
        key_path: None,
        credential_id: None,
        startup_command: None,
        notes: None,
        last_connected_at: None,
    };

    assert_eq!(
        machine_key_for_server(&server).as_deref(),
        Some("web-01.example.com:2202")
    );
}

#[test]
fn onekey_auth_only_renders_credential_field_in_server_form() {
    assert_eq!(
        auth_specific_fields(AuthType::OneKey),
        vec![AuthSpecificField::OneKeyCredential]
    );
}

#[test]
fn empty_editor_empty_store_returns_none() {
    let store = MockSecretStore::new();
    assert!(resolve_test_password(Some("n1"), SecretKind::Password, "", &store).is_none());
}

#[test]
fn empty_editor_stored_returns_secret() {
    let store = MockSecretStore::with_secret("n1", SecretKind::Password, "from-keychain");
    let pw = resolve_test_password(Some("n1"), SecretKind::Password, "", &store).unwrap();
    assert_eq!(&*pw, "from-keychain");
}

#[test]
fn filled_editor_ignores_keychain() {
    // keychain 存了旧密码,form 敲了新密码 → 必须用 form 的新密码,
    // 否则用户改 host 后测试会被旧密码污染。
    let store = MockSecretStore::with_secret("n1", SecretKind::Password, "old-pw");
    let pw = resolve_test_password(Some("n1"), SecretKind::Password, "new-pw", &store).unwrap();
    assert_eq!(&*pw, "new-pw");
}

#[test]
fn empty_editor_no_backend_returns_none() {
    let store = MockSecretStore::new();
    store.inject_get_error(SshSecretStoreError::NoBackend);
    assert!(resolve_test_password(Some("n1"), SecretKind::Password, "", &store).is_none());
}

#[test]
fn empty_editor_keyring_error_returns_none() {
    let store = MockSecretStore::new();
    store.inject_get_error(SshSecretStoreError::Keyring("locked".into()));
    assert!(resolve_test_password(Some("n1"), SecretKind::Password, "", &store).is_none());
}

#[test]
fn onekey_lookup_uses_shared_credential_id_and_kind() {
    let store = MockSecretStore::with_secret("cred-1", SecretKind::OneKeyPassword, "shared-pw");
    let pw = resolve_test_password(Some("cred-1"), SecretKind::OneKeyPassword, "", &store).unwrap();
    assert_eq!(&*pw, "shared-pw");
}

fn credential(
    id: &str,
    username: &str,
    kind: OneKeyCredentialKind,
    key_path: Option<&str>,
) -> SshOneKeyCredential {
    let now = chrono::Utc::now().naive_utc();
    SshOneKeyCredential {
        id: id.to_string(),
        label: "shared".to_string(),
        username: username.to_string(),
        kind,
        key_path: key_path.map(ToString::to_string),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn onekey_test_connection_uses_shared_password_credential() {
    let store = MockSecretStore::with_secret("cred-1", SecretKind::OneKeyPassword, "shared-pw");
    let credentials = vec![credential(
        "cred-1",
        "shared-user",
        OneKeyCredentialKind::Password,
        None,
    )];
    let server = SshServerInfo {
        node_id: "server-1".to_string(),
        host: "example.com".to_string(),
        port: 22,
        username: "draft-user".to_string(),
        auth_type: AuthType::OneKey,
        key_path: None,
        credential_id: Some("cred-1".to_string()),
        startup_command: None,
        notes: None,
        last_connected_at: None,
    };

    let (server, pw) = resolve_test_server_and_password(server, &credentials, "", &store).unwrap();

    assert_eq!(server.username, "shared-user");
    assert_eq!(server.auth_type, AuthType::Password);
    assert_eq!(server.key_path, None);
    assert_eq!(&*pw.unwrap(), "shared-pw");
}

#[test]
fn onekey_test_connection_prefers_editor_password() {
    let store = MockSecretStore::with_secret("cred-1", SecretKind::OneKeyPassword, "old-pw");
    let credentials = vec![credential(
        "cred-1",
        "shared-user",
        OneKeyCredentialKind::Password,
        None,
    )];
    let server = SshServerInfo {
        node_id: "server-1".to_string(),
        host: "example.com".to_string(),
        port: 22,
        username: "draft-user".to_string(),
        auth_type: AuthType::OneKey,
        key_path: None,
        credential_id: Some("cred-1".to_string()),
        startup_command: None,
        notes: None,
        last_connected_at: None,
    };

    let (_, pw) =
        resolve_test_server_and_password(server, &credentials, "typed-pw", &store).unwrap();

    assert_eq!(&*pw.unwrap(), "typed-pw");
}

#[test]
fn onekey_key_credential_resolves_test_connection_to_key_auth() {
    let store = MockSecretStore::with_secret("cred-1", SecretKind::Passphrase, "key-passphrase");
    let credentials = vec![credential(
        "cred-1",
        "key-user",
        OneKeyCredentialKind::Key,
        Some("/home/me/.ssh/id_ed25519"),
    )];
    let server = SshServerInfo {
        node_id: "server-1".to_string(),
        host: "example.com".to_string(),
        port: 22,
        username: "draft-user".to_string(),
        auth_type: AuthType::OneKey,
        key_path: None,
        credential_id: Some("cred-1".to_string()),
        startup_command: None,
        notes: None,
        last_connected_at: None,
    };

    let (server, pw) = resolve_test_server_and_password(server, &credentials, "", &store).unwrap();

    assert_eq!(server.username, "key-user");
    assert_eq!(server.auth_type, AuthType::Key);
    assert_eq!(server.key_path.as_deref(), Some("/home/me/.ssh/id_ed25519"));
    assert_eq!(&*pw.unwrap(), "key-passphrase");
}

#[test]
fn missing_lookup_id_returns_none_when_editor_empty() {
    let store = MockSecretStore::new();
    assert!(resolve_test_password(None, SecretKind::OneKeyPassword, "", &store).is_none());
}

#[test]
fn selecting_onekey_dropdown_item_does_not_rebuild_dropdown_while_it_is_borrowed() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SshTreeChangedNotifier::new());

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut view = SshServerView::new("server-1".to_string(), ctx);
            view.node = Some(SshNode {
                id: "server-1".to_string(),
                parent_id: None,
                kind: NodeKind::Server,
                name: "server".to_string(),
                sort_order: 0,
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
                is_collapsed: false,
            });
            view.auth_type = AuthType::OneKey;
            view.onekey_credentials = vec![credential(
                "cred-1",
                "shared-user",
                OneKeyCredentialKind::Password,
                None,
            )];
            view.rebuild_onekey_credential_dropdown(ctx);
            view
        });
        let presenter = app.presenter(window_id).unwrap();
        let mut updated = std::collections::HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        app.update(|ctx| {
            let mut presenter = presenter.borrow_mut();
            presenter.invalidate(
                WindowInvalidation {
                    updated,
                    ..Default::default()
                },
                ctx,
            );
            presenter.build_scene(vec2f(640., 480.), 1., None, ctx);
        });

        let dropdown = view.read(&app, |view, _| view.onekey_credential_dropdown.clone());
        dropdown.update(&mut app, |dropdown, ctx| {
            dropdown.handle_action(
                &DropdownAction::SelectActionAndClose(SshServerAction::SelectOneKeyCredential(
                    Some(0),
                )),
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert_eq!(
                view.selected_onekey_credential_id.as_deref(),
                Some("cred-1")
            );
        });
    });
}
