use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 连接状态，仅用于 UI 层显示，不持久化。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionStatus {
    Unknown,
    Online,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    Folder,
    Server,
}

impl NodeKind {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            NodeKind::Folder => "folder",
            NodeKind::Server => "server",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "folder" => Some(NodeKind::Folder),
            "server" => Some(NodeKind::Server),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AuthType {
    Password,
    Key,
    OneKey,
}

impl AuthType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            AuthType::Password => "password",
            AuthType::Key => "key",
            AuthType::OneKey => "onekey",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "password" => Some(AuthType::Password),
            "key" => Some(AuthType::Key),
            "onekey" => Some(AuthType::OneKey),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OneKeyCredentialKind {
    Password,
    Key,
}

impl OneKeyCredentialKind {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            OneKeyCredentialKind::Password => "password",
            OneKeyCredentialKind::Key => "key",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "password" => Some(OneKeyCredentialKind::Password),
            "key" => Some(OneKeyCredentialKind::Key),
            _ => None,
        }
    }
}

/// 树节点（folder 或 server），不含 server-only metadata。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: NodeKind,
    pub name: String,
    pub sort_order: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    /// 仅对 folder 有意义，UI 据此决定是否隐藏子节点。SQLite 持久化让重启后状态保持。
    pub is_collapsed: bool,
}

/// Server 节点的连接配置。`password` / `passphrase` 不在此处，走 keychain。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshServerInfo {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub key_path: Option<String>,
    pub credential_id: Option<String>,
    pub startup_command: Option<String>,
    pub notes: Option<String>,
    pub last_connected_at: Option<NaiveDateTime>,
}

/// 已保存的多级 SSH 访问路径。只包含结构信息，不包含任何秘密或运行时 socket。
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SshRoute {
    pub id: String,
    pub name: String,
    pub target_node_id: Option<String>,
    pub hops: Vec<SshRouteHop>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub last_connected_at: Option<NaiveDateTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SshRouteHop {
    pub node_id: Option<String>,
    pub target_alias: String,
    #[serde(default)]
    pub port: Option<u16>,
}

impl SshServerInfo {
    pub fn new_default(node_id: String) -> Self {
        Self {
            node_id,
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_type: AuthType::Password,
            key_path: None,
            credential_id: None,
            startup_command: None,
            notes: None,
            last_connected_at: None,
        }
    }

    /// 从现有服务器克隆配置，生成新的 node_id。
    pub fn clone_from_template(source: &Self, new_node_id: String) -> Self {
        Self {
            node_id: new_node_id,
            host: source.host.clone(),
            port: source.port,
            username: source.username.clone(),
            auth_type: source.auth_type,
            key_path: source.key_path.clone(),
            credential_id: source.credential_id.clone(),
            startup_command: source.startup_command.clone(),
            notes: source.notes.clone(),
            last_connected_at: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SshOneKeyCredential {
    pub id: String,
    pub label: String,
    pub username: String,
    pub kind: OneKeyCredentialKind,
    pub key_path: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl SshOneKeyCredential {
    pub fn display_label(&self) -> String {
        if self.username.is_empty() {
            self.label.clone()
        } else {
            format!("{} ({})", self.label, self.username)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSshAuth {
    pub username: String,
    pub auth_type: AuthType,
    pub key_path: Option<String>,
    pub secret_lookup_id: String,
    pub secret_kind: crate::secrets::SecretKind,
}
