use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use futures::{AsyncReadExt as _, AsyncWriteExt as _, TryStreamExt as _};
use http_client::StatusCode;
use remote_server::setup::RemotePlatform;
use remote_server::transport::Error;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use warp_core::safe_warn;

const BUNDLED_REMOTE_SERVER_DIR_NAME: &str = "remote-server";
const BUNDLED_REMOTE_SERVER_MANIFEST_NAME: &str = "manifest.json";
const REMOTE_SERVER_TARBALL_CACHE_FILE_NAME: &str = "infinishell.tar.gz";

const REMOTE_SERVER_TARBALL_DOWNLOAD_ATTEMPTS: usize = 3;
// The local SCP fallback download can run over slow or captive networks. Match
// the install-script timeout so slow client-side downloads have the same budget
// as remote-host downloads.
const REMOTE_SERVER_TARBALL_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

// Keep retry backoff short because retries only cover transient HTTP failures;
// the longer timeout above handles slow successful downloads.
const REMOTE_SERVER_TARBALL_DOWNLOAD_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Exit codes where SCP fallback would not help because the failure is on the
/// remote host itself, not a network/download issue.
pub(super) fn should_try_install(error: &Error) -> bool {
    !matches!(error, Error::ScriptFailed { exit_code, .. } if *exit_code == 2)
}

#[derive(Deserialize)]
struct BundledRemoteServerManifest {
    version: String,
    artifacts: Vec<BundledRemoteServerArtifact>,
}

#[derive(Debug, Deserialize)]
struct BundledRemoteServerArtifact {
    os: String,
    arch: String,
    file: String,
    sha256: String,
}

/// 尝试使用安装包内置的 tarball。`None` 表示当前安装包没有可用内置资源，
/// 调用方应继续走远端下载路径。
pub(super) async fn install_bundled(socket_path: &Path) -> Option<Result<(), Error>> {
    let platform = match super::super::detect_remote_platform(socket_path).await {
        Ok(platform) => platform,
        Err(error) => return Some(Err(error)),
    };
    let client_tarball_path = match bundled_remote_server_tarball(&platform).await {
        Ok(Some(path)) => path,
        Ok(None) => return None,
        Err(error) => {
            safe_warn!(
                safe: ("Bundled remote-server tarball is unavailable; falling back to download"),
                full: ("Bundled remote-server tarball is unavailable; falling back to download: {error:#}")
            );
            return None;
        }
    };

    log::info!("Installing remote server from bundled tarball");
    Some(install_tarball(socket_path, &client_tarball_path).await)
}

/// Installs the remote server via SCP fallback.
///
/// The tarball is downloaded or reused from the local cache first, then uploaded
/// to the remote host and passed to the install script as an already-downloaded
/// archive. This avoids requiring the remote host to download the tarball itself.
pub(super) async fn install(socket_path: &Path) -> Result<(), Error> {
    let platform = super::super::detect_remote_platform(socket_path).await?;

    let client_tarball_path = cached_remote_server_tarball(&platform)
        .await
        .map_err(Error::Other)?;
    install_tarball(socket_path, &client_tarball_path).await
}

async fn install_tarball(socket_path: &Path, client_tarball_path: &Path) -> Result<(), Error> {
    let timeout = remote_server::setup::SCP_INSTALL_TIMEOUT;
    let install_dir = remote_server::setup::remote_server_dir();
    let remote_tarball_name = format!("infinishell-upload-{}.tar.gz", uuid::Uuid::new_v4());
    let remote_tarball_path = format!("{install_dir}/{remote_tarball_name}");

    // The normal install script creates this directory before downloading, but
    // SCP fallback can run after a failure that happened before that point.
    // Ensure the destination exists before uploading the staged tarball.
    let mkdir_output = remote_server::ssh::run_ssh_command(
        socket_path,
        &format!("mkdir -p {install_dir}"),
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await
    .map_err(Error::from)?;
    if !mkdir_output.status.success() {
        let code = mkdir_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr).to_string();
        return Err(Error::ScriptFailed {
            exit_code: code,
            stderr,
        });
    }

    log::info!("Uploading tarball to remote at {remote_tarball_path}");
    remote_server::ssh::scp_upload(
        socket_path,
        client_tarball_path,
        &remote_tarball_path,
        timeout,
    )
    .await
    .map_err(Error::Other)?;

    log::info!("Running extraction via install script with tarball at {remote_tarball_path}");
    let script = remote_server::setup::install_script(Some(&remote_tarball_path));

    let output = remote_server::ssh::run_ssh_script(socket_path, &script, timeout)
        .await
        .map_err(Error::from)?;
    if output.status.success() {
        Ok(())
    } else {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(Error::ScriptFailed {
            exit_code: code,
            stderr,
        })
    }
}

async fn bundled_remote_server_tarball(
    platform: &RemotePlatform,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(resources_dir) = warp_core::paths::bundled_resources_dir() else {
        return Ok(None);
    };
    let bundled_dir = resources_dir.join(BUNDLED_REMOTE_SERVER_DIR_NAME);
    let manifest_path = bundled_dir.join(BUNDLED_REMOTE_SERVER_MANIFEST_NAME);
    let manifest_json = match async_fs::read_to_string(&manifest_path).await {
        Ok(manifest_json) => manifest_json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read bundled remote-server manifest '{}'",
                    manifest_path.display()
                )
            });
        }
    };
    let manifest: BundledRemoteServerManifest = serde_json::from_str(&manifest_json)
        .context("Failed to parse bundled remote-server manifest")?;
    let expected_version = remote_server::setup::remote_server_artifact_version();
    let artifact = select_bundled_artifact(&manifest, platform, expected_version)?;

    let tarball_path = bundled_dir.join(&artifact.file);
    verify_tarball_sha256(&tarball_path, &artifact.sha256).await?;
    Ok(Some(tarball_path))
}

fn select_bundled_artifact<'a>(
    manifest: &'a BundledRemoteServerManifest,
    platform: &RemotePlatform,
    expected_version: &str,
) -> anyhow::Result<&'a BundledRemoteServerArtifact> {
    anyhow::ensure!(
        manifest.version == expected_version,
        "Bundled remote-server manifest version mismatch: expected '{expected_version}', got '{}'",
        manifest.version
    );

    let expected_file = remote_server::setup::remote_server_tarball_name(platform);
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| {
            artifact.os == platform.os.as_str() && artifact.arch == platform.arch.as_str()
        })
        .with_context(|| {
            format!(
                "Bundled remote-server manifest has no artifact for {}-{}",
                platform.os.as_str(),
                platform.arch.as_str()
            )
        })?;
    anyhow::ensure!(
        artifact.file == expected_file,
        "Bundled remote-server manifest has unexpected artifact file '{}'",
        artifact.file
    );
    Ok(artifact)
}

async fn verify_tarball_sha256(path: &Path, expected_sha256: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        expected_sha256.len() == 64 && expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid SHA-256 value in bundled remote-server manifest"
    );

    let mut file = async_fs::File::open(path)
        .await
        .with_context(|| format!("Failed to open bundled tarball '{}'", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes_read = file
            .read(&mut buffer)
            .await
            .with_context(|| format!("Failed to read bundled tarball '{}'", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    let actual_sha256 = hex::encode(hasher.finalize());
    anyhow::ensure!(
        actual_sha256.eq_ignore_ascii_case(expected_sha256),
        "Bundled remote-server tarball SHA-256 mismatch"
    );
    Ok(())
}

fn remote_server_tarball_cache_root() -> PathBuf {
    warp_core::paths::cache_dir()
        .join("remote-server")
        .join("tarballs")
}

fn remote_server_tarball_cache_temp_dir() -> PathBuf {
    remote_server_tarball_cache_root().join(".tmp")
}

fn current_remote_server_tarball_cache_version() -> &'static str {
    remote_server::setup::remote_server_artifact_version()
}

fn remote_server_tarball_cache_path(platform: &RemotePlatform) -> PathBuf {
    remote_server_tarball_cache_root()
        .join(current_remote_server_tarball_cache_version())
        .join(format!(
            "{}-{}",
            platform.os.as_str(),
            platform.arch.as_str()
        ))
        .join(REMOTE_SERVER_TARBALL_CACHE_FILE_NAME)
}

async fn is_valid_cached_tarball(path: &Path) -> bool {
    async_fs::metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

/// Returns a local tarball for the remote platform.
///
/// Reuses an existing cached tarball when available; otherwise downloads the
/// tarball into the cache and returns the newly cached path.
async fn cached_remote_server_tarball(platform: &RemotePlatform) -> anyhow::Result<PathBuf> {
    let cache_path = remote_server_tarball_cache_path(platform);
    if is_valid_cached_tarball(&cache_path).await {
        log::info!(
            "Using cached remote-server tarball at {}",
            cache_path.display()
        );
        return Ok(cache_path);
    }

    if async_fs::metadata(&cache_path).await.is_ok() {
        let _ = async_fs::remove_file(&cache_path).await;
    }

    let url = remote_server::setup::download_tarball_url(platform);
    log::info!(
        "Downloading remote-server tarball from {url} into cache at {}",
        cache_path.display()
    );
    download_remote_server_tarball_to_cache(&url, &cache_path).await?;
    Ok(cache_path)
}

async fn download_remote_server_tarball_to_cache(
    url: &str,
    cache_path: &Path,
) -> anyhow::Result<()> {
    let parent = cache_path
        .parent()
        .context("remote-server tarball cache path has no parent directory")?;
    async_fs::create_dir_all(parent).await.with_context(|| {
        format!(
            "Failed to create remote-server tarball cache directory '{}'",
            parent.display()
        )
    })?;
    let temp_dir = remote_server_tarball_cache_temp_dir();
    async_fs::create_dir_all(&temp_dir).await.with_context(|| {
        format!(
            "Failed to create remote-server tarball cache temp directory '{}'",
            temp_dir.display()
        )
    })?;

    // Download into a unique temp path first so a failed or partial download
    // never appears at the shared cache path that other installs may reuse.
    let temp_path = temp_dir.join(format!(
        ".{REMOTE_SERVER_TARBALL_CACHE_FILE_NAME}.{}.tmp",
        uuid::Uuid::new_v4()
    ));

    if let Err(e) = download_remote_server_tarball_with_retries(url, &temp_path).await {
        let _ = async_fs::remove_file(&temp_path).await;
        return Err(e);
    }
    if !is_valid_cached_tarball(&temp_path).await {
        let _ = async_fs::remove_file(&temp_path).await;
        anyhow::bail!("Downloaded remote-server tarball from {url} was empty");
    }

    if is_valid_cached_tarball(cache_path).await {
        let _ = async_fs::remove_file(&temp_path).await;
        return Ok(());
    }

    // Publish the validated temp file to the shared cache path. If another
    // concurrent fallback populated the cache after the check above, that valid
    // cache hit is good enough for this install, so discard our temp file.
    match async_fs::rename(&temp_path, cache_path).await {
        Ok(()) => Ok(()),
        Err(_e) if is_valid_cached_tarball(cache_path).await => {
            let _ = async_fs::remove_file(&temp_path).await;
            Ok(())
        }
        Err(e) => {
            let _ = async_fs::remove_file(&temp_path).await;
            Err(e).with_context(|| {
                format!(
                    "Failed to move remote-server tarball into cache at '{}'",
                    cache_path.display()
                )
            })
        }
    }
}

async fn download_remote_server_tarball_with_retries(
    url: &str,
    temp_path: &Path,
) -> anyhow::Result<()> {
    let http_client = http_client::Client::new();
    let mut last_retryable_error = None;

    for attempt in 1..=REMOTE_SERVER_TARBALL_DOWNLOAD_ATTEMPTS {
        match download_remote_server_tarball_internal(&http_client, url, temp_path).await {
            Ok(()) => return Ok(()),
            Err(DownloadAttemptError::Permanent(e)) => return Err(e),
            Err(DownloadAttemptError::Retryable(e)) => {
                last_retryable_error = Some(e);
                if attempt < REMOTE_SERVER_TARBALL_DOWNLOAD_ATTEMPTS {
                    log::warn!("Remote-server tarball download attempt {attempt} failed; retrying");
                    tokio::time::sleep(REMOTE_SERVER_TARBALL_DOWNLOAD_RETRY_DELAY).await;
                }
            }
        }
    }

    Err(last_retryable_error.unwrap_or_else(|| {
        anyhow::anyhow!("Remote-server tarball download failed without an error")
    }))
}

enum DownloadAttemptError {
    Retryable(anyhow::Error),
    Permanent(anyhow::Error),
}

async fn download_remote_server_tarball_internal(
    http_client: &http_client::Client,
    url: &str,
    temp_path: &Path,
) -> Result<(), DownloadAttemptError> {
    let response = http_client
        .get(url)
        .timeout(REMOTE_SERVER_TARBALL_DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            DownloadAttemptError::Retryable(anyhow::anyhow!(
                "Failed to download remote-server tarball from {url}: {e}"
            ))
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let error =
            anyhow::anyhow!("Remote-server tarball download failed with status {status}: {body}");
        return if is_retryable_download_status(status) {
            Err(DownloadAttemptError::Retryable(error))
        } else {
            Err(DownloadAttemptError::Permanent(error))
        };
    }

    let mut file = async_fs::File::create(temp_path).await.map_err(|e| {
        DownloadAttemptError::Permanent(anyhow::anyhow!(
            "Failed to create remote-server tarball cache file '{}': {e}",
            temp_path.display()
        ))
    })?;
    let mut bytes_stream = response.bytes_stream();
    while let Some(chunk) = bytes_stream.try_next().await.map_err(|e| {
        DownloadAttemptError::Retryable(anyhow::anyhow!(
            "Failed to read remote-server tarball response body from {url}: {e}"
        ))
    })? {
        file.write_all(&chunk).await.map_err(|e| {
            DownloadAttemptError::Permanent(anyhow::anyhow!(
                "Failed to write remote-server tarball cache file '{}': {e}",
                temp_path.display()
            ))
        })?;
    }
    file.sync_data().await.map_err(|e| {
        DownloadAttemptError::Permanent(anyhow::anyhow!(
            "Failed to sync remote-server tarball cache file '{}': {e}",
            temp_path.display()
        ))
    })?;

    Ok(())
}

fn is_retryable_download_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
}

#[cfg(test)]
#[path = "scp_fallback_tests.rs"]
mod tests;
