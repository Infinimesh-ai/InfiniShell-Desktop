function warp_remote_shell_from_probe_output() {
    local probe_output="$1"
    local marker="__WARP_REMOTE_SHELL__"
    local line
    local remote_shell=""

    while IFS= read -r line; do
        case "$line" in
            "$marker"*) remote_shell="${line#"$marker"}" ;;
        esac
    done <<< "$probe_output"

    printf '%s' "${remote_shell%$'\r'}"
}

function warp_remote_shell_supports_bootstrap() {
    case "$1" in
        bash | */bash | zsh | */zsh) return 0 ;;
        *) return 1 ;;
    esac
}

function warp_remote_shell_may_be_windows() {
    [[ -z "$1" || "$1" == '$SHELL' ]]
}

function warp_powershell_encoded_command() {
    if ! command -v iconv >/dev/null 2>&1 || ! command -v base64 >/dev/null 2>&1; then
        return 1
    fi

    local encoded_command
    encoded_command=$(printf '%s' "$1" | command iconv -f UTF-8 -t UTF-16LE | command base64 | command -p tr -d '\r\n') || return 1
    if [[ -z "$encoded_command" ]]; then
        return 1
    fi
    printf 'powershell.exe -NoLogo %s-EncodedCommand %s' "$2" "$encoded_command"
}

function warp_windows_powershell_capability_probe_command() {
    local probe_script="\$os=if(\$PSVersionTable.PSVersion.Major -le 5 -or \$IsWindows -or \$env:OS -eq 'Windows_NT'){'windows'}else{'unknown'};[Console]::Out.WriteLine('__WARP_REMOTE_CAPS__v=1;os={0};shell=powershell' -f \$os)"
    warp_powershell_encoded_command "$probe_script" '-NoProfile -NonInteractive '
}

function warp_windows_powershell_from_probe_output() {
    local probe_output="$1"
    local line
    local capability=""
    local nonempty_line_count=0

    while IFS= read -r line; do
        line="${line%$'\r'}"
        if [[ -n "$line" ]]; then
            nonempty_line_count=$((nonempty_line_count + 1))
            capability="$line"
        fi
    done <<< "$probe_output"

    if [[ $nonempty_line_count -eq 1 && "$capability" == '__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell' ]]; then
        printf 'powershell'
    fi
}

function warp_windows_powershell_bootstrap_command() {
    local remote_session_id="$1"
    local ssh_hook_hex="$2"
    local client_version="$3"
    local protocol_version="$4"
    local init_shell_hex='@@WARP_WINDOWS_REMOTE_INIT_SHELL_HEX@@'

    case "$client_version" in *[![:alnum:]._+-]*) client_version="" ;; esac
    case "$protocol_version" in *[![:alnum:]._+-]*) protocol_version="" ;; esac

    local bootstrap_script="\$env:TERM_PROGRAM = 'WarpTerminal'
\$env:WARP_IS_SSH = '1'
\$env:WARP_CLIENT_VERSION = '$client_version'
\$env:WARP_CLI_AGENT_PROTOCOL_VERSION = '$protocol_version'
[Console]::Out.Write(([char]27) + ']9278;d;$ssh_hook_hex' + ([char]7))
\$h = '$init_shell_hex'
\$bytes = New-Object byte[] (\$h.Length / 2)
for (\$i = 0; \$i -lt \$bytes.Length; \$i++) { \$bytes[\$i] = [Convert]::ToByte(\$h.Substring(\$i * 2, 2), 16) }
\$initShell = [Text.Encoding]::UTF8.GetString(\$bytes).Replace('@@WARP_SESSION_ID@@', '$remote_session_id')
. ([ScriptBlock]::Create(\$initShell))"
    warp_powershell_encoded_command "$bootstrap_script" '-NoExit '
}
