//! Windows remote-server 的 PowerShell 命令生成。

const PLATFORM_PROBE_SCRIPT: &str = r#"$architecture = if ($env:PROCESSOR_ARCHITEW6432) {
    $env:PROCESSOR_ARCHITEW6432
} else {
    $env:PROCESSOR_ARCHITECTURE
}
[Console]::Out.WriteLine(('Windows {0}' -f $architecture))"#;

const INSTALL_SCRIPT_TEMPLATE: &str = r#"$ErrorActionPreference = 'Stop'
$installRelativeDir = {install_relative_dir}
$binaryName = {binary_name}
$archiveUrl = {archive_url}
$stagingArchive = {staging_archive}
$installDir = Join-Path $HOME $installRelativeDir
$destinationBinary = Join-Path $installDir $binaryName

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$temporaryDir = Join-Path $installDir ('.install-{0}' -f [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $temporaryDir | Out-Null

try {
    $archivePath = Join-Path $temporaryDir 'infinishell.zip'
    if ($null -ne $stagingArchive -and $stagingArchive.Length -gt 0) {
        if ($stagingArchive -eq '~') {
            $stagingArchive = $HOME
        } elseif ($stagingArchive.StartsWith('~/') -or $stagingArchive.StartsWith('~\')) {
            $stagingArchive = Join-Path $HOME $stagingArchive.Substring(2)
        }
        Move-Item -LiteralPath $stagingArchive -Destination $archivePath -Force
    } else {
        Invoke-WebRequest -UseBasicParsing -Uri $archiveUrl -OutFile $archivePath
    }

    Expand-Archive -LiteralPath $archivePath -DestinationPath $temporaryDir -Force
    $sourceBinary = Join-Path $temporaryDir 'infinishell.exe'
    if (-not (Test-Path -LiteralPath $sourceBinary -PathType Leaf)) {
        throw 'infinishell.exe was not found in the remote-server archive'
    }

    $sourceResources = Join-Path $temporaryDir 'resources'
    if (Test-Path -LiteralPath $sourceResources -PathType Container) {
        $destinationResources = Join-Path $installDir {bundled_resources_dir_name}
        if (Test-Path -LiteralPath $destinationResources) {
            Remove-Item -LiteralPath $destinationResources -Recurse -Force
        }
        Move-Item -LiteralPath $sourceResources -Destination $destinationResources
    }

    Move-Item -LiteralPath $sourceBinary -Destination $destinationBinary -Force
    & $destinationBinary --version
    if ($LASTEXITCODE -ne 0) {
        throw ('installed remote-server exited with code {0}' -f $LASTEXITCODE)
    }
} finally {
    if (Test-Path -LiteralPath $temporaryDir) {
        Remove-Item -LiteralPath $temporaryDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}"#;

pub(super) fn platform_probe_script() -> String {
    PLATFORM_PROBE_SCRIPT.to_string()
}

pub(super) fn binary_check_script(relative_binary_path: &str) -> String {
    let relative_binary_path = single_quoted(relative_binary_path);
    format!(
        "$binaryPath = Join-Path $HOME {relative_binary_path}; if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {{ exit 1 }}; & $binaryPath --version; exit $LASTEXITCODE"
    )
}

pub(super) fn removal_script(relative_binary_path: &str) -> String {
    let relative_binary_path = single_quoted(relative_binary_path);
    format!(
        "$binaryPath = Join-Path $HOME {relative_binary_path}; Remove-Item -LiteralPath $binaryPath -Force -ErrorAction SilentlyContinue"
    )
}

pub(super) fn proxy_script(relative_binary_path: &str, identity_key: &str) -> String {
    let relative_binary_path = single_quoted(relative_binary_path);
    let identity_key = single_quoted(identity_key);
    format!(
        "$binaryPath = Join-Path $HOME {relative_binary_path}; & $binaryPath remote-server-proxy --identity-key {identity_key}; exit $LASTEXITCODE"
    )
}

pub(super) fn install_script(
    install_relative_dir: &str,
    binary_name: &str,
    archive_url: &str,
    staging_archive: Option<&str>,
    bundled_resources_dir_name: &str,
) -> String {
    let staging_archive = match staging_archive {
        Some(path) => single_quoted(path),
        None => "$null".to_string(),
    };
    INSTALL_SCRIPT_TEMPLATE
        .replace(
            "{install_relative_dir}",
            &single_quoted(install_relative_dir),
        )
        .replace("{binary_name}", &single_quoted(binary_name))
        .replace("{archive_url}", &single_quoted(archive_url))
        .replace("{staging_archive}", &staging_archive)
        .replace(
            "{bundled_resources_dir_name}",
            &single_quoted(bundled_resources_dir_name),
        )
}

fn single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
