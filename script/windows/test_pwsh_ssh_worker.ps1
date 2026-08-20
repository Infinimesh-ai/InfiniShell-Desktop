#!/usr/bin/env powershell.exe

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $WorkerPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$workspaceRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$worker = (Resolve-Path -LiteralPath $WorkerPath).Path
$sshExecutable = (Get-Command ssh.exe -CommandType Application -ErrorAction Stop |
    Select-Object -First 1).Source
$wrapperPath = Join-Path $workspaceRoot 'app\assets\bundled\bootstrap\pwsh_ssh_wrapper.ps1'
$assetsRoot = Join-Path $workspaceRoot 'app\assets\bundled\bootstrap'
$packagedBootstrap = Join-Path `
    ([IO.Path]::GetTempPath()) `
    "infinishell-pwsh-bootstrap-$([Guid]::NewGuid().ToString('N')).ps1"

try {
    & (Join-Path $PSScriptRoot 'prepare_pwsh_bootstrap.ps1') `
        -DestinationPath $packagedBootstrap
    $packagedContent = [IO.File]::ReadAllText($packagedBootstrap)
    $null = [ScriptBlock]::Create($packagedContent)
    if ([Regex]::IsMatch($packagedContent, '(?m)^[\t ]*#include ')) {
        throw 'Packaged PowerShell bootstrap still contains an include directive'
    }
    if (-not $packagedContent.Contains("'--commands-base64'")) {
        throw 'Packaged PowerShell bootstrap does not use the Base64 worker protocol'
    }

    . $wrapperPath
    function Warp-Encode-HexString([string]$str) {
        [BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($str)).Replace('-', '')
    }
    $script:WarpBashInitShell = [IO.File]::ReadAllText(
        (Join-Path $assetsRoot 'bash_init_shell.sh')
    )
    $script:WarpZshInitShell = [IO.File]::ReadAllText(
        (Join-Path $assetsRoot 'zsh_init_shell.sh')
    )
    $script:WarpPwshInitShell = [IO.File]::ReadAllText(
        (Join-Path $assetsRoot 'pwsh_init_shell.ps1')
    )

    $posixCommand = Warp-New-RemoteBootstrapCommand `
        -RemoteSessionId 2 `
        -SshHookHex '' `
        -EmitSshHook $false
    $windowsCommand = Warp-New-WindowsBootstrapCommand `
        -RemoteSessionId 2 `
        -SshHookHex '' `
        -EmitSshHook $false
    $workerArgs = @(Warp-New-RustSshWorkerArguments `
        -SessionId 1 `
        -RemoteSessionId 2 `
        -SshExecutable $sshExecutable `
        -PosixCommand $posixCommand `
        -WindowsCommand $windowsCommand `
        -SshArgs @('-V'))

    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = @(& $worker @workerArgs 2>&1 | ForEach-Object { [string]$_ })
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $previousErrorActionPreference
    $joinedOutput = $output -join [Environment]::NewLine

    if ($joinedOutput.Contains("unexpected argument '-p'")) {
        throw "PowerShell split the POSIX bootstrap command: $joinedOutput"
    }
    if ($joinedOutput.Contains('Usage: infinishell-ssh.exe rust-ssh-session')) {
        throw "The SSH worker rejected the PowerShell argument vector: $joinedOutput"
    }
    if ($exitCode -ne 0 -or -not $joinedOutput.Contains('OpenSSH_for_Windows')) {
        throw "The SSH worker did not preserve the native OpenSSH fallback: $joinedOutput"
    }

    Write-Output 'Windows PowerShell SSH worker argument round-trip passed.'
} finally {
    if (Test-Path -LiteralPath $packagedBootstrap -PathType Leaf) {
        Remove-Item -LiteralPath $packagedBootstrap -Force
    }
}
