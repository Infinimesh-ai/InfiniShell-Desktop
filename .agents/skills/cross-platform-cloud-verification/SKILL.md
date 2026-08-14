---
name: cross-platform-cloud-verification
description: Orchestrates cost-conscious cross-platform verification on the repository's GitHub Actions self-hosted runners after cheaper local checks pass. Use whenever a code change, build, native dependency, UI behavior, filesystem behavior, or test has material cross-platform implications.
compatibility: Requires an authenticated GitHub CLI (`gh`) with Actions read/workflow permissions, plus the repository's Linux and Windows runners described in `docs/self-hosted-runners.zh-CN.md`.
---

# Cross-platform cloud verification

Use `.github/workflows/cross-platform-preflight.yml` as the only remote execution control plane.
This repository does not depend on Oz or `oz-dev`.

## 1. Finish the local gate first

1. Inspect the diff and identify the affected operating systems.
2. Run the relevant local build and focused tests.
3. Fix deterministic failures before consuming remote compute.
4. Record `git rev-parse HEAD` and confirm the exact commit exists on a remote branch. Before the
   workflow reaches the default branch, use a `ci/**` branch; afterwards prefer `workflow_dispatch`.

Do not commit or push only to make the workflow reachable unless the user has authorized it. If the
commit is local-only, ask for the minimum required push. Never test the default branch when it does
not contain the intended change.

## 2. Select the smallest useful platform set

Use Linux x64 for platform-neutral Rust/network/protocol changes. Add Windows x64 when the change
touches desktop code, native dependencies, process creation, paths, filesystem behavior, packaging,
or cross-platform conditional compilation. Select another architecture only with evidence of ABI,
FFI, unsafe, SIMD, serialization, or architecture-specific behavior.

The manual workflow accepts:

- `run_linux`
- `run_windows`
- `full_workspace_tests`

Keep `full_workspace_tests=false` for ordinary iteration. Enable it only after focused jobs pass or
when release-level coverage is required.

## 3. Check runner availability

Resolve the repository from the current checkout, then list repository runners:

```sh
gh api "repos/{owner}/{repo}/actions/runners" \
  --jq '.runners[] | [.id, .name, .os, .status, .busy, (.labels | map(.name) | join(","))] | @tsv'
```

Required labels are documented in `docs/self-hosted-runners.zh-CN.md`. A runner selected for the
workflow must be online and carry all labels from its `runs-on` expression. If runner-list access is
not permitted, dispatch the workflow and treat a job that remains queued as an availability block;
do not invent runner metadata.

## 4. Dispatch the exact remote ref

Before this workflow exists on the default branch, bootstrap verification by pushing the exact
commit to a `ci/**` branch:

```sh
git push origin HEAD:refs/heads/ci/<topic>
```

Once the workflow exists on the default branch, dispatch it with explicit platform inputs:

```sh
gh workflow run cross-platform-preflight.yml \
  --ref '<remote-branch>' \
  -f run_linux=true \
  -f run_windows=true \
  -f full_workspace_tests=false
```

Set platform inputs from the matrix selected in step 2. Then locate and watch the new run:

```sh
gh run list \
  --workflow cross-platform-preflight.yml \
  --branch '<remote-branch>' \
  --event workflow_dispatch \
  --limit 5

gh run watch '<run-id>' --exit-status
```

Do not dispatch a second run merely because the first is slow. Retry at most once for a clear runner,
network, or GitHub service failure. A compilation or test failure is product evidence and must return
to local diagnosis.

## 5. Collect evidence

```sh
gh run view '<run-id>' --json url,headSha,status,conclusion,jobs
gh run view '<run-id>' --log-failed
```

Verify that `headSha` is the intended commit. Report each selected platform independently:

```text
Cross-platform verification: <passed | failed | incomplete>
Local gate: <commands completed before dispatch>
Commit: <tested SHA>
Run: <GitHub Actions URL>
Results:
- Linux x64: <passed | failed | blocked> — <checks/evidence>
- Windows x64: <passed | failed | blocked> — <checks/evidence>
Unverified:
- <relevant omitted platform and one concise reason>
Conclusion:
- <what the results establish and what remains>
```

Use `passed` only when every selected relevant platform passed. Use `failed` for product failures and
`incomplete` when a required runner was unavailable or infrastructure blocked the run.

## Security and cost guardrails

- The self-hosted workflow must not listen to `pull_request`; fork approval is not a security
  boundary for persistent runners.
- Do not add secrets or write permissions to the preflight workflow.
- Do not modify source, commit, or push from a verification job.
- Prefer focused checks; run the full workspace suite only when its additional coverage is warranted.
- Preserve negative results and platform gaps instead of broadening or repeating runs to obtain green.
