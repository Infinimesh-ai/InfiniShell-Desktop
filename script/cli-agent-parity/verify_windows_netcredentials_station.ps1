param(
    [ValidateSet('MappingJob', 'Stdio')]
    [string]$CaseSet = 'MappingJob'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# 每个固定病例的非零退出单独记录；后续病例仍各执行一次，不重试失败病例。
$PSNativeCommandUseErrorActionPreference = $false

# 两组独立诊断：原四例保持；Stdio 仅不写/写 marker 两例，不执行 CLI、Node 或 AppContainer。
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
        case_set = $CaseSet
        scope = '仅新登录会话的指定生命周期对照；不代表普通交互用户、AppContainer、Node 或 npm 通过'
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidenceRoot 'build.safe.json') -Encoding utf8

    $env:INFINISHELL_WINDOWS_NETCREDENTIALS_HELPER = $helper
    $env:INFINISHELL_WINDOWS_NETCREDENTIALS_HELPER_SHA256 = $helperHash
    # 原基线保持无映射且查询前关 Job；其他三例每例新 nonce/目录，顺序各一次。
    $cases = @(
        @{ name = 'baseline'; map = $false; retain_empty_job = $false; test = 'netcredentials_two_stage_station_records_identity_and_cleanup' }
        @{ name = 'device-map'; map = $true; retain_empty_job = $false; test = 'netcredentials_two_stage_private_device_map_records_identity_and_cleanup' }
        @{ name = 'retained-empty-job'; map = $false; retain_empty_job = $true; test = 'netcredentials_two_stage_retained_empty_job_records_identity_and_cleanup' }
        @{ name = 'device-map-retained-empty-job'; map = $true; retain_empty_job = $true; test = 'netcredentials_two_stage_private_device_map_retained_empty_job_records_identity_and_cleanup' }
    )
    if ($CaseSet -eq 'Stdio') {
        $cases = @(
            @{ name = 'stdio-without-write'; map = $false; retain_empty_job = $false; write_marker = $false; test = 'netcredentials_stdio_without_write_records_original_and_released_lifetime' }
            @{ name = 'stdio-with-write'; map = $false; retain_empty_job = $false; write_marker = $true; test = 'netcredentials_stdio_with_write_records_original_and_released_lifetime' }
        )
    }
    $results = @()
    $testExit = 0
    $stage = 'native_test'
    foreach ($case in $cases) {
        $caseNonce = [guid]::NewGuid().ToString('N')
        $caseRoot = Join-Path $evidenceRoot ($case.name + '-' + $caseNonce)
        if (Test-Path -LiteralPath $caseRoot) { throw '病例目录已存在' }
        New-Item -ItemType Directory -Path $caseRoot | Out-Null
        $env:INFINISHELL_WINDOWS_NETCREDENTIALS_EVIDENCE_DIR = $caseRoot
        $env:INFINISHELL_WINDOWS_NETCREDENTIALS_NONCE = $caseNonce
        $filter = 'test(=windows::appcontainer::console_tests::' + $case.test + ')'
        cargo nextest run --no-fail-fast --retries 0 --success-output immediate -p command --lib --run-ignored only -E $filter
        $caseExit = $LASTEXITCODE
        if ($caseExit -ne 0) { $testExit = 1 }
        $results += @{
            name = $case.name
            nonce = $caseNonce
            evidence_directory = $caseRoot
            with_device_map = $case.map
            retain_empty_job = $case.retain_empty_job
            case_set = $CaseSet
            write_marker = $(if ($CaseSet -eq 'Stdio') { $case.write_marker } else { $null })
            test = $case.test
            exit_code = $caseExit
        }
        # 每例立即保存已有结果；后续病例不覆盖原例的失败或原三秒 LSA 判定。
        @{
            schema = 2
            stage = $stage
            status = 'in_progress'
            cases = $results
            g09_closed = $false
            cleanup_ready = $false
        } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'workflow.safe.json') -Encoding utf8
    }
    @{
        schema = 2
        stage = $stage
        status = 'completed'
        test_exit_code = $testExit
        cases = $results
        g09_closed = $false
        case_set = $CaseSet
        scope = '仅指定对照原期限结果；关闭管道后结果不覆盖原失败，不排除 AppContainer/console/debugger 组合'
        cleanup_ready = $false
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'workflow.safe.json') -Encoding utf8
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
