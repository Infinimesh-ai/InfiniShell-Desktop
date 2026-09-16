#!/bin/bash
# Determines whether the current Warp build supports structured CLI agent notifications.
#
# Usage:
#   source "$SCRIPT_DIR/should-use-structured.sh"
#   if should_use_structured; then
#       # ... send structured notification
#   else
#       # ... legacy fallback or exit
#   fi
#
# Returns 0 (true) when structured notifications are safe to use, 1 (false) otherwise.

should_use_structured() {
    # No protocol version advertised → Warp doesn't know about structured notifications.
    [ -z "${WARP_CLI_AGENT_PROTOCOL_VERSION:-}" ] && return 1
    # No client version advertised → can't verify the build can render them.
    [ -z "${WARP_CLIENT_VERSION:-}" ] && return 1
    return 0
}
