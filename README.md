<div align="center">

<img src="assets/zap-logo.svg" alt="InfiniShell" width="128" />

# InfiniShell

[简体中文](./README.zh-CN.md) · [日本語](./README.ja.md)

<sub><i>Currently based on <a href="https://github.com/warpdotdev/warp">Warp</a>; evolving independently going forward.</i></sub>

</div>

InfiniShell is an open, local-first terminal with first-class AI and agent support. Plug in any AI provider, bring in any CLI agent, manage SSH hosts inside the terminal — with keys, history and agent state staying on your machine by default.

## What InfiniShell adds over upstream Warp

- **No mandatory cloud** — no account, login, Drive sync or cloud agent history required.
- **BYOP AI providers** — any OpenAI-compatible endpoint, plus native OpenAI / Anthropic / Gemini / DeepSeek / Ollama protocols. Keys stay local.
- **Third-party CLI agents** — DeepSeek-TUI / Codex CLI / Claude Code / Google Antigravity (`agy`) wired into Blocks and the notification center.
- **Built-in SSH host manager** — manage hosts, configs and sessions inside the terminal, with tmux integration.
- **Editable system prompts** — minijinja templates rendered on the client.
- **Rendering fixes** — tuned Markdown pipeline; CJK soft-wrap caret and bold subpixel fixes.
- **Localized UI** — English / Simplified Chinese / Japanese out of the box, community-extensible.
- **Privacy defaults** — Cloud Agent / Computer Use / Referral / telemetry off by default.

## Migrating from Zap, OpenWarp or Warp

Upgrading from **Zap** (the immediately preceding name)? Nothing to do —
InfiniShell moves your old `.zap` directories over automatically on first
launch.

If you used the project back when it was **OpenWarp**, or are coming from
upstream **Warp**, see
[docs/migrate-from-warp.md](docs/migrate-from-warp.md) to bring your settings
across.

## Roadmap

See [docs/roadmap.md](docs/roadmap.md).

## Acknowledgements

- [Warp](https://github.com/warpdotdev/warp) — the upstream terminal InfiniShell is built on.
- [DeepSeek-TUI](https://github.com/Hmbown/DeepSeek-TUI) — first-class CLI agent partner.
