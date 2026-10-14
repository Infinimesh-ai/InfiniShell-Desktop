[CmdletBinding()]
param([string] $OutputDirectory)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:OS -ne 'Windows_NT') { throw '此测试需要 Windows 和 Inno Setup 编译器' }
$compiler = Get-Command ISCC -ErrorAction SilentlyContinue
if (-not $compiler) {
    foreach ($programRoot in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if (-not $programRoot) { continue }
        $knownCompiler = Join-Path $programRoot 'Inno Setup 6\ISCC.exe'
        if (Test-Path -LiteralPath $knownCompiler -PathType Leaf) {
            $compiler = Get-Command $knownCompiler -ErrorAction Stop
            break
        }
    }
}
if (-not $compiler) { throw '需要现有 Inno Setup 6 的 ISCC.exe；PATH 与标准安装目录均未找到' }
$testId = [Guid]::NewGuid().ToString('N')
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path ([IO.Path]::GetTempPath()) "infinishell-registry-$testId"
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) {
    if (@(Get-ChildItem -LiteralPath $OutputDirectory -Force).Count -ne 0) {
        throw '证据目录必须为空，不能覆盖既有结果'
    }
} else {
    New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
}

$keyPath = "Software\InfiniShellRegistryTests\$testId"
$registry = [Microsoft.Win32.Registry]::CurrentUser
$existing = $registry.OpenSubKey($keyPath)
if ($existing) { $existing.Dispose(); throw '随机测试键已存在，拒绝覆盖' }
$root = $registry.CreateSubKey($keyPath)
$root.SetValue('TestId', $testId, [Microsoft.Win32.RegistryValueKind]::String)
$root.Dispose()
$deniedKeys = @()
$result = [ordered]@{
    schema_version = 1; test_id = $testId; status = 'incomplete'
    registry_root = "HKCU\$keyPath"; started_at = [DateTime]::UtcNow.ToString('o')
    compiler = $compiler.Source; harness_exit_code = $null; assertions = 0
    cleanup_passed = $false; error = $null; finished_at = $null
}

function Set-Fixture {
    param([string] $CaseName)
    $uri = $registry.CreateSubKey("$keyPath\$CaseName\infinishell")
    $uri.SetValue('', 'InfiniShell')
    $uri.SetValue('URL Protocol', '')
    $uri.SetValue('InfiniShellOwnerPath', 'C:\安装 测试\InfiniShell\infinishell.exe')
    $uri.Dispose()
    $command = $registry.CreateSubKey("$keyPath\$CaseName\infinishell\shell\open\command")
    $command.SetValue('', '"C:\安装 测试\InfiniShell\infinishell.exe" "%1"')
    $command.Dispose()
    $aumid = $registry.CreateSubKey("$keyPath\$CaseName\AppUserModelId\dev.infinishell.InfiniShell")
    $aumid.SetValue('DisplayName', 'InfiniShell')
    $aumid.SetValue('InfiniShellOwnerPath', 'C:\安装 测试\InfiniShell\infinishell.exe')
    $aumid.Dispose()
}

function Set-Denial {
    param([string] $Subkey, [Security.AccessControl.RegistryRights] $Rights)
    $key = $registry.OpenSubKey($Subkey,
        [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree,
        ([Security.AccessControl.RegistryRights]::ReadKey -bor [Security.AccessControl.RegistryRights]::ChangePermissions))
    try {
        $acl = $key.GetAccessControl()
        $original = $acl.GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]::Access)
        $rule = [Security.AccessControl.RegistryAccessRule]::new(
            [Security.Principal.WindowsIdentity]::GetCurrent().User,
            $Rights, [Security.AccessControl.AccessControlType]::Deny)
        $acl.AddAccessRule($rule)
        $key.SetAccessControl($acl)
        return [pscustomobject]@{ path = $Subkey; original_acl = $original }
    } finally { $key.Dispose() }
}

try {
    foreach ($case in @('write-denied', 'read-denied', 'delete-denied')) { Set-Fixture $case }
    $deniedKeys += Set-Denial "$keyPath\write-denied\infinishell" ([Security.AccessControl.RegistryRights]::SetValue)
    $deniedKeys += Set-Denial "$keyPath\read-denied\infinishell" ([Security.AccessControl.RegistryRights]::QueryValues)
    $deniedKeys += Set-Denial "$keyPath\delete-denied\AppUserModelId\dev.infinishell.InfiniShell" ([Security.AccessControl.RegistryRights]::SetValue)

    $compilerArgs = @(
        (Join-Path $PSScriptRoot 'windows-registration-tests.iss'),
        "/DTestId=$testId", "/DEvidenceDir=$OutputDirectory"
    )
    & $compiler.Source @compilerArgs 2>&1 | Tee-Object -FilePath (Join-Path $OutputDirectory 'compile.log') | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "ISCC 编译失败：$LASTEXITCODE" }

    # 同时编译实际安装器，覆盖预处理宏、include 和事件接线；此包仅编译，绝不执行。
    $fixture = Join-Path $OutputDirectory 'compile-only-input'
    New-Item -ItemType Directory -Path (Join-Path $fixture 'resources') -Force | Out-Null
    foreach ($name in @('infinishell.exe', 'infinishell-ssh.exe', 'infinishell-station-bootstrap.exe')) {
        Copy-Item -LiteralPath (Join-Path $OutputDirectory 'registration-test.exe') -Destination (Join-Path $fixture $name)
    }
    [IO.File]::WriteAllText((Join-Path $fixture 'pwsh.ps1'), '# 仅供安装器编译的测试夹具')
    [IO.File]::WriteAllText((Join-Path $fixture 'resources\fixture.txt'), 'compile-only fixture')
    & $compiler.Source (Join-Path $PSScriptRoot 'windows-installer.iss') `
        /DArch=x64 /DReleaseChannel=oss /DMyAppName=InfiniShell /DMyAppExeName=infinishell.exe `
        /DAppUserModelId=dev.infinishell.InfiniShell /DInnoAppId=infinishell `
        "/DTargetProfileDir=$fixture" /DOutputName=compile-only-installer "/O$OutputDirectory" `
        2>&1 | Tee-Object -FilePath (Join-Path $OutputDirectory 'production-compile.log') | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "实际安装器编译失败：$LASTEXITCODE" }

    $logPath = Join-Path $OutputDirectory 'inno.log'
    $process = Start-Process -FilePath (Join-Path $OutputDirectory 'registration-test.exe') -ArgumentList @(
        '/SP-', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$logPath`""
    ) -Wait -PassThru
    $result.harness_exit_code = $process.ExitCode
    # InitializeSetup=False 的既定退出码为 1，必须同时有完整成功收据。
    if ($process.ExitCode -ne 1) { throw "测试引擎退出码异常：$($process.ExitCode)" }
    $completed = [IO.File]::ReadAllText((Join-Path $OutputDirectory 'completed.txt'))
    if ($completed -cne $testId) { throw '缺少本轮完整成功收据' }
    $assertions = @(Get-Content -LiteralPath (Join-Path $OutputDirectory 'assertions.log'))
    if ($assertions.Count -eq 0 -or @($assertions | Where-Object { $_ -notmatch '^PASS: ' }).Count -or
        $assertions[-1] -cne 'PASS: completion receipt') {
        throw '断言证据不完整'
    }
    $result.assertions = $assertions.Count
    $result.status = 'passed'
} catch {
    $result.status = 'failed'
    $result.error = $_.Exception.Message
} finally {
    try {
        # 仅恢复本轮自己添加的 ACL；没有宽泛递归删除或权限修复。
        foreach ($denied in $deniedKeys) {
            $key = $registry.OpenSubKey($denied.path,
                [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree,
                [Security.AccessControl.RegistryRights]::ChangePermissions)
            try {
                $acl = New-Object Security.AccessControl.RegistrySecurity
                $acl.SetSecurityDescriptorSddlForm($denied.original_acl, [Security.AccessControl.AccessControlSections]::Access)
                $key.SetAccessControl($acl)
            } finally { $key.Dispose() }
        }
        # 失败时保留测试键与文件；成功时只清理仍有唯一身份标记的测试键。
        if ($result.status -eq 'passed') {
            $root = $registry.OpenSubKey($keyPath)
            try {
                if ($null -eq $root -or $root.GetValue('TestId') -cne $testId) {
                    throw '测试键身份不匹配，保留现场'
                }
            } finally { if ($root) { $root.Dispose() } }
            $registry.DeleteSubKeyTree($keyPath, $false)
            $remaining = $registry.OpenSubKey($keyPath)
            if ($remaining) { $remaining.Dispose(); throw '测试键清理未完成' }
            $result.cleanup_passed = $true
        }
    } catch {
        $result.status = 'failed'
        $result.error = "$($result.error) 清理失败：$($_.Exception.Message)"
    }
    $result.finished_at = [DateTime]::UtcNow.ToString('o')
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'result.json') -Encoding UTF8
}
Write-Host "Windows 注册回归：$($result.status)，证据：$OutputDirectory"
if ($result.status -ne 'passed') { exit 1 }
