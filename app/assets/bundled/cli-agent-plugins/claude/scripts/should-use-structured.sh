#!/bin/bash
# Grok 兼容加载 Claude hooks 时必须静默退出，不能切回硬编码 Claude 的旧协议。
if [ -n "${GROK_HOOK_EVENT:-}" ] || [ -n "${GROK_SESSION_ID:-}" ]; then
    exit 0
fi

# 保留上游对于尚不能渲染结构化通知的旧 Warp 发布版本的判断。
LAST_BROKEN_DEV=""
LAST_BROKEN_STABLE="v0.2026.03.25.08.24.stable_05"
LAST_BROKEN_PREVIEW="v0.2026.03.25.08.24.preview_05"

should_use_structured() {
    [ -z "${WARP_CLI_AGENT_PROTOCOL_VERSION:-}" ] && return 1
    [ -z "${WARP_CLIENT_VERSION:-}" ] && return 1
    local threshold=""
    case "$WARP_CLIENT_VERSION" in
        *dev*) threshold="$LAST_BROKEN_DEV" ;;
        *stable*) threshold="$LAST_BROKEN_STABLE" ;;
        *preview*) threshold="$LAST_BROKEN_PREVIEW" ;;
    esac
    if [ -n "$threshold" ] && [[ ! "$WARP_CLIENT_VERSION" > "$threshold" ]]; then
        return 1
    fi
    return 0
}
