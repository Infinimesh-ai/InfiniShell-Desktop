#[path = "installation/scp_fallback.rs"]
pub(super) mod scp_fallback;

use std::path::Path;

use anyhow::Result;
use remote_server::ssh::SshCommandError;
use remote_server::transport::{Error, InstallOutcome, InstallSource};
use warp_core::safe_warn;

/// 返回适合目标平台的本地安装包，优先使用随应用分发的资源，再回退到缓存/下载。
pub(crate) async fn client_tarball_for_platform(
    platform: &remote_server::setup::RemotePlatform,
) -> Result<std::path::PathBuf, Error> {
    scp_fallback::client_tarball_for_platform(platform).await
}

/// 运行 SSH remote-server 安装流程。优先上传安装包内置的 tarball；内置资源
/// 不可用时才让远端直接下载，最后回退到客户端缓存/下载后通过 SCP 上传。
pub(super) async fn install_binary(socket_path: &Path) -> InstallOutcome {
    let binary_path = remote_server::setup::remote_server_binary();
    log::info!("Installing remote server binary to {binary_path}");
    let mut outcome = match scp_fallback::install_bundled(socket_path).await {
        Some(Ok(())) => InstallOutcome {
            source: Some(InstallSource::Client),
            result: Ok(()),
        },
        Some(Err(error)) if !scp_fallback::should_try_install(&error) => InstallOutcome {
            source: Some(InstallSource::Client),
            result: Err(error),
        },
        Some(Err(error)) => {
            safe_warn!(
                safe: ("Bundled remote-server install failed; falling back to download"),
                full: ("Bundled remote-server install failed; falling back to download: {error:#}")
            );
            install_with_download_fallback(socket_path).await
        }
        None => install_with_download_fallback(socket_path).await,
    };

    // Post-install verification: confirm the binary actually landed at the
    // expected path and is functional. This catches silent install failures
    // that would otherwise surface as a cryptic IPC handshake error.
    if outcome.result.is_ok() {
        log::info!("Running post-install verification for {binary_path}");
        let check_cmd = remote_server::setup::binary_check_command();
        let verify = remote_server::ssh::run_ssh_command(
            socket_path,
            &check_cmd,
            remote_server::setup::CHECK_TIMEOUT,
        )
        .await;
        match verify {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let code = output.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                outcome.result = Err(Error::Other(anyhow::anyhow!(
                    "Post-install verification failed: binary not found or not \
                     executable at {binary_path} (exit {code}): {stderr}"
                )));
            }
            Err(e) => {
                outcome.result = Err(Error::Other(anyhow::anyhow!(
                    "Post-install verification failed: {e}"
                )));
            }
        }
    }

    outcome
}

async fn install_with_download_fallback(socket_path: &Path) -> InstallOutcome {
    match install_on_server(socket_path).await {
        Ok(()) => InstallOutcome {
            source: Some(InstallSource::Server),
            result: Ok(()),
        },
        Err(server_err) => {
            if scp_fallback::should_try_install(&server_err) {
                log::info!("Remote server install failed; falling back to SCP upload");
                match scp_fallback::install(socket_path).await {
                    Ok(()) => InstallOutcome {
                        source: Some(InstallSource::Client),
                        result: Ok(()),
                    },
                    Err(e) => InstallOutcome {
                        source: Some(InstallSource::Client),
                        result: Err(e),
                    },
                }
            } else {
                InstallOutcome {
                    source: Some(InstallSource::Server),
                    result: Err(server_err),
                }
            }
        }
    }
}

/// Runs the install script on the remote host to download and install the
/// binary directly from the CDN.
async fn install_on_server(socket_path: &Path) -> Result<(), Error> {
    let script = remote_server::setup::install_script(None);
    match remote_server::ssh::run_ssh_script(
        socket_path,
        &script,
        remote_server::setup::INSTALL_TIMEOUT,
    )
    .await
    {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(Error::ScriptFailed { exit_code, stderr })
        }
        Err(SshCommandError::TimedOut { .. }) => Err(Error::TimedOut),
        Err(e) => Err(Error::Other(e.into())),
    }
}
