#!/bin/bash
# 只使用本次 Stop 的载荷，避免读取异步转录时串入下一条提示词。

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"
if ! should_use_structured; then
    [ "$TERM_PROGRAM" = "WarpTerminal" ] && exec "$SCRIPT_DIR/legacy/on-stop.sh"
    exit 0
fi
source "$SCRIPT_DIR/build-payload.sh"
INPUT=$(cat)
[ "$(printf '%s' "$INPUT" | jq -r '.stop_hook_active // false' 2>/dev/null)" = "true" ] && exit 0

RESPONSE=$(printf '%s' "$INPUT" | jq -r '.last_assistant_message | select(type == "string")' 2>/dev/null)
TRANSCRIPT_PATH=$(printf '%s' "$INPUT" | jq -r '.transcript_path // empty' 2>/dev/null)
EVENT="stop"
# 没有最终文本或仍有后台工作时只报告通知，不声称整项任务已经完成。
if [ -z "$RESPONSE" ] || [ "$(printf '%s' "$INPUT" | jq '(.background_tasks // []) | length' 2>/dev/null)" != "0" ]; then
    EVENT="notification"
fi
if [ ${#RESPONSE} -gt 200 ]; then RESPONSE="${RESPONSE:0:197}..."; fi
BODY=$(build_payload "$INPUT" "$EVENT" --arg response "$RESPONSE" --arg transcript_path "$TRANSCRIPT_PATH")
"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
