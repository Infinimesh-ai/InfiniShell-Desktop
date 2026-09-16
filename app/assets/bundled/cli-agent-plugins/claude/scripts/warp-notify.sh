#!/bin/bash
# Warp notification utility using OSC escape sequences
# Usage: warp-notify.sh <title> <body>
#
# For structured Warp notifications, title should be "warp://cli-agent"
# and body should be a JSON string matching the cli-agent notification schema.
#
# Output behavior:
#   - On old Claude Code: writes OSC 777 directly to /dev/tty (no stdout)
#   - On new Claude Code (>= 2.1.141): prints {terminalSequence: ...} JSON to
#     stdout so the caller can pass it through as hook output

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"
source "$SCRIPT_DIR/emit-terminal-sequence.sh"

# Only emit notifications when we've confirmed the Warp build can render them.
if ! should_use_structured; then
    exit 0
fi

TITLE="${1:-Notification}"
BODY="${2:-}"

# OSC 777 format: \033]777;notify;<title>;<body>\007
SEQ=$(printf '\033]777;notify;%s;%s\007' "$TITLE" "$BODY")
# 新版 terminalSequence 只接收原始 OSC，由 Claude 自己处理 tmux；DCS 只用于兼容 TTY。
if [ -n "${TMUX:-}" ] && ! _supports_terminal_sequence; then
    ESC=$'\033'
    TMUX_SEQ="${SEQ//$ESC/$ESC$ESC}"
    if (printf '\033Ptmux;%s\033\\' "$TMUX_SEQ" > /dev/tty) 2>/dev/null; then
        exit 0
    fi
    # 没有控制终端时仍交还原始 OSC，不能把 DCS 放入 JSON 回退字段。
fi
emit_terminal_sequence "$SEQ"
