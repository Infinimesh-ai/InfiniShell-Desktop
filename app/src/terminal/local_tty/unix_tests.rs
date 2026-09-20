use super::*;

fn shell_starter(shell_type: ShellType, shell_path: &str) -> DirectShellStarter {
    DirectShellStarter::new_for_test(shell_type, PathBuf::from(shell_path), Vec::new())
}

fn env_value(command: &Command, key: &str) -> Option<Option<String>> {
    command
        .get_envs()
        .find(|(env_key, _)| *env_key == std::ffi::OsStr::new(key))
        .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
}

#[test]
fn host_bash_command_sets_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Bash, "/bin/bash"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}

#[test]
fn host_non_bash_command_does_not_set_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Zsh, "/bin/zsh"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(env_value(&command, "HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "HISTSIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTSIZE"), None);
}

#[test]
fn host_notification_worker_replaces_stale_override_and_respects_capability() {
    for enabled in [true, false] {
        let _guard = FeatureFlag::HOANotifications.override_enabled(enabled);
        let command = build_host_shell_command(
            shell_starter(ShellType::Bash, "/bin/bash"),
            None,
            HashMap::from([(
                WARP_CLI_AGENT_NOTIFY_EXECUTABLE_ENV.into(),
                "/old-host/worker".into(),
            )]),
            None,
            false,
            false,
            false,
            false,
            true,
        );
        let advertised = env_value(&command, WARP_CLI_AGENT_NOTIFY_EXECUTABLE_ENV);
        if enabled {
            assert_eq!(
                advertised,
                Some(Some(
                    std::env::current_exe()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                ))
            );
        } else {
            assert_eq!(advertised, Some(None));
        }
    }
}

#[test]
fn container_does_not_advertise_a_host_notification_worker() {
    let _guard = FeatureFlag::HOANotifications.override_enabled(true);
    let starter = DockerSandboxShellStarter::new(shell_starter(ShellType::Bash, "sbx"), None);
    let command = build_docker_sandbox_command(
        &starter,
        None,
        HashMap::from([(
            WARP_CLI_AGENT_NOTIFY_EXECUTABLE_ENV.into(),
            "/host/worker".into(),
        )]),
        false,
        false,
        false,
        false,
        true,
    );
    assert_eq!(
        env_value(&command, WARP_CLI_AGENT_NOTIFY_EXECUTABLE_ENV),
        Some(None)
    );
}

#[test]
fn docker_sandbox_command_sets_history_size_sentinels() {
    let docker_starter =
        DockerSandboxShellStarter::new(shell_starter(ShellType::Bash, "sbx"), None);
    let command = build_docker_sandbox_command(
        &docker_starter,
        None,
        HashMap::new(),
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}

#[test]
fn new_pty_replaces_inherited_cli_agent_terminal_with_its_own_slave() {
    let directory = tempfile::tempdir().unwrap();
    let inherited = directory.path().join("inherited");
    let actual = directory.path().join("actual");
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(r#"printf '%s' "$WARP_CLI_AGENT_TTY" > "$1"; tty > "$2""#)
        .arg("pty-test")
        .arg(&inherited)
        .arg(&actual)
        .env(WARP_CLI_AGENT_TTY_ENV, "/dev/stale-outer-terminal");
    let mut spawned =
        spawn_command_in_pty(command, &SizeInfo::new_without_font_metrics(80, 24), true).unwrap();
    // 保持 leader 存活直到子进程退出，避免 SIGHUP 使真实 PTY 身份检查失真。
    let _leader = unsafe { File::from_raw_fd(spawned.result.leader_fd) };
    assert!(spawned.child.wait().unwrap().success());
    let advertised = std::fs::read_to_string(inherited).unwrap();
    assert!(advertised.starts_with("/dev/"));
    assert_ne!(advertised, "/dev/stale-outer-terminal");
    assert_eq!(advertised, std::fs::read_to_string(actual).unwrap().trim());
}
