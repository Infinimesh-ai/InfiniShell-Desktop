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
