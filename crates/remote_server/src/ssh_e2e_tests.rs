use std::collections::HashSet;
use std::time::Duration;

use base64::Engine as _;
use warpui_core::r#async::{FutureExt as _, executor};

use crate::client::{InitializeParams, RemoteServerClient};
use crate::proto::write_file_chunk_response;

const SSH_HOST_ENV: &str = "WARP_WINDOWS_SSH_E2E_HOST";
const REMOTE_BINARY_ENV: &str = "WARP_WINDOWS_SSH_E2E_REMOTE_BINARY";
const LOCAL_ARCHIVE_ENV: &str = "WARP_WINDOWS_SSH_E2E_LOCAL_ARCHIVE";
const INSTALLED_REMOTE_BINARY: &str = r"%USERPROFILE%\.infinishell\remote-server\infinishell.exe";

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("运行测试前必须设置 {name}"))
}

fn powershell_encoded_command(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn powershell_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn ssh_powershell_command(host: &str, script: &str) -> command::r#async::Command {
    let encoded = powershell_encoded_command(script);
    let mut command = command::r#async::Command::new("ssh");
    command
        .arg("-T")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg(host)
        .arg("powershell.exe")
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-EncodedCommand")
        .arg(encoded);
    command
}

fn ssh_command(host: &str, remote_command: &str) -> command::r#async::Command {
    let mut command = command::r#async::Command::new("ssh");
    command
        .arg("-T")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg(host)
        .arg(remote_command);
    command
}

async fn remote_process_ids(host: &str, remote_binary: &str) -> HashSet<u32> {
    let script = format!(
        "$binary = [Environment]::ExpandEnvironmentVariables('{}'); Get-Process | Where-Object {{ $_.Path -eq $binary }} | ForEach-Object {{ $_.Id }}",
        powershell_literal(remote_binary),
    );
    let output = ssh_powershell_command(host, &script)
        .output()
        .with_timeout(Duration::from_secs(20))
        .await
        .expect("枚举 Windows 测试进程不应超时")
        .expect("枚举 Windows 测试进程命令应退出");
    assert!(
        output.status.success(),
        "枚举 Windows 测试进程失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

async fn stop_test_daemon(host: &str, remote_binary: &str, baseline_process_ids: &HashSet<u32>) {
    let current_process_ids = remote_process_ids(host, remote_binary).await;
    let test_process_ids = current_process_ids
        .difference(baseline_process_ids)
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    if test_process_ids.is_empty() {
        return;
    }
    let script = format!(
        "Get-Process -Id {test_process_ids} -ErrorAction SilentlyContinue | Stop-Process -Force"
    );
    let result = ssh_powershell_command(host, &script)
        .output()
        .with_timeout(Duration::from_secs(20))
        .await;
    assert!(
        matches!(result, Ok(Ok(ref output)) if output.status.success()),
        "清理 Windows 测试 daemon 失败: {result:?}"
    );
}

async fn upload_archive(host: &str, local_archive: &str, remote_archive: &str) {
    let script = format!(
        "$path = Join-Path $HOME '{}'; New-Item -ItemType Directory -Force -Path (Split-Path -Parent $path) | Out-Null",
        powershell_literal(remote_archive),
    );
    let setup = ssh_powershell_command(host, &script)
        .output()
        .with_timeout(Duration::from_secs(20))
        .await
        .expect("Windows 上传目录准备不应超时")
        .expect("Windows 上传目录准备命令应退出");
    assert!(
        setup.status.success(),
        "Windows 上传目录准备失败: {}",
        String::from_utf8_lossy(&setup.stderr)
    );

    let host = host.to_owned();
    let local_archive = local_archive.to_owned();
    let destination = format!("{host}:{remote_archive}");
    let output = tokio::task::spawn_blocking(move || {
        command::blocking::Command::new("scp")
            .arg("-q")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=10")
            .arg(local_archive)
            .arg(destination)
            .output()
    })
    .with_timeout(crate::setup::SCP_INSTALL_TIMEOUT)
    .await
    .expect("Windows 归档上传不应超过生产上限")
    .expect("Windows 归档上传任务不应 panic")
    .expect("Windows 归档上传命令应退出");
    assert!(
        output.status.success(),
        "Windows 归档上传失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn install_archive(host: &str, remote_archive: &str) {
    let platform = crate::setup::RemotePlatform {
        os: crate::setup::RemoteOs::Windows,
        arch: crate::setup::RemoteArch::X86_64,
    };
    let remote_archive = format!("~/{remote_archive}");
    let command = crate::setup::install_command(&platform, Some(&remote_archive));
    assert_eq!(
        command.dialect,
        crate::setup::RemoteShellDialect::PowerShell
    );
    let output = ssh_powershell_command(host, &command.script)
        .output()
        .with_timeout(crate::setup::INSTALL_TIMEOUT)
        .await
        .expect("Windows 归档安装不应超过生产上限")
        .expect("Windows 归档安装命令应退出");
    assert!(
        output.status.success(),
        "Windows 归档安装失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn initialize_round_trip(host: &str, remote_binary: &str) {
    let baseline_process_ids = remote_process_ids(host, remote_binary).await;
    let identity_key = format!("ssh-e2e-{}", uuid::Uuid::new_v4());
    let remote_command =
        format!("\"{remote_binary}\" remote-server-proxy --identity-key \"{identity_key}\"",);

    let mut child = ssh_command(host, &remote_command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("应能启动 SSH proxy 子进程");
    let stdin = child.stdin.take().expect("SSH proxy 应提供 stdin");
    let stdout = child.stdout.take().expect("SSH proxy 应提供 stdout");
    let stderr = child.stderr.take().expect("SSH proxy 应提供 stderr");
    let background_executor = executor::Background::default();
    let (client, _event_rx, _failure_rx, _host_response_rx, stderr_tail) =
        RemoteServerClient::from_child_streams(stdin, stdout, stderr, &background_executor);

    let initialize_params = || InitializeParams {
        user_id: String::new(),
        user_email: String::new(),
        crash_reporting_enabled: false,
        codebase_index_limits: None,
    };
    let result = client
        .initialize(None, initialize_params())
        .with_timeout(Duration::from_secs(45))
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result.map_err(|error| error.to_string()));

    let response = result.unwrap_or_else(|error| {
        let stderr = stderr_tail.drain().unwrap_or_default();
        panic!("Windows SSH Initialize 失败: {error}; proxy stderr: {stderr}")
    });
    assert!(
        !response.host_id.is_empty(),
        "InitializeResponse 应包含 host_id"
    );

    let second_response = client
        .initialize(None, initialize_params())
        .with_timeout(Duration::from_secs(15))
        .await
        .expect("同一 Windows SSH 连接的第二次 Initialize 不应超时")
        .expect("同一 Windows SSH 连接的第二次 Initialize 应成功");
    assert_eq!(
        second_response.host_id, response.host_id,
        "同一 daemon 的两次 Initialize 应返回相同 host_id"
    );

    let write_path = format!(
        "~/.infinishell/remote-server/ssh-e2e-write-{}.txt",
        uuid::Uuid::new_v4()
    );
    let write_content = format!("write-file-after-initialize-{identity_key}");
    let write_response = client
        .write_file_chunk(
            write_path.clone(),
            0,
            write_content.as_bytes().to_vec(),
            true,
            None,
        )
        .with_timeout(Duration::from_secs(30))
        .await
        .expect("Windows WriteFileChunk 响应不应超时")
        .expect("Windows WriteFileChunk 应成功");
    match write_response.result {
        Some(write_file_chunk_response::Result::Success(success)) => {
            assert_eq!(success.next_offset, write_content.len() as u64);
        }
        Some(write_file_chunk_response::Result::Error(error)) => {
            panic!("Windows WriteFileChunk 失败: {}", error.message);
        }
        None => panic!("Windows WriteFileChunk 响应不应缺少 result"),
    }

    let verify_script = format!(
        "$path = [Environment]::ExpandEnvironmentVariables('{}').Replace('~', $HOME); $content = Get-Content -LiteralPath $path -Raw; Remove-Item -LiteralPath $path -Force; if ($content -cne '{}') {{ exit 1 }}",
        powershell_literal(&write_path),
        powershell_literal(&write_content),
    );
    let verify_output = ssh_powershell_command(host, &verify_script)
        .output()
        .with_timeout(Duration::from_secs(20))
        .await
        .expect("验证 Windows WriteFile 落盘不应超时")
        .expect("验证 Windows WriteFile 落盘命令应退出");
    assert!(
        verify_output.status.success(),
        "Windows WriteFileChunk 内容或路径不正确: {}",
        String::from_utf8_lossy(&verify_output.stderr)
    );

    drop(client);
    drop(child);
    stop_test_daemon(host, remote_binary, &baseline_process_ids).await;
}

/// 真实走通 client → SSH exec channel → Windows proxy → named pipe daemon → Initialize → WriteFileChunk。
///
/// 该测试默认忽略，避免工作区测试依赖外部主机。运行前先在 Windows 测试机用当前源码
/// 构建可执行文件，再设置：
///
/// `WARP_WINDOWS_SSH_E2E_HOST=<ssh-host>`
/// `WARP_WINDOWS_SSH_E2E_REMOTE_BINARY=<dedicated-windows-exe-path>`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn windows_ssh_proxy_daemon_initialize_round_trip() {
    let host = required_env(SSH_HOST_ENV);
    let remote_binary = required_env(REMOTE_BINARY_ENV);
    initialize_round_trip(&host, &remote_binary).await;
}

/// 使用客户端安装包先安装，再走完整的 Initialize → WriteFileChunk 链路。
///
/// 该测试覆盖归档内容、生产 PowerShell 安装脚本和安装后协议，不依赖已经
/// 初始化前的扩展 `WriteFile` RPC。Rust broker 的 SCP 上传由 app 侧测试覆盖；
/// Initialize 后再验证文件浏览器同款 `WriteFileChunk` 可以安全承载 PowerShell shell bootstrap。
/// 运行时额外设置：
///
/// `WARP_WINDOWS_SSH_E2E_LOCAL_ARCHIVE=<infinishell-windows-x86_64.zip>`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn windows_ssh_archive_install_and_initialize_round_trip() {
    let host = required_env(SSH_HOST_ENV);
    let local_archive = required_env(LOCAL_ARCHIVE_ENV);
    let remote_archive = format!(
        ".infinishell/remote-server/infinishell-upload-{}.zip",
        uuid::Uuid::new_v4()
    );

    upload_archive(&host, &local_archive, &remote_archive).await;
    install_archive(&host, &remote_archive).await;
    initialize_round_trip(&host, INSTALLED_REMOTE_BINARY).await;
}
