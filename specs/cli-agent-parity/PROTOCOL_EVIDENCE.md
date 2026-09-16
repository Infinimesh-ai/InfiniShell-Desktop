# CLI 真实协议验证记录

验证日期：2026-09-16。平台：macOS arm64。本记录保存 P0 接口依据及配套实现证据，不代表 InfiniShell 产品生命周期或跨平台验收已经完成。

## 受测版本与传输选择

| CLI | 本次受测版本 | 已实测接口 | 实施决定与开放限制 |
| --- | --- | --- | --- |
| Codex CLI | `0.147.0` | `app-server --stdio`、JSON Schema 导出、初始化、新建、两轮真实模型、审批允许/拒绝、追加指令确认、取消、新进程恢复同一会话与后续真实回合 | 托管模式采用原生 app-server；本次只证明该版本，不推定更早版本兼容 |
| Claude Code | `2.1.273`，由官方 npm 包隔离安装 | `--input-format stream-json --output-format stream-json --verbose --replay-user-messages`、初始化控制请求、输入确认、未登录失败、interrupt 控制确认 | 采用双向 stream-json/Agent SDK 控制语义；缺 Claude 测试登录，真实审批、运行中追加/取消及成功恢复未通过，托管模式不能据此开放 |
| Grok Build | `1.0.30 (04b7ffed98c6)` | `agent stdio` ACP v1 初始化、cached_token 登录、新建、空会话在新进程 load/resume、空闲 cancel 后 close；缺失历史错误及真实模型 402；原生与 Claude 兼容 hooks 实际执行 | 候选为原生 ACP，按运行时能力协商；当前账号 Grok Build 额度耗尽，模型生命周期与权限策略实效未通过，托管模式不能据此开放 |

版本下限暂设为各受测版本的候选基线，**不是完整最低兼容版本声明**。尚未对每款 CLI 的旧版本、Linux 或 Windows 建立完整验收。

## 隔离、来源与可复核性

- 在系统临时目录创建独立项目和各 CLI 的专用配置根，未更改正常用户设置。Claude npm 安装在临时前缀内，未全局安装。
- Codex/Grok 的真实模型验证使用现有已登录账户；仅把认证文件复制到权限为 `0600` 的临时配置，随临时目录销毁。证据和仓库中不包含认证文件、令牌、邮箱或账户标识。
- `fixtures/` 中的 NDJSON 来自真实子进程。只替换个人路径、主机/账户信息及临时项目路径，没有把模拟成功写成真实结果。Claude 的缺登录结果由 CLI 自己生成，`model: "<synthetic>"` 是原始字段。
- `probe_protocol.py` 默认不继承认证环境，只探测初始化和错误路径。调用方式：`python3 script/cli-agent-parity/probe_protocol.py --cli codex --executable <绝对可执行文件路径> --output <输出文件>`，`--cli` 也支持 `claude` 和 `grok`。
- Codex schema 由受测二进制的 `codex app-server generate-json-schema --out <临时目录>` 导出；精简索引保存在 `fixtures/codex-0.147.0-schema-contract.json`，索引保留请求的 required 字段和关键枚举。
- 完整 fixture 清单及 SHA-256 见 `fixtures/manifest.json`；其中模型请求确实经过供应商服务，未使用本地 API 替身。

## Codex app-server 契约

1. 每条连接先发送 `initialize` 请求，等待结果，再发 `initialized` 通知。受测版本响应含 `userAgent`、`codexHome`、`platformFamily`、`platformOs`。
2. `thread/start` 返回 `thread.id`、`sessionId`、持久化路径及实际权限设置。两轮 `turn/start` 均捕获到 `turn/completed`，`turn.status=completed`，文本为 `PROBE_ONE` / `PROBE_TWO`。
3. **请求响应不是运行确认。** 受测版本可能先返回 `turn/start` 的 `inProgress` 对象，再发送 `turn/started`。在两者之间发 `turn/steer` 或 `turn/interrupt` 会收到 `-32600 no active turn ...`。控制命令应等待对应 `turnId` 的真实 `turn/started`，并核对当前实例。
4. `turn/steer` 必须带 `threadId`、`expectedTurnId`、`input`；`clientUserMessageId` 可承载客户端消息关联 ID。等 `turn/started` 后实测得到 `{turnId}` 确认，但紧接着取消，所以这次不能算追加文本已被模型处理。
5. `turn/interrupt` 带 `threadId` / `turnId`。正确时序下请求获确认，最终收到 `turn/completed` 且状态 `interrupted`；不能把 interrupt 请求确认直接当取消完成。
6. 使用 `approvalPolicy=untrusted`、`sandbox=workspace-write` 在临时目录实测两个命令审批。`item/commandExecution/requestApproval` 的原始请求 ID 必须原样回传；允许用 `{decision:"accept"}`，拒绝用 `{decision:"decline"}`。允许实际生成目标文件，拒绝未生成文件。拒绝后 CLI 正常回复 `DENIED`，该回合最终 `completed`，所以工具拒绝不等于整项任务失败。
7. 当前 schema 还区分 `acceptForSession`、持久化策略修改与 `cancel`；本次只执行单次 `accept` / `decline`，未操作持久化许可。
8. 停止 app-server 后，用新进程初始化，再 `thread/resume` 同一 ID 成功，随后真实模型回合返回 `RESUMED`。这证明原生历史继续，不证明 InfiniShell 重启后自动重新关联活跃进程。
9. 不存在的 ID 返回 `-32600 no rollout found ...`。不能把该错误改为新建会话，也不能在请求结果未知时重放已交付输入。

## Claude 双向 stream-json 契约

1. 命令使用 `--print --input-format stream-json --output-format stream-json --verbose`。双向控制以 `type=control_request`、`request_id` 和 `request.subtype` 关联；已实测 `initialize` / `interrupt` 返回 `control_response`。
2. 用户输入带稳定 `uuid`。使用 `--replay-user-messages` 后捕获到原 UUID 的 `user`，`isReplay=true`，以及 `command_lifecycle` 的 `queued`、`started`；回放确认不是重复执行指令。
3. **错误优先于 success 标签。** 未登录时实测结果同时包含 `type=result`、`subtype=success`、`is_error=true`、`terminal_reason=api_error`、`result="Not logged in · Please run /login"`。只有检查 `subtype` 会把认证失败误报成功。应优先处理 `is_error`、终止原因和错误事件。
4. 受测版本的帮助把原有 `default` 权限模式改为 `manual`。不应无版本判断地给全部 Claude 版本传同一个显式默认模式；继承 CLI 配置时不传强制覆盖。
5. `dontAsk` 与 `--permission-prompts none` 是不同维度：前者拒绝需要询问的调用，后者禁止向托管权限主持者等待答案。官方说明后一个参数要求 `2.1.259` 或更新版本。两者都不意味着所有工具均禁止，也不等于 `bypassPermissions`。
6. Agent SDK 官方文档提供持续输入、权限回调、interrupt 与 resume 的完整路径；本次因为缺少 Claude 登录，只验证了控制通道和认证失败，不能声称真实审批、运行中控制、两轮完成、历史恢复通过。

## Grok ACP 与 hooks 契约

### 能力协商和错误

- 实测 `initialize` 请求为 ACP `protocolVersion=1`。返回 `agentCapabilities.loadSession=true`、`sessionCapabilities={list:{},resume:{},close:{}}`，并包含 `promptCapabilities`、认证方法和模型信息。
- 未登录时当前默认模型返回 `image=false`、`audio=false`、`embeddedContext=true`；能力必须在实际会话/模型变化时复核，不能由 CLI 名字静态推定图片能力。
- 未登录配置只公布 `grok.com`；带有效隔离登录记录时增加 `cached_token`。`authenticate` 成功后 `session/new` 返回稳定 `sessionId`。
- 没有登录时 `session/new` 返回 `-32000 Authentication required`；对不存在的会话调用 `session/load` 和 `session/resume` 均返回 `-32603`，`data.code=FS_NOT_FOUND`。
- 已登录的两轮 `session/prompt` 均返回 `-32603`，内部 HTTP 状态 `402`，信息 `Grok Build usage balance exhausted`。没有 `end_turn`，不能显示成功；不再重复消耗该失败路径。
- 受测版本 `--session-id` 只用于创建新会话，恢复应使用 `--resume`/`--continue`。
- 额外通过无模型请求的空历史测试：新建后发送 `session/cancel` 通知，再 `session/close` 成功；进程退出后分别用新的进程 `session/load` 和 `session/resume` 同一 ID 均成功。恢复响应没有顶层 `sessionId`，只有 `models`、`configOptions` 和 `_meta`。见 `grok-1.0.30-empty-session-recovery.ndjson`。这只证明空历史恢复与空闲 cancel 后连接存活，不能当作正在运行的模型已取消或完整历史恢复通过。
- Rust `grok.rs` 已实现版本核对、独立 leader socket、ACP 协商、已公布的 cached_token 鉴权和新建；事件同时区分 reported/verified 能力。未通过真实模型验收的提交、steer、审批、取消和历史恢复不开放，不发送请求或编造消息确认。

### hooks 的真实字段、继承及状态陷阱

- 临时 git 项目中通过会话级 `--trust` 授权测试脚本后，同时创建 `.grok/hooks/probe.json` 和 `.claude/settings.json`。Grok 原生与 Claude 兼容 hook 都实际收到同一批事件，见 `grok-1.0.30-hooks.ndjson`。
- `hookEventName` 的实际值为 `session_start`、`user_prompt_submit`、`stop_failure`、`session_end`、`stop`；同一载荷还有兼容 `hook_event_name`，其值为 `SessionStart`、`UserPromptSubmit` 等 PascalCase。`sessionId`/`session_id`、`transcriptPath`/`transcript_path`、`permissionMode`/`permission_mode` 同时存在。转换器兼容两种字段，冲突时应拒绝或降级。
- 所有 hook 子进程都有 `GROK_HOOK_EVENT`、`GROK_HOOK_NAME`、`GROK_SESSION_ID`、`GROK_WORKSPACE_ROOT`，包括来自 `.claude` 的脚本；`CLAUDE_PROJECT_DIR` 也存在，故不能用它猜 Claude 身份。插件应以 `GROK_HOOK_EVENT` 和匹配的 `GROK_SESSION_ID` 判定 Grok。
- 设置子进程环境 `GROK_CLAUDE_HOOKS_ENABLED=0` 后，`grok inspect --json` 将 Claude 来源标记 `disabled:true` / `compatibilityStatus:disabled`，原生来源保持启用。此为隔离开关实测，不应默认替用户禁用所有原有 Claude hooks。
- 真实 402 错误顺序是 `StopFailure` → `SessionEnd(reason=shutdown)` → `Stop(reason=shutdown,stopHookActive=false)`。最后一个 Stop **不是成功**。按会话/回合处理失败粘性与迟到事件，不能允许清理回调覆盖错误。
- 本次启动参数含 `--permission-mode dontAsk`，但失败路径 hooks 的 `permissionMode` 为 `default`；尚未产生真实工具审批，故不能声称受测版本已经按此参数拒绝所有需审批工具。
- `PermissionDenied` 仅表示工具被拒绝，不是等待审批。当前官方 hook 清单未提供可依赖的 PermissionRequest 事件；托管审批应依赖通过真实请求验证的 ACP 权限回路。
- 握手 `_meta["x.ai/hooks"]` 声明阻塞事件 `pre_tool_use`、`stop`、`subagent_stop`，与网页只写 PreToolUse 的描述不同；不要把静态文档的缺省解释扩展为已验证能力。

## 随附插件的安装与恢复验证

随附目录为 `app/assets/bundled/cli-agent-plugins/grok/`，四个发布文件固定为 `.grok-plugin/plugin.json`、`hooks/hooks.json`、`hooks/notify.cjs`、`README.md`。脚本只依赖 Node.js 标准库。

- 真实 `grok plugin validate` 通过；`plugin install --trust <带空格与中文的本地目录>` 成功。实际注册表是 `GROK_HOME/installed-plugins/registry.json`，`repos` 内有 `kind.type=Local`、`kind.source_path`、安装 `path` 和 `plugins.<name>.version`。
- 受测版本复制本地插件文件，却把 `plugin update` 描述为 `local symlink, already live`，返回 0 且没有更新文件/版本。同源重复 install 报 `already installed`；改用另一来源直接 install 会制造重复同名注册。见 `grok-1.0.30-plugin-install-update.json`。
- 安装器采用已在真实 CLI 验证的小事务：备份单个插件的四个文件，`plugin uninstall --keep-data infinishell-grok`，再安装新版本。安装失败时从持久备份重新安装旧版本。测试把来源删除作为安装失败注入，随后恢复成功；无关 `[ui].screen_mode` 始终保留。见 `grok-1.0.30-plugin-transaction.json`。这份 fixture 的插件旧版本是测试副本，CLI 行为来自真实二进制。
- 已禁用、重复同名、版本与实际文件不一致、多插件仓库、自定义来源或配置覆盖不进入自动更新；失败不通过覆盖整份用户配置恢复。
- Node 脚本的 11 项测试通过，包含真实 402 回放、Claude/Grok 双来源重复、过期回合、失败粘性、Windows 路径和 C0/C1、tmux 字节转义及跨进程状态文件。
- macOS 控制 PTY 内运行真实 Node 脚本并回放真实 hook，捕获到 `agent=grok,event=session_start` 的 OSC 777。见 `grok-1.0.30-plugin-fixture-pty.json`。这验证脚本到控制终端的通路，不代表 Grok 成功模型回合或 InfiniShell 渲染验收。
- 无登录 Grok 会在 hook 会话开始前退出，未触发任何插件状态。该失败探测保留在 `grok-1.0.30-bundled-plugin-pty.json`，不能把其零通知结果计为插件完整生命周期通过。

## 真正 Rust 适配器的显式验收入口

`app/src/ai/cli_agent_runtime/codex_live_tests.rs` 定义 ignored 测试，直接启动生产 `codex::connect`。它检查双语多行两轮、同一消息 ID 重投、临时文件命令审批允许/拒绝、收到 TurnStarted 后追加随机标记并验证最终文本已采用标记、真实 interrupted、关闭后新进程恢复原 ID，以及从历史收回追加标记。这项测试仍待完整编译后执行，未作为已通过证据。

运行前先构建 `warp` 的 libtest 二进制及同一源码的主程序或 TUI 二进制。后者提供隐藏监督 worker 入口，不能用 libtest 二进制代替：

```sh
python3 script/cli-agent-parity/run_codex_adapter_live.py \
  --test-binary <已编译的-warp-libtest-绝对路径> \
  --supervisor <同一源码已编译的主程序或TUI绝对路径> \
  --codex <codex-绝对路径> \
  --credential-source <已有-Codex-auth.json-绝对路径> \
  --output <证据目录>/codex-adapter-live.ndjson
```

脚本给独立测试进程设置临时 CODEX_HOME，复制权限为 0600 的认证副本并自动清理；Rust 不修改进程全局环境。证据包含当前提交、工作区是否仍有修改、受测 libtest 与监督 worker 的 SHA-256、版本和逐阶段结果。测试零匹配或只返回进程退出 0 均不算成功。该入口验证适配器与 CLI 进程重启，不代表 GUI 重启、任务协调器持久化或同一提交的跨平台验收。

## 仍未满足的验收

| 项目 | 当前状态 |
| --- | --- |
| Codex app-server 基本新建、两轮、审批允许/拒绝、取消、历史继续 | 已有真实 CLI 证据；仍需产品连接与同一修改提交验证 |
| Codex steer 已被模型实际应用、活跃进程重连、重复输入语义 | 只确认 steer 接收，后续立刻取消；其余未通过 |
| Claude 成功双轮、审批允许/拒绝、运行中控制、继续/恢复 | 缺测试登录，未通过 |
| Grok 成功双轮、审批允许/拒绝、取消、继续/恢复 | Grok Build 额度耗尽，未通过 |
| 原生终端输入法/多行/图片、完整父子消息、InfiniShell 应用重启 | 本 P0 进程探测不覆盖，未通过 |
| Linux/Windows、ConPTY、完整 SSH/UI、双语 UI 布局 | 本机协议验证不覆盖，未通过；独立回环 SSH/tmux 脚本传输见 `SSH_TMUX_VERIFICATION.md`，不能代替此门槛 |

## 官方依据

- [Codex app-server](https://learn.chatgpt.com/docs/app-server)：握手、thread/turn、审批和 interrupt。实现时使用受测二进制的 schema 约束请求。
- [Claude 程序化运行](https://code.claude.com/docs/en/headless)：stream-json、权限主持者与无交互拒绝、版本要求。
- [Claude CLI 参数](https://code.claude.com/docs/en/cli-reference)、[持续输入](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode)、[权限与用户输入](https://code.claude.com/docs/en/agent-sdk/user-input)：控制接口候选与原生差异。
- [Grok headless / ACP](https://docs.x.ai/build/cli/headless-scripting)：握手和会话请求示例。
- [Grok hooks](https://docs.x.ai/build/features/hooks)、[技能与插件](https://docs.x.ai/build/features/skills-plugins-marketplaces)：字段、环境、原生配置与 Claude 兼容加载。实际行为差异以上述 fixtures 为准。

探测脚本、协议夹具及技术说明无需本地化变更。随附插件安装器与 Grok 能力限制新增了用户提示，英文和简体中文消息已同步；i18n 门禁与双语布局由主验收流程统一执行，当前不计为完成。
