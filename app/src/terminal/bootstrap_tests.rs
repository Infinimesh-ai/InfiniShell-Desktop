use super::*;

use command::blocking::Command;
use serde_json::json;

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
            "hello_world" => "\u{FEFF}hello world!",
            "whitespace" => "no whitespace\n\n\n yes whitespace!",
            "bundled/bootstrap/pwsh_init_shell.ps1" => "\u{FEFF}Write-Output 'remote'",
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

#[test]
fn embeds_the_windows_remote_init_shell_as_bom_free_hex() {
    let script = embed_windows_remote_init_shell(
        format!("before {WINDOWS_REMOTE_INIT_SHELL_HEX_PLACEHOLDER} after"),
        &TestAssetProvider,
    );

    assert_eq!(
        script,
        "before 57726974652d4f7574707574202772656d6f746527 after"
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
#[test]
fn windows_capability_probe_requires_one_exact_versioned_line() {
    assert_eq!(
        remote_windows_powershell_from_probe_output(
            "__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell\r\n"
        ),
        "powershell"
    );
    assert_eq!(
        remote_windows_powershell_from_probe_output(
            "banner\n__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell\n"
        ),
        ""
    );
    assert_eq!(
        remote_windows_powershell_from_probe_output(
            "__WARP_REMOTE_CAPS__v=2;os=windows;shell=powershell\n"
        ),
        ""
    );
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

#[cfg(unix)]
fn remote_windows_powershell_from_probe_output(probe_output: &str) -> String {
    let script = format!(
        "{}\nwarp_windows_powershell_from_probe_output \"$1\"",
        include_str!("../../assets/bundled/bootstrap/ssh_remote_shell_probe.sh")
    );
    let output = Command::new("bash")
        .arg("-c")
        .arg(script)
        .arg("bootstrap-test")
        .arg(probe_output)
        .output()
        .expect("bash 应能执行 Windows PowerShell 能力探测辅助函数");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("探测输出应为 UTF-8")
}

#[test]
fn powershell_ssh_wrapper_classifies_sessions_and_preserves_arguments() {
    let Some(powershell) = powershell_executable() else {
        eprintln!("当前环境没有 PowerShell,跳过 PowerShell SSH wrapper 测试");
        return;
    };
    let helper_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/bundled/bootstrap/pwsh_ssh_wrapper.ps1"
    );
    let script = r#"
. $env:WARP_TEST_SSH_HELPER
$interactive = @(
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-p', '2222', 'user@host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-J', 'cloud', 'user@host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-t', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-w', '0:0', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-N', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-n', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-L', '8080:localhost:80', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-f', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-E', 'ssh.log', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-e', 'none', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-v', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-T', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('-W', 'target', 'host')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @('host', 'uname')),
    [bool](Warp-Test-InteractiveSshSession -SshArgs @())
)
$script:capturedSshArgs = @()
function Warp-Invoke-SshExecutable {
    param([object[]]$SshArgs)
    $script:capturedSshArgs = @($SshArgs)
}
Warp-Invoke-PlainSsh -SshArgs @('host', '', 'two words', 'quote"value', '$literal', '*.rs', '中文')
$passthrough = @($script:capturedSshArgs)
$script:WarpBashInitShell = 'echo bash'
$script:WarpZshInitShell = 'echo zsh'
$script:WarpPwshInitShell = 'function prompt { return $null } # @@WARP_SESSION_ID@@'
function Warp-Encode-HexString([string]$str) {
    [BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($str)).Replace('-', '')
}
$global:_warpSessionId = 99
function Warp-Test-IsWindows { return $true }
$env:WARP_RUST_SSH_EXECUTABLE = ''
$script:windowsProbeArgs = @()
$script:windowsBootstrapArgs = @()
function Warp-Invoke-SshExecutable {
    param([object[]]$SshArgs)
    if ($SshArgs[0] -ceq '-G') {
        $global:LASTEXITCODE = 0
        return 'remotecommand none'
    }
    if ($SshArgs[-1] -eq 'echo __WARP_REMOTE_SHELL__$SHELL') {
        $script:windowsProbeArgs = @($SshArgs)
        $global:LASTEXITCODE = 0
        return '__WARP_REMOTE_SHELL__/bin/bash'
    }
    $script:windowsBootstrapArgs = @($SshArgs)
    $global:LASTEXITCODE = 17
}
Warp-Invoke-EnhancedSsh -SshArgs @('windows-host', '', 'two words', '$literal')
$windowsStatus = $global:LASTEXITCODE
function Warp-Test-IsWindows { return $false }
$script:fallbackSshArgs = @()
$script:capabilityProbeArgs = @()
$script:closedOwnedControlMaster = $false
function Warp-Invoke-SshExecutable {
    param([object[]]$SshArgs)
    if ($SshArgs[0] -ceq '-G') {
        $global:LASTEXITCODE = 0
        return 'remotecommand none'
    }
    if ($SshArgs[-1] -eq 'echo __WARP_REMOTE_SHELL__$SHELL') {
        $global:LASTEXITCODE = 0
        return '__WARP_REMOTE_SHELL__'
    }
    if ($SshArgs[0] -ceq '-O') {
        $script:closedOwnedControlMaster = $true
        $global:LASTEXITCODE = 0
        return
    }
    if ($SshArgs[-1] -like 'powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand *') {
        $script:capabilityProbeArgs = @($SshArgs)
        $global:LASTEXITCODE = 0
        return '__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell'
    }
    $script:fallbackSshArgs = @($SshArgs)
    $global:LASTEXITCODE = 23
}
$env:SSH_SOCKET_DIR = $env:TEMP
if ([String]::IsNullOrEmpty($env:SSH_SOCKET_DIR)) {
    $env:SSH_SOCKET_DIR = '/tmp'
}
$env:WARP_SSH_REUSE_CONTROL_MASTER = '0'
Warp-Invoke-EnhancedSsh -SshArgs @('remote-host', 'two words') 6>$null
$fallbackStatus = $global:LASTEXITCODE
$bootstrapCommand = Warp-New-RemoteBootstrapCommand -RemoteSessionId 42 -SshHookHex '7B7D'
$capabilityProbeCommand = Warp-New-PowerShellCapabilityProbeCommand
$encodedCapabilityProbe = ($capabilityProbeCommand -split ' ')[-1]
$decodedCapabilityProbe = [Text.Encoding]::Unicode.GetString(
    [Convert]::FromBase64String($encodedCapabilityProbe)
)
$remoteBootstrapSyntax = @()
foreach ($shell in @('bash', 'zsh')) {
    $shellCommand = Get-Command -Name $shell -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -ne $shellCommand) {
        & $shellCommand.Source -n -c $bootstrapCommand
        $remoteBootstrapSyntax += $global:LASTEXITCODE -eq 0
    }
}
ConvertTo-Json -Compress -Depth 4 -InputObject @{
    interactive = $interactive
    remote_shell = Warp-Get-RemoteShellFromProbeOutput @('banner', "__WARP_REMOTE_SHELL__/usr/bin/zsh`r`n")
    supported_shells = @(
        [bool](Warp-Test-RemoteShellSupportsBootstrap '/bin/bash'),
        [bool](Warp-Test-RemoteShellSupportsBootstrap '/usr/bin/zsh'),
        [bool](Warp-Test-RemoteShellSupportsBootstrap '/bin/BASH'),
        [bool](Warp-Test-RemoteShellSupportsBootstrap '$SHELL'),
        [bool](Warp-Test-RemoteShellSupportsBootstrap '')
    )
    passthrough = $passthrough
    windows_probe_args = $script:windowsProbeArgs
    windows_bootstrap_args = $script:windowsBootstrapArgs
    windows_status = $windowsStatus
    wrapper_uses_rust_worker = [bool]((Get-Content -Raw -LiteralPath $env:WARP_TEST_SSH_HELPER).Contains('rust-ssh-session'))
    fallback_ssh_args = $script:fallbackSshArgs
    capability_probe_args = $script:capabilityProbeArgs
    capability_probe_shell = Warp-Get-PowerShellCapabilityFromProbeOutput '__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell'
    capability_probe_rejections = @(
        [bool][String]::IsNullOrEmpty((Warp-Get-PowerShellCapabilityFromProbeOutput '__WARP_REMOTE_CAPS__v=2;os=windows;shell=powershell')),
        [bool][String]::IsNullOrEmpty((Warp-Get-PowerShellCapabilityFromProbeOutput "banner`n__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell")),
        [bool][String]::IsNullOrEmpty((Warp-Get-PowerShellCapabilityFromProbeOutput '__WARP_REMOTE_CAPS__v=1;os=Windows;shell=powershell'))
    )
    decoded_capability_probe = $decodedCapabilityProbe
    fallback_status = $fallbackStatus
    closed_owned_control_master = $script:closedOwnedControlMaster
    remote_bootstrap_syntax = $remoteBootstrapSyntax
}
"#;

    let output = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("WARP_TEST_SSH_HELPER", helper_path)
        .output()
        .expect("PowerShell 应能执行 SSH wrapper 的纯逻辑测试");
    assert!(
        output.status.success(),
        "PowerShell SSH wrapper 测试失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("PowerShell 应输出有效 JSON");
    assert_eq!(
        result["interactive"],
        json!([
            true, true, true, true, false, false, false, false, false, false, false, false, false,
            false, false, false
        ])
    );
    assert_eq!(result["remote_shell"], json!("/usr/bin/zsh"));
    assert_eq!(
        result["supported_shells"],
        json!([true, true, false, false, false])
    );
    assert_eq!(
        result["passthrough"],
        json!([
            "host",
            "",
            "two words",
            "quote\"value",
            "$literal",
            "*.rs",
            "中文"
        ])
    );
    assert_eq!(result["windows_probe_args"], json!([]));
    let windows_bootstrap_args = result["windows_bootstrap_args"]
        .as_array()
        .expect("Windows PowerShell SSH bootstrap 参数应为数组");
    assert_eq!(
        &windows_bootstrap_args[..],
        &[
            json!("windows-host"),
            json!(""),
            json!("two words"),
            json!("$literal")
        ]
    );
    assert_eq!(result["windows_status"], json!(17));
    assert_eq!(result["wrapper_uses_rust_worker"], json!(true));
    let fallback_args = result["fallback_ssh_args"]
        .as_array()
        .expect("降级 SSH 参数应为数组");
    assert_eq!(fallback_args[0], json!("-o"));
    assert_eq!(fallback_args[1], json!("ControlMaster=no"));
    assert_eq!(fallback_args[2], json!("-o"));
    assert!(
        fallback_args[3]
            .as_str()
            .expect("降级 ControlPath 应为字符串")
            .ends_with("99")
    );
    assert_eq!(fallback_args[4], json!("-t"));
    assert_eq!(
        &fallback_args[5..7],
        &[json!("remote-host"), json!("two words")]
    );
    assert!(
        fallback_args[7]
            .as_str()
            .expect("Windows bootstrap 命令应为字符串")
            .starts_with("powershell.exe -NoLogo -NoExit -EncodedCommand ")
    );
    assert_eq!(result["fallback_status"], json!(23));
    let capability_probe_args = result["capability_probe_args"]
        .as_array()
        .expect("Windows 能力探测参数应为数组");
    assert_eq!(capability_probe_args[0], json!("-o"));
    assert_eq!(capability_probe_args[1], json!("ControlMaster=no"));
    assert!(
        capability_probe_args
            .last()
            .and_then(|value| value.as_str())
            .expect("能力探测命令应为字符串")
            .starts_with("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
    );
    assert_eq!(result["capability_probe_shell"], json!("powershell"));
    assert_eq!(
        result["capability_probe_rejections"],
        json!([true, true, true])
    );
    let decoded_capability_probe = result["decoded_capability_probe"]
        .as_str()
        .expect("能力探测 payload 应为字符串");
    assert!(decoded_capability_probe.contains("__WARP_REMOTE_CAPS__v=1"));
    assert!(decoded_capability_probe.contains("os={0};shell=powershell"));
    assert_eq!(result["closed_owned_control_master"], json!(false));
    let remote_bootstrap_syntax = result["remote_bootstrap_syntax"]
        .as_array()
        .expect("远端 bootstrap 语法检查结果应为数组");
    assert!(
        remote_bootstrap_syntax
            .iter()
            .all(|value| value == &json!(true)),
        "远端 bootstrap 语法检查失败: {remote_bootstrap_syntax:?}"
    );
}

fn powershell_executable() -> Option<&'static str> {
    #[cfg(windows)]
    let candidates = ["powershell.exe", "pwsh.exe"];
    #[cfg(not(windows))]
    let candidates = ["pwsh"];

    candidates.into_iter().find(|candidate| {
        Command::new(candidate)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-Command",
                "$PSVersionTable.PSVersion.Major",
            ])
            .output()
            .is_ok()
    })
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
