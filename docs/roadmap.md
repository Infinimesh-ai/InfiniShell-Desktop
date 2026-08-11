# Roadmap

InfiniShell's agent stack will be built as a standalone open-source service, independent of the Warp client. The terminal becomes one client among many — a TUI, an IDE plugin or a cloud worker can all drive the same engine.

## Phase 1 — Build the agent harness core

- Design and build a standalone open-source agent service from scratch — agent loop, tool runtime, conversation/session state, prompt templating, provider routing — not tied to Warp's existing client code. InfiniShell becomes its first consumer.
- Define a stable IPC / JSON-RPC protocol: prompts, streaming tokens, tool calls, file diffs, status, attachments.
- Ship the harness as a reusable open-source service — a headless daemon, a standalone TUI, IDE plugins and other terminals can all talk to it.
- Local-only by default; credentials, history, skills and MCP servers stay on disk.
- Versioned protocol + capability negotiation so clients and harness can upgrade independently.
- Pluggable tool registry: built-in shell / read / edit / search tools plus user-provided ones over a uniform RPC surface.

## Phase 2 — Hosted agent runtime

- Run the same harness on a server, accepting tasks from any client.
- Async task delegation: kick off long-running work, monitor progress, come back later.
- Isolated execution sandboxes per task (containers / VMs), with configurable preinstalled toolchains and setup scripts.
- Repo-aware execution: clone, branch, run tests, lint, type-check; surface verifiable terminal logs and test output.
- Git workflow integration: create branches, commits and pull requests with diff + log citations.
- Per-task secrets and network policy (default offline; explicit allowlist when egress is needed).
- Multi-task parallelism with quota, scheduling and cancellation.
- Repo / org / project memory files (`AGENTS.md` / equivalents) honored across runs.
- Fully self-hostable: single-node Docker, multi-node cluster, or bring-your-own Kubernetes. No mandatory SaaS dependency.

## Phase 3 — Multi-surface collaboration

- Single account / identity shared across InfiniShell terminal, headless TUI, IDE plugins and web UI.
- Session handoff: start on web, continue in the terminal; or hand a terminal session off to a desktop reviewer.
- Background agents and multi-agent teams: a lead agent decomposes work and dispatches subtasks to peer agents.
- Routines: run a task on a schedule, via API call, or in response to repository / CI / issue-tracker events.
- Inbound channels: push tasks from chat (Slack / Discord / Telegram / webhooks) into the harness.
- Outbound integrations: GitHub / GitLab / Gitea, issue trackers, CI systems, MCP servers, code review.
- Live observability of remote runs: streaming logs, intermediate diffs, mid-flight steering and cancellation.
- Shareable conversation / task links for team review, with permission scopes.
- End-to-end open source: harness, sandbox runtime, web UI and integrations — all self-hostable.

> Roadmap items are exploratory and may shift as the harness lands and real usage feedback arrives.

---

[简体中文](./roadmap.zh-CN.md) · [日本語](./roadmap.ja.md)
