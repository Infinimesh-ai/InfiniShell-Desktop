<div align="center">

<img src="assets/zap-logo.svg" alt="InfiniShell" width="128" />

# InfiniShell

**An open, local-first terminal with first-class AI and agent support.**

[简体中文](./README.zh-CN.md)

[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE-AGPL)
[![Rust](https://img.shields.io/badge/rust-1.92-orange.svg)](rust-toolchain.toml)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](#getting-started)

<sub><i>Built on <a href="https://github.com/warpdotdev/warp">Warp</a>, descended from
<a href="https://github.com/zerx-lab/zap">Zap</a>; evolving independently going forward.</i></sub>

</div>

---

Plug in any AI provider, bring in any CLI agent, manage SSH hosts inside the
terminal — and organize those hosts into projects your agent can understand
and operate on. Keys, history and agent state stay on your machine by
default. No account. No mandatory cloud.

## Highlights

- 🔌 **BYOP AI providers** — any OpenAI-compatible endpoint, plus native
  OpenAI / Anthropic / Gemini / DeepSeek / Ollama protocols. API keys never
  leave your machine.
- 🤖 **Third-party CLI agents** — DeepSeek-TUI, Codex CLI, Claude Code and
  Google Antigravity (`agy`) wired into Blocks and the notification center.
- 🖥️ **Built-in SSH host manager** — manage hosts, configs and sessions inside
  the terminal, with tmux integration and per-machine agent memory.
- 🗂️ **Project-scoped agent mode** — group SSH servers, a Git repo and ops
  rules into a project; agent conversations pick up the project context
  automatically, run commands on project hosts, and fan out across hosts
  with canary-first batch execution. See
  [below](#project-scoped-agent-ops).
- 📝 **Editable system prompts** — minijinja templates rendered on the client;
  see exactly what your agent is told, and change it.
- 🈶 **Localized UI** — English, Simplified Chinese and Japanese out of the
  box, community-extensible. CJK rendering fixes (soft-wrap caret, bold
  subpixel) included.
- 🔒 **Privacy by default** — no account, no login, no Drive sync, no cloud
  agent history. Cloud Agent / Computer Use / telemetry are off by default;
  most of the reporting code paths are removed outright.

## Project-scoped agent ops

Think Codex / Claude Code-style project scoping, but aimed at SSH operations:
organize a set of servers, a Git repository URL and your ops rules/habits into
a **project** (open the panel with `Ctrl+7`, or `Alt+7` on Linux/Windows), and
the agent gains a project-level view.

- **Automatic context injection** — connect to any host in the project (from
  the projects panel, the SSH manager, or a hand-typed `ssh`) and the
  conversation automatically carries that project's host inventory, repo URL
  and rules. The injected block is explicitly framed as "reference data, not
  instructions" to resist prompt injection.
- **Single-host execution** — once the SSH session is warpified, the agent's
  `run_shell_command` runs directly on the remote host with structured command
  blocks, reusing the existing approval flow and command allowlist.
- **Cross-host batch execution** — the `run_command_on_hosts` tool orchestrates
  one command across multiple project hosts: canary-first (a failing first
  host aborts the rest), per-host exit codes and output aggregation, and a
  per-command timeout cap. Before running, a dedicated approval card lists the
  command and target hosts — reject, run once, or always allow (which writes
  the command into the allowlist so future identical batches auto-execute).
- **Local everything** — project data lives in local SQLite; SSH credentials
  stay in the OS keychain (Keychain/DPAPI/keyring). No cloud dependency, same
  as everywhere else.

Typical workflow: create a project → link servers, set the repo URL and rules
→ start an agent conversation from the project (or just connect to any project
host) → have the agent inspect, reconfigure, or roll out across the fleet.

## Getting Started

### Build from source

Requires [Rust](https://rustup.rs/) (the toolchain version is pinned by
[`rust-toolchain.toml`](rust-toolchain.toml)).

```bash
git clone https://github.com/Infinimesh-ai/InfiniShell-Desktop.git
cd InfiniShell-Desktop
./script/bootstrap   # one-time platform-specific setup
cargo run            # build and run InfiniShell
```

Before sending a change:

```bash
./script/presubmit   # fmt, clippy, and tests
```

### Coming from Warp or OpenWarp?

See [docs/migrate-from-warp.md](docs/migrate-from-warp.md) to bring your
settings across. Note that InfiniShell does **not** migrate configuration
automatically — the guide walks you through the manual steps.

## How it relates to Warp

InfiniShell tracks [upstream Warp](https://github.com/warpdotdev/warp) for the
terminal core — rendering, blocks, the agent runtime — and diverges
deliberately where local-first principles demand it: the account system, cloud
sync, and telemetry pipelines are removed rather than merely disabled, and the
AI layer speaks to *your* providers instead of a hosted gateway.

Internal crate names keep their upstream `warp_*` prefixes on purpose: they
mark which code is upstream heritage, which keeps future syncs tractable.

## Roadmap

See [docs/roadmap.md](docs/roadmap.md).

## Contributing

Issues and PRs are welcome — start with
[CONTRIBUTING.md](CONTRIBUTING.md) for the workflow, spec process and testing
expectations. Security reports go through [SECURITY.md](SECURITY.md).

## License

Licensed under [AGPL-3.0-only](LICENSE-AGPL). Portions inherited from upstream
Warp are additionally available under [MIT](LICENSE-MIT); see the individual
crate manifests for specifics.

## Acknowledgements

- [Warp](https://github.com/warpdotdev/warp) — the upstream terminal
  InfiniShell is built on.
- [Zap](https://github.com/zerx-lab/zap) — the upstream fork InfiniShell
  directly descends from; the BYOP, SSH-manager and i18n groundwork began
  there.
