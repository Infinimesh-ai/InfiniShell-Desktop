#!/bin/bash

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_DIR="$REPO_ROOT/plugins/orchestration/scripts"

PASSED=0
FAILED=0

assert_eq() {
    local test_name="$1"
    local expected="$2"
    local actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo "  ✓ $test_name"
        PASSED=$((PASSED + 1))
    else
        echo "  ✗ $test_name"
        echo "    expected: $expected"
        echo "    actual:   $actual"
        FAILED=$((FAILED + 1))
    fi
}

assert_contains() {
    local test_name="$1"
    local haystack="$2"
    local needle="$3"
    if printf '%s' "$haystack" | grep -Fq "$needle"; then
        echo "  ✓ $test_name"
        PASSED=$((PASSED + 1))
    else
        echo "  ✗ $test_name"
        echo "    expected to find: $needle"
        echo "    actual:           $haystack"
        FAILED=$((FAILED + 1))
    fi
}

assert_json_field() {
    local test_name="$1"
    local json="$2"
    local field="$3"
    local expected="$4"
    local actual
    actual=$(echo "$json" | jq -r "$field" 2>/dev/null)
    assert_eq "$test_name" "$expected" "$actual"
}

assert_file_exists() {
    local test_name="$1"
    local path="$2"
    assert_eq "$test_name" "true" "$([ -f "$path" ] && echo true || echo false)"
}

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT

FAKE_OZ="$TEST_TMP/fake-oz.sh"
FAKE_OZ_LOG="$TEST_TMP/fake-oz.log"
FAKE_OZ_WATCH="$TEST_TMP/watch.ndjson"
STATE_ROOT="$TEST_TMP/state"
HOOK_INPUT='{"session_id":"sess-123","cwd":"/tmp/project"}'
STATE_DIR="$STATE_ROOT/sess-123"

cat >"$FAKE_OZ" <<'EOF'
#!/bin/bash
set -euo pipefail

printf '%s\n' "$*" >> "${FAKE_OZ_LOG:?}"

if [ "$#" -ge 3 ] && [ "$1" = "run" ] && [ "$2" = "message" ] && [ "$3" = "watch" ]; then
    if [ -n "${FAKE_OZ_WATCH_FILE:-}" ] && [ -f "${FAKE_OZ_WATCH_FILE:-}" ]; then
        since_sequence=0
        for ((i = 1; i <= $#; i++)); do
            if [ "${!i}" = "--since-sequence" ]; then
                next_index=$((i + 1))
                since_sequence="${!next_index}"
                break
            fi
        done

        while IFS= read -r line; do
            [ -n "$line" ] || continue
            sequence=$(printf '%s' "$line" | jq -r '.sequence // 0' 2>/dev/null || printf '0')
            if [ "$sequence" -gt "$since_sequence" ]; then
                printf '%s\n' "$line"
            fi
        done < "$FAKE_OZ_WATCH_FILE"
    fi
    exit 0
fi

if [ "$#" -ge 4 ] && [ "$1" = "run" ] && [ "$2" = "message" ] && [ "$3" = "mark-delivered" ]; then
    exit 0
fi

if [ "$#" -ge 4 ] && [ "$1" = "run" ] && [ "$2" = "message" ] && [ "$3" = "delivered" ]; then
    exit 0
fi

exit 0
EOF
chmod +x "$FAKE_OZ"

export OZ_CLI="$FAKE_OZ"
export OZ_RUN_ID="child-run-123"
export OZ_PARENT_RUN_ID="parent-run-456"
export OZ_PARENT_STATE_ROOT="$STATE_ROOT"
export FAKE_OZ_LOG
export FAKE_OZ_WATCH_FILE="$FAKE_OZ_WATCH"

mkdir -p "$STATE_ROOT"
: >"$FAKE_OZ_LOG"

echo "=== on-session-start.sh ==="
rm -rf "$STATE_DIR"
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/on-session-start.sh")
assert_eq "session start emits no output" "" "$OUTPUT"
assert_eq "session start writes listener pid" "true" "$([ -f "$STATE_DIR/listener.pid" ] && echo true || echo false)"
assert_eq "session start creates state directory" "true" "$([ -d "$STATE_DIR/staged" ] && echo true || echo false)"
kill_listener_pid="$(cat "$STATE_DIR/listener.pid" 2>/dev/null || true)"
if [ -n "$kill_listener_pid" ]; then
    kill "$kill_listener_pid" 2>/dev/null || true
fi
rm -rf "$STATE_DIR"

echo ""
echo "=== oz-parent-listener.sh ==="
cat >"$FAKE_OZ_WATCH" <<'EOF'
{"sequence":42,"message_id":"msg-123","sender_run_id":"parent-run-456","subject":"Please pivot","body":"Inspect the failing tests before editing code.","occurred_at":"2026-04-17T15:46:00Z"}
EOF

bash "$SCRIPT_DIR/oz-parent-listener.sh" "$STATE_DIR"

LISTENER_FILE="$STATE_DIR/staged/00000000000000000042-msg-123.json"
assert_eq "listener stages message" "true" "$([ -f "$LISTENER_FILE" ] && echo true || echo false)"
assert_json_field "listener writes stored subject" "$(cat "$LISTENER_FILE")" ".subject" "Please pivot"
assert_eq "listener updates last sequence" "42" "$(cat "$STATE_DIR/last-sequence")"
assert_contains "listener invokes message watch" "$(cat "$FAKE_OZ_LOG")" "run message watch child-run-123 --since-sequence 0 --output-format ndjson"

echo ""
echo "=== on-prompt-submit.sh ==="
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/drain-mailbox.sh" UserPromptSubmit)
assert_json_field "prompt submit outputs hook event name" "$OUTPUT" ".hookSpecificOutput.hookEventName" "UserPromptSubmit"
assert_contains "prompt submit includes subject" "$OUTPUT" "Please pivot"
assert_contains "prompt submit includes body" "$OUTPUT" "Inspect the failing tests before editing code."
assert_eq "prompt submit removes surfaced message" "false" "$([ -f "$LISTENER_FILE" ] && echo true || echo false)"
assert_contains "prompt submit marks message delivered" "$(cat "$FAKE_OZ_LOG")" "run message mark-delivered msg-123"

echo ""
echo "=== on-post-tool-use.sh ==="
mkdir -p "$STATE_DIR/staged"
cat >"$STATE_DIR/staged/00000000000000000043-msg-456.json" <<'EOF'
{"sequence":43,"message_id":"msg-456","sender_run_id":"parent-run-456","subject":"Another update","body":"There is still more work to do.","occurred_at":"2026-04-17T15:47:00Z"}
EOF
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/drain-mailbox.sh" PostToolUse)
assert_json_field "post tool use outputs hook event name" "$OUTPUT" ".hookSpecificOutput.hookEventName" "PostToolUse"
assert_contains "post tool use includes staged body" "$OUTPUT" "There is still more work to do."
assert_eq "post tool use removes surfaced message" "false" "$([ -f "$STATE_DIR/staged/00000000000000000043-msg-456.json" ] && echo true || echo false)"

echo ""
echo "=== on-stop.sh ==="
mkdir -p "$STATE_DIR/staged"
cat >"$STATE_DIR/staged/00000000000000000044-msg-789.json" <<'EOF'
{"sequence":44,"message_id":"msg-789","sender_run_id":"parent-run-456","subject":"Queued update","body":"There is still another staged instruction.","occurred_at":"2026-04-17T15:48:00Z"}
EOF

OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/on-stop.sh")
assert_json_field "stop blocks when staged messages remain" "$OUTPUT" ".decision" "block"
assert_contains "stop reason references pending parent messages" "$OUTPUT" "pending parent message"

rm -f "$STATE_DIR/staged/"*.json
export OZ_PARENT_STOP_LINGER_ATTEMPTS=0
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/on-stop.sh")
unset OZ_PARENT_STOP_LINGER_ATTEMPTS
assert_eq "stop exits silently when no staged messages remain" "" "$OUTPUT"
assert_eq "stop leaves state directory for session end" "true" "$([ -d "$STATE_DIR" ] && echo true || echo false)"
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/on-session-end.sh")
assert_eq "session end emits no output" "" "$OUTPUT"
assert_eq "session end cleans up state directory" "false" "$([ -d "$STATE_DIR" ] && echo true || echo false)"

echo ""
echo "=== stop linger ==="
mkdir -p "$STATE_DIR/staged"
export OZ_PARENT_STOP_LINGER_ATTEMPTS=10
export OZ_PARENT_STOP_LINGER_POLL_SECONDS=0.05
(
    sleep 0.1
    cat >"$STATE_DIR/staged/00000000000000000045-msg-790.json" <<'EOF'
{"sequence":45,"message_id":"msg-790","sender_run_id":"parent-run-456","subject":"Late update","body":"This parent message arrived during the linger window.","occurred_at":"2026-04-17T15:49:00Z"}
EOF
) &
linger_writer_pid=$!
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/on-stop.sh")
wait "$linger_writer_pid"
assert_json_field "stop lingers for late-arriving staged messages" "$OUTPUT" ".decision" "block"
assert_contains "stop linger reason references pending parent messages" "$OUTPUT" "pending parent message"
unset OZ_PARENT_STOP_LINGER_ATTEMPTS
unset OZ_PARENT_STOP_LINGER_POLL_SECONDS
rm -rf "$STATE_DIR"

echo ""
echo "=== non-child sessions are ignored ==="
unset OZ_PARENT_RUN_ID
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/on-session-start.sh")
assert_eq "non-child session start emits no output" "" "$OUTPUT"
assert_eq "non-child session start does not create state directory" "false" "$([ -d "$STATE_DIR" ] && echo true || echo false)"
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/drain-mailbox.sh" UserPromptSubmit)
assert_eq "non-child prompt submit emits no output" "" "$OUTPUT"
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/drain-mailbox.sh" PostToolUse)
assert_eq "non-child post tool use emits no output" "" "$OUTPUT"
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/on-stop.sh")
assert_eq "non-child stop emits no output" "" "$OUTPUT"
OUTPUT=$(printf '%s' "$HOOK_INPUT" | bash "$SCRIPT_DIR/on-session-end.sh")
assert_eq "non-child session end emits no output" "" "$OUTPUT"
export OZ_PARENT_RUN_ID="parent-run-456"

echo ""
echo "=== plugin manifests ==="
assert_file_exists "marketplace manifest exists" "$REPO_ROOT/.agents/plugins/marketplace.json"
assert_file_exists "warp plugin manifest exists" "$REPO_ROOT/plugins/warp/.codex-plugin/plugin.json"
assert_file_exists "oz plugin manifest exists" "$REPO_ROOT/plugins/orchestration/.codex-plugin/plugin.json"
assert_file_exists "warp hook config exists" "$REPO_ROOT/plugins/warp/hooks/hooks.json"
assert_file_exists "oz hook config exists" "$REPO_ROOT/plugins/orchestration/hooks/hooks.json"
assert_file_exists "github workflow exists" "$REPO_ROOT/.github/workflows/test.yml"
assert_json_field "marketplace name" "$(cat "$REPO_ROOT/.agents/plugins/marketplace.json")" ".name" "codex-warp"
assert_json_field "warp plugin name" "$(cat "$REPO_ROOT/plugins/warp/.codex-plugin/plugin.json")" ".name" "warp"
assert_json_field "oz plugin name" "$(cat "$REPO_ROOT/plugins/orchestration/.codex-plugin/plugin.json")" ".name" "orchestration"
assert_contains "warp hooks use PLUGIN_ROOT" "$(cat "$REPO_ROOT/plugins/warp/hooks/hooks.json")" '${PLUGIN_ROOT}/scripts/on-session-start.sh'
assert_contains "warp hooks include prompt submit" "$(cat "$REPO_ROOT/plugins/warp/hooks/hooks.json")" '${PLUGIN_ROOT}/scripts/on-prompt-submit.sh'
assert_contains "warp hooks include post tool use" "$(cat "$REPO_ROOT/plugins/warp/hooks/hooks.json")" '${PLUGIN_ROOT}/scripts/on-post-tool-use.sh'
assert_contains "oz hooks use PLUGIN_ROOT" "$(cat "$REPO_ROOT/plugins/orchestration/hooks/hooks.json")" '${PLUGIN_ROOT}/scripts/drain-mailbox.sh UserPromptSubmit'
assert_contains "oz hooks include session end" "$(cat "$REPO_ROOT/plugins/orchestration/hooks/hooks.json")" '${PLUGIN_ROOT}/scripts/on-session-end.sh'


echo ""
echo "=== Results: $PASSED passed, $FAILED failed ==="

if [ "$FAILED" -gt 0 ]; then
    exit 1
fi
