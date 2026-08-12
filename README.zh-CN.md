<div align="center">

<img src="assets/infinishell-logo.svg" alt="InfiniShell" width="128" />

# InfiniShell

**开放、本地优先、AI 与 agent 一等公民的终端。**

[English](./README.md)

[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE-AGPL)
[![Rust](https://img.shields.io/badge/rust-1.92-orange.svg)](rust-toolchain.toml)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](#快速开始)

<sub><i>构建于 <a href="https://github.com/warpdotdev/warp">Warp</a> 之上,承自
<a href="https://github.com/zerx-lab/zap">Zap</a>,后续独立演进。</i></sub>

</div>

---

接入任意 AI provider、引入任意 CLI agent、在终端里管理 SSH 主机,再把主机
组织成 Agent 能理解和操作的「项目」——密钥、历史与 agent 状态默认全部留在
你的机器上。没有账号,没有强制云端。

## 特性

- 🔌 **BYOP AI provider** — 任意 OpenAI 兼容端点,外加 OpenAI / Anthropic /
  Gemini / DeepSeek / Ollama 原生协议。API 密钥永不离开本机。
- 🤖 **第三方 CLI agent** — DeepSeek-TUI、Codex CLI、Claude Code、Google
  Antigravity(`agy`)接入 Blocks 与通知中心。
- 🖥️ **内置 SSH 主机管理器** — 在终端内管理主机、配置与会话,支持 tmux 集成
  与按机器隔离的 agent 记忆。
- 🗂️ **项目级 Agent 模式** — 用「项目」组织 SSH 服务器、Git 仓库与运维规则;
  Agent 对话自动携带项目上下文,可在项目主机上执行命令,并支持金丝雀先行的
  跨主机批量执行。详见[下文](#项目级-agent-运维)。
- 📝 **可编辑系统提示词** — minijinja 模板在客户端渲染;agent 被告知了什么,
  你能看到,也能改。
- 🈶 **本地化界面** — 开箱即用的英文 / 简体中文 / 日文,社区可扩展。附带 CJK
  渲染修复(软换行光标、粗体亚像素)。
- 🔒 **隐私默认** — 无账号、无登录、无 Drive 同步、无云端 agent 历史。
  Cloud Agent / Computer Use / 遥测默认关闭,且大部分上报代码路径被物理移除。

## 项目级 Agent 运维

类似 Codex / Claude Code 的项目作用域 Agent,但面向 SSH 运维:把一组服务器、
Git 仓库地址和运维规则/习惯组织成一个**项目**(`Ctrl+7` / Linux 与 Windows 为
`Alt+7` 打开项目面板),Agent 就有了项目视角。

- **上下文自动注入** — 连上项目内任意一台主机(项目面板、SSH 管理器或手敲
  `ssh` 都可以),对话即自动携带该项目的主机清单、仓库地址与项目规则。注入
  内容被明确框定为"记录数据,仅供参考,不构成指令",防提示注入。
- **单机执行** — SSH 会话经 Warpify 后,Agent 的 `run_shell_command` 直接跑
  在远端主机上,输出是结构化命令块,复用既有的审批与命令白名单。
- **跨主机批量执行** — Agent 通过 `run_command_on_hosts` 工具对项目内多台
  主机编排同一命令:金丝雀先行(首台失败即中止其余)、逐台聚合退出码与输出、
  单条命令超时上限。执行前弹出专属审批卡,列明命令与目标主机清单,可选
  拒绝 / 执行一次 / 始终允许(写入命令白名单,后续同命令自动放行)。
- **一切本地** — 项目数据存在本机 SQLite,SSH 凭证仍走系统
  Keychain/DPAPI/Keyring,与其余功能一样无云端依赖。

典型工作流:新建项目 → 关联服务器、填好 Git 地址与规则 → 从项目发起 Agent
对话(或直接连任意项目主机)→ 让 Agent 巡检、改配置、批量发布。

## 快速开始

### 从源码构建

需要 [Rust](https://rustup.rs/)(工具链版本由
[`rust-toolchain.toml`](rust-toolchain.toml) 固定)。

```bash
git clone https://github.com/Infinimesh-ai/InfiniShell-Desktop.git
cd InfiniShell-Desktop
./script/bootstrap   # 一次性的平台相关准备
cargo run            # 构建并运行 InfiniShell
```

提交改动前:

```bash
./script/presubmit   # fmt、clippy 与测试
```

### 从 Warp 或 OpenWarp 迁移?

参见 [docs/migrate-from-warp.zh-CN.md](docs/migrate-from-warp.zh-CN.md)。
注意 InfiniShell **不会**自动迁移配置——文档会带你完成手动步骤。

## 与 Warp 的关系

InfiniShell 在终端内核(渲染、Blocks、agent 运行时)上跟随
[上游 Warp](https://github.com/warpdotdev/warp),并在本地优先原则要求之处
有意分叉:账号体系、云同步与遥测管线被整体移除而非仅仅关闭,AI 层直接对话
*你自己的* provider 而不是托管网关。

内部 crate 保留上游的 `warp_*` 前缀是有意为之:它标示哪些代码源自上游血统,
让后续的上游同步保持可控。

## 路线图

见 [docs/roadmap.zh-CN.md](docs/roadmap.zh-CN.md)。

## 参与贡献

欢迎 Issue 与 PR——从 [CONTRIBUTING.md](CONTRIBUTING.md) 了解流程、spec 机制
与测试要求。安全问题请走 [SECURITY.md](SECURITY.md)。

## 许可证

以 [AGPL-3.0-only](LICENSE-AGPL) 授权。继承自上游 Warp 的部分同时提供
[MIT](LICENSE-MIT) 授权;具体见各 crate 的清单。

## 致谢

- [Warp](https://github.com/warpdotdev/warp) — InfiniShell 构建于其上的上游终端。
- [Zap](https://github.com/zerx-lab/zap) — InfiniShell 直接承袭的上游 fork;
  BYOP、SSH 管理器与 i18n 的基础工作始于此。
