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
    )) {
        throw 'Unsupported notification hook'
    }
    $root = [Environment]::GetEnvironmentVariable('PLUGIN_ROOT')
    if ([String]::IsNullOrWhiteSpace($root) -or -not [IO.Path]::IsPathRooted($root)) {
        throw 'Missing absolute PLUGIN_ROOT'
    }
    $script = [IO.Path]::GetFullPath((Join-Path $root ('scripts/' + $NotificationHook)))
    if (-not [IO.File]::Exists($script)) {
        throw 'Notification script is missing'
    }
    $bash = @(Get-Command bash.exe -CommandType Application -All -ErrorAction Stop |
        Where-Object { [IO.File]::Exists((Join-Path (Split-Path $_.Source) 'msys-2.0.dll')) })
    if ($bash.Count -eq 0) {
        throw 'Git Bash usr/bin must be available on PATH'
    }
    $null = Get-Command jq.exe -CommandType Application -ErrorAction Stop
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $bash[0].Source
    $info.UseShellExecute = $false
    # 直接继承原始句柄；PS 5.1 的重定向 StreamWriter 会按控制台编码预先写入 BOM。
    $info.RedirectStandardInput = $false
    $info.RedirectStandardOutput = $false
    $info.RedirectStandardError = $false
    $info.CreateNoWindow = $false
    $script = $script.Replace([char] 92, [char] 47)
    $info.Arguments = '--noprofile --norc -- ' + (ConvertTo-NativeArgument $script)
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    if (-not $process.Start()) { throw 'Git Bash did not start' }
    try {
        $process.WaitForExit()
        $code = $process.ExitCode
    } finally {
        $process.Dispose()
    }
    exit $code
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
