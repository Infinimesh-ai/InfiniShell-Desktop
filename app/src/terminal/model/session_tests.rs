use std::collections::HashMap;
use std::sync::Arc;

use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{App, AppContext, Element, Entity, ModelHandle, TypedActionView, View, ViewContext};

use super::command_executor::testing::TestCommandExecutor;
use super::{
    BootstrapSessionType, ControlMasterOwnership, Session, SessionId, SessionInfo, Sessions,
    SessionsEvent, SshSessionTransportDescriptor,
};
use crate::terminal::model::ansi::{SSHValue, SshTransportValue};

struct TestView {
    events: Vec<SessionsEvent>,
}

impl Entity for TestView {
    type Event = usize;
}

impl View for TestView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn ui_name() -> &'static str {
        "TestView"
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

impl TestView {
    fn new(model: ModelHandle<Sessions>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&model, |me, _, event, _| {
            me.events.push(event.to_owned());
        });
        Self { events: Vec::new() }
    }
}

#[test]
fn legacy_ssh_transport_maps_to_control_master() {
    let value = SSHValue {
        socket_path: Some("/tmp/warp-control".into()),
        external_control_master: true,
        ..Default::default()
    };

    assert_eq!(
        SshSessionTransportDescriptor::from_ssh_value(&value),
        SshSessionTransportDescriptor::ControlMaster {
            socket_path: "/tmp/warp-control".into(),
            ownership: ControlMasterOwnership::UserOwned,
        }
    );
}

#[test]
fn versioned_ssh_transport_accepts_consistent_legacy_fields() {
    let value = SSHValue {
        socket_path: Some("/tmp/warp-control".into()),
        transport: Some(SshTransportValue {
            version: 1,
            transport_type: "control_master".to_owned(),
            socket_path: Some("/tmp/warp-control".into()),
            ownership: Some("warp_managed".to_owned()),
            endpoint: None,
            capability: None,
        }),
        ..Default::default()
    };

    assert_eq!(
        SshSessionTransportDescriptor::from_ssh_value(&value),
        SshSessionTransportDescriptor::ControlMaster {
            socket_path: "/tmp/warp-control".into(),
            ownership: ControlMasterOwnership::WarpManaged,
        }
    );
}

#[test]
fn conflicting_or_unknown_ssh_transport_is_unavailable() {
    let conflicting = SSHValue {
        socket_path: Some("/tmp/legacy-control".into()),
        transport: Some(SshTransportValue {
            version: 1,
            transport_type: "control_master".to_owned(),
            socket_path: Some("/tmp/new-control".into()),
            ownership: Some("warp_managed".to_owned()),
            endpoint: None,
            capability: None,
        }),
        ..Default::default()
    };
    let unknown_version = SSHValue {
        transport: Some(SshTransportValue {
            version: 2,
            transport_type: "control_master".to_owned(),
            socket_path: Some("/tmp/warp-control".into()),
            ownership: Some("warp_managed".to_owned()),
            endpoint: None,
            capability: None,
        }),
        ..Default::default()
    };

    assert_eq!(
        SshSessionTransportDescriptor::from_ssh_value(&conflicting),
        SshSessionTransportDescriptor::Unavailable
    );
    assert_eq!(
        SshSessionTransportDescriptor::from_ssh_value(&unknown_version),
        SshSessionTransportDescriptor::Unavailable
    );
}

#[test]
fn versioned_ssh_transport_accepts_loopback_rust_broker() {
    let capability = "ab".repeat(32);
    let value = SSHValue {
        transport: Some(SshTransportValue {
            version: 1,
            transport_type: "rust_broker".to_owned(),
            socket_path: None,
            ownership: None,
            endpoint: Some("127.0.0.1:49152".to_owned()),
            capability: Some(capability.clone()),
        }),
        ..Default::default()
    };

    let descriptor = SshSessionTransportDescriptor::from_ssh_value(&value);
    let debug = format!("{descriptor:?}");
    assert!(!debug.contains(&capability));
    assert!(debug.contains("<redacted>"));
    assert_eq!(
        descriptor,
        SshSessionTransportDescriptor::RustBroker {
            endpoint: "127.0.0.1:49152".to_owned(),
            capability,
        }
    );
}

#[test]
fn rust_broker_rejects_non_loopback_or_malformed_capability() {
    for (endpoint, capability) in [
        ("192.0.2.1:22", "ab".repeat(32)),
        ("127.0.0.1:49152", "not-a-capability".to_owned()),
    ] {
        let value = SSHValue {
            transport: Some(SshTransportValue {
                version: 1,
                transport_type: "rust_broker".to_owned(),
                socket_path: None,
                ownership: None,
                endpoint: Some(endpoint.to_owned()),
                capability: Some(capability),
            }),
            ..Default::default()
        };
        assert_eq!(
            SshSessionTransportDescriptor::from_ssh_value(&value),
            SshSessionTransportDescriptor::Unavailable
        );
    }
}

#[test]
fn unavailable_transport_keeps_wrapper_identity_without_remote_server_support() {
    let wrapper = super::IsSSHWrapperSession::Yes {
        transport: SshSessionTransportDescriptor::Unavailable,
    };

    assert!(wrapper.transport().is_some());
    assert!(!wrapper.supports_ssh_remote_server());
    assert!(wrapper.control_master().is_none());
}

#[test]
fn test_set_env_var_emits_event() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Sessions::new_for_test());
        let session_id: SessionId = 0.into();
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(model_handle.clone(), ctx)
        });
        view_handle.read(&app, |view, _ctx| {
            assert!(view.events.is_empty());
        });
        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
            let expected_session_id = session_id;
            let event = view.events.first().expect("checked length already");
            if let SessionsEvent::EnvironmentVariablesUpdated { session_id } = event {
                assert_eq!(*session_id, expected_session_id);
            } else {
                assert!(matches!(
                    event,
                    SessionsEvent::EnvironmentVariablesUpdated { .. }
                ));
            }
        });
    });
}

#[test]
fn test_set_env_var_emits_no_event_when_no_change() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Sessions::new_for_test());
        let session_id: SessionId = 0.into();
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(model_handle.clone(), ctx)
        });
        view_handle.read(&app, |view, _ctx| {
            assert!(view.events.is_empty());
        });
        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
        });

        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
        });
    });
}

#[test]
fn test_malicious_histfile_path_does_not_execute_injected_commands() {
    App::test((), |_app| async move {
        // If escaping is missing, `touch /tmp/warp_injection_test` would execute
        // as a side effect of reading history.
        let marker = "/tmp/warp_injection_test";
        // Clean up in case a previous broken run left the marker.
        let _ = std::fs::remove_file(marker);

        let malicious_histfile = format!("/tmp/x'; touch {marker}; echo '");

        let session_info = SessionInfo::new_for_test()
            .with_session_type(BootstrapSessionType::WarpifiedRemote)
            .with_histfile(Some(malicious_histfile));
        let session = Session::new(session_info, Arc::new(TestCommandExecutor::default()));

        // read_history for a WarpifiedRemote session calls read_history_from_file,
        // which builds `cat '{escaped_path}'` and executes it via TestCommandExecutor
        let _ = session.read_history(false).await;

        assert!(
            !std::path::Path::new(marker).exists(),
            "Injected command executed — escaping regression!"
        );
    });
}

#[cfg(not(windows))]
#[test]
fn can_resolve_cwd_to_native_path_accepts_posix_path() {
    let session = Session::test();
    assert!(session.can_resolve_cwd_to_native_path("/Users/foo/bar"));
}

#[cfg(windows)]
#[test]
fn can_resolve_cwd_to_native_path_accepts_windows_drive_path() {
    let session = Session::test();
    assert!(session.can_resolve_cwd_to_native_path(r"E:\CLAUDE-BASE"));
}

#[cfg(windows)]
#[test]
fn can_resolve_cwd_to_native_path_rejects_unix_encoded_path_on_windows() {
    let session_info =
        SessionInfo::new_for_test().with_shell_type(crate::terminal::shell::ShellType::Bash);
    let session = Session::new(session_info, Arc::new(TestCommandExecutor::default()));
    assert!(!session.can_resolve_cwd_to_native_path("/E:/CLAUDE-BASE"));
}

#[cfg(windows)]
#[test]
fn powershell_read_command_embeds_escaped_path_without_args() {
    use std::ffi::{OsStr, OsString};

    use super::powershell_read_all_text_command;

    // The path is embedded directly inside a single-quoted PowerShell literal.
    let raw = r"C:\Users\dev\AppData\Roaming\Microsoft\Windows\PowerShell\PSReadLine\ConsoleHost_history.txt";
    let command = powershell_read_all_text_command(OsStr::new(raw));
    assert_eq!(
        command,
        OsString::from(format!("[System.IO.File]::ReadAllText('{raw}')"))
    );

    // A single quote in the path is doubled so it can't terminate the literal.
    let command = powershell_read_all_text_command(OsStr::new(r"C:\o'brien\history.txt"));
    assert_eq!(
        command,
        OsString::from(r"[System.IO.File]::ReadAllText('C:\o''brien\history.txt')")
    );
}
