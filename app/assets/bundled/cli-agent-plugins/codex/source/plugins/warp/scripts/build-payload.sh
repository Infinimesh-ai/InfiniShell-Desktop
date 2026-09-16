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

build_payload() {
    local input="$1" event="$2"
    shift 2
    local protocol_version session_id cwd project turn_id
    protocol_version=$(negotiate_protocol_version)
    session_id=$(printf '%s' "$input" | jq -r '.session_id // empty' 2>/dev/null)
    cwd=$(printf '%s' "$input" | jq -r '.cwd // empty' 2>/dev/null)
    turn_id=$(printf '%s' "$input" | jq -r '.turn_id | select(type == "string" and length > 0)' 2>/dev/null)
    project=""
    [ -n "$cwd" ] && project=$(basename "$cwd")

    # 不透传未经此版本证明存在的 event_id、sequence 或 Claude 的 prompt_id。
    jq -anc \
        --argjson v "$protocol_version" \
        --arg agent "codex" --arg event "$event" \
        --arg session_id "$session_id" --arg cwd "$cwd" --arg project "$project" \
        --arg turn_id "$turn_id" "$@" '
        {v:$v, agent:$agent, event:$event, session_id:$session_id, cwd:$cwd, project:$project}
        + $ARGS.named
        | if .turn_id == "" then
            del(.turn_id)
            | if .event == "stop" or .event == "stop_failure" then
                .event = "notification" | .terminal_unverified = true | .error_type = (.error_type // "uncorrelated_hook")
              else . end
          else . end'
}
