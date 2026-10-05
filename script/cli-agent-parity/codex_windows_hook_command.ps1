$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

function ConvertTo-NativeArgument([string] $Value) {
    # Windows PS 5.1 没有 ArgumentList；按 Windows CRT 的反斜杠与双引号规则编码 argv。
    $result = New-Object System.Text.StringBuilder
    [void] $result.Append('"')
    $backslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq [char] 92) {
            $backslashes++
            continue
        }
        if ($character -eq [char] 34) {
            [void] $result.Append(('\' * (2 * $backslashes + 1)))
        } else {
            [void] $result.Append(('\' * $backslashes))
        }
        [void] $result.Append($character)
        $backslashes = 0
    }
    [void] $result.Append(('\' * (2 * $backslashes)))
    [void] $result.Append('"')
    $result.ToString()
}

# BEGIN_NOTIFICATION_LAUNCH
try {
    if ($NotificationHook -notin @(
        'on-session-start.sh', 'on-prompt-submit.sh', 'on-stop.sh',
        'on-permission-request.sh', 'on-post-tool-use.sh'
    )) { throw 'Unsupported notification hook' }
    $root = [Environment]::GetEnvironmentVariable('PLUGIN_ROOT')
    if ([String]::IsNullOrWhiteSpace($root) -or $root -notmatch '^(?:[a-zA-Z]:[\\/]|\\\\[^?\.\\]+\\[^\\]+)') {
        throw 'Missing absolute PLUGIN_ROOT'
    }
    $root = [IO.Path]::GetFullPath($root)
    $script = [IO.Path]::GetFullPath((Join-Path $root 'scripts/on-notification.ps1'))
    # 路径只作为 argv；拒绝入口及其祖先的重解析点，避免验证根目录后跳到另一棵树。
    $item = Get-Item -LiteralPath $script -Force
    if ($item.PSIsContainer) { throw 'Notification script is missing' }
    while ($null -ne $item) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Notification path is a reparse point' }
        if ($item -is [IO.FileInfo]) { $item = $item.Directory } else { $item = $item.Parent }
    }
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = Join-Path $PSHOME 'powershell.exe'
    $info.UseShellExecute = $false
    # 保留原始 stdin/stdout/stderr 与当前真实控制台，不经 PS 文本管道转码。
    $info.RedirectStandardInput = $false
    $info.RedirectStandardOutput = $false
    $info.RedirectStandardError = $false
    $info.CreateNoWindow = $false
    $info.Arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ' +
        (ConvertTo-NativeArgument $script) + ' -NotificationHook ' + (ConvertTo-NativeArgument $NotificationHook)
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    if (-not $process.Start()) { throw 'PowerShell did not start' }
    try {
        $process.WaitForExit()
        $code = $process.ExitCode
    } finally {
        $process.Dispose()
    }
    exit $code
} catch {
    [Console]::Error.WriteLine('infinishell_codex_hook_transport_error: native_launcher_failed')
    exit 1
}
