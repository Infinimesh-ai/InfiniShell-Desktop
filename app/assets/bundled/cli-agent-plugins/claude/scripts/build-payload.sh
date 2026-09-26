#!/bin/bash
# 基于 Warp MIT 插件的最小修补：保留 Claude 原生 prompt_id，不生成伪序号。

PLUGIN_CURRENT_PROTOCOL_VERSION=1

negotiate_protocol_version() {
    local warp_version="${WARP_CLI_AGENT_PROTOCOL_VERSION:-1}"
    if [ "$warp_version" -lt "$PLUGIN_CURRENT_PROTOCOL_VERSION" ] 2>/dev/null; then
        echo "$warp_version"
    else
        echo "$PLUGIN_CURRENT_PROTOCOL_VERSION"
    fi
}

# Claude 原生 hook 可由额外 shell 派生；只提供候选，不授予输入权限。
# 这里只观察最近的原生进程候选；接收端仍须核验映像、UID、TTY 和 socket peer。
claude_process_evidence() {
    local platform pid command_name next_pid attempt process_id_candidate=""
    local claude_config_directory tty_path=""
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
        if [ "${command_name##*/}" = claude ] || [ "${command_name##*/}" = 2.1.280 ]; then
            process_id_candidate="$pid"
            break
        fi
        next_pid=$(ps -o ppid= -p "$pid" 2>/dev/null) || return 1
        next_pid="${next_pid//[[:space:]]/}"
        [ "$next_pid" != "$pid" ] || return 1
        pid="$next_pid"
    done
    [ -n "$process_id_candidate" ] || return 1

    # 不读取配置、进程参数或认证文件；相对 HOME 不能作为有效来源。
    if [ -n "${CLAUDE_CONFIG_DIR:-}" ]; then
        claude_config_directory="$CLAUDE_CONFIG_DIR"
    else
        [ -n "${HOME:-}" ] || return 1
        claude_config_directory="$HOME/.claude"
    fi
    case "$claude_config_directory" in /*) ;; *) return 1 ;; esac
    claude_config_directory=$(cd -- "$claude_config_directory" 2>/dev/null && pwd -P) || return 1
    tty_path=$(ps -o tty= -p "$process_id_candidate" 2>/dev/null) || return 1
    tty_path="/dev/${tty_path//[[:space:]]/}"
    case "$tty_path" in
        /dev/tty|*'..'*|*[![:print:]]*|*' '*) return 1 ;;
        /dev/*) ;;
        *) return 1 ;;
    esac
    [ ! -L "$tty_path" ] && [ -c "$tty_path" ] || return 1
    jq -nc --argjson process_id_candidate "$process_id_candidate" \
        --arg claude_config_directory "$claude_config_directory" --arg tty_path "$tty_path" \
        '{process_id_candidate:$process_id_candidate,claude_config_directory:$claude_config_directory,tty_path:$tty_path}'
}

build_payload() {
    local input="$1" event="$2"
    shift 2
    local protocol_version session_id cwd project prompt_id process_evidence transcript_path
    protocol_version=$(negotiate_protocol_version)
    session_id=$(printf '%s' "$input" | jq -r '.session_id // empty' 2>/dev/null)
    cwd=$(printf '%s' "$input" | jq -r '.cwd // empty' 2>/dev/null)
    prompt_id=$(printf '%s' "$input" | jq -r '.prompt_id | select(type == "string" and length > 0)' 2>/dev/null)
    project=""
    [ -n "$cwd" ] && project=$(basename "$cwd")

    process_evidence=$(claude_process_evidence) || process_evidence=null
    transcript_path=$(printf '%s' "$input" | jq -r '.transcript_path | select(type == "string" and length > 0)' 2>/dev/null)

    # ASCII JSON 使 C0/C1 均无法逃逸 OSC，终端解析后仍保留中英文原文。
    jq -anc \
        --argjson v "$protocol_version" \
        --arg agent "claude" --arg event "$event" \
        --arg session_id "$session_id" --arg cwd "$cwd" --arg project "$project" \
        --argjson claude_process_evidence "$process_evidence" \
        --arg transcript_path "$transcript_path" \
        --arg prompt_id "$prompt_id" "$@" '
        {v:$v, agent:$agent, event:$event, session_id:$session_id, cwd:$cwd, project:$project}
        + $ARGS.named
        | if .claude_process_evidence == null then del(.claude_process_evidence) else . end
        | if .transcript_path == "" then del(.transcript_path) else . end
        | if .prompt_id == "" then
            del(.prompt_id)
            | if .event == "stop" or .event == "stop_failure" then
                .event = "notification" | .terminal_unverified = true | .error_type = (.error_type // "uncorrelated_hook")
              else . end
          else . end'
}
