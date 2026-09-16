# Codex CLI、Grok Build、Claude Code 支持优化与能力对齐计划

## 1. 目标与范围

让用户在 InfiniShell 中使用三款 CLI 时，可以通过一致的入口完成启动、输入、查看状态、处理审批、停止任务、继续会话和查看结果。各 CLI 的模型、权限和协议差异由适配层处理；尚未验证或上游不提供的能力应明确显示为不可用，不能通过猜测终端文本伪造支持。

本计划已进入实施阶段。目标覆盖 P0–P5，全部验收满足前保持 Goal active；具体版本证据见 [PROTOCOL_EVIDENCE.md](PROTOCOL_EVIDENCE.md)。

- 评估日期：2026-09-16。
- 仓库基线：`6921a9925`。
- 本机已核实：Codex CLI `0.147.0`、Grok `1.0.30 (04b7ffed98c6)` 的版本及相关帮助。
- 当前 shell 的 PATH 未找到 `claude`；这不代表所有主机均未安装。
- 本地版本和最新官方文档可能不同；最低兼容版本由 P0 实测确定。
- 本期覆盖桌面 GUI、其终端中的三款 CLI，以及本地子任务。平台目标为 macOS、Linux、Windows；SSH/tmux 单独验收。
- 模型、MCP、各 CLI 自己的多代理和沙箱能力按各自配置与协议保留差异；本次统一应用集成体验，不跨 CLI 自动复制这些配置。
- InfiniShell 自身的 `warp_tui` 前端、云端任务、模型 API/BYOP、SuperGrok OAuth 重接属于独立工作，不作为本次 CLI 对齐的前置条件。

## 2. 当前基线

“已有”表示存在代码路径，不表示本次已经完成实际 CLI 验收。

| 能力 | Codex CLI | Claude Code | Grok Build |
| --- | --- | --- | --- |
| 命令识别、安装检测、品牌和工具栏 | 已有 | 已有 | 无专用适配；可用自定义规则启用通用工具栏 |
| 富输入提交 | 专用括号粘贴策略 | 延迟 Enter 策略 | 通用提交，尚未验证 |
| 技能菜单 | 已有来源过滤，使用 `$` 前缀 | 已有来源过滤，使用 `/` 前缀 | 无 `.grok/skills` 适配 |
| 图片、文件上下文、改动查看 | 有共用入口，需逐平台实测 | 有共用入口，需逐平台实测 | 尚未针对 Grok 验证 |
| 结构化状态和通知 | OSC 777 插件；OSC 9 基础兜底 | OSC 777 插件 | 无专用事件处理器和插件管理器 |
| 插件安装与更新 | 已有 | 已有 | 未接入 |
| 本地子任务启动 | 已有子 pane 路径，受开关控制 | 已有子 pane 路径 | 未实现 |
| 本地子任务权限 | 启动命令固定跳过审批及沙箱 | 启动命令固定跳过审批 | 未实现 |
| 子任务持久化和恢复 | 有缺口，任务配置未完整落盘 | 有缺口，Driver 的导出被跳过 | 未实现 |
| 应用托管结构化协议 | 当前主路径未接 app-server | 现有 Driver 与隐藏子 pane 是两条不同路径 | 当前未接 ACP |

### 必须先处理的事实

1. `CLIAgent` 已包含 Codex/Claude，未包含 Grok。命令、图标、技能、输入策略、插件和 harness 的判断分布在多个模块，新增时容易漏接。
2. Codex 的结构化通知需要插件；仅收到 OSC 9 文本时无法可靠区分审批与完成。不能把基础通知当成完整生命周期。
3. Claude/Codex 隐藏子 pane 写死权限跳过参数。Claude 的环境准备还会写用户全局 onboarding、项目信任与危险模式提示设置；Codex 本地路径已刻意避免类似全局改写。
4. `local_child_task_config` 当前没有完整落盘目标；CLI 会话 ID、应用任务 ID、启动尝试之间尚未形成完整恢复契约。
5. Grok hooks 使用 camelCase 字段；公开清单没有明确列出 `PermissionRequest`。`PermissionDenied` 不能映射为“正在等待审批”。
6. Grok 会兼容加载 Claude 的配置、技能和 hooks，存在错误上报为 Claude、重复事件及重复通知的实际适配风险。
7. Grok 本机帮助规定 `--session-id` 创建新会话，恢复使用 `--resume`/`--continue`；不能直接照搬文档中不同的描述。
8. 现有跨平台 workflow 的定向测试筛选尚未显式覆盖完整 CLI 集成测试组，需要在实现阶段补上。

## 3. 对齐后的产品约定

### 3.1 两种使用方式

**终端交互模式**：用户运行原生 CLI，保留其 TUI、登录和审批界面。InfiniShell 提供工具栏、富输入、文件与改动入口、经过验证的状态和通知。审批需要人工操作时，可跳转到对应终端。

**应用托管任务模式**：用户从 InfiniShell 创建或派发任务，应用管理启动、输入、结果、审批、取消和恢复。只有相应版本通过生命周期验证后才开放。不能依靠隐藏一个等待审批的 TUI 或无条件绕过权限来实现无人值守。

两种方式共享身份、能力描述、任务状态和结果展示；底层传输允许不同。普通 PTY 会话不能无缝转换为协议会话，除非上游明确支持并已验证。

### 3.2 统一体验的验收目标

| 用户动作 | 三款 CLI 的共同目标 | 能力不足时的行为 |
| --- | --- | --- |
| 发现和启动 | 显示安装情况、版本、可用模式，支持选择项目目录 | 给出安装或升级入口，不把 PATH 命中当作已登录 |
| 输入 | 中英文、多行和长提示词只发送一次；草稿不因切换会话丢失 | 使用已验证的终端输入方式，禁用不支持的附件 |
| 技能与上下文 | 仅显示该 CLI 实际能理解的技能；路径、图片和评审意见正确传递 | 保留来源与调用差异，不强行统一 `$`/`/` |
| 状态 | 启动中、运行中、等待用户、成功、失败、已取消、连接中断含义一致 | 缺少可信事件时显示状态未知，不能推断成功 |
| 审批 | 定位到正确任务，用户能够明确批准或拒绝 | PTY 模式返回原生终端；托管模式仅在可回传审批或已明确选择“无交互、需审批即拒绝”的受限策略时启动 |
| 停止 | 取消当前任务，识别停止完成，不误杀其他会话 | 超时后显式提供终止进程操作，不静默转成成功 |
| 继续与恢复 | 区分继续原会话、重试失败请求、创建新会话；可识别恢复失败 | 说明限制，保留本地记录，不悄悄新建来冒充恢复 |
| 插件管理 | 识别未装、过旧、禁用和安装失败；支持恢复到原状态 | 插件不可用时仍能使用普通终端 |
| 模型及推理设置 | 按版本暴露真实可用选项；作用于当前任务 | 默认继承 CLI 设置；缺能力时不展示虚假的统一选项 |

“三方对齐”首先指共同用户动作有可靠的完成路径。原生 GUI 审批、会话分叉、推理档位等扩展能力按实测结果单列，不通过降低其他 CLI 能力来制造表面一致。

## 4. 技术路线

### 4.1 复用现有结构，收拢能力判断

- 在现有 `CLIAgent` 周围增加小型能力描述，覆盖命令身份、技能来源、输入策略、附件方式和可用会话操作。静态能力与运行时版本、插件状态分开。
- `Harness` 继续表示任务执行方式，通过显式映射关联 `CLIAgent`；不要把枚举值、安装成功、登录成功和功能开关混成一个布尔值。
- 先迁移 Codex/Claude/Grok 涉及的判断，保持其他已支持 CLI 的行为不变；不新建通用插件平台或大规模重构。
- 扩展枚举时检查设置、共享会话、遥测和持久化的序列化兼容，保留旧值与未知值的读取行为。

### 4.2 输入和附件

- 复用 `use_agent_footer`，每款 CLI 独立验证括号粘贴、Enter 时序、中文输入法、多行输入和取消后的输入归属。
- 对图片分别验证剪贴板、路径或协议附件支持，不假定三款 CLI 都接受同一个 Ctrl-V 快捷键。
- 文件、技能、代码评审意见复用现有上下文构造，按实际 CLI 语法发送；覆盖路径空格、引号、中文、Windows 路径和 SSH 远端路径。
- 普通管理命令，如 `grok plugin`、`codex --version`，不能被识别为正在运行的交互会话；建立三方一致的识别规则和例外夹具。

### 4.3 事件、状态和插件

- 为 Grok 建立独立转换器，将上游 hooks 映射到现有版本化 `CLIAgentNotification`/OSC 777 协议；优先沿用 v1，只有新增语义无法兼容时才升级协议。
- 明确事件来源：CLI 结构化事件、基础桌面通知、本地输入动作、进程退出。来源强度不同，不能互相替代。
- 使用应用会话实例、启动代次和上游会话 ID 绑定事件；有上游事件 ID 时用于去重，没有时也要防止两个通知通道重复消费和旧实例回调污染新任务。
- 单个工具失败不等于任务失败；开始停止不等于已取消；进程退出不自动等于任务成功。补足断线、未知状态和竞态测试。
- 三款普通 PTY 的 Stop hook 不能证明所有 handlers 已允许结束，即使带当前 turn/prompt 和响应也只记录响应并显示 Unknown；同回合后续工具、审批和明确失败仍须有效。托管原生 Completed 使用独立终态契约，见 [Stop 契约审计](STOP_HOOK_COMPLETION_AUDIT.md)。
- 对已绑定本地任务的兼容 PTY 子 pane，结果未知持久化为 Unconfirmed，继续占用当前原生会话和 generation；不能误记断线或创建最终结果。明确断线与应用启动恢复分别处理，见 [持久状态与迁移](UNCONFIRMED_TASK_STATE.md)。
- Grok 与 Claude 共存必须检验插件继承：适配器报告真实运行的 CLI 身份，不能依靠包名或 hook 文件来源猜测。
- 规划 Grok 通知插件与安装器的版本绑定。P0 决定使用仓库随附插件目录还是团队维护的独立插件仓库；安装器不得引用尚未存在的发布地址。
- 现有 Codex/Claude 上游插件固定最低兼容版本；需要新增事件时，插件改动与桌面兼容发布有明确依赖。

### 4.4 本地启动、权限和生命周期

- 移除“隐藏子任务必然跳过权限”的假设。启动参数由明确的任务权限策略决定，默认继承用户 CLI 设置；无人值守模式必须有可用审批回路或已选择的受限策略。
- Claude 本地启动与 Codex 对齐，停止自动写全局信任、onboarding 和危险提示设置；首次需要交互时转到可见终端或明确报告尚未就绪。
- 子进程通过 `crates/command`；普通交互经现有 PTY，托管协议进程优先结构化 argv。按平台处理进程组、Windows Job/ConPTY 生命周期、EOF、退出码和取消。
- 当前本地子 pane 拒绝 PowerShell；计划中明确补 Windows 启动方式并验证，不能只增加 Grok 枚举就宣称跨平台子任务可用。
- 持久化最小任务记录：应用任务 ID、CLI 类型与版本、上游会话 ID、项目目录、执行模式、非敏感启动选项、父子关系、最终结果及恢复信息。优先扩展既有本地持久化；需要改 schema 时走 migration。
- 进程 PID 只可用于当前实例校验；重启恢复不能据此直接连接或终止进程。恢复前核对主机、工作目录和上游会话是否仍存在。
- 取消、输入、审批、恢复必须针对对应任务实例；关闭窗口或进程退出后，过时异步回调不得继续向新 PTY 写入。
- 父子任务消息复用现有信箱和运行标识，但分别验证直接子 pane、插件和 Driver 的真实路径。覆盖父→子追加指令、子→父进度/阻塞/结果，统一消息关联 ID、顺序、投递状态、接收确认、取消后的处理和最终结果回收，防止重连重复执行；不能用提示词要求子代理发送消息来替代软件侧的可靠交付。

### 4.5 应用托管协议的候选

| CLI | 候选接口 | P0 必须核实的部分 |
| --- | --- | --- |
| Codex | 官方 `app-server` | 安装版本的握手、turn/thread、审批、取消、恢复与事件终态 |
| Claude | 官方 headless/stream-json；必要时评估 Agent SDK | 双向输入、权限响应、interrupt/resume、稳定会话 ID；与现有 Driver 的取舍 |
| Grok | 官方 `grok agent stdio` ACP；headless 流用于可验证的补充场景 | 能力协商，以及权限、取消、恢复的实际实现；公开示例仅证实部分生命周期 |

先验证每款 CLI 的新建、两轮交互、审批允许/拒绝、取消、退出和恢复，再冻结内部最小会话接口。不能因为“支持 ACP”就写死未证实的方法，也不要求 Codex/Claude 强行转换为 ACP。

## 5. 分阶段实施与 PR 拆分

### P0：兼容性与协议验证

**交付**：三款 CLI 的受测版本表、官方接口依据、原始事件夹具、能力缺口和运输方式决策。

- 固定当前版本和计划支持的最低版本；补齐 Claude 测试环境。
- 在临时项目及隔离的测试配置中采集 PTY/hooks/协议结果；测试不改用户正常配置。
- 对 Grok 验证权限、取消、恢复和 Claude hook 继承；对 Codex 验证当前 `0.147.0` 与文档的差异。
- 确认插件源码与发布归属、平台覆盖和安装回退方式。
- 真实模型验证使用明确的测试凭据；CI 的确定性测试使用模拟进程与事件夹具。

**退出条件**：每项目标有“已验证接口”或具体缺口；受管模式尚缺关键接口时不进入默认开放。

### P1：身份、能力与基础交互对齐

**PR 1：身份和能力。** 增加 Grok、版本/安装探测、技能来源与调用格式、品牌复用；收拢三方能力映射，保持原有 CLI 兼容。

**PR 2：富输入和上下文。** 完成三方提交策略、附件、文件引用、评审意见、草稿和焦点处理；修复现有不一致。

**退出条件**：三方可从同一类入口发现和使用；中文、多行和长文本不丢失、不重复，所有开放的附件方式实测通过。

### P2：事件、通知和会话状态对齐

**PR 3：Grok 插件及事件转换。** 完成插件安装/升级/禁用状态、版本检查、独立事件适配、Claude 继承场景隔离。

**PR 4：共用状态可靠性。** 对齐开始、运行、等待、终态、进程退出和取消语义；处理重复、乱序、过时事件与通知兜底。

**退出条件**：三方可信事件覆盖的状态一致；没有可信事件时显示明确降级。重复事件仅触发一次通知，旧会话不能更新新会话。

### P3：本地任务启动与权限对齐

**PR 5A：已有启动路径修复。** 改造 Claude/Codex 固定绕过权限和 Claude 全局配置写入，同时交付可见审批回跳或明确的无交互拒绝策略；纳入第一轮交付。

**PR 5B：扩展本地启动。** 补 Grok harness 映射和可用性判断，完成 Windows 启动路径；纳入第二轮交付。

依赖 P0 的接口验证。不能只删除权限跳过参数而留下不可见审批窗口；审批回路与启动参数必须同一 PR 完整落地。

其中 Claude 全局配置写入的修复应在 P0 后优先拆出小 PR，可与 P1 并行；权限策略和审批状态的完整改造仍依赖 P2。

**退出条件**：三方子任务不会因隐藏审批永久挂起；批准、拒绝和取消有明确结果；用户既有全局配置不被启动流程改写。

### P4：任务记录、继续、恢复及托管控制

**PR 6：任务记录、消息和恢复状态机。** 建立应用任务与 CLI 会话映射，保存父子关系、消息确认及结果。先交付恢复状态和控制契约；旧信箱或插件若依赖已移除服务，先打通本地投递与结果闭环。

**PR 7：生命周期和托管协议适配。** 分别完成三方原生 session ID 捕获、继续、重连、取消和历史恢复；托管传输采用 P0 验证后的 Codex app-server、Claude 接口、Grok ACP。复用共同任务状态和控制入口，按适配器拆为小 PR，每个独立通过同一套契约测试。区分“重新打开仍在运行的任务”和“启动新进程恢复历史会话”，防止同一会话并发恢复。

**退出条件**：三方通过“新建 → 两轮输入 → 审批 → 取消 → 继续 → 应用重启 → 恢复 → 结果回收”。未通过的 CLI/版本只开放已验证模式，不能把整个托管能力标为完成。

### P5：发布验收和兼容维护

**PR 8：验证与发布收口。** 完成完整三方矩阵、中英文布局、跨平台和 SSH/tmux 验证；整理最低版本、已知差异、插件升级和回退说明。

新增产品入口和运行路径用同一开关控制；按 CLI、能力分批开启，防止单个上游接口变化拖累所有已有 CLI。

**退出条件**：所有目标平台分别有验证证据，未覆盖平台和能力明确列出；首个正式发布范围不含未通过的组合。

## 6. 写入范围与并行边界

| 工作块 | 主要文件/目录 | 依赖与限制 |
| --- | --- | --- |
| 身份与发现 | `app/src/terminal/cli_agent.rs`、`crates/warp_cli/src/agent.rs`、`crates/ai/src/skills/skill_provider.rs` | 先确定共同枚举与能力约定，单一写入负责人 |
| 输入与附件 | `app/src/terminal/view/use_agent_footer/`、输入与上下文相关模块 | 依赖 PR 1；可与独立插件制作并行 |
| 插件与事件 | `app/src/terminal/cli_agent_sessions/{plugin_manager,listener,event}/`、`crates/warp_core/src/cli_agent_protocol.rs` | 上游适配器可分开，公共协议/状态模型由一个负责人整合 |
| 启动与编排 | `app/src/pane_group/pane/local_harness_launch.rs`、`app/src/ai/local_harness_setup.rs`、`harness_availability.rs`、`agent_sdk/driver/` | 依赖真实接口与状态约定；不可三方同时改同一个穷尽 match |
| 持久化与恢复 | 既有本地任务/会话存储和对应 migrations | 先审计当前可复用记录；不照搬旧云端恢复 spec |
| UI、本地化及发布 | `app/src/settings_view/`、`app/i18n/`、feature flags、workflow | 与实现同步，不把本地化留到最后集中补 |

P0 的三方信息收集可以并行。PR 1/4/5/6 的公共文件顺序推进；原生协议适配器在接口冻结后才分写入域并行实现。

## 7. 验证计划

### 7.1 必须覆盖的行为

1. **识别**：正常命令、绝对路径、别名、环境变量前缀、空格路径；管理命令不误识别。
2. **输入**：中文输入法、中英混排、多行、长文本、快速 Enter、连续两轮、输入途中取消、切换 pane。
3. **上下文**：技能前缀与来源、文件路径、图片、评审意见；本地与远端路径不混用。
4. **事件**：真实夹具解析、缺字段、未知事件/版本、重复/乱序、双通知通道、Claude/Grok 共存、旧进程的晚到事件。
5. **状态**：工具失败后继续、审批等待与拒绝、CLI 崩溃、只收到基础通知、正常退出但无终态、取消与最后输出同时到达；Stop hook 要求同回合继续且没有新 PromptSubmit 时，不提前锁定成功或吞掉有效后续事件。
6. **任务控制**：启动失败、首次登录、审批回跳、允许/拒绝/取消/超时、关闭应用后过期审批失效、无交互拒绝策略、子进程退出、进程树清理、父任务关闭、不会误伤其他会话。
7. **恢复**：任务记录缺失/过旧、上游会话不存在、项目移动、应用重启、CLI 升级、重复恢复请求；不可重放已有副作用。
8. **插件**：未安装、过旧、禁用、离线、安装中断、升级不生效、卸载和回退；保留用户已有配置。
9. **双向消息**：运行中追加指令→子任务确认收到→进度和结果回传→重启后续传；重复投递不重复执行，取消后的消息不会进入新会话。

### 7.2 检查命令与平台

每个功能 PR 先执行受影响的定向测试，再执行仓库强制门禁：

```sh
cargo test -p warp --lib i18n::tests
cargo check -p warp
cargo nextest run --no-fail-fast -p warp --lib -E 'test(cli_agent) | test(local_harness) | test(harness_availability)'
```

- 涉及 `ai`、`warp_cli`、公共通知协议或持久化时增加对应 crate 和模块测试；新增 Rust 测试按仓库约定放独立测试文件。
- 新增 GUI 集成测试使用仓库的 `crates/integration` 框架。真实 CLI 验证覆盖受测版本，模拟测试不能替代输入和生命周期实测。
- 所有用户可见功能完成英文本源与简体中文同步，变量一致；两种语言分别检查布局、换行、截断和错误提示。
- 本机 macOS 通过后，通过 `.github/workflows/cross-platform-preflight.yml` 验证同一提交的 Linux x64、Windows x64；补 CLI 相关定向测试筛选，构建通过不等于交互通过。
- 图形会话检查、三款 CLI smoke、PowerShell/ConPTY、Linux/Windows 图片粘贴、SSH/tmux 结果分别记录；CI 没覆盖的项目不能算通过。
- 远端测试使用包含实际修改的提交。文档计划阶段不触发远端任务；实施时按已有授权与仓库跨平台技能规则处理推送。
- 发布前按仓库要求执行适用的完整 nextest 门禁；保留 `warp_tui`/GUI 集成测试的现有独立验证边界，不能把排除项标为已验证。

## 8. 完成定义与近期顺序

**第一轮交付：P0–P2 与 PR 5A。** 让三款 CLI 的识别、工具栏、输入、技能和可验证通知达到一致体验；修复已有 Claude/Codex 子任务的配置与权限问题，同时取得后续受管模式的真实接口证据。

**第二轮交付：PR 5B 与 P4。** 完成 Grok/Windows 子任务扩展、任务记录、双向消息、恢复和经过验证的托管控制。

**正式对齐发布：P5。** 必须提交能力矩阵和实际验证证据，区分默认可用、需插件、需升级、仅终端模式和尚不支持；不得以“存在 enum/按钮/接口”代替功能验收。

实施阶段每个功能变更均须完成本地化审计与门禁。本轮新增状态未知、连接中断、插件禁用及原生审批终端提示，中英文已同步编写；已有中间构建的 i18n 与部分双语布局证据，bundle7 的授权/安装/更新六张说明布局通过。最终构建与全部变更的双语验收仍未完成。

## 9. 依据

### 仓库代码

- `app/src/terminal/cli_agent.rs:160`：CLI 身份、命令前缀、技能来源、图标。
- `app/src/terminal/view/use_agent_footer/mod.rs:123`：三方输入策略的主要扩展点；`:351` 为自定义识别兜底。
- `app/src/terminal/cli_agent_sessions/listener/mod.rs:94`：事件处理器；`:168` 为 Codex 双通知通道。
- `app/src/terminal/cli_agent_sessions/mod.rs:169`：真实结构化事件与细粒度状态资格。
- `app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs:82`：Codex 插件管理。
- `app/src/pane_group/pane/local_harness_launch.rs:118`：权限跳过参数；`:218` 为 Codex 本地启动；`:238` 为任务配置落盘缺口。
- `app/src/ai/agent_sdk/driver/harness/claude_code.rs:353`：当前跳过会话导出；`:412` 为 Claude 全局配置写入。
- `app/src/ai/agent_sdk/driver/harness/mod.rs:132`：Driver 支持边界。
- `app/src/ai/harness_availability.rs:32`：本地 harness 集合与可用性；`:96` 为模型目录现状。
- `app/src/ai/local_harness_setup.rs:34`：本地 Codex 子任务开关。
- `crates/ai/src/skills/skill_provider.rs:32`：技能来源及路径定义。
- `.github/workflows/cross-platform-preflight.yml`：现有 Linux/Windows 验证入口。

### 官方接口资料

- [Codex app-server 生命周期](https://learn.chatgpt.com/docs/app-server#lifecycle-overview)：thread/turn、继续、分叉、流事件和取消。
- [Codex hooks](https://learn.chatgpt.com/docs/hooks#matcher-patterns)：当前事件种类与工具覆盖；最低版本需另行核实。
- [Grok hooks](https://docs.x.ai/build/features/hooks)：事件、字段和兼容加载行为。
- [Grok 技能与插件](https://docs.x.ai/build/features/skills-plugins-marketplaces)：技能来源与插件机制。
- [Grok headless / ACP](https://docs.x.ai/build/cli/headless-scripting)：结构化集成候选。
- [Grok 会话](https://docs.x.ai/build/features/sessions)：会话继续与恢复，须与安装版本帮助对照。
- [Claude CLI 参考](https://code.claude.com/docs/en/cli-reference)、[程序化运行](https://code.claude.com/docs/en/headless)：结构化输出、会话恢复和权限选项；P0 固定受测版本。
- [Claude hooks](https://code.claude.com/docs/en/hooks)：审批及失败事件；事件通知与审批决定回传是不同能力。

## 10. 执行看板（2026-09-16，持续更新）

当前工作分支 `codex/cli-agent-parity`，起点 `6921a9925955a1955503e259cd935eaea4ac2ac0`。原有未跟踪计划已保留，无用户代码改动被覆盖。实现检查点 `dbee1ecae81da54a1749de88a7caffb3099d7eb7` 已提交并推送；尚未发布。插件检查点 328d5ed352 已推送；验证树现以它为基线同步冻结的 macOS 资源域修复，另一任务网页搜索改动未纳入。

| 阶段 | 当前进展 | 尚未满足 |
| --- | --- | --- |
| P0 | Codex 0.147.0 已实测双轮、允许/拒绝、steer 接收、取消及新进程 resume；Claude 2.1.273 与 Grok 1.0.30 已采集真实控制和错误事件；Claude macOS 无凭据初始化及空闲 EOF 自行退出通过，固定 Linux/Windows 原生文件完整下载与摘要核验通过 | Claude 缺测试登录；第三轮 Linux 的无凭据原生初始化/EOF 通过，Windows 因前置夹具失败待验；Grok 真实请求 402 额度耗尽；最低版本范围未完成 |
| P1 | Grok 身份、发现、版本探测、技能源已实现；管理命令排除、输入代次隔离已有回归 | 附件失败/连续提交/语音旧回调保护、托管图片/文件/技能/评审入口均已接入并新增回归；最终测试、三方真实 PTY 与双语布局未完成 |
| P2 | OSC 9 不再推断成功；三款普通 PTY Stop 保留响应并显示 Unknown，同回合有效事件仍可处理；Codex bundle7 持久来源、五项原生授权和六张双语说明布局通过；授权冷缓存及按平台正常路径组件核对清单的修复已有本地回归；独立控制 PTY 和原生 Codex TUI 经回环 SSH/tmux 通知传输已有实证 | bundle8 已验证当前通知与英文 Unknown、Unconfirmed 双语布局；中文 Unknown 新事件、英文历史选择框裁切修复及最终同提交 GUI 待验；旧来源回退、build16/19/21 失败保留，build20 相关测试撤下未跑，后续 build23 617 项通过；插件完整组合、其他平台及产品 SSH 交互仍待验；SSH 探针最终 SetEnv 已在干净 dbee1ecae 实跑通过，完整产品 SSH 仍待验 |
| P3 | Claude/Codex 不再强制越权；本地 Claude 不再改全局配置或自动安装插件；派发前展示原生终端 | Grok/Windows 本地任务扩展、审批全流程实测 |
| P4 | 已实现 SQLite 提交后确认、输入与新代原子写入、原生会话/历史/父代、消息接收来源、结果原子领取及工具协调器；兼容 PTY 的 Unconfirmed 活动占用与独立索引迁移已实现，真实 SQLite/Diesel 回归进入 build23 并通过；Codex/Claude 已接托管面板，Codex 工具跨进程恢复、两轮/审批/追加/取消/恢复与读图已有 Rust 适配器真实证据；独立进程监督、恢复退出回执与未启动 CAS 已接入，macOS 4 项监督夹具已验证诊断与恢复拒绝；真实 GUI 已完成 Codex 父子双向消息、结果回收和应用重启继续，历史完整结果新增分页查询；跨进程 Sent 故障注入、结果唯一性及无凭据空闲崩溃的恢复拒绝已通过 | Grok 关键执行仍门控；macOS 已追加专属资源域实现，本地 check、国际化 11 项、运行时/信箱 173 项与 command 5 项通过；新 worker 的 4 项 C 监督用例、真实 Codex 缺失会话/空闲崩溃通过；通用 libtest 组以同字节本机副本重验 4 项通过，初次加载停滞原因未明；真实 Codex 运行工具的宿主/CLI 两种崩溃已确认完整清理，旧异常进程组回执继续拒绝恢复；Oz 父历史桥、三方完整流程及最终同提交 GUI 验收仍未完成；Unconfirmed 双语任务详情已在 bundle8 验证，英文历史选择框修复待复验 |
| P5 | macOS build23 相关 617/617、i18n 11 项、check20 及 Python gate5 76 项通过（gate6 再验 76 项、5.142s）；Linux/Windows workflow 已接固定 Claude 获取及无凭据探针；SSH/tmux 独立记录 | 328d5ed352 第三轮 Linux 已通过本轮全部步骤，GUI integration/full workspace 按配置跳过；Windows 三处夹具修复本机通过、原生复验待新提交；历史选择框及中文 Unknown、最终发布门禁与三方图形/端到端验收仍待完成 |

原生 CLI 协议探测与产品集成验收分别记录，前者不能替代后者。未登录、额度耗尽、未运行或失败的验证均不计为完成；独立实现和确定性测试继续推进。

新增证据分别见 [Stop 契约审计](STOP_HOOK_COMPLETION_AUDIT.md)、[Unconfirmed 与独立迁移](UNCONFIRMED_TASK_STATE.md)、[Codex 原生 hook 传输](CODEX_HOOK_TRANSPORT_VERIFICATION.md)、[原生 SSH/tmux](CODEX_NATIVE_SSH_TMUX_VERIFICATION.md)、[bundle7 插件与双语布局](validation/macos-gui-bundle7-plugin-report.md)及 [Claude 无凭据验证](CLAUDE_NO_CREDENTIALS_VERIFICATION.md)。build20 只有 i18n 通过，相关测试在编译前主动撤下；build21 的非 UTF-8 文件名夹具被 macOS 文件系统拒绝，未进入产品校验，现限定 Linux 待验。后续 build23 的本地门禁通过仍是 dirty 中间工作树证据，不能替代最终同 SHA。bundle8 已确认当前 GUI 通知、英文 Unknown 与 Unconfirmed 双语任务详情；中文新事件、历史选择框裁切修复及最终同提交 GUI 仍待验。独立传输、初始化或下载不能替代模型生命周期。

本轮 [build23 门禁快照](validation/macos-local-gates-build23.json)与 [bundle8 构建快照](validation/macos-gui-bundle8-build.json)保留实际摘要及 dirty 标记；构建时含另一任务的网页搜索修改，该组文件已排除本目标暂存，最终提交须在独立工作树重验。[Darwin coalition / launchd 审计](DARWIN_COALITION_LAUNCHD_AUDIT.md)尚未证明可在普通应用权限下取得专属清理域与可靠空域回执，不能消除已记录的真实工具残留失败。

当前提交的详细进展见 [验证记录](VALIDATION_REPORT.md#当前提交验证)：dbee1ecae 的干净 macOS cargo check、脚本门禁、无模型 SSH/tmux 与 Claude 控制边界已通过；Linux/Windows 两轮 Actions 的前置失败被单独保留，不能算平台通过。bundle8 已闭合当前真实通知链并确认 Unknown 未误报成功，其输入偏差和英文历史选择框裁切也保留为未通过项。

新增 [macOS 受控 launchd / coalition 原型](DARWIN_LAUNCHD_COALITION_PROTOTYPE.md)已验证专属域跨 setsid/双重 fork 保持、仅按已知身份清理后 CID 查询返回 ESRCH；根 SIGKILL 与 bootout 均不能单独证明完成。上述是原型阶段证据；后续生产接入、真实 worker 与插件事务的当前结果见看板及验证报告。私有接口/内核版本的边界仍保留，不把原型、版本号或原生命令退出码当成完整验收成功。

后续检查点 `c55385a69` 的干净 macOS 构建、国际化 11 项及相关模块 617 项已通过。同 SHA 第二轮预检仍失败：Linux 已解决 Python 安装，传输夹具失败；Windows 下载解压后安装失败。两平台 Rust/原生验收未开始。后续 Windows 使用官方固定 NuGet 包作为作业私有解释器，Linux 夹具修复独立推进，详见 [当前验证记录](VALIDATION_REPORT.md#当前提交验证)。

插件事务新增门禁已通过：隔离树 cargo check、国际化 11 项、插件回归 163 项；Grok 真实生产安装器五阶段 1 项通过，Claude 真实升级/修补失败/安装父进程强杀后重试各 1 项通过，均未提交模型输入。失败注入与强杀覆盖范围、代码摘要和原始记录见 [验证报告](VALIDATION_REPORT.md#插件事务追加验证)。这些中间结果不替代最终同提交与三方完整生命周期。

当前第三轮同提交结果见 [平台记录](THIRD_PLATFORM_RUN_328D5ED35.md)。后续 macOS 产品接入用独占 launchd job、原生身份及资源域销毁证明替代旧进程组确认，保留直接原生退出状态；新 worker 的 C 夹具组 4 项与通用组同字节副本重验 4 项通过，初次加载停滞原因仍未确定；真实 Codex 无凭据缺失会话和空闲崩溃通过。[真实运行工具两种崩溃](validation/macos-codex-coalition-tool-gates-1.json)分别确认全部目标退出、同一系统启动内资源域销毁及生产清理证明，修复原工具残留路径。这些是冻结源码中间构建，最终同提交 GUI/平台仍待验；[本地门禁](validation/macos-coalition-local-gates-2.json)与[接入契约](DARWIN_COALITION_PRODUCT_CONTRACT.md)保留各自边界。
