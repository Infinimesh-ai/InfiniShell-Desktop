use super::*;

#[cfg(unix)]
use command::blocking::Command;

struct TestAssetProvider;

impl AssetProvider for TestAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "bundled/bootstrap/fish.sh" => "# this is a comment\nthis_is_a_command",
            "bundled/bootstrap/zsh.sh" => {
                "asdf\n#include whitespace\n    prepended whitespace\n\n\n"
            }
            "bundled/bootstrap/pwsh.ps1" => {
                r#"# This is a comment
                Write-Output 'Testing some output'
                function test1 {
                    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
                    param([string]$command)
                    Invoke-Expression $command
                }"#
            }
            "hello_world" => "hello world!",
            "whitespace" => "no whitespace\n\n\n yes whitespace!",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

#[test]
fn test_include_directive() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &TestAssetProvider)),
        "hello world!\n"
    );
}

#[test]
fn test_trims_comments() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Fish, &TestAssetProvider)),
        "this_is_a_command\n"
    );
}

#[test]
fn test_trims_whitespace() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Zsh, &TestAssetProvider)),
        "asdf\nno whitespace\n yes whitespace!\n prepended whitespace\n"
    );
}

#[test]
fn test_trims_powershell_specifics() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::PowerShell, &TestAssetProvider)),
        " Write-Output 'Testing some output'\n function test1 {\n param([string]$command)\n Invoke-Expression $command\n }\n"
    );
}

#[cfg(unix)]
#[test]
fn remote_shell_probe_accepts_bash() {
    assert!(remote_shell_probe_supports_bootstrap(
        "__WARP_REMOTE_SHELL__/bin/bash\n"
    ));
}

#[cfg(unix)]
#[test]
fn remote_shell_probe_accepts_zsh_after_login_output() {
    assert!(remote_shell_probe_supports_bootstrap(
        "Authorized access only\n__WARP_REMOTE_SHELL__/usr/bin/zsh\r\n"
    ));
}

#[cfg(unix)]
#[test]
fn remote_shell_probe_rejects_cmd_literal() {
    assert!(!remote_shell_probe_supports_bootstrap(
        "__WARP_REMOTE_SHELL__$SHELL\r\n"
    ));
}

#[cfg(unix)]
#[test]
fn remote_shell_probe_rejects_empty_powershell_value() {
    assert!(!remote_shell_probe_supports_bootstrap(
        "__WARP_REMOTE_SHELL__\r\n"
    ));
}

#[cfg(unix)]
#[test]
fn remote_shell_probe_rejects_unsupported_posix_shell() {
    assert!(!remote_shell_probe_supports_bootstrap(
        "__WARP_REMOTE_SHELL__/usr/bin/fish\n"
    ));
}

#[cfg(unix)]
fn remote_shell_probe_supports_bootstrap(probe_output: &str) -> bool {
    let script = format!(
        "{}\nremote_shell=$(warp_remote_shell_from_probe_output \"$1\")\nwarp_remote_shell_supports_bootstrap \"$remote_shell\"",
        include_str!("../../assets/bundled/bootstrap/ssh_remote_shell_probe.sh")
    );
    Command::new("bash")
        .arg("-c")
        .arg(script)
        .arg("bootstrap-test")
        .arg(probe_output)
        .status()
        .expect("bash 应能执行远端 shell 探测辅助函数")
        .success()
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
