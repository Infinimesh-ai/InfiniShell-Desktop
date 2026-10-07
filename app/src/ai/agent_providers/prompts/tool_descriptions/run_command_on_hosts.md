Run the same shell command across SSH hosts in the current project (available only in project sessions).

Use when <project_context> lists multiple hosts and the task requires deployment, inspection, or configuration changes across them. For a single host, continue using run_shell_command.

Parameters:
- node_ids: Target node_ids from the host list in <project_context>, up to 20 per call.
- command: The complete shell command to run on each host. Hosts run sequentially.
- canary (default true): Run on the first host first. If it fails (a nonzero exit code, error, or timeout), stop before the remaining hosts and mark them canary_aborted. Set false explicitly only after confirming it is safe to run on every host.
- timeout_seconds (default 120, maximum 600): The timeout for each host. A timeout returns the current output snapshot.

Behavior:
- Hosts without an open session get a new SSH tab. Wait up to 60 seconds for the connection; failure is reported as session_not_ready.
- A host already running a long command is left alone and reported as busy.
- Avoid pagers (for example, append | cat to git log or man) and interactive programs, which would wait until the timeout.

JSON output:
{"status": "ok"|"error", "canary_aborted": bool, "results": [{"node_id", "host", "status": "ok"|"error"|"timeout"|"busy"|"session_not_ready"|"canary_aborted", "exit_code", "output", "duration_ms"}]}
Output longer than 10000 characters is truncated and followed by a truncation marker.
