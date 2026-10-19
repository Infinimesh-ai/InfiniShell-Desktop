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
# 缺字段兼容旧 CLI；已提供的后台任务和定时唤醒字段必须都是空数组。
PENDING_WORK_EMPTY=$(printf '%s' "$INPUT" | jq -r '
    . as $input
    | all(["background_tasks", "session_crons"][];
        . as $field
        | if ($input | has($field)) then
            $input[$field] | if type == "array" then length == 0 else false end
          else true end)
' 2>/dev/null)
# 没有最终文本、仍有后台工作或载荷畸形时只报告通知，不推断任务成功。
if [ -z "$RESPONSE" ] || [ "$PENDING_WORK_EMPTY" != "true" ]; then
    EVENT="notification"
fi
if [ ${#RESPONSE} -gt 200 ]; then RESPONSE="${RESPONSE:0:197}..."; fi
BODY=$(build_payload "$INPUT" "$EVENT" --arg response "$RESPONSE" --arg transcript_path "$TRANSCRIPT_PATH")
"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
