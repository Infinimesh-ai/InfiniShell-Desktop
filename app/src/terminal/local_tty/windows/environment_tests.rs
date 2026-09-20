use std::collections::HashMap;

use super::*;

use crate::terminal::SizeInfo;
use crate::terminal::local_tty::shell::DirectShellStarter;
use crate::terminal::shell::ShellType;

#[test]
fn local_worker_overrides_stale_environment_and_is_not_forwarded_to_wsl() {
    for enabled in [true, false] {
        let _guard = FeatureFlag::HOANotifications.override_enabled(enabled);
        let options = PtyOptions {
            size: SizeInfo::new_without_font_metrics(80, 24),
            window_id: None,
            shell_starter: ShellStarter::Direct(DirectShellStarter::new_for_test(
                ShellType::PowerShell,
                "powershell.exe".into(),
                Vec::new(),
            )),
            start_dir: None,
            env_vars: HashMap::from([(
                WARP_CLI_AGENT_NOTIFY_EXECUTABLE_ENV.into(),
                "C:\\old-host\\worker.exe".into(),
            )]),
            enable_ssh_wrapper: false,
            reuse_ssh_control_master: false,
            shell_debug_mode: false,
            honor_ps1: false,
            node_version_chip_enabled: false,
            close_fds: true,
        };
        let block = get_shell_environment_variables(&options);
        let prefix = format!("{WARP_CLI_AGENT_NOTIFY_EXECUTABLE_ENV}=");
        let advertised = block
            .split(|value| *value == 0)
            .filter_map(|entry| String::from_utf16(entry).ok())
            .find_map(|entry| entry.strip_prefix(&prefix).map(str::to_owned));
        if enabled {
            assert_eq!(
                advertised,
                Some(
                    crate::remote_server::rust_ssh::worker_executable()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                )
            );
        } else {
            assert_eq!(advertised, None);
        }
        assert!(
            !wsl_env_allowlist(false)
                .to_string_lossy()
                .contains(WARP_CLI_AGENT_NOTIFY_EXECUTABLE_ENV)
        );
    }
}

#[test]
fn wsl_env_allowlist_includes_client_version_without_notifications_flag() {
    let _guard = FeatureFlag::HOANotifications.override_enabled(false);

    let wslenv = wsl_env_allowlist(false).to_string_lossy().into_owned();

    assert_eq!(
        wslenv.split(':').collect::<Vec<_>>(),
        vec![
            format!("{HONOR_PS1_NAME}/u"),
            format!("{USE_SSH_WRAPPER_NAME}/u"),
            format!("{SSH_REUSE_CONTROL_MASTER_NAME}/u"),
            format!("{SHELL_DEBUG_MODE_NAME}/u"),
            format!("{TERM_PROGRAM_NAME}/u"),
            format!("{IS_LOCAL_SESSION_NAME}/u"),
            format!("{SSH_SOCKET_DIR}/u"),
            format!("{WARP_CLIENT_VERSION_ENV}/u"),
            format!("{TERMINAL_SESSION_UUID_ENV}/u"),
            format!("{FOCUS_URL_ENV}/u"),
            format!("{PROMPT_NODE_VERSION_ENABLED_NAME}/u"),
        ],
    );
}

#[test]
fn wsl_env_allowlist_includes_cli_agent_protocol_when_notifications_flag_is_enabled() {
    let _guard = FeatureFlag::HOANotifications.override_enabled(true);

    let wslenv = wsl_env_allowlist(true).to_string_lossy().into_owned();

    assert_eq!(
        wslenv.split(':').collect::<Vec<_>>(),
        vec![
            format!("{HONOR_PS1_NAME}/u"),
            format!("{USE_SSH_WRAPPER_NAME}/u"),
            format!("{SSH_REUSE_CONTROL_MASTER_NAME}/u"),
            format!("{SHELL_DEBUG_MODE_NAME}/u"),
            format!("{TERM_PROGRAM_NAME}/u"),
            format!("{IS_LOCAL_SESSION_NAME}/u"),
            format!("{SSH_SOCKET_DIR}/u"),
            format!("{WARP_CLIENT_VERSION_ENV}/u"),
            format!("{TERMINAL_SESSION_UUID_ENV}/u"),
            format!("{FOCUS_URL_ENV}/u"),
            format!("{PROMPT_NODE_VERSION_ENABLED_NAME}/u"),
            format!("{WARP_CLI_AGENT_PROTOCOL_VERSION_ENV}/u"),
            format!("{INITIAL_WORKING_DIR_NAME}/pu"),
        ],
    );
}
