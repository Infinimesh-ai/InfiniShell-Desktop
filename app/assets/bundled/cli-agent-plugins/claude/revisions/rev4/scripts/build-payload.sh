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

build_payload() {
    local input="$1" event="$2"
    shift 2
    local protocol_version session_id cwd project prompt_id
    protocol_version=$(negotiate_protocol_version)
    session_id=$(printf '%s' "$input" | jq -r '.session_id // empty' 2>/dev/null)
    cwd=$(printf '%s' "$input" | jq -r '.cwd // empty' 2>/dev/null)
    prompt_id=$(printf '%s' "$input" | jq -r '.prompt_id | select(type == "string" and length > 0)' 2>/dev/null)
    project=""
    [ -n "$cwd" ] && project=$(basename "$cwd")

    # ASCII JSON 使 C0/C1 均无法逃逸 OSC，终端解析后仍保留中英文原文。
    jq -anc \
        --argjson v "$protocol_version" \
        --arg agent "claude" --arg event "$event" \
        --arg session_id "$session_id" --arg cwd "$cwd" --arg project "$project" \
        --arg prompt_id "$prompt_id" "$@" '
        {v:$v, agent:$agent, event:$event, session_id:$session_id, cwd:$cwd, project:$project}
        + $ARGS.named
        | if .prompt_id == "" then
            del(.prompt_id)
            | if .event == "stop" or .event == "stop_failure" then
                .event = "notification" | .terminal_unverified = true | .error_type = (.error_type // "uncorrelated_hook")
              else . end
          else . end'
}
