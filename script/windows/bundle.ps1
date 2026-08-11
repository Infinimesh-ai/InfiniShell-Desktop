#!/usr/bin/env powershell
#
# Bundle the application for release.

Param (
    # Build dev bundles by default.
    [Switch]$DEBUG_BUILD = $False,

    [Alias('check-only')]
    [Switch]$CHECK_ONLY,
    [ValidateSet('app', 'tui')]
    [String]$ARTIFACT = 'app',

    [ValidateSet('local', 'dev', 'preview', 'stable', 'oss')]
    [String]$CHANNEL = 'dev',

    [Alias('release-tag')]
    [String]$RELEASE_TAG = '',
    [String]$FEATURES = 'release_bundle,crash_reporting,gui',

    # Builds only the InfiniShell binary, skips the installer.
    [Switch]$SKIP_BUILD_INSTALLER = $False,
    # Builds only the installer, skips the InfiniShell binary. Use this if the InfiniShell
    # binary has already been built.
    [Switch]$SKIP_BUILD_BINARY = $False,

    [ValidateSet('x64', 'arm64')]
    [String]$ARCH = '',
    [Switch]$REQUIRE_SIGNATURES = $False,

    # A signtool command for Inno Setup to sign the setup engine and uninstaller.
    # Uses $f as the file placeholder, e.g.:
    #   'signtool.exe sign /fd SHA256 ... $f'
    # When empty, the installer is built without signing.
    [Alias('sign-tool-cmd')]
    [String]$SIGN_TOOL_CMD = ''
)

if ($RELEASE_TAG) {
    $env:GIT_RELEASE_TAG = $RELEASE_TAG
}

# Use provided ARCH parameter if set, otherwise detect from system
if (-not $ARCH) {
    if ($env:PROCESSOR_ARCHITECTURE -eq 'AMD64') {
        $ARCH = 'x64'
    } elseif ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
        $ARCH = 'arm64'
    } else {
        throw "Unsupported processor architecture: $env:PROCESSOR_ARCHITECTURE"
    }
}

if ($ARCH -eq 'arm64') {
    $FILE_ENDING = 'Setup-arm64'
    $PLATFORM_TARGET = 'aarch64-pc-windows-msvc'
} else {
    # If x64, then we just use the filename "InfiniShellSetup.exe" for example
    $FILE_ENDING = 'Setup'
    $PLATFORM_TARGET = 'x86_64-pc-windows-msvc'
}

$ErrorActionPreference = 'Stop'

function Assert-ValidSignature {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute(
        'PSUseCompatibleCommands',
        '',
        Justification = 'Release signature validation only runs on Windows.'
    )]
    param([string] $Path)

    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ("$($signature.Status)" -ne 'Valid') {
        throw "File does not have a valid Authenticode signature: $Path ($($signature.Status))"
    }
}

$WORKSPACE_ROOT_DIR = $(Get-Location).Path
$CARGO_TARGET_DIR = $WORKSPACE_ROOT_DIR + '\target'
$WINDOWS_INSTALLER_DIR = $WORKSPACE_ROOT_DIR + '\script\windows'
$IS_TUI = $ARTIFACT -eq 'tui'

if ($DEBUG_BUILD) {
    $CARGO_PROFILE = 'dev'
} elseif ($IS_TUI -and (("$CHANNEL" -eq 'local') -or ("$CHANNEL" -eq 'dev'))) {
    $CARGO_PROFILE = 'rclida'
} elseif ($IS_TUI) {
    $CARGO_PROFILE = 'rcli'
} elseif (("$CHANNEL" -eq 'local') -or ("$CHANNEL" -eq 'dev')) {
    # For dev bundles, we want to enable debug assertions to
    # catch violations that would otherwise silently pass in
    # a normal release build (e.g. in stable).
    $CARGO_PROFILE = 'rltoda'
} else {
    $CARGO_PROFILE = 'rlto'
}

if ($CARGO_PROFILE -eq 'dev') {
    $CARGO_TARGET_OUTPUT_DIR = "$CARGO_TARGET_DIR" + '\' + $PLATFORM_TARGET + '\debug'
} else {
    $CARGO_TARGET_OUTPUT_DIR = "$CARGO_TARGET_DIR" + '\' + $PLATFORM_TARGET + '\' + "$CARGO_PROFILE"
}
$BUNDLE_ID = "dev.warp.$app_name"

# Update parameters based on the target release channel.
#
# APP_NAME here must match the value used in Rust as the
# application name; see app/src/channel.rs.
#
# WARP_BIN is the name of the binary produced by cargo;
# BINARY_NAME is the desired name of the binary in the final package.
if ("$CHANNEL" -eq 'local') {
    $WARP_BIN = 'warp'
    $BINARY_NAME = 'warp.exe'
    $APP_NAME = 'WarpLocal'
} elseif ("$CHANNEL" -eq 'dev') {
    $WARP_BIN = 'dev'
    $BINARY_NAME = 'dev.exe'
    $APP_NAME = 'WarpDev'
    $FEATURES = "$FEATURES,agent_mode_debug"
} elseif ("$CHANNEL" -eq 'preview') {
    $WARP_BIN = 'preview'
    $BINARY_NAME = 'preview.exe'
    $APP_NAME = 'WarpPreview'
    $FEATURES = "$FEATURES,preview_channel"
} elseif ("$CHANNEL" -eq 'stable') {
    $WARP_BIN = 'stable'
    $BINARY_NAME = 'warp.exe'
    $APP_NAME = 'InfiniShell'
} elseif ("$CHANNEL" -eq 'oss') {
    $WARP_BIN = 'infinishell'
    $BINARY_NAME = 'infinishell.exe'
    $APP_NAME = 'InfiniShell'
    # OSS channel 使用本地 crash reporting,不启用 release 默认特性集合。
    # autoupdate 走 GitHub Release(zerx-lab/warp),仅下载到 Downloads,不调 Inno Setup。
    # NLD 分类器跟随上游:所有 app 渠道统一在下方追加 nld_classifier_v3/nld_heuristic_v2。
    $FEATURES = 'release_bundle,gui,autoupdate'
}

if ($IS_TUI) {
    $WARP_BIN = switch ($CHANNEL) {
        'local' { 'warp-tui' }
        'oss' { 'warp-tui-oss' }
        Default { "warp-tui-$CHANNEL" }
    }
    $BINARY_NAME = "$WARP_BIN.exe"
    $APP_NAME = switch ($CHANNEL) {
        'local' { 'WarpAgentCLI' }
        'dev' { 'WarpAgentCLIDev' }
        'preview' { 'WarpAgentCLIPreview' }
        'stable' { 'WarpAgentCLI' }
        'oss' { 'WarpAgentCLIOss' }
    }
    $CLI_NAME = switch ($CHANNEL) {
        'local' { 'warp' }
        'dev' { 'warp-dev' }
        'preview' { 'warp-preview' }
        'stable' { 'warp' }
        'oss' { 'warp-oss' }
    }
    $INSTALL_DIR_NAME = switch ($CHANNEL) {
        'local' { 'tui-local' }
        'dev' { 'tui-dev' }
        'preview' { 'tui-preview' }
        'stable' { 'tui' }
        'oss' { 'tui-oss' }
    }
    $FEATURES = 'release_bundle,standalone,voice_input'
    if ("$CHANNEL" -ne 'oss') {
        $FEATURES = "$FEATURES,crash_reporting"
    }
} else {
    # All app channels ship the v3 classifier and v2 heuristic.
    $FEATURES = "$FEATURES,nld_classifier_v3,nld_heuristic_v2"
}

$BINARY_PATH = "$CARGO_TARGET_OUTPUT_DIR\$BINARY_NAME"
# AUMID(Windows AppUserModel ID)—— 必须与进程端 `ChannelState::app_id()` 生成的完全一致,
# 否则 Windows ToastNotificationManager 会在 Start Menu 快捷方式 / 进程 AUMID 不匹配时
# 静默吞掉 toast。OSS(InfiniShell)在 `app/src/bin/infinishell.rs` 里是
# `dev.infinishell.InfiniShell`,其他官方 channel 是 `dev.warp.<Name>`。
#
# OSS 的 AUMID 刻意**不**由 $APP_NAME 拼接:organization 段是 `infinishell` 而非 `warp`。
if ("$CHANNEL" -eq 'oss') {
    $AUMID = "dev.infinishell.InfiniShell"
} else {
    $AUMID = "dev.warp.$APP_NAME"
}
$BUNDLE_ID = $AUMID
$INSTALLER_OUTPUT_DIR = "$WINDOWS_INSTALLER_DIR\Output"
$INSTALLER_NAME = "$($APP_NAME)$($FILE_ENDING)"
$INSTALLER_PATH = "$($INSTALLER_OUTPUT_DIR)\$($INSTALLER_NAME).exe"
$PDB_BASENAME = if ($IS_TUI) {
    # rustc normalizes hyphens to underscores in crate names, and MSVC uses
    # that normalized crate name for the PDB even though Cargo exposes the
    # executable under its original hyphenated target name.
    $WARP_BIN.Replace('-', '_')
} else {
    $WARP_BIN
}
$PDB_PATH = "$CARGO_TARGET_OUTPUT_DIR\$PDB_BASENAME.pdb"
$CARGO_PACKAGE = if ($IS_TUI) { 'warp_tui' } else { 'warp' }
$INSTALLER_SCRIPT = if ($IS_TUI) {
    "$WINDOWS_INSTALLER_DIR\tui-installer.iss"
} else {
    "$WINDOWS_INSTALLER_DIR\windows-installer.iss"
}

# The CARGO_FULL_PROFILE environment variable is read by the `cargo` build
# script (`app/build.rs`) to determine where to place `conpty.dll`.
if ($DEBUG_BUILD) {
    $env:CARGO_FULL_PROFILE = 'debug'
} else {
    $env:CARGO_FULL_PROFILE = $CARGO_PROFILE
}

# If we only want to check that compilation will succeed, perform the checks
# then exit.  We use this script to invoke `cargo check` to ensure that we are
# using the same feature flags and profile that we would be using in production.
if ($CHECK_ONLY) {
    cargo check -p $CARGO_PACKAGE --profile "$CARGO_PROFILE" --bin "$WARP_BIN" --features "$FEATURES" --target $PLATFORM_TARGET
    if (-Not $?) {
        Write-Error "Failed to verify InfiniShell $WARP_BIN compilation with profile $CARGO_PROFILE"
        exit 1
    }
    exit 0
}

if (-Not $SKIP_BUILD_BINARY) {
    Write-Output "Building InfiniShell for channel $CHANNEL and bundle id $BUNDLE_ID"
    $env:CARGO_BIN_NAME = $CHANNEL
    # PE 资源里的 ProductName / FileDescription(任务管理器的进程分组名)走展示名。
    # OSS 的展示品牌是 InfiniShell,而 $APP_NAME 仍是安装包资产名,两者需要分开。
    $env:WARP_APP_NAME = if ("$CHANNEL" -eq 'oss') { 'InfiniShell' } else { $APP_NAME }
    cargo build -p $CARGO_PACKAGE --profile "$CARGO_PROFILE" --bin "$WARP_BIN" --features "$FEATURES" --target $PLATFORM_TARGET
    if (-Not $?) {
        Write-Error "Failed to build InfiniShell $WARP_BIN binary with profile $CARGO_PROFILE"
        exit 1
    }

    # If we desire an executable name different from the cargo bin, rename it.
    if ("$WARP_BIN.exe" -ne $BINARY_NAME) {
        $binarySource = "$CARGO_TARGET_OUTPUT_DIR\$WARP_BIN.exe"
        Write-Output "Renaming executable $WARP_BIN.exe to $BINARY_NAME"
        Move-Item -Path "$binarySource" -Destination "$BINARY_PATH" -Force
    }
}

if ($SKIP_BUILD_INSTALLER) {
    # If this is being run within a GitHub action, set an output variable with the
    # location of the binary so it can be referenced by subsequent actions.
    if ($env:GITHUB_ACTIONS -eq 'true') {
        Write-Output '::echo::on'
        "target_profile_dir=$CARGO_TARGET_OUTPUT_DIR" >> "$env:GITHUB_OUTPUT"
        "binary_path=$BINARY_PATH" >> "$env:GITHUB_OUTPUT"
        "pdb_file_path=$PDB_PATH" >> "$env:GITHUB_OUTPUT"
        Write-Output '::echo::off'
    }
    exit 0
}

Write-Output "Built for $ARCH with executable at $BINARY_PATH"

# Prepare bundled resources
$BUNDLED_RESOURCES_DIR = "$CARGO_TARGET_OUTPUT_DIR\resources"
Write-Output "Preparing bundled resources..."
# Only forward --target to the schema generator when the build target is
# runnable on the host; otherwise `cargo run` would try to execute a
# cross-compiled binary (e.g. aarch64-pc-windows-msvc on an x64 runner)
# and fail.
if ($env:PROCESSOR_ARCHITECTURE -eq 'AMD64') {
    $HOST_TARGET = 'x86_64-pc-windows-msvc'
} elseif ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    $HOST_TARGET = 'aarch64-pc-windows-msvc'
} else {
    $HOST_TARGET = ''
}
if ($PLATFORM_TARGET -eq $HOST_TARGET) {
    $SCHEMA_CARGO_TARGET = $PLATFORM_TARGET
} else {
    $SCHEMA_CARGO_TARGET = ''
}
& "$WINDOWS_INSTALLER_DIR\prepare_bundled_resources.ps1" -DestinationDir "$BUNDLED_RESOURCES_DIR" -Channel "$CHANNEL" -CargoProfile "$CARGO_PROFILE" -CargoFeatures "$FEATURES" -CargoTarget "$SCHEMA_CARGO_TARGET"
if (-Not $?) {
    Write-Error 'Failed to prepare bundled resources'
    exit 1
}
if ($IS_TUI) {
    $WINDOWS_ASSETS_DIR = "$WORKSPACE_ROOT_DIR\app\assets\windows\$ARCH"
    $requiredPayloadFiles = @(
        $BINARY_PATH,
        (Join-Path $WINDOWS_ASSETS_DIR 'conpty.dll'),
        (Join-Path $WINDOWS_ASSETS_DIR 'OpenConsole.exe'),
        (Join-Path $WINDOWS_ASSETS_DIR 'vcruntime140.dll'),
        (Join-Path $WINDOWS_ASSETS_DIR 'vcruntime140_1.dll'),
        (Join-Path $WINDOWS_ASSETS_DIR 'msvcp140.dll')
    )
    foreach ($requiredFile in $requiredPayloadFiles) {
        if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
            throw "Required Warp Agent CLI payload file does not exist: $requiredFile"
        }
        if ($REQUIRE_SIGNATURES) {
            Assert-ValidSignature -Path $requiredFile
        }
    }
}

Write-Output 'Building InfiniShell installer'
# Inno Setup `AppId` 决定注册表 Uninstall 条目与升级跟踪键。OSS 下固定为 `infinishell`,
# 避免留在默认的 `warp-terminal-oss` 上。其他 channel 走 .iss 里的默认
# `warp-terminal-{ReleaseChannel}`。
if ("$CHANNEL" -eq 'oss') {
    $INNO_APP_ID = 'infinishell'
} else {
    $INNO_APP_ID = "warp-terminal-$CHANNEL"
}
$ISCC_ARGS = @(
    "$INSTALLER_SCRIPT",
    "/DReleaseChannel=$CHANNEL",
    "/DMyAppExeName=$BINARY_NAME",
    "/DTargetProfileDir=$CARGO_TARGET_OUTPUT_DIR",
    "/DMyAppName=$APP_NAME",
    "/DMyAppVersion=$env:GIT_RELEASE_TAG",
    "/DArch=$ARCH",
    "/DOutputName=$INSTALLER_NAME",
    "/DAppUserModelId=$AUMID",
    "/DInnoAppId=$INNO_APP_ID"
)
if ($IS_TUI) {
    $ISCC_ARGS += @(
        "/DWindowsAssetsDir=$WINDOWS_ASSETS_DIR",
        "/DCLIName=$CLI_NAME",
        "/DInstallDirName=$INSTALL_DIR_NAME"
    )
}
# Also accept the sign tool command via env var
if (-not $SIGN_TOOL_CMD -and $env:SIGN_TOOL_CMD) {
    $SIGN_TOOL_CMD = $env:SIGN_TOOL_CMD
}
if ($SIGN_TOOL_CMD) {
    $ISCC_ARGS += '/DSIGN_TOOL=1'
    $ISCC_ARGS += "/Scodesign=$SIGN_TOOL_CMD"
}
& ISCC @ISCC_ARGS
if (-Not $?) {
    Write-Error "Failed to build $APP_NAME installer"
    exit 1
}

# If this is being run within a GitHub action, set an output variable with the
# location of the installer so it can be referenced by subsequent actions.
if ($env:GITHUB_ACTIONS -eq 'true') {
    Write-Output '::echo::on'
    $INSTALLER_PATH = $INSTALLER_PATH -replace '\\', '/'
    "installer_path=$INSTALLER_PATH" >> "$env:GITHUB_OUTPUT"
    "pdb_file_path=$PDB_PATH" >> "$env:GITHUB_OUTPUT"
    Write-Output '::echo::off'
}

if ($IS_TUI) {
    Write-Output "Application installer: $INSTALLER_PATH"
}
