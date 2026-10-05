#!/bin/bash
# Hook script for Codex UserPromptSubmit event.
# Sends a structured Warp notification when the user submits a prompt.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

if ! should_use_structured; then
    exit 0
fi

source "$SCRIPT_DIR/build-payload.sh"

INPUT=$(cat)

# 只有原生 Windows jq 需要二进制输出，保留用户 LF/CRLF；Unix 不要求 --binary。
JQ_RAW_ARGS=(-r)
case "$(uname -s)" in
    MINGW*|MSYS*) JQ_RAW_ARGS=(--binary -r) ;;
esac
QUERY=$(echo "$INPUT" | jq "${JQ_RAW_ARGS[@]}" '.prompt // empty' 2>/dev/null)
if [ -n "$QUERY" ] && [ ${#QUERY} -gt 200 ]; then
    QUERY="${QUERY:0:197}..."
fi

BODY=$(build_payload "$INPUT" "prompt_submit" \
    --arg query "$QUERY")

"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
