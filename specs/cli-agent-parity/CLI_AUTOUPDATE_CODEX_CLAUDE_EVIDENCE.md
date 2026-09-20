# Codex / Claude 最新版隔离输入与自动升级证据

核对时间：2026-09-19T10:04:26.763483+00:00。这份记录为自动升级开发提供固定、可复查的官方输入；不把帮助命令成功计为完整运行协议验收。

## 官方版本与通道

| CLI | 官方最新正式版 | 一致性来源 | 本次准备 |
| --- | --- | --- | --- |
| Codex | `0.155.1` | [GitHub 正式发行](https://github.com/openai/codex/releases/tag/rust-v0.155.1)、[npm latest](https://registry.npmjs.org/@openai%2fcodex/latest)；非预发布，发布时间 `2026-09-18T20:03:04Z` | macOS arm64 完整 `codex-package` |
| Claude Code | `2.1.278` | [native latest](https://downloads.claude.ai/claude-code-releases/latest)、[npm latest](https://registry.npmjs.org/@anthropic-ai%2fclaude-code/latest)、[固定清单](https://downloads.claude.ai/claude-code-releases/2.1.278/manifest.json) | macOS arm64 官方原生单文件 |

Claude `stable` 实际为 `2.1.267`，不能代表用户要求的最新正式版。固定清单的构建时间为 `2026-09-19T01:22:08Z`，commit 为 `809c980662e3525645594dc8b74f78c38a348db1`。未来执行应重新解析 latest，再绑定选定版本和摘要；这份观察不保证远端标签以后不变。

## 输入、摘要和签名

两款可执行文件位于新的私有目录 `/private/var/folders/qp/t9_qxjfn4xl22sbd83t4qh7m0000gp/T/infinishell-cli-latest-codex-claude-ed7d8264`，未覆盖用户安装或旧验收输入。

| 文件 | 字节数 | SHA256 |
| --- | ---: | --- |
| Codex 完整归档 | 122962085 | `e6e08717da9e35b72332eff753527fe79a9ae876081033c5c6820a8e5f58b943` |
| Codex `bin/codex` | 228803200 | `8eaf1ad12fe6bf89b1710330f58900014322c7c5af677e43be116d8ac5fc0a9e` |
| Codex `bin/codex-code-mode-host` | 62802592 | `59a702a68f1ef79fceaca644db46b8385ceefbb66035e78b8ade7cdcc21fda55` |
| Claude 原生文件 | 217695408 | `bd245662fb8a0e321b3bf133e930371d6563c387527885f30b2613aef3ba14d6` |
| Claude 清单 | 2161 | `d1bf63d94621d6aa6fb84297b235ddb8f5aaadc9252cef8020d170ef661b2f28` |

Codex 归档同时匹配 GitHub asset digest 和官方 `codex-package_SHA256SUMS`。完整解包保留 52 个成员、42 个文件、317010263 字节，包括主程序、code-mode host、rg、zsh 和新增 voice 运行文件；拒绝链接、特殊文件、重复或越界路径。30 个 Mach-O 文件均通过 `codesign --verify --strict`。主程序签名团队为 `OpenAI OpCo, LLC (2DC432GLL2)`。macOS 该归档未附独立 `.sigstore` 文件，不能借 Linux 签名文件声称其已验证。

Claude 文件与官方清单的大小和摘要一致，`codesign --verify --strict` 成功，签名团队为 `Anthropic PBC (Q6L2SF6YDW)`。发布清单 detached signature 使用私有环境中的 PGPy 0.6.0 验证，公钥指纹精确等于官方文档的 `31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE`；原清单签名通过、追加换行后的清单被拒。PGPy 的撤销、key-flags、自签名检查未实现，本次未把这些检查或 Gatekeeper 公证在线查询计为成功。Python 3.14 首次因缺少 `imghdr` 无法导入，补装私有兼容包后实际重验成功；没有写用户 keyring。[Claude 官方完整性说明](https://code.claude.com/docs/en/setup#binary-integrity-and-code-signing)

## 已验证的接口边界

11 个隔离版本/帮助进程均 exit 0，未发送模型输入、未创建协议会话。环境只保留基础系统路径并另建 HOME、CODEX_HOME、CLAUDE_CONFIG_DIR；关闭 Claude 更新与非必要流量，不加载用户认证。

- Codex 实际返回 `codex-cli 0.155.1`；`app-server --stdio --help` 接受现有启动参数。固定版本 config schema 仍包含 `notify` 和 `hooks`。这些静态事实不证明通知实际触发或现有 app-server 事件处理已兼容。
- Claude 实际返回 `2.1.278 (Claude Code)`；`--print --input-format stream-json --output-format stream-json --verbose --replay-user-messages --permission-prompt-tool stdio --permission-prompts host --plugin-dir <隔离目录> --help` 成功。`plugin --help` 列出安装、更新、启用、禁用等入口，未执行插件变更或通知。
- 仍需在新版本上独立验证初始化、权限策略、真实审批、两轮交互、追加、取消、继续、重启恢复、父子回收、插件生命周期与普通 PTY 通知。

## 更新来源、命令与回滚边界

Codex 当前官方版本存在 `codex update`；固定 `0.147.0` 官方源码也有该子命令，但本次只执行了新版本的帮助。`0.155.1` 的 update 命令从 InstallContext 选择 npm、bun、pnpm、VitePlus、Homebrew 或 standalone；未知来源直接报错，帮助中没有 `--yes` 或目标版本参数。standalone 更新重新运行官方安装器并设置 `CODEX_NON_INTERACTIVE=1`。安装器支持 `--release VERSION` / `CODEX_RELEASE`，完整版本目录位于 `CODEX_HOME/packages/standalone/releases`，再切换 current 和可见入口。[官方更新说明](https://learn.chatgpt.com/docs/codex/cli)、[固定更新实现](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/tui/src/update_action.rs)

npm 的正式更新配方是 `npm install -g @openai/codex`，默认解析 latest；显式 `@latest` 可表达相同意图。Homebrew 为 `brew upgrade --cask codex`。包布局不等于安装来源；本次私有解包应当仍视为未注册来源，不能因为文件名叫 codex 就修改系统安装。官方自身对 macOS Brew 的路径启发式较宽，InfiniShell 不应把 `/usr/local` 前缀单独当成来源证明。[来源实现](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/install-context/src/lib.rs)

Claude `update` / `upgrade` 没有 channel 或 version 参数，受既有 `autoUpdatesChannel` 约束。`install [target]` 支持 latest、stable 或具体版本，但官方说明安装通道可成为后续默认；后续隔离实测已证明本次版本的 `--settings` 进程覆盖可用于 update 且不写持久通道，见下节。遇到通道差异不能直接改写用户全局配置。Homebrew `claude-code` 跟随 stable，`claude-code@latest` 才跟随 latest；WinGet 与 Linux 包管理器应保留各自来源。`DISABLE_AUTOUPDATER` 只关闭后台，`DISABLE_UPDATES` 才关闭所有更新路径。[官方安装与更新](https://code.claude.com/docs/en/setup)

前述输入准备阶段未执行升级；下节补充 Claude 的五次隔离真实安装/切换。任意故障下的事务回滚仍未验证。候选开发应保留旧版本完整运行包，在版本/来源/摘要与静态能力复核后切换；正在运行的会话不应被原地换包。应用管理的版本切换不能冒充 npm、Homebrew、WinGet 的注册升级，也不能把 CLI 降级等同其会话数据库/插件配置向后兼容。

## 仓库对齐与未完成项

只读核对了 `prepare_codex_cli.py`、`prepare_claude_cli.py` 和两个生产适配器，未修改它们。旧准备器固定 Codex 0.147.0 / Claude 2.1.273；Codex 旧 `MAX_MEMBERS=32` 与固定成员白名单不覆盖当前 macOS 52 成员，后续应按各平台实际发行包重建输入契约。不得仅替换版本字符串或放宽成任意归档。

本次仅 macOS arm64 实际执行。安全 JSON 保留其他平台官方清单，但 Linux、Windows 和 macOS x64 未下载或运行；npm registry 包签名/attestation 未验证，Codex 更新与任意故障回滚未验收；Claude 隔离通道切换结果见下节。生产硬编码版本、插件兼容边界、协议测试夹具仍需要单独实现和门禁；P0–P5 与最新版全链路不能据此记录为完成。

证据：[安全验证 JSON](validation/cli-autoupdate-codex-claude-20260919.safe.json)。JSON 包含逐文件摘要、11 次真实帮助结果、签名结果、官方来源 URL 和版本入口；不含凭据、会话或模型正文。


## Claude 真实通道切换补充（零模型输入）

在新的合成 HOME 中，先由已验签 2.1.278 原生文件执行 `install 2.1.273` 建立官方 native 安装。之后每个用例只运行一次，使用独立副本并重定位 native launcher 的符号链接，不复用用户安装。基线只含合成 `permissions.defaultMode=default`、deny 规则和非敏感偏好；没有预置跳过审批、登录、onboarding 或项目信任授权。

| 用例 | 实际 argv（省略隔离绝对入口） | 前后版本 | 持久通道/配置结果 |
| --- | --- | --- | --- |
| 建立旧版本 | `claude install 2.1.273` | 新装 → 2.1.273 | 生成官方版本文件、launcher、native 安装元数据；既有合成 settings 未变 |
| 本次选择 Latest | `claude --settings '{"autoUpdatesChannel":"latest"}' update` | 2.1.273 → 2.1.278 | 持久通道仍 stable；settings 与有效全局 JSON 字节均未变 |
| 本次选择 Stable | `claude --settings '{"autoUpdatesChannel":"stable"}' update` | 2.1.278 → 2.1.267 | 持久通道仍 latest；settings 与有效全局 JSON 字节均未变 |
| 官方持久切换 Stable | `claude install stable` | 2.1.278 → 2.1.267 | settings 重序列化，唯一语义差异是通道 latest → stable |
| 官方持久切回 Latest | `claude install latest` | 2.1.267 → 2.1.278 | settings 重序列化，唯一语义差异是通道 stable → latest |

五次操作与操作后的五次 `--version` 均 exit 0，未发模型输入。四次更新/切换均保留有效全局运行 JSON 的完整字节；permissions、deny、defaultMode 和其他合成 settings 语义不变。原有版本文件保留且摘要不变；2.1.267/2.1.273 另行核对各自官方签名清单、文件摘要和 macOS codesign，均通过。PGPy 验签限制与前节相同。

因此，InfiniShell 可以保存自己的 FollowInstallation / Latest / Stable 偏好：FollowInstallation 使用官方 `claude update`（文档依据，本组未单独再跑无覆盖命令）；Latest / Stable 使用表中的单进程 `--settings` 覆盖，不必改写 Claude 持久通道。用户若明确要求改变 Claude 自身默认通道，则 `install latest/stable` 是已实测的官方入口，但应说明它会重新写 settings 文件。升级器应按版本输出、目标文件和配置快照判定成功，不能只看进程 exit 0。

本次显式设置 `CLAUDE_CONFIG_DIR=<HOME>/.claude` 后，有效全局运行配置实际位于 `<HOME>/.claude/.claude.json`；settings 仍是 `<HOME>/.claude/settings.json`。另一个合成 `<HOME>/.claude.json` 也纳入快照并保持不变。不能只监控旧默认路径就宣称配置未改。环境只设置 `DISABLE_AUTOUPDATER=1` 关闭后台更新，没有设置 `DISABLE_UPDATES`；产品必须尊重用户或组织的禁用策略。

这组用例没有 minimumVersion、requiredMinimumVersion/requiredMaximumVersion，未验证组织策略、权限错误、并发配置写入、Windows 锁文件或任意失败回滚。Stable 降级和再回到 Latest 是实际成功记录，不证明旧 CLI 能打开新版会话数据或插件状态。

安全记录：[Claude 通道切换 JSON](validation/cli-autoupdate-claude-channels-20260919.safe.json)。只归档 argv、版本、配置摘要/差异布尔、合成字段路径和官方校验结果；完整合成快照与日志保留在私有目录，未归档随机安装标识。

## Codex Alpha 渠道：官方源码约束

固定 0.155.1 的 `codex update` 没有通道参数；更新动作默认 Latest。TUI 更新检查仅查询 GitHub `releases/latest`，npm 检查要求其 `latest` 指向一致版本。不能把当前二进制含 `-alpha` 当作 native updater 会保持 Alpha 的证据。[固定更新检查](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/tui/src/updates.rs)

已下载的官方 install.sh 接受 `--release` 或 `CODEX_RELEASE` 指定完整 `x.y.z-alpha.N` / `x.y.z-beta.N`，不接受裸 `alpha` / `beta`。指定完整版本后移除 `packages/standalone/auto-update-version` 标记；不会持久保存 Alpha 跟随设置。若为已确认的 standalone 来源，给 `codex update` 的进程环境设置完整 `CODEX_RELEASE`，可沿其源码调用链传给安装器；此路径仅源码核实，未执行 Alpha 安装。

若产品提供 Alpha 偏好，需要自己保存偏好并从已验证的官方标签解析具体版本，再走实际安装来源对应的配方。npm 可以显式选择其真实存在的 tag；本组没有重复查询 npm dist-tags，由总验证流程记录当前标签。不能假设 Homebrew、WinGet 或其它 CLI 存在同名渠道，也不能用普通 `codex update` 冒充 Alpha 跟随。

Codex installer 的来源布局与 package 布局是不同事实。脚本在目标 BIN_DIR 不在进程 PATH 时可能追加/重写 shell profile，无已核实的 `--no-modify-path` 参数；还会切换 current/launcher 并处理安装冲突。它不是仅复制一个可执行文件的无副作用接口。本组只读源码，未执行 Codex 安装/更新。
