use std::time::Duration;

use super::*;

#[test]
fn detects_interactive_session_commands_across_platforms() {
    for command in [
        "ssh root@example.com",
        "command ssh localhost",
        "ssh.exe -p 2222 root@example.com",
        "/usr/bin/ssh host",
        r#""C:\Windows\System32\OpenSSH\ssh.exe" -p 22 host"#,
        r#"& "C:\Program Files\OpenSSH\ssh.exe" host"#,
        "warp_run_generator_command 42 'ssh host'",
        " warp_run_generator_command 42 'ssh host'",
        "Zap-Run-GeneratorCommand 42 'ssh host' -ErrorAction Ignore",
        r#"warp_run_generator_command 42 '"C:\Windows\System32\OpenSSH\ssh.exe" host'"#,
        "gcloud compute ssh --zone us-west1-a my-instance",
        "eb ssh --profile my-profile my-env",
        "doctl compute ssh --region nyc1 my-droplet",
        "mosh root@example.com",
        "sftp root@example.com",
        "telnet example.com",
    ] {
        assert_eq!(
            command_starts_non_terminating_session(command),
            true,
            "{command}"
        );
    }
}

#[test]
fn does_not_detect_unrelated_or_non_interactive_ssh_commands() {
    for command in [
        "",
        "echo ssh",
        "git status",
        "ssh-add-key",
        "ssh -T user@host",
        "ssh -v user@host -W localhost:22",
        "ssh user@host ls",
        "ssh.exe user@host ls",
        r#""C:\Windows\System32\OpenSSH\ssh.exe" user@host ls"#,
        r#"& "C:\Program Files\OpenSSH\ssh.exe" user@host ls"#,
        "warp_run_generator_command 42 'ssh user@host ls'",
        "Zap-Run-GeneratorCommand 42 'git status' -ErrorAction Ignore",
        "rsync myfile.txt ssh://user@server.com",
        // 右引号后还粘着字符,故意拒绝 tokenize,避免被错切成 `ssh`
        // 然后通过 `ssh hello-world` 误判为交互会话。
        r#""ssh"hello-world"#,
        // 未闭合的引号同样拒绝 tokenize。
        r#""ssh hello world"#,
    ] {
        assert_eq!(
            command_starts_non_terminating_session(command),
            false,
            "{command}"
        );
    }
}

#[test]
fn shortens_on_completion_delay_for_interactive_sessions() {
    assert_eq!(
        effective_read_shell_command_delay("ssh host", Some(ShellCommandDelay::OnCompletion)),
        ActionResultDelay::OnCompletion {
            timeout: ShellCommandExecutor::MAX_WAIT_DURATION
        }
    );
    assert_eq!(
        effective_read_shell_command_delay(
            r#"& "C:\Program Files\OpenSSH\ssh.exe" host"#,
            Some(ShellCommandDelay::OnCompletion)
        ),
        ActionResultDelay::OnCompletion {
            timeout: ShellCommandExecutor::MAX_WAIT_DURATION
        }
    );
    assert_eq!(
        effective_read_shell_command_delay(
            "warp_run_generator_command 42 'ssh host'",
            Some(ShellCommandDelay::OnCompletion)
        ),
        ActionResultDelay::OnCompletion {
            timeout: ShellCommandExecutor::MAX_WAIT_DURATION
        }
    );
    assert_eq!(
        effective_read_shell_command_delay("mosh host", None),
        ActionResultDelay::OnCompletion {
            timeout: ShellCommandExecutor::MAX_WAIT_DURATION
        }
    );
}

#[test]
fn preserves_explicit_or_non_interactive_read_delays() {
    assert_eq!(
        effective_read_shell_command_delay(
            "ssh host",
            Some(ShellCommandDelay::Duration(Duration::from_secs(8)))
        ),
        ActionResultDelay::Duration(Duration::from_secs(8))
    );
    assert_eq!(
        effective_read_shell_command_delay("git status", Some(ShellCommandDelay::OnCompletion)),
        ActionResultDelay::OnCompletion {
            timeout: ShellCommandExecutor::MAX_AGENT_DELAY_DURATION
        }
    );
    assert_eq!(
        effective_read_shell_command_delay("git status", None),
        ActionResultDelay::Default
    );
}

#[test]
fn requested_command_wait_until_completion_does_not_use_snapshot_timeout() {
    assert_eq!(
        action_result_delay_for_requested_command(true),
        ActionResultDelay::UntilCompletion
    );
    assert_eq!(
        action_result_delay_for_requested_command(false),
        ActionResultDelay::Default
    );
}

#[test]
fn preemption_logic_covers_until_completion_timeout() {
    use ActionResultDelay::{Default, Duration as DurationDelay, OnCompletion, UntilCompletion};
    use WakeReason::*;

    // BlockFinished 从不抢占 —— 它是“命令真正完成”的信号。
    assert!(!compute_is_preempted(BlockFinished, UntilCompletion));
    assert!(!compute_is_preempted(BlockFinished, Default));
    assert!(!compute_is_preempted(
        BlockFinished,
        OnCompletion {
            timeout: Duration::from_secs(1)
        }
    ));

    // ForceRefresh 总是抢占,与 delay 无关。
    assert!(compute_is_preempted(ForceRefresh, UntilCompletion));
    assert!(compute_is_preempted(ForceRefresh, Default));

    // Timeout + OnCompletion / UntilCompletion 是抢占。
    assert!(compute_is_preempted(
        Timeout,
        OnCompletion {
            timeout: Duration::from_secs(1)
        }
    ));
    // #138: pager 卡死兜底超时必须被标记为抢占,避免 server 误解为“命令完成”。
    assert!(compute_is_preempted(Timeout, UntilCompletion));

    // Timeout + Default / Duration 不是抢占 —— agent 本来就预期会拿到中间快照。
    assert!(!compute_is_preempted(Timeout, Default));
    assert!(!compute_is_preempted(
        Timeout,
        DurationDelay(Duration::from_secs(1))
    ));
}

/// 复现挂死案例:heredoc 结束符必须保持独占一行,闭合 `)` 不能被拼到它后面。
/// 拼成 `PY)` 后 shell 永远等不到结束符,停在 PS2 上,命令永不结束。
#[test]
fn multiline_heredoc_keeps_delimiter_and_closer_on_their_own_lines() {
    let command = "python3 - <<'PY'\nprint('ok')\nPY";

    for shell in [ShellType::Bash, ShellType::Zsh] {
        let wrapped = wrap_command_without_pager(Some(shell), command);
        let lines: Vec<&str> = wrapped.lines().collect();

        assert_eq!(
            lines[lines.len() - 2],
            "PY",
            "heredoc 结束符被污染: {wrapped}"
        );
        assert_eq!(lines[lines.len() - 1], ")", "闭合括号未独占一行: {wrapped}");
        assert!(wrapped.contains("PAGER=cat"), "pager 抑制丢失: {wrapped}");
    }
}

/// 同一类破绽:命令以尾随 `#` 注释结尾时,闭合 token 会被注释掉。
#[test]
fn multiline_trailing_comment_does_not_swallow_closer() {
    let command = "echo start\necho done # 收尾";

    assert!(wrap_command_without_pager(Some(ShellType::Bash), command).ends_with("\n)"));
    assert!(wrap_command_without_pager(Some(ShellType::Fish), command).ends_with("\nend"));
    assert!(wrap_command_without_pager(Some(ShellType::PowerShell), command).ends_with("\n}"));
}

/// 单行命令必须保持原有的单行形态:`bytes_to_execute_command` 在不支持 bracketed
/// paste 的 shell 上会把 `\n` 换成 `\r`,多加换行会把一条命令拆成多个 block。
#[test]
fn single_line_command_stays_on_one_line() {
    let command = "cargo check";

    for shell in [
        ShellType::Bash,
        ShellType::Zsh,
        ShellType::Fish,
        ShellType::PowerShell,
    ] {
        let wrapped = wrap_command_without_pager(Some(shell), command);
        assert!(
            !wrapped.contains('\n'),
            "{shell:?} 单行命令被拆行: {wrapped}"
        );
        assert!(wrapped.contains(command), "{shell:?} 命令丢失: {wrapped}");
    }
}

/// 未知 shell 无法安全装饰,原样放过。
#[test]
fn unknown_shell_passes_command_through() {
    let command = "python3 - <<'PY'\nprint('ok')\nPY";
    assert_eq!(wrap_command_without_pager(None, command), command);
}
