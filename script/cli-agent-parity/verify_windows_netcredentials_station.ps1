$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# 只验证本轮新登录会话的两段路径；不执行旧 NUL、Node 或未命名窗口站对照。
if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or
    [string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) {
    throw '缺少 GitHub runner 的证据目录环境'
}
$runnerRoot = [IO.Path]::GetFullPath($env:RUNNER_TEMP)
if ($runnerRoot.StartsWith('\\') -or -not [IO.Directory]::Exists($runnerRoot)) {
    throw '证据目录必须位于既有本地 runner 文件系统'
}
$nonce = [guid]::NewGuid().ToString('N')
$evidenceRoot = Join-Path $runnerRoot ('infinishell-netcredentials-' + $nonce)
if (Test-Path -LiteralPath $evidenceRoot) { throw '本轮证据目录已存在' }
$directory = New-Item -ItemType Directory -Path $evidenceRoot
$owner = [Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = [Security.AccessControl.DirectorySecurity]::new()
$acl.SetOwner($owner)
$acl.SetAccessRuleProtection($true, $false)
$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
    $owner,
    [Security.AccessControl.FileSystemRights]::FullControl,
    [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
    [Security.AccessControl.PropagationFlags]::None,
    [Security.AccessControl.AccessControlType]::Allow
))
Set-Acl -LiteralPath $directory.FullName -AclObject $acl
"INFINISHELL_WINDOWS_NETCREDENTIALS_EVIDENCE_DIR=$evidenceRoot" |
    Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8

$stage = 'prepare_toolchain'
@{
    schema = 1
    stage = $stage
    status = 'started'
    native_test_started = $false
    cleanup_ready = $false
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidenceRoot 'preparation.safe.json') -Encoding utf8
try {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere)) { throw '缺少 MSVC 查找工具' }
    $vsInstall = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($vsInstall)) {
        throw '缺少 MSVC x64 编译工具'
    }
    $vcvars = Join-Path $vsInstall.Trim() 'VC\Auxiliary\Build\vcvars64.bat'
    $source = (Resolve-Path -LiteralPath 'crates/command/src/fixtures/windows_netcredentials_station_helper.c').Path
    $helper = Join-Path $evidenceRoot 'netcredentials-station.exe'
    $object = Join-Path $evidenceRoot 'netcredentials-station.obj'
    $buildScript = Join-Path $evidenceRoot 'build-helper.cmd'
    foreach ($boundPath in @($vcvars, $evidenceRoot, $source)) {
        if ($boundPath -match '[%!"&|<>^]' -or $boundPath -match '[^\x20-\x7e]') {
            throw '本轮 CMD 编译路径必须为不含元字符的 ASCII 路径'
        }
    }
    if (-not (Test-Path -LiteralPath $vcvars)) { throw '缺少 MSVC x64 编译环境' }
    @(
        '@echo off'
        "call `"$vcvars`" >nul || exit /b 1"
        "cl.exe /nologo /std:c11 /utf-8 /W4 /WX /GS- /Zl /c /Fo`"$object`" `"$source`" || exit /b 1"
        "link.exe /nologo /NODEFAULTLIB /ENTRY:ProbeMain /SUBSYSTEM:WINDOWS /MACHINE:X64 /OUT:`"$helper`" `"$object`" kernel32.lib advapi32.lib user32.lib secur32.lib || exit /b 1"
    ) | Set-Content -LiteralPath $buildScript -Encoding ascii
    $stage = 'build_helper'
    & cmd.exe /d /v:off /c "`"$buildScript`"" *> (Join-Path $evidenceRoot 'helper-build.log')
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $helper)) {
        throw '两段原生窗口站 helper 构建失败，原日志已保留'
    }
    $helperHash = (Get-FileHash -LiteralPath $helper -Algorithm SHA256).Hash.ToLowerInvariant()
    $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    @{
        schema = 1
        source_sha256 = $sourceHash
        executable_sha256 = $helperHash
        executable_bytes = (Get-Item -LiteralPath $helper).Length
        commit = $env:GITHUB_SHA
        scope = '仅非管理员服务身份的新登录会话两段窗口站候选；不代表普通交互用户、AppContainer、Node 或 npm 通过'
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidenceRoot 'build.safe.json') -Encoding utf8

    $env:INFINISHELL_WINDOWS_NETCREDENTIALS_HELPER = $helper
    $env:INFINISHELL_WINDOWS_NETCREDENTIALS_HELPER_SHA256 = $helperHash
    $env:INFINISHELL_WINDOWS_NETCREDENTIALS_EVIDENCE_DIR = $evidenceRoot
    $env:INFINISHELL_WINDOWS_NETCREDENTIALS_NONCE = $nonce
    # 不重试；失败收据在测试有界清理后仍由下一步骤归档。
    $stage = 'native_test'
    cargo nextest run --no-fail-fast --retries 0 --success-output immediate -p command --lib --run-ignored only -E 'test(=windows::appcontainer::console_tests::netcredentials_two_stage_station_records_identity_and_cleanup)'
    $testExit = $LASTEXITCODE
    @{
        schema = 1
        stage = $stage
        status = 'completed'
        test_exit_code = $testExit
        cleanup_ready = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidenceRoot 'workflow.safe.json') -Encoding utf8
    exit $testExit
} catch {
    # 只保留阶段及数值错误，不序列化环境、账户或系统异常正文。
    @{
        schema = 1
        stage = $stage
        status = 'failed'
        exception_type = $_.Exception.GetType().FullName
        hresult = $_.Exception.HResult
        cleanup_ready = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidenceRoot 'workflow-failure.safe.json') -Encoding utf8
    throw
}
