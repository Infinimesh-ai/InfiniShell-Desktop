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

# 只检查安装依赖，不注册插件、不执行通知、不修改用户 shell 或授权配置。
try {
    if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1 -or [IntPtr]::Size -ne 8) {
        throw 'Unsupported Windows PowerShell runtime'
    }
    $system = Join-Path ([Environment]::GetEnvironmentVariable('SystemRoot')) 'System32'
    $powershell = [IO.Path]::GetFullPath((Join-Path $system 'WindowsPowerShell/v1.0/powershell.exe'))
    $cmd = [IO.Path]::GetFullPath((Join-Path $system 'cmd.exe'))
    if ([Diagnostics.Process]::GetCurrentProcess().MainModule.FileName -ne $powershell -or
        (Get-Command powershell.exe -CommandType Application -ErrorAction Stop).Source -ne $powershell -or
        (Get-Command cmd.exe -CommandType Application -ErrorAction Stop).Source -ne $cmd -or
        [Environment]::GetEnvironmentVariable('COMSPEC') -ne $cmd) {
        throw 'The native hook shell must resolve to the Windows system executables'
    }
    # 固定 Codex 的 cmd /C 会读取 AutoRun；和既有原生探针一样只检查，不清除用户设置。
    foreach ($hive in @([Microsoft.Win32.Registry]::CurrentUser, [Microsoft.Win32.Registry]::LocalMachine)) {
        $key = $hive.OpenSubKey('Software\Microsoft\Command Processor')
        if ($null -ne $key) {
            try {
                $autorun = $key.GetValue('AutoRun')
                if ($null -ne $autorun -and [string] $autorun -ne '') { throw 'Custom cmd AutoRun is unsupported' }
            } finally { $key.Dispose() }
        }
    }
    foreach ($variable in @('BASH_ENV', 'ENV')) {
        if (-not [String]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($variable))) {
            throw 'Custom shell initialization is unsupported'
        }
    }
    $bash = @(Get-Command bash.exe -CommandType Application -All -ErrorAction Stop |
        Where-Object { [IO.File]::Exists((Join-Path (Split-Path $_.Source) 'msys-2.0.dll')) })
    if ($bash.Count -eq 0) { throw 'Git Bash usr/bin must be available on PATH' }
    $null = Get-Command jq.exe -CommandType Application -ErrorAction Stop
    # 通过正式启动器选中的同一个 Bash 检查 jq；逐字比较，不归一化中文及 LF/CRLF。
    $probe = @'
case "$OSTYPE" in msys*) ;; *) exit 1 ;; esac
[ -n "$BASH_VERSION" ] || exit 1
jq --binary --null-input --join-output --arg text $'中文\nEnglish\r\n' '$text'
'@
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $bash[0].Source
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = New-Object Text.UTF8Encoding($false, $true)
    $info.Arguments = '--noprofile --norc -c ' + (ConvertTo-NativeArgument $probe)
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    if (-not $process.Start()) { throw 'Git Bash did not start' }
    try {
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(8000)) {
            $process.Kill()
            $process.WaitForExit()
            throw 'Dependency check timed out'
        }
        $output = $process.StandardOutput.ReadToEnd()
        $errorOutput = $process.StandardError.ReadToEnd()
        if ($process.ExitCode -ne 0 -or $output -cne "中文`nEnglish`r`n" -or $errorOutput -ne '') {
            throw 'Git Bash and jq binary output did not preserve the expected bytes'
        }
    } finally { $process.Dispose() }
    [Console]::Out.WriteLine('infinishell-codex-windows-dependencies-v1')
    exit 0
} catch {
    # 只返回固定状态，不输出用户 PATH、注册表内容或异常原文。
    [Console]::Error.WriteLine('Windows notification dependencies are unavailable')
    exit 1
}
