pub mod auth;
pub mod client;
pub mod codebase_index_proto;
pub mod host_id;
pub mod host_response;
pub mod manager;
pub mod protocol;
pub mod repo_metadata_proto;
pub mod setup;
#[cfg(not(target_family = "wasm"))]
pub mod ssh;
pub mod transport;

pub use host_id::HostId;

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "ssh_e2e_tests.rs"]
mod ssh_e2e_tests;

#[allow(clippy::large_enum_variant)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/remote_server.rs"));

    // ── ClientMessage constructors ──────────────────────────────────
    //
    // These helpers wrap inner message types in the appropriate
    // HostScopedRequest / SessionScopedRequest / Notification envelope
    // so call sites don't need triple-nested struct literals.

    impl ClientMessage {
        /// Build a `ClientMessage` carrying a host-scoped request.
        pub fn host_scoped(request_id: String, inner: host_scoped_request::Message) -> Self {
            Self {
                request_id,
                message: Some(client_message::Message::HostScoped(HostScopedRequest {
                    message: Some(inner),
                })),
            }
        }

        /// Build a `ClientMessage` carrying a session-scoped request.
        pub fn session_scoped(request_id: String, inner: session_scoped_request::Message) -> Self {
            Self {
                request_id,
                message: Some(client_message::Message::SessionScoped(
                    SessionScopedRequest {
                        message: Some(inner),
                    },
                )),
            }
        }

        /// Build a `ClientMessage` carrying a notification (fire-and-forget).
        pub fn notification(inner: notification::Message) -> Self {
            Self {
                request_id: String::new(),
                message: Some(client_message::Message::Notification(Notification {
                    message: Some(inner),
                })),
            }
        }

        /// 构造连接级 SSH 隧道消息。
        pub fn tunnel(
            request_id: String,
            stream_id: String,
            inner: tunnel_client_message::Message,
        ) -> Self {
            Self {
                request_id,
                message: Some(client_message::Message::Tunnel(TunnelClientMessage {
                    stream_id,
                    message: Some(inner),
                })),
            }
        }
    }

    impl InitializeResponse {
        /// 判断远端 daemon 是否显式宣告支持某项协议能力。
        pub fn supports(&self, capability: RemoteServerCapability) -> bool {
            self.capabilities.contains(&(capability as i32))
        }
    }
}
