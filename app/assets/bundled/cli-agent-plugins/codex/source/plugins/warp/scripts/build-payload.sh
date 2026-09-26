#!/bin/bash
# 基于 Warp MIT 插件的最小修补：保留 Codex 0.147.0 原生 turn_id。

PLUGIN_CURRENT_PROTOCOL_VERSION=1

negotiate_protocol_version() {
    local warp_version="${WARP_CLI_AGENT_PROTOCOL_VERSION:-1}"
    if [ "$warp_version" -lt "$PLUGIN_CURRENT_PROTOCOL_VERSION" ] 2>/dev/null; then
        echo "$warp_version"
    else
        echo "$PLUGIN_CURRENT_PROTOCOL_VERSION"
    fi
}

# 0.156.1 的 hook 在 app-server 内由外层 shell 派生并脱离控制终端。
# 这里只观察最近的原生进程候选；接收端仍须核验映像、UID、TTY 和 socket peer。
codex_process_evidence() {
    local platform pid command_name next_pid attempt daemon_pid_candidate=""
    local codex_home tty_path=""
    platform=$(uname -s) || return 1
    case "$platform" in Darwin|Linux) ;; *) return 1 ;; esac
    pid="$PPID"
    for attempt in 1 2 3 4 5 6 7 8 9 10 11 12; do
        case "$pid" in ''|*[!0-9]*|0|1) return 1 ;; esac
        if [ "$platform" = Linux ]; then
            command_name=$(readlink "/proc/$pid/exe" 2>/dev/null) || return 1
        else
            command_name=$(ps -o comm= -p "$pid" 2>/dev/null) || return 1
        fi
        if [ "${command_name##*/}" = codex ]; then
            daemon_pid_candidate="$pid"
            break
        fi
        next_pid=$(ps -o ppid= -p "$pid" 2>/dev/null) || return 1
        next_pid="${next_pid//[[:space:]]/}"
        [ "$next_pid" != "$pid" ] || return 1
        pid="$next_pid"
    done
    [ -n "$daemon_pid_candidate" ] || return 1

    # 不读取配置、进程参数或认证文件；相对 HOME 不能作为有效来源。
    if [ -n "${CODEX_HOME:-}" ]; then
        codex_home="$CODEX_HOME"
    else
        [ -n "${HOME:-}" ] || return 1
        codex_home="$HOME/.codex"
    fi
    case "$codex_home" in /*) ;; *) return 1 ;; esac
    codex_home=$(cd -- "$codex_home" 2>/dev/null && pwd -P) || return 1
    if [ -n "${TMUX:-}" ]; then
        case "${TMUX_PANE:-}" in
            %*[!0-9]*|%|'') return 1 ;;
            %*) ;;
            *) return 1 ;;
        esac
        tty_path=$(tmux display-message -p -t "$TMUX_PANE" '#{pane_tty}' 2>/dev/null) || return 1
    elif [ -n "${SSH_TTY:-}" ]; then
        tty_path="$SSH_TTY"
    elif [ -n "${WARP_CLI_AGENT_TTY:-}" ]; then
        tty_path="$WARP_CLI_AGENT_TTY"
    else
        tty_path=$(ps -o tty= -p "$daemon_pid_candidate" 2>/dev/null) || return 1
        tty_path="/dev/${tty_path//[[:space:]]/}"
    fi
    case "$tty_path" in
        /dev/tty|*'..'*|*[![:print:]]*|*' '*) return 1 ;;
        /dev/*) ;;
        *) return 1 ;;
    esac
    [ ! -L "$tty_path" ] && [ -c "$tty_path" ] || return 1
    jq -nc --argjson daemon_pid_candidate "$daemon_pid_candidate" \
        --arg codex_home "$codex_home" --arg tty_path "$tty_path" \
        '{daemon_pid_candidate:$daemon_pid_candidate,codex_home:$codex_home,tty_path:$tty_path}'
}

build_payload() {
    local input="$1" event="$2"
    shift 2
    local protocol_version session_id cwd project turn_id process_evidence
    protocol_version=$(negotiate_protocol_version)
    session_id=$(printf '%s' "$input" | jq -r '.session_id // empty' 2>/dev/null)
    cwd=$(printf '%s' "$input" | jq -r '.cwd // empty' 2>/dev/null)
    turn_id=$(printf '%s' "$input" | jq -r '.turn_id | select(type == "string" and length > 0)' 2>/dev/null)
    project=""
    [ -n "$cwd" ] && project=$(basename "$cwd")

    process_evidence=$(codex_process_evidence) || process_evidence=null

    # 不透传未经此版本证明存在的 event_id、sequence 或 Claude 的 prompt_id。
    jq -anc \
        --argjson v "$protocol_version" \
        --arg agent "codex" --arg event "$event" \
        --arg session_id "$session_id" --arg cwd "$cwd" --arg project "$project" \
        --argjson codex_process_evidence "$process_evidence" \
        --arg turn_id "$turn_id" "$@" '
        {v:$v, agent:$agent, event:$event, session_id:$session_id, cwd:$cwd, project:$project}
        + $ARGS.named
        | if .codex_process_evidence == null then del(.codex_process_evidence) else . end
        | if .turn_id == "" then
            del(.turn_id)
            | if .event == "stop" or .event == "stop_failure" then
                .event = "notification" | .terminal_unverified = true | .error_type = (.error_type // "uncorrelated_hook")
              else . end
          else . end'
}
