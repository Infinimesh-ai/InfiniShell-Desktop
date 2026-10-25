# Local test storage and cleanup

[简体中文](local-test-storage.zh-CN.md)

This page defines shared safety gates. Machine-specific test roots, archive locations, and cleanup schedules belong in the untracked `.local/README.md` and its references. Read them before building or testing. If absent in a worktree, locate the primary checkout with `git worktree list --porcelain` and read its `.local/README.md`. Existing machine rules remain effective. Missing configuration requires establishing a safe directory first; do not guess paths or weaken gates.

## Before each run

- Use a `0700` test root and a fresh short `r-<random>` directory with mode `0700`. Set `TMPDIR` only for that run; do not change `HOME` or `CODEX_HOME`, reuse active directories, or move them.
- Verify resolved path, current-user ownership, ordinary directory type, device/inode, and the configured internal filesystem. Reject symlink ancestors, group/other-writable ancestors, and nested mounts. Do not change existing permissions, volume ownership, ACLs, or TCC to bypass a gate.
- Keep source, Cargo `target`, caches, credentials, and unique acceptance evidence outside the temporary root. This policy does not authorize moving caches.
- Record start/end times, command, source revision, directory/process identities, archive location, status, and `cleanup_ready` (initially `false`) in `records/<run-id>.json`. Do not record tokens or credential bodies.

## Real multiprocess tests

When required by the machine configuration, use a short `TMPDIR` and `--features warpui/test-util,rust-embed/debug-embed` for real launchd/multiprocess tests to avoid resource reads blocking on external source storage. Do not enable `release_bundle`, change product timeouts, or hardcode machine paths/features into the product or other platform runners for this workaround. Record features, filters, and source/artifact identity. Passing under this configuration does not validate other builds.

## Completion and follow-up cleanup

1. Verify by exact run identity that the command, descendants, supervisors, and dedicated launchd services have exited and open files are released. A missing PID, broad process-name query, or empty limited-permission query is insufficient. Do not terminate unknown processes.
2. Archive and verify small logs, source identity, success/failure results, and cleanup evidence. Retain the directory with `cleanup_ready=false` if the archive is unavailable, diagnosis is pending, or exit evidence is incomplete.
3. Set `cleanup_ready=true` only after all gates pass. Recheck owner, device/inode, and resolved path before deletion. Do not follow links, cross devices/mounts, or delete source, caches, credentials, external archives, or unique evidence. Retain records and mark successful cleanup `cleaned`.
4. Normally clean up immediately after completion. Use the existing machine schedule for later checks, considering only completed runs beyond the configured retention period with explicit `cleanup_ready=true`. Retain active, unknown, replaced, mounted, or symlink directories; name and age alone never authorize deletion.
