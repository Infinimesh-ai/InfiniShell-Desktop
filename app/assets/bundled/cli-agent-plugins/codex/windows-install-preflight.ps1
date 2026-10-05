$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version 2

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
    # 通知直接使用系统 PowerShell，不再启动 Bash 或 jq。
    [Console]::Out.WriteLine('infinishell-codex-windows-native-notifications-v2')
    exit 0
} catch {
    # 只返回固定状态，不输出用户 PATH、注册表内容或异常原文。
    [Console]::Error.WriteLine('Windows notification runtime is unavailable')
    exit 1
}
