# Prevent history from being written to file, among other interactive features.
$global:_warpPrivateHistoryPath = $null
# 显式私有启动仅适用于本机 Windows；目录由调用方预先创建和控制，不修改共享配置或 ACL。
if (($PSEdition -eq 'Desktop' -or $IsWindows) -and $env:WARP_IS_LOCAL_SHELL_SESSION -eq '1' -and (Test-Path Env:WARP_POWERSHELL_PRIVATE_STARTUP_ROOT)) {
    try {
        $privateRoot = $env:WARP_POWERSHELL_PRIVATE_STARTUP_ROOT
        if ($privateRoot -notmatch '^[A-Za-z]:[\\/]') { throw 'invalid_root' }
        $privateRoot = Get-Item -LiteralPath ([IO.Path]::GetFullPath($privateRoot)) -Force -ErrorAction Stop
        if (-not $privateRoot.PSIsContainer -or $null -eq $privateRoot.Parent) { throw 'invalid_root' }
        for ($ancestor = $privateRoot; $null -ne $ancestor; $ancestor = $ancestor.Parent) {
            if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'reparse_root' }
        }
        $privateSession = Join-Path $privateRoot.FullName ('session-@@WARP_SESSION_ID@@-' + [Guid]::NewGuid().ToString('N'))
        $null = New-Item -ItemType Directory -Path $privateSession -ErrorAction Stop
        $global:_warpPrivateHistoryPath = Join-Path $privateSession 'ConsoleHost_history.txt'
        $privateHistory = [IO.File]::Open($global:_warpPrivateHistoryPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $privateHistory.Dispose()
        # 在首次 ReadLine 前设置两项，避免 SaveNothing 的初始化回退仍读取默认历史路径。
        Import-Module PSReadLine -ErrorAction Stop
        Set-PSReadLineOption -HistorySaveStyle SaveNothing -HistorySavePath $global:_warpPrivateHistoryPath -ErrorAction Stop
        $privateOptions = Get-PSReadLineOption -ErrorAction Stop
        if ($privateOptions.HistorySaveStyle -ne 'SaveNothing' -or $privateOptions.HistorySavePath -cne $global:_warpPrivateHistoryPath) { throw 'history_options' }
    } catch {
        [Console]::Error.WriteLine('WARP_POWERSHELL_PRIVATE_STARTUP_FAILED')
        [Environment]::Exit(1)
    }
    Remove-Variable privateRoot, ancestor, privateSession, privateHistory, privateOptions -ErrorAction Ignore
}
Remove-Module -Name PSReadline

$global:_warpOriginalPrompt = $function:global:prompt

if ($PSEdition -eq 'Desktop' -or $IsWindows) {
    $EP = [Microsoft.PowerShell.ExecutionPolicy]
    # MachinePolicy and UserPolicy scopes cannot be overridden. If either is Restricted, there's nothing we can do.
    if ((Get-ExecutionPolicy -Scope MachinePolicy) -eq $EP::Restricted -or (Get-ExecutionPolicy -Scope UserPolicy) -eq $EP::Restricted) {
        Write-Error 'ExecutionPolicy is Restricted. Unable to Warpify this PowerShell session.'
    } elseif ((Get-ExecutionPolicy) -eq $EP::Restricted -and (Get-ExecutionPolicy -Scope MachinePolicy) -eq $EP::Undefined -and (Get-ExecutionPolicy -Scope UserPolicy) -eq $EP::Undefined) {
        $global:_warp_PSProcessExecPolicy = $(Get-ExecutionPolicy -Scope Process)
        Set-ExecutionPolicy -Scope Process -ExecutionPolicy RemoteSigned -Force
    }
}

# We must wait until pwsh attempts to show the first prompt before writing an OSC string.
# Trying to do so beforehand will prevent the "Write-Host" command from being submitted, as pwsh
# ignores submissions prior to the first prompt.
function prompt {
    # Reset the prompt back to the default to avoid infinite loops if sourcing the bootstrap script has an error.
    $function:global:prompt = $global:_warpOriginalPrompt
    $username = [Environment]::UserName
    $global:_warpSessionId = [uint64]@@WARP_SESSION_ID@@
    $msg = ConvertTo-Json -Compress -InputObject @{ hook = 'InitShell'; value = @{ session_id = $_warpSessionId; shell = 'pwsh'; user = $username; hostname = [System.Net.Dns]::GetHostName() } }
    $encodedMsg = [BitConverter]::ToString([System.Text.Encoding]::UTF8.GetBytes($msg)).Replace('-', '')
    $oscStart = "$([char]0x1b)]9278;"
    $oscEnd = "`a"
    $oscJsonMarker = 'd'
    $oscParameterSeparator = ';'
    Write-Host "${oscStart}${oscJsonMarker}${oscParameterSeparator}${encodedMsg}${oscEnd}"
    return $null
}
