#requires -Version 5.1
<# 重发旧应用版本的安装逻辑。仅解包、编译与运行隔离注册表测试，不运行产品安装器。 #>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$VerifiedDirectory,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedInstallerSha256 = '52c987801e27310ffb08351d13c255ca114f05a40d8e0ccb815b638b042a0cc7'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT' -or $PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1 -or
    $env:GITHUB_REPOSITORY -cne 'Infinimesh-ai/InfiniShell-Desktop' -or $env:GITHUB_SHA -cnotmatch '^[0-9a-f]{40}$') {
    throw '需要正式仓库、精确提交与 Windows PowerShell 5.1。'
}
function Assert-Physical([string]$Path) {
    $cursor = [IO.Path]::GetFullPath($Path)
    if ($cursor -notmatch '^[A-Za-z]:\\') { throw '必须使用本机绝对路径。' }
    while ($cursor) {
        if ((Test-Path -LiteralPath $cursor) -and ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw '拒绝重解析路径。' }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}
function Save-Json([string]$Name, $Value) {
    $path = Join-Path $evidence $Name
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 30))
    $stream = [IO.File]::Open($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}
function Invoke-Native([string]$Label, [string]$Executable, [string[]]$Arguments, [int]$TimeoutSeconds = 1800) {
    foreach ($value in @($Executable) + $Arguments) { if ($value -match '["\r\n]' -or $value.EndsWith('\')) { throw '原生参数不能含引号、换行或末尾反斜杠。' } }
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    $info.Arguments = (($Arguments | ForEach-Object { '"' + $_ + '"' }) -join ' ')
    $info.UseShellExecute = $false; $info.CreateNoWindow = $true
    $info.WorkingDirectory = $root
    $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
    $output = [IO.File]::Open((Join-Path $evidence ($Label + '.stdout.log')), 'CreateNew', 'Write', 'Read')
    $errors = [IO.File]::Open((Join-Path $evidence ($Label + '.stderr.log')), 'CreateNew', 'Write', 'Read')
    $process = [Diagnostics.Process]::new()
    try {
        $process.StartInfo = $info
        if (-not $process.Start()) { throw '原生工具未启动。' }
        $stdout = $process.StandardOutput.BaseStream.CopyToAsync($output)
        $stderr = $process.StandardError.BaseStream.CopyToAsync($errors)
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $commands.Add(@{label=$Label;pid=$process.Id;status='timed_out_retained'})
            throw '原生工具超时；保留进程与证据，不自动重复执行或清理。'
        }
        $null = $stdout.GetAwaiter().GetResult(); $null = $stderr.GetAwaiter().GetResult()
        $output.Flush($true); $errors.Flush($true)
        $commands.Add(@{label=$Label;executable=$Executable;arguments=$Arguments;pid=$process.Id;exit_code=$process.ExitCode})
        if ($process.ExitCode -ne 0) { throw ('阶段失败：' + $Label + '，退出码 ' + $process.ExitCode) }
    } finally { $output.Dispose(); $errors.Dispose(); $process.Dispose() }
}
function Invoke-Gate([string]$Action, [string]$Label = '', [string[]]$Extra = @()) {
    if (-not $Label) { $Label = $Action }
    Invoke-Native $Label $python (@('-I', $controller, $Action, '--output', $root, '--verified', $verified, '--repo', $repo, '--expected', $ExpectedInstallerSha256) + $Extra)
}
function Capture-Signature([string]$Path, [string]$Name, [bool]$RequireValid) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    $status = $signature.Status.ToString()
    $signer = $null
    if ($signature.SignerCertificate) { $signer = @{subject=$signature.SignerCertificate.Subject;thumbprint=$signature.SignerCertificate.Thumbprint} }
    Save-Json $Name @{authenticode_status=$status;installer_sha256=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant();signer=$signer}
    if ($RequireValid -and ($status -cne 'Valid' -or $null -eq $signer)) { throw '编译器官方签名无效。' }
    if ($status -cnotin @('Valid', 'NotSigned')) { throw '安装器 Authenticode 状态不可接受。' }
}
$root = [IO.Path]::GetFullPath($OutputDirectory)
$verified = [IO.Path]::GetFullPath($VerifiedDirectory)
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
foreach ($path in @($root, $verified, $repo)) { Assert-Physical $path }
if (Test-Path -LiteralPath $root) { throw '输出目录已存在；保留失败轮次，拒绝覆盖。' }
if ($root.StartsWith($verified + '\', [StringComparison]::OrdinalIgnoreCase) -or $verified.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) { throw '来源与输出目录不能相互包含。' }
$evidence = Join-Path $root 'evidence'
$null = New-Item -ItemType Directory -Path $evidence
$python = (Get-Command python.exe -ErrorAction Stop).Source
$controller = Join-Path $PSScriptRoot 'reissue_release_installer.py'
$tool = Join-Path $verified 'tool\innounp.exe'
$original = Join-Path $verified 'windows\InfiniShellSetup.exe'
$compiler = Join-Path $root 'compiler\{app}\ISCC.exe'
$powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$commands = [Collections.Generic.List[object]]::new()
$result = [ordered]@{status='failed';controller_sha=$env:GITHUB_SHA;started_utc=[DateTime]::UtcNow.ToString('o');error=$null;commands=$commands}
try {
    Invoke-Gate 'prepare'
    Capture-Signature (Join-Path $root 'compiler-distribution.exe') 'compiler-signature.json' $true
    Invoke-Native 'compiler-members' $tool @('-s', '-h', '-b', '-o', '-u', (Join-Path $root 'compiler-distribution.exe'), '{app}\*')
    Invoke-Gate 'listing' 'check-compiler-members' @('--label', 'compiler-members')
    Invoke-Native 'compiler-extract' $tool @('-x', '-b', '-q', '-o', '-u', ('-d' + (Join-Path $root 'compiler')), (Join-Path $root 'compiler-distribution.exe'), '{app}\*')
    Invoke-Native 'original-members' $tool @('-s', '-h', '-b', '-o', '-u', $original, '{app}\*')
    Invoke-Gate 'listing' 'check-original-members' @('--label', 'original-members')
    Invoke-Native 'original-extract' $tool @('-x', '-b', '-q', '-o', '-u', ('-d' + (Join-Path $root 'original-payload')), $original, '{app}\*')
    Invoke-Gate 'stage'
    foreach ($suite in @(@{name='full';script='test_windows_environment.ps1'}, @{name='reissue';script='test_windows_environment_reissue.ps1'})) {
        Invoke-Native ('path-tests-' + $suite.name) $powershell @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File',
            (Join-Path $root ('source\script\windows\' + $suite.script)), '-CompilerPath', $compiler,
            '-OutputDirectory', (Join-Path $root ('test-output\' + $suite.name))) 5400
    }
    Invoke-Gate 'tests'
    $candidate = Join-Path $root 'candidate'
    $null = New-Item -ItemType Directory -Path $candidate
    Invoke-Native 'installer-compile' $compiler @((Join-Path $root 'source\script\windows\windows-installer.iss'),
        '/DReleaseChannel=oss', '/DMyAppExeName=infinishell.exe', ('/DTargetProfileDir=' + (Join-Path $root 'original-payload\{app}')),
        '/DMyAppName=InfiniShell', '/DMyAppVersion=v0.2026.10.14.16.45.oss_00', '/DArch=x64', '/DOutputName=InfiniShellSetup',
        '/DAppUserModelId=dev.infinishell.InfiniShell', '/DInnoAppId=infinishell', ('/O' + $candidate))
    $installer = Join-Path $candidate 'InfiniShellSetup.exe'
    Capture-Signature $installer 'installer-signature.json' $false
    Invoke-Native 'rebuilt-members' $tool @('-s', '-h', '-b', '-o', '-u', $installer, '{app}\*')
    Invoke-Gate 'listing' 'check-rebuilt-members' @('--label', 'rebuilt-members')
    Invoke-Native 'rebuilt-extract' $tool @('-x', '-b', '-q', '-o', '-u', ('-d' + (Join-Path $root 'rebuilt-payload')), $installer, '{app}\*')
    Invoke-Gate 'finish'
    $result.status = 'remote_reissue_build_and_regression_passed'
} catch { $result.error = $_.Exception.Message }
finally {
    $result.finished_utc = [DateTime]::UtcNow.ToString('o')
    Save-Json 'execution-result.json' $result
    try { Invoke-Gate 'archive' 'archive-evidence' } catch {
        $result.status = 'failed'
        Save-Json 'archive-failure.json' @{status='failed';error=$_.Exception.Message;delivery_allowed=$false}
        Write-Error -ErrorAction Continue ('证据归档失败：' + $_.Exception.Message)
    }
}
Write-Output ($result | ConvertTo-Json -Depth 20 -Compress)
if ($result.status -cne 'remote_reissue_build_and_regression_passed') { exit 1 }
exit 0
