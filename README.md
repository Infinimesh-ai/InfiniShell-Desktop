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
terminal — with keys, history and agent state staying on your machine by
default. No account. No mandatory cloud.

## Highlights

- 🔌 **BYOP AI providers** — any OpenAI-compatible endpoint, plus native
  OpenAI / Anthropic / Gemini / DeepSeek / Ollama protocols. API keys never
  leave your machine.
- 🤖 **Third-party CLI agents** — DeepSeek-TUI, Codex CLI, Claude Code and
  Google Antigravity (`agy`) wired into Blocks and the notification center.
- 🖥️ **Built-in SSH host manager** — manage hosts, configs and sessions inside
  the terminal, with tmux integration and per-machine agent memory.
- 📝 **Editable system prompts** — minijinja templates rendered on the client;
  see exactly what your agent is told, and change it.
- 🈶 **Localized UI** — English, Simplified Chinese and Japanese out of the
  box, community-extensible. CJK rendering fixes (soft-wrap caret, bold
  subpixel) included.
- 🔒 **Privacy by default** — no account, no login, no Drive sync, no cloud
  agent history. Cloud Agent / Computer Use / telemetry are off by default;
  most of the reporting code paths are removed outright.

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
