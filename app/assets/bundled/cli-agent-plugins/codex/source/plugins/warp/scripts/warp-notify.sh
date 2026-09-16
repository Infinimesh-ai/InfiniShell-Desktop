#!/bin/bash
# Warp notification utility using OSC escape sequences.
# Usage: warp-notify.sh <title> <body>
#
# For structured Warp notifications, title should be "warp://cli-agent"
# and body should be a JSON string matching the cli-agent notification schema.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

# Only emit notifications when we've confirmed the Warp build can render them.
if ! should_use_structured; then
    exit 0
fi

TITLE="${1:-Notification}"
BODY="${2:-}"

# OSC 777 format: \033]777;notify;<title>;<body>\007
# Write directly to /dev/tty to ensure it reaches the terminal
SEQ=$(printf '\033]777;notify;%s;%s\007' "$TITLE" "$BODY")
if [ -n "${TMUX:-}" ]; then
    # tmux 需要 DCS 透传封套，内部每个 ESC 都必须双写；不修改用户透传配置。
    ESC=$'\033'
    SEQ="${SEQ//$ESC/$ESC$ESC}"
    (printf '\033Ptmux;%s\033\\' "$SEQ" > /dev/tty) 2>/dev/null || true
else
    (printf '%s' "$SEQ" > /dev/tty) 2>/dev/null || true
fi
