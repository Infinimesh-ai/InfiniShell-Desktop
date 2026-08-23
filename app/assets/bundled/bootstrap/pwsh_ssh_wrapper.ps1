[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseApprovedVerbs', '', Scope = 'Function', Target = 'Warp-*', Justification = 'Warp-* functions are ours')]
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidGlobalVars', '', Scope = 'Function', Target = 'Warp-*', Justification = 'Warp session state is intentionally global')]
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '', Scope = 'Function', Target = 'Warp-*', Justification = 'The fallback notice belongs in the interactive terminal')]
param()

# PowerShell 会优先解析别名和函数。安装全局 wrapper 前先保存真实的 OpenSSH
# 可执行文件路径,避免用户 profile 中的 ssh 别名递归调用 wrapper。
$script:WarpSshExecutablePath = $null

function Warp-Test-IsWindows {
    if ($PSVersionTable.PSVersion.Major -le 5) {
        return $true
    }
    return $IsWindows -or $env:OS -eq 'Windows_NT'
}

function Warp-Test-InteractiveSshSession {
    param([object[]]$SshArgs)

    $positionalCount = 0
    $parseOptions = $true
    for ($argumentIndex = 0; $argumentIndex -lt $SshArgs.Count; $argumentIndex++) {
        $argument = [string]$SshArgs[$argumentIndex]
        if ($parseOptions -and $argument -eq '--') {
            $parseOptions = $false
            continue
        }

        if ($parseOptions -and $argument.Length -gt 1 -and $argument[0] -eq '-') {
            if ($argument.StartsWith('--')) {
                return $false
            }

            for ($optionIndex = 1; $optionIndex -lt $argument.Length; $optionIndex++) {
                $option = $argument[$optionIndex]
                # 这些选项改变 session 类型、stdio、转发或复用语义，
                # 当前 Rust transport 不应默默忽略它们。
                if ('DELMNORSWTefnvwy'.Contains($option)) {
                    return $false
                }

                if ('1246AaCfgKkMNnqsTtVvXxYy'.Contains($option)) {
                    continue
                }

                if ('bcDeFiJLlmoOpRSWw'.Contains($option)) {
                    if ($optionIndex + 1 -eq $argument.Length) {
                        $argumentIndex++
                        if ($argumentIndex -ge $SshArgs.Count) {
                            return $false
                        }
                    }
                    break
                }

                return $false
            }
            continue
        }

        $positionalCount++
    }

    return $positionalCount -eq 1
}

function Warp-Invoke-SshExecutable {
    param([object[]]$SshArgs)

    & $script:WarpSshExecutablePath @SshArgs
}

function Warp-Invoke-PlainSsh {
    param([object[]]$SshArgs)

    Warp-Invoke-SshExecutable -SshArgs $SshArgs
}

function Warp-Get-SshConfigValue {
    param(
        [object[]]$SshArgs,
        [string]$Name
    )

    $configOutput = Warp-Invoke-SshExecutable -SshArgs (@('-G') + $SshArgs) 2>$null
    if ($global:LASTEXITCODE -ne 0) {
        return $null
    }

    $value = $null
    foreach ($line in @($configOutput)) {
        if ([string]$line -match "^$([Regex]::Escape($Name))\s+(.*)$") {
            $value = $Matches[1]
        }
    }
    return $value
}

function Warp-New-RemoteSessionId {
    $randomNumberGenerator = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        for ($attempt = 0; $attempt -lt 4; $attempt++) {
            $bytes = New-Object byte[] 8
            $randomNumberGenerator.GetBytes($bytes)
            $sessionId = [BitConverter]::ToUInt64($bytes, 0)
            if ($sessionId -ne 0) {
                return $sessionId
            }
        }
    } finally {
        $randomNumberGenerator.Dispose()
    }
    return [UInt64]0
}

function Warp-Get-RemoteShellFromProbeOutput {
    param([object[]]$ProbeOutput)

    $marker = '__WARP_REMOTE_SHELL__'
    $remoteShell = ''
    foreach ($outputItem in @($ProbeOutput)) {
        foreach ($line in ([string]$outputItem -split "`r?`n")) {
            if ($line.StartsWith($marker)) {
                $remoteShell = $line.Substring($marker.Length).TrimEnd("`r")
            }
        }
    }
    return $remoteShell
}

function Warp-Test-RemoteShellSupportsBootstrap {
    param([string]$RemoteShell)

    return $RemoteShell -cmatch '(^|/)(bash|zsh)$'
}

function Warp-New-PowerShellCapabilityProbeCommand {
    $probeScript = @'
$os = if ($PSVersionTable.PSVersion.Major -le 5 -or $IsWindows -or $env:OS -eq 'Windows_NT') { 'windows' } else { 'unknown' }
[Console]::Out.WriteLine('__WARP_REMOTE_CAPS__v=1;os={0};shell=powershell' -f $os)
'@
    $encodedProbe = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($probeScript))
    return "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand $encodedProbe"
}

function Warp-Get-PowerShellCapabilityFromProbeOutput {
    param([object[]]$ProbeOutput)

    $lines = @()
    foreach ($outputItem in @($ProbeOutput)) {
        foreach ($line in ([string]$outputItem -split "`r?`n")) {
            $line = $line.TrimEnd("`r")
            if (-not [String]::IsNullOrWhiteSpace($line)) {
                $lines += $line
            }
        }
    }
    if ($lines.Count -ne 1) {
        return $null
    }
    if ($lines[0] -cmatch '^__WARP_REMOTE_CAPS__v=1;os=windows;shell=(powershell|pwsh)$') {
        return $Matches[1]
    }
    return $null
}

function Warp-Test-ControlPathIsSafe {
    param([string]$ControlPath)

    return -not [String]::IsNullOrEmpty($ControlPath) -and $ControlPath -match '^[A-Za-z0-9._/~@:+,-]+$'
}

function Warp-Get-SafeRemoteEnvironmentValue {
    param([string]$Value)

    if (-not [String]::IsNullOrEmpty($Value) -and $Value -match '^[A-Za-z0-9._+-]+$') {
        return $Value
    }
    return ''
}

function Warp-Get-SafeRemoteRelativePath {
    param([string]$Value)

    if ([String]::IsNullOrEmpty($Value) -or $Value -notmatch '^[A-Za-z0-9._/+-]+$' -or $Value.StartsWith('/')) {
        return ''
    }
    foreach ($segment in ($Value -split '/')) {
        if ([String]::IsNullOrEmpty($segment) -or $segment -eq '.' -or $segment -eq '..') {
            return ''
        }
    }
    return $Value
}

function Warp-Get-NextSshHopDepth {
    if ([String]::IsNullOrEmpty($env:WARP_SSH_HOP_DEPTH)) {
        return 1
    }
    [UInt32]$currentDepth = 0
    if (-not [UInt32]::TryParse($env:WARP_SSH_HOP_DEPTH, [ref]$currentDepth) -or $currentDepth -ge 8) {
        return 9
    }
    return $currentDepth + 1
}

function Warp-Encode-NativeCommandArgument {
    param([string]$Command)

    return [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Command))
}

function Warp-New-RustSshWorkerArguments {
    param(
        [UInt64]$SessionId,
        [UInt64]$RemoteSessionId,
        [string]$ControlScope = 'local',
        [UInt32]$HopDepth = 1,
        [string]$SshExecutable,
        [string]$PosixCommand,
        [string]$WindowsCommand,
        [object[]]$SshArgs
    )

    # Windows PowerShell 5.1 会用 legacy 规则重建 native command line。原始脚本中的
    # 双引号会打断参数边界，例如把 `command -p mktemp` 的 `-p` 暴露给 clap。
    $posixCommandBase64 = Warp-Encode-NativeCommandArgument $PosixCommand
    $windowsCommandBase64 = Warp-Encode-NativeCommandArgument $WindowsCommand
    return @(
        'rust-ssh-session',
        '--session-id', [string]$SessionId,
        '--remote-session-id', [string]$RemoteSessionId,
        '--control-scope', $ControlScope,
        '--hop-depth', [string]$HopDepth,
        '--ssh-executable', $SshExecutable,
        '--commands-base64',
        '--posix-command', $posixCommandBase64,
        '--windows-command', $windowsCommandBase64,
        '--'
    ) + $SshArgs
}

function Warp-New-RemoteBootstrapCommand {
    param(
        [UInt64]$RemoteSessionId,
        [string]$SshHookHex,
        [bool]$EmitSshHook = $true
    )

    $honorPs1 = if ($env:WARP_HONOR_PS1 -eq '1') { '1' } else { '0' }
    $bashInitShell = "WARP_HONOR_PS1='$honorPs1'`n" +
        $script:WarpBashInitShell.Replace('@@WARP_SESSION_ID@@', [string]$RemoteSessionId).Replace("`r`n", "`n")
    $zshInitShell = "unsetopt RCS GLOBAL_RCS`nWARP_HONOR_PS1='$honorPs1'`n" +
        $script:WarpZshInitShell.Replace('@@WARP_SESSION_ID@@', [string]$RemoteSessionId).Replace("`r`n", "`n")
    $bashInitShellHex = Warp-Encode-HexString $bashInitShell
    $zshInitShellHex = Warp-Encode-HexString $zshInitShell
    $clientVersion = Warp-Get-SafeRemoteEnvironmentValue $env:WARP_CLIENT_VERSION
    $protocolVersion = Warp-Get-SafeRemoteEnvironmentValue $env:WARP_CLI_AGENT_PROTOCOL_VERSION
    $useSshWrapper = if ($env:WARP_USE_SSH_WRAPPER -eq '1') { '1' } else { '0' }
    $reuseControlMaster = if ($env:WARP_SSH_REUSE_CONTROL_MASTER -eq '1') { '1' } else { '0' }
    $recursiveSsh = if ($env:WARP_RECURSIVE_SSH_EXTENSION -eq '1') { '1' } else { '0' }
    $nextHopDepth = Warp-Get-NextSshHopDepth

    $sshHookCommand = if ($EmitSshHook) {
        "printf '\e]9278;d;%s\x07' '$SshHookHex'"
    } else {
        ':'
    }

    $remoteCommand = @'
export TERM_PROGRAM='WarpTerminal'
export WARP_IS_SSH='1'
export WARP_USE_SSH_WRAPPER='__WARP_USE_SSH_WRAPPER__'
export WARP_SSH_REUSE_CONTROL_MASTER='__WARP_SSH_REUSE_CONTROL_MASTER__'
test -n '__WARP_CLIENT_VERSION__' && export WARP_CLIENT_VERSION='__WARP_CLIENT_VERSION__'
test -n '__WARP_PROTOCOL_VERSION__' && export WARP_CLI_AGENT_PROTOCOL_VERSION='__WARP_PROTOCOL_VERSION__'
export WARP_RECURSIVE_SSH_EXTENSION='__WARP_RECURSIVE_SSH__'
export WARP_SSH_HOP_DEPTH='__WARP_SSH_HOP_DEPTH__'
SSH_SOCKET_DIR="${XDG_RUNTIME_DIR:-$HOME/.cache}/infinishell-ssh"
export SSH_SOCKET_DIR
command -p mkdir -p "$SSH_SOCKET_DIR" && command -p chmod 700 "$SSH_SOCKET_DIR"
__WARP_SSH_HOOK_COMMAND__

warp_decode_hex() {
  if command -pv xxd >/dev/null 2>&1; then
    printf '%s' "$1" | command -p xxd -p -r
  else
    _warp_hex="$1"
    _warp_hex_index=0
    while test "$_warp_hex_index" -lt "${#_warp_hex}"; do
      builtin printf "\\x${_warp_hex:$_warp_hex_index:2}"
      _warp_hex_index=$((_warp_hex_index + 2))
    done
    unset _warp_hex _warp_hex_index
  fi
}

case "${SHELL##*/}" in
  bash)
    WARP_BASH_INIT_SHELL_HEX='__WARP_BASH_INIT_SHELL_HEX__'
    exec -a bash bash --rcfile <(warp_decode_hex "$WARP_BASH_INIT_SHELL_HEX")
    ;;
  zsh)
    WARP_TMP_DIR="$(command -p mktemp -d warptmp.XXXXXX)"
    WARP_ZSH_INIT_SHELL_HEX='__WARP_ZSH_INIT_SHELL_HEX__'
    if test -n "$WARP_TMP_DIR" && test -d "$WARP_TMP_DIR"; then
      warp_decode_hex "$WARP_ZSH_INIT_SHELL_HEX" > "$WARP_TMP_DIR/.zshenv"
    else
      echo 'Failed to bootstrap InfiniShell. Continuing with a non-bootstrapped shell.'
      exec -l zsh -g
    fi
    TMPPREFIX="$HOME/.zshtmp-" WARP_SSH_RCFILES="${ZDOTDIR:-$HOME}" ZDOTDIR="$WARP_TMP_DIR" exec -l zsh -g
    ;;
esac
'@

    $remoteCommand = $remoteCommand.Replace('__WARP_CLIENT_VERSION__', $clientVersion)
    $remoteCommand = $remoteCommand.Replace('__WARP_PROTOCOL_VERSION__', $protocolVersion)
    $remoteCommand = $remoteCommand.Replace('__WARP_USE_SSH_WRAPPER__', $useSshWrapper)
    $remoteCommand = $remoteCommand.Replace('__WARP_SSH_REUSE_CONTROL_MASTER__', $reuseControlMaster)
    $remoteCommand = $remoteCommand.Replace('__WARP_RECURSIVE_SSH__', $recursiveSsh)
    $remoteCommand = $remoteCommand.Replace('__WARP_SSH_HOP_DEPTH__', [string]$nextHopDepth)
    $remoteCommand = $remoteCommand.Replace('__WARP_SSH_HOOK_COMMAND__', $sshHookCommand)
    $remoteCommand = $remoteCommand.Replace('__WARP_BASH_INIT_SHELL_HEX__', $bashInitShellHex)
    $remoteCommand = $remoteCommand.Replace('__WARP_ZSH_INIT_SHELL_HEX__', $zshInitShellHex)
    return $remoteCommand.Replace("`r`n", "`n")
}

function Warp-New-WindowsBootstrapCommand {
    param(
        [UInt64]$RemoteSessionId,
        [string]$SshHookHex,
        [bool]$EmitSshHook = $true
    )

    $clientVersion = Warp-Get-SafeRemoteEnvironmentValue $env:WARP_CLIENT_VERSION
    $protocolVersion = Warp-Get-SafeRemoteEnvironmentValue $env:WARP_CLI_AGENT_PROTOCOL_VERSION
    $recursiveSsh = if ($env:WARP_RECURSIVE_SSH_EXTENSION -eq '1') { '1' } else { '0' }
    $nextHopDepth = Warp-Get-NextSshHopDepth
    $remoteWorkerRelativePath = Warp-Get-SafeRemoteRelativePath $env:WARP_REMOTE_SSH_EXECUTABLE_RELATIVE_PATH
    $initShell = $script:WarpPwshInitShell.Replace('@@WARP_SESSION_ID@@', [string]$RemoteSessionId)
    $sshHookCommand = if ($EmitSshHook) {
        "[Console]::Out.Write(([char]27) + ']9278;d;$SshHookHex' + ([char]7))"
    } else {
        ''
    }
    $bootstrapScript = @"
`$env:TERM_PROGRAM = 'WarpTerminal'
`$env:WARP_IS_SSH = '1'
`$env:WARP_IS_LOCAL_SHELL_SESSION = '0'
`$env:WARP_CLIENT_VERSION = '$clientVersion'
`$env:WARP_CLI_AGENT_PROTOCOL_VERSION = '$protocolVersion'
`$env:WARP_RECURSIVE_SSH_EXTENSION = '$recursiveSsh'
`$env:WARP_SSH_HOP_DEPTH = '$nextHopDepth'
`$env:WARP_REMOTE_SSH_EXECUTABLE_RELATIVE_PATH = '$remoteWorkerRelativePath'
if (-not [String]::IsNullOrEmpty('$remoteWorkerRelativePath')) {
    `$env:WARP_RUST_SSH_EXECUTABLE = Join-Path `$HOME '$remoteWorkerRelativePath'
}
$sshHookCommand
$initShell
"@
    $encodedBootstrap = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($bootstrapScript))
    return "powershell.exe -NoLogo -NoExit -EncodedCommand $encodedBootstrap"
}

function Warp-Close-OwnedControlMaster {
    param(
        [string]$ControlPath,
        [object[]]$SshArgs
    )

    Warp-Invoke-SshExecutable -SshArgs (@('-O', 'exit', '-o', "ControlPath=$ControlPath") + $SshArgs) *> $null
}

function Warp-New-SshHookHex {
    param(
        [UInt64]$RemoteSessionId,
        [string]$RemoteShell,
        [Collections.IDictionary]$Transport,
        [string]$SocketPath,
        [bool]$ExternalControlMaster
    )

    $hookValue = @{
        transport = $Transport
        remote_shell = $RemoteShell
        session_id = $global:_warpSessionId
        remote_session_id = $RemoteSessionId
        external_control_master = $ExternalControlMaster
    }
    if (-not [String]::IsNullOrEmpty($SocketPath)) {
        $hookValue.socket_path = $SocketPath
    }

    $sshHook = ConvertTo-Json -Compress -InputObject @{
        hook = 'SSH'
        value = $hookValue
    }
    return Warp-Encode-HexString $sshHook
}

function Warp-Invoke-DirectEnhancedSsh {
    param(
        [object[]]$SshArgs,
        [UInt64]$RemoteSessionId
    )

    # Windows OpenSSH 没有 ControlMaster。由 Rust worker 持有唯一连接，
    # 远端探测、交互 shell 与 remote-server exec channel 全部复用该 session。
    $workerPath = $env:WARP_RUST_SSH_EXECUTABLE
    if ([String]::IsNullOrWhiteSpace($workerPath) -or -not (Test-Path -LiteralPath $workerPath -PathType Leaf)) {
        Warp-Invoke-PlainSsh -SshArgs $SshArgs
        return
    }

    $posixBootstrapCommand = Warp-New-RemoteBootstrapCommand `
        -RemoteSessionId $RemoteSessionId `
        -SshHookHex '' `
        -EmitSshHook $false
    $windowsBootstrapCommand = Warp-New-WindowsBootstrapCommand `
        -RemoteSessionId $RemoteSessionId `
        -SshHookHex '' `
        -EmitSshHook $false
    $controlScope = if ($env:WARP_IS_SSH -eq '1') { 'remote' } else { 'local' }
    $hopDepth = Warp-Get-NextSshHopDepth
    $workerArgs = Warp-New-RustSshWorkerArguments `
        -SessionId $global:_warpSessionId `
        -RemoteSessionId $RemoteSessionId `
        -ControlScope $controlScope `
        -HopDepth $hopDepth `
        -SshExecutable $script:WarpSshExecutablePath `
        -PosixCommand $posixBootstrapCommand `
        -WindowsCommand $windowsBootstrapCommand `
        -SshArgs $SshArgs
    & $workerPath @workerArgs
}

function Warp-Invoke-EnhancedSsh {
    param([object[]]$SshArgs)

    if ($null -eq $global:_warpSessionId) {
        Warp-Invoke-PlainSsh -SshArgs $SshArgs
        return
    }
    if ((Warp-Get-NextSshHopDepth) -gt 8) {
        Warp-Invoke-PlainSsh -SshArgs $SshArgs
        return
    }

    $remoteCommand = Warp-Get-SshConfigValue -SshArgs $SshArgs -Name 'remotecommand'
    if (-not [String]::IsNullOrEmpty($remoteCommand) -and $remoteCommand -ne 'none') {
        Warp-Invoke-PlainSsh -SshArgs $SshArgs
        return
    }

    $remoteSessionId = Warp-New-RemoteSessionId
    if ($remoteSessionId -eq 0) {
        Warp-Invoke-PlainSsh -SshArgs $SshArgs
        return
    }

    if (Warp-Test-IsWindows) {
        Warp-Invoke-DirectEnhancedSsh -SshArgs $SshArgs -RemoteSessionId $remoteSessionId
        return
    }

    if ([String]::IsNullOrEmpty($env:SSH_SOCKET_DIR)) {
        Warp-Invoke-PlainSsh -SshArgs $SshArgs
        return
    }

    $controlPath = Join-Path -Path $env:SSH_SOCKET_DIR -ChildPath ([string]$global:_warpSessionId)
    $controlMasterMode = 'yes'
    $externalControlMaster = $false
    if ($env:WARP_SSH_REUSE_CONTROL_MASTER -eq '1') {
        $userControlPath = Warp-Get-SshConfigValue -SshArgs $SshArgs -Name 'controlpath'
        if ($userControlPath -ne 'none' -and (Warp-Test-ControlPathIsSafe $userControlPath)) {
            Warp-Invoke-SshExecutable -SshArgs (@('-O', 'check', '-o', "ControlPath=$userControlPath") + $SshArgs) *> $null
            if ($global:LASTEXITCODE -eq 0) {
                $controlPath = $userControlPath
                $controlMasterMode = 'no'
                $externalControlMaster = $true
            }
        }
    }

    $probeArgs = @(
        '-o', "ControlMaster=$controlMasterMode",
        '-o', 'ControlPersist=60',
        '-o', "ControlPath=$controlPath"
    ) + $SshArgs + @('echo __WARP_REMOTE_SHELL__$SHELL')
    $probeOutput = Warp-Invoke-SshExecutable -SshArgs $probeArgs
    $probeStatus = $global:LASTEXITCODE
    if ($probeStatus -ne 0) {
        Warp-Invoke-SshExecutable -SshArgs (@('-O', 'check', '-o', "ControlPath=$controlPath") + $SshArgs) *> $null
        if ($global:LASTEXITCODE -ne 0) {
            $global:LASTEXITCODE = $probeStatus
            return
        }
    }

    $remoteShell = Warp-Get-RemoteShellFromProbeOutput $probeOutput
    if (-not (Warp-Test-RemoteShellSupportsBootstrap $remoteShell)) {
        # POSIX 探测无法识别 Windows OpenSSH 的默认 PowerShell。复用已认证的
        # ControlMaster 运行固定、版本化的二次探测，命中后直接进入 Windows
        # bootstrap；不再把已经识别的 Windows 远端退回普通 SSH。
        $remotePowerShell = $null
        if ([String]::IsNullOrEmpty($remoteShell) -or $remoteShell -ceq '$SHELL') {
            $capabilityProbeCommand = Warp-New-PowerShellCapabilityProbeCommand
            $capabilityProbeOutput = Warp-Invoke-SshExecutable -SshArgs (
                @('-o', 'ControlMaster=no', '-o', "ControlPath=$controlPath") +
                $SshArgs + @($capabilityProbeCommand)
            )
            $capabilityProbeStatus = $global:LASTEXITCODE
            if ($capabilityProbeStatus -eq 0) {
                $remotePowerShell = Warp-Get-PowerShellCapabilityFromProbeOutput $capabilityProbeOutput
            }
        }
        if (-not [String]::IsNullOrEmpty($remotePowerShell)) {
            Write-Verbose "Detected versioned Windows PowerShell SSH capability: $remotePowerShell"
            $controlMasterOwnership = if ($externalControlMaster) { 'user_owned' } else { 'warp_managed' }
            $sshHookHex = Warp-New-SshHookHex `
                -RemoteSessionId $remoteSessionId `
                -RemoteShell 'pwsh' `
                -Transport @{
                    version = 1
                    type = 'control_master'
                    socket_path = $controlPath
                    ownership = $controlMasterOwnership
                } `
                -SocketPath $controlPath `
                -ExternalControlMaster $externalControlMaster
            $windowsBootstrapCommand = Warp-New-WindowsBootstrapCommand `
                -RemoteSessionId $remoteSessionId `
                -SshHookHex $sshHookHex
            Warp-Invoke-SshExecutable -SshArgs (
                @('-o', 'ControlMaster=no', '-o', "ControlPath=$controlPath", '-t') +
                $SshArgs + @($windowsBootstrapCommand)
            )
            return
        }
        Write-Host 'InfiniShell shell integration is unavailable for this remote shell; continuing with standard SSH.'
        Warp-Invoke-SshExecutable -SshArgs (@('-o', 'ControlMaster=no', '-o', "ControlPath=$controlPath", '-t') + $SshArgs)
        $sshStatus = $global:LASTEXITCODE
        if (-not $externalControlMaster) {
            Warp-Close-OwnedControlMaster -ControlPath $controlPath -SshArgs $SshArgs
        }
        $global:LASTEXITCODE = $sshStatus
        return
    }

    $remoteShellName = $remoteShell.Substring($remoteShell.LastIndexOf('/') + 1)
    $controlMasterOwnership = if ($externalControlMaster) { 'user_owned' } else { 'warp_managed' }
    $sshHookHex = Warp-New-SshHookHex `
        -RemoteSessionId $remoteSessionId `
        -RemoteShell $remoteShellName `
        -Transport @{
            version = 1
            type = 'control_master'
            socket_path = $controlPath
            ownership = $controlMasterOwnership
        } `
        -SocketPath $controlPath `
        -ExternalControlMaster $externalControlMaster

    $bootstrapCommand = Warp-New-RemoteBootstrapCommand -RemoteSessionId $remoteSessionId -SshHookHex $sshHookHex
    Warp-Invoke-SshExecutable -SshArgs (@('-o', 'ControlMaster=no', '-o', "ControlPath=$controlPath", '-t') + $SshArgs + @($bootstrapCommand))
}

function Warp-Ssh {
    $sshArgs = @($args)
    if (Warp-Test-InteractiveSshSession -SshArgs $sshArgs) {
        Warp-Send-JsonMessage @{
            hook = 'PreInteractiveSSHSession'
            value = @{ session_id = $global:_warpSessionId }
        }

        if ($env:WARP_USE_SSH_WRAPPER -eq '1') {
            Warp-Invoke-EnhancedSsh -SshArgs $sshArgs
        } else {
            Warp-Invoke-PlainSsh -SshArgs $sshArgs
        }
    } else {
        Warp-Invoke-PlainSsh -SshArgs $sshArgs
    }
}

function Warp-Install-SshWrapper {
    $isRecursiveRemote = $env:WARP_IS_SSH -eq '1' -and $env:WARP_RECURSIVE_SSH_EXTENSION -eq '1'
    if ($env:WARP_IS_LOCAL_SHELL_SESSION -ne '1' -and -not $isRecursiveRemote) {
        return
    }

    $sshCommand = Get-Command -Name ssh -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -eq $sshCommand) {
        return
    }

    $script:WarpSshExecutablePath = $sshCommand.Source
    try {
        Set-Item -Path Function:global:ssh -Value (Get-Command Warp-Ssh).ScriptBlock -Force -ErrorAction Stop
        if (Test-Path Alias:ssh) {
            Remove-Item -Path Alias:ssh -Force -ErrorAction Stop
        }
    } catch {
        Write-Verbose "无法安装 InfiniShell SSH wrapper: $($_.Exception.Message)"
    }
}
