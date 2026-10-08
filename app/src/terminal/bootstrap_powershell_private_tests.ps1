param([ValidateSet('private', 'two_sessions', 'default', 'relative', 'missing', 'file', 'junction', 'missing_init', 'remote', 'init_options_failure', 'bootstrap_options_failure', 'local_inherited_ssh')][string]$Case)
$ErrorActionPreference = 'Stop'
$fixture = $env:WARP_TEST_FIXTURE
$env:WARP_TEST_RC_MARKER = Join-Path $fixture 'rc-executed.txt'
$fakeHistory = Join-Path $fixture 'fake-shared-history.txt'
[IO.File]::WriteAllText($fakeHistory, 'FAKE_SHARED_HISTORY')

# 启动参数已禁止自动 profile；四个路径全部替换为夹具，不读取真实用户 profile。
$global:PROFILE = Join-Path $fixture 'profile4.ps1'
$names = @('AllUsersAllHosts', 'AllUsersCurrentHost', 'CurrentUserAllHosts', 'CurrentUserCurrentHost')
for ($i = 0; $i -lt 4; $i++) {
    $path = Join-Path $fixture ('profile' + ($i + 1) + '.ps1')
    [IO.File]::WriteAllText($path, '[IO.File]::AppendAllText($env:WARP_TEST_RC_MARKER, ''' + ($i + 1) + ''')')
    $global:PROFILE | Add-Member -NotePropertyName $names[$i] -NotePropertyValue $path -Force
}
Import-Module PSReadLine -ErrorAction Stop
Set-PSReadLineOption -HistorySaveStyle SaveIncrementally -HistorySavePath $fakeHistory
$env:WARP_IS_LOCAL_SHELL_SESSION = '1'
Remove-Item Env:WARP_IS_SSH -ErrorAction Ignore
Remove-Item Env:WARP_POWERSHELL_PRIVATE_STARTUP_ROOT -ErrorAction Ignore
$root = Join-Path $fixture 'private root 中文[1]'
$null = New-Item -ItemType Directory -Path $root
if ($Case -ne 'default') { $env:WARP_POWERSHELL_PRIVATE_STARTUP_ROOT = $root }
if ($Case -eq 'relative') { $env:WARP_POWERSHELL_PRIVATE_STARTUP_ROOT = 'relative-root' }
if ($Case -eq 'missing') { $env:WARP_POWERSHELL_PRIVATE_STARTUP_ROOT = Join-Path $fixture 'missing' }
if ($Case -eq 'file') { $env:WARP_POWERSHELL_PRIVATE_STARTUP_ROOT = $fakeHistory }
if ($Case -eq 'junction') {
    $target = Join-Path $fixture 'junction-target'
    $null = New-Item -ItemType Directory -Path (Join-Path $target 'child')
    $junction = Join-Path $fixture 'junction'
    $null = New-Item -ItemType Junction -Path $junction -Target $target
    $env:WARP_POWERSHELL_PRIVATE_STARTUP_ROOT = Join-Path $junction 'child'
}
if ($Case -eq 'remote') {
    $env:WARP_IS_LOCAL_SHELL_SESSION = '0'
    $env:WARP_IS_SSH = '1'
}
if ($Case -eq 'local_inherited_ssh') { $env:WARP_IS_SSH = '1' }
if ($Case -eq 'init_options_failure') { function global:Set-PSReadLineOption { throw 'fixture_options_failure' } }

# 保持假共享历史独占打开：init、模块重载和完整 bootstrap 都不能读取或改写它。
$historyLock = [IO.File]::Open($fakeHistory, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
try {
    if ($Case -ne 'missing_init') { . $env:WARP_TEST_INIT }
    [IO.File]::WriteAllText((Join-Path $fixture 'after-init.txt'), 'reached')
    $unloaded = $null -eq (Get-Module PSReadLine)
    $firstHistory = $global:_warpPrivateHistoryPath
    if ($Case -eq 'two_sessions') {
        Import-Module PSReadLine
        . $env:WARP_TEST_INIT
    }
    if ($Case -eq 'bootstrap_options_failure') { function global:Set-PSReadLineOption { throw 'fixture_options_failure' } }
    # 调用真实 prompt 生成 InitShell；捕获的是产品实际 OSC，不重写回报函数。
    $initOutput = @(prompt 6>&1 | ForEach-Object { [string]$_ })
    $bootstrap = [IO.File]::ReadAllText((Join-Path $env:WARP_TEST_ASSETS 'bundled/bootstrap/pwsh.ps1'))
    $bootstrap = [regex]::Replace($bootstrap, '(?m)^#include ([^\r\n]+)', {
        param($m)
        [IO.File]::ReadAllText((Join-Path $env:WARP_TEST_ASSETS $m.Groups[1].Value)).TrimStart([char]0xFEFF)
    })
    $bootstrapOutput = @(. ([ScriptBlock]::Create($bootstrap)) 6>&1 | ForEach-Object { [string]$_ })
    $options = Get-PSReadLineOption
    $hook = $null
    foreach ($match in [regex]::Matches(($bootstrapOutput -join ''), '\x1b\]9278;d;([0-9A-F]+)\x07')) {
        $hex = $match.Groups[1].Value
        $bytes = New-Object byte[] ($hex.Length / 2)
        for ($i = 0; $i -lt $bytes.Length; $i++) { $bytes[$i] = [Convert]::ToByte($hex.Substring($i * 2, 2), 16) }
        $message = [Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json
        if ($message.hook -eq 'Bootstrapped') { $hook = $message }
    }
    $result = @{
        powershell_version = $PSVersionTable.PSVersion.ToString()
        psreadline_version = (Get-Module PSReadLine).Version.ToString()
        rc_executed = [IO.File]::Exists($env:WARP_TEST_RC_MARKER)
        rc_sequence = $(if ([IO.File]::Exists($env:WARP_TEST_RC_MARKER)) { [IO.File]::ReadAllText($env:WARP_TEST_RC_MARKER) } else { '' })
        unloaded_after_init = $unloaded
        style = [string]$options.HistorySaveStyle
        history_private = $options.HistorySavePath.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
        history_empty = $(if ($null -ne $global:_warpPrivateHistoryPath) { (Get-Item -LiteralPath $global:_warpPrivateHistoryPath).Length -eq 0 } else { $false })
        real_bootstrapped_path_matches = ($null -ne $hook -and $hook.value.histfile -ceq $options.HistorySavePath)
        init_shell_emitted = (($initOutput -join '') -match '496E69745368656C6C')
        distinct_history = ($null -ne $firstHistory -and $firstHistory -cne $global:_warpPrivateHistoryPath)
    }
    [IO.File]::WriteAllText((Join-Path $fixture 'result.json'), ($result | ConvertTo-Json -Compress))
} finally {
    $historyLock.Dispose()
}
