#!/bin/bash
# 只转发带原生回合身份的 Stop；hook 继续运行时不报告成功。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"
if ! should_use_structured; then exit 0; fi
source "$SCRIPT_DIR/build-payload.sh"
INPUT=$(cat)
[ "$(printf '%s' "$INPUT" | jq -r '.stop_hook_active // false' 2>/dev/null)" = "true" ] && exit 0

RESPONSE=$(printf '%s' "$INPUT" | jq -r '.last_assistant_message | select(type == "string")' 2>/dev/null)
TRANSCRIPT_PATH=$(printf '%s' "$INPUT" | jq -r '.transcript_path // empty' 2>/dev/null)
EVENT="stop"
[ -z "$RESPONSE" ] && EVENT="notification"
if [ ${#RESPONSE} -gt 200 ]; then RESPONSE="${RESPONSE:0:197}..."; fi
BODY=$(build_payload "$INPUT" "$EVENT" --arg response "$RESPONSE" --arg transcript_path "$TRANSCRIPT_PATH")
"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
