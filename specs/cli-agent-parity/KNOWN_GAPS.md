# CLI 能力缺项与验收缺口

更新日期：2026-09-26。历史核对基线为 `3c5910678`，原产品冻结于 `ee839b4fc`、相关原生验证提交为 `50e1515bc`；输入实现提交 `84102bb1c687f87a2425bc1937784e77250c416c`已继续实现图片与文件卡片，npm 来源绑定另见 `b6f93f662`。新的在线收据为工作区源码摘要绑定，不是最终 SHA 验收。本表固定讨论 Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41`，不将历史版本或未登录探针外推为所有版本、账号与模型的能力。

当前交付具有阶段性合并基础，但完整体验对齐仍有下列缺项。**安全拒绝、明确降级、原生安装成功或入口可见均不等于对应功能完成。** 原先“P0–P5 全部完成、剩余项为空”的汇总结论不能作为完整 Goal 的验收结论；具体通过记录仍按其实际范围有效。

本表集中记录已确认的边界。目录精简后，历史原始通过、失败、截图与摘要由固定 Git 提交链接追溯，当前树只保留验证结论和必要夹具。功能证据见[验证结论](VALIDATION_REPORT.md)，平台与源码对应见[验证结论](VALIDATION_REPORT.md)，当前用户可用范围见[支持说明](RELEASE_SUPPORT.md)。当前状态和后续工作以本表为准，历史报告中的阶段性“完成”不关闭这些条目。

## 记录规则与优先顺序

- **功能缺项**：当前应用没有接通所列能力；拒绝路径已实现不算该能力已实现。
- **模式限制**：仅部分权限策略、输入组合或安装来源可用，必须标出范围。
- **原生语义差异**：保留各 CLI 真实协议差异；不能以实现统一按钮冒充统一执行语义，也不要求虚构上游不存在的接口。
- **验收缺口**：已有实现或局部证据不足以证明指定场景通过；不直接推定功能故障。
- **高**：直接影响原始输入、附件、子任务、升级或平台验收目标，应先处理；**中**：组合输入、扩展场景或持续兼容维护，随后处理。这里的优先级不对应 [PLAN](PLAN.md) 的 P0–P5 阶段编号。

以下条目均未关闭。“替代方式”只是当前可行操作，不是用户已接受的范围删减。关闭功能缺项需要实现和实际验证，并在验证结论中记录来源；若真实上游接口限制无法实现，需要当前认证、版本、模型和模式的证据，并由用户确认范围调整，不能自行把“不支持”改记为完成。原生语义差异应以真实合同和准确 UI 呈现验收，不强求不存在的同回合控制。后续每次关闭还应记录源码提交、平台、版本、模式、正例和失败边界，并同步能力矩阵。

## 功能与模式

### G01 — Grok 普通终端富输入自动提交

- **状态／优先级／范围**：功能缺项，高；P1 富输入、P2 可信状态、P5 验收。
- **实际情况与影响**：普通 PTY 仍拒绝 Grok 自动提交。本次 macOS 原生 `1.0.41/grok-4.7` 探针确认 `idle_prompt` 在 help 模态、已有未提交草稿及后台工具仍存活时也可能出现，不能证明编辑器为空或 Enter 安全；新启动空会话超过 66 秒又未产生该事件。负例已保留，尚无可靠自动发送实现。
- **当前替代方式**：保留草稿，明确点击复制，关闭富输入后由用户粘贴到原生 CLI；也可使用已验证的托管文本输入。
- **源码／证据**：[普通终端提交与拒绝路径](../../app/src/terminal/view/use_agent_footer/mod.rs#L1153)、[历史明确复制验收](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/OFFICIAL_40_RECOVERY_AND_INPUT_GUARDS.md)。
- **关闭条件**：固定版本的真实普通 PTY 能在可信输入就绪时自动发送中文、多行和长文本；审批、旧会话、重连与重复点击不得误批准、丢失或重投。取得实际回合接收与结果证据后，才可移除无条件拒绝。

### G02 — Grok 托管图片输入

- **状态／优先级／范围**：功能缺项，高；P0 真实能力判断、P1 附件、P4 托管输入。
- **实际情况与影响**：本次输入增量已接通固定 `1.0.41/grok-4.7`、Inherit、无已选技能根会话的 PNG ACP 内容，保持未知版本、模型及固定策略拒绝。macOS 生产适配器取得新建图片、关闭后同原生 ID 冷恢复并提交独立图片的正例，真实历史图片字节与持久引用一致，重复消息未重复执行。原生仍声明 `image:false`，已认证正例否定此前未登录探针的能力外推。84102 GUI 图片选择器遗漏及缺省 direct 握手失败均已保留；工作区显式选择 `--leader`，保留协议阶段校验，后续 macOS 真实 GUI 已取得两张独立 PNG 字节/识色正例、同原生 ID 新进程冷恢复、应用重启后同宿主重关联且未重投，两代自然退出并清理。两轮各自绑定源码与签名后应用摘要；最终提交、跨平台及其余要求仍待验，条目不关闭。
- **当前替代方式**：满足固定版本、模型与 Inherit 无技能条件时可走新 PNG 路径；其他组合保留草稿或选用另一个已验入口。文字替代不计本功能完成。
- **源码／证据**：[输入准备](../../app/src/ai/cli_agent_runtime/managed_input.rs#L35)、[ACP 图片适配](../../app/src/ai/cli_agent_runtime/grok.rs#L2217)、[明确未登录的固定版本收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/final-same-commit-20260925/run-36116931025/windows/grok-fixed-acp.json)。
- **关闭条件**：先在已授权账号及明确模型下验证原生图片合同；若具备接口，完成类型化输入、持久引用、继续／恢复及失败保留，证明原生接收字节与实际图片回答。若接口确实缺失，保留具体证据并由用户决定范围，不能仅凭未登录声明关闭。

### G03 — Grok 普通终端图片粘贴

- **状态／优先级／范围**：功能缺项，高；P0、P1、P5。
- **实际情况与影响**：普通终端图片门禁仍保留。本次 macOS 原生 Ctrl+V 后再粘贴 bracketed text 会重复附图；改用普通 UTF-8 输入并以 Alt+Enter 换行取得恰好一段文字加一张图片及正确识色，但没有剪贴板消费 ACK 或可信输入就绪，未接通应用自动链。G02 的 ACP 正例不替代本项。
- **当前替代方式**：使用文字描述，或使用已经验证的其他 CLI 图片入口；不能把手工粘贴成功当作现有应用自动路径已通过。
- **源码／证据**：[图片按键策略](../../app/src/terminal/view/use_agent_footer/mod.rs#L96)、[图片投递门禁](../../app/src/terminal/view/use_agent_footer/mod.rs#L972)。
- **关闭条件**：分别确认三平台固定 Grok 原生图片接收方式，并在普通 PTY 实测剪贴板／拖放、焦点、审批等待与多附件顺序，证明图片进入正确会话且失败保留草稿。

### G04 — Claude 托管图片格式扩展及验收

- **状态／优先级／范围**：模式限制，中；P1 附件、P5 验收。
- **实际情况与影响**：本次输入增量已扩展固定 `2.1.280` 的 PNG、JPEG（含 jpg MIME 归一化）、静态 GIF、WebP，校验真实格式、字节、像素、整批原生帧预算及持久文件。macOS、`claude-opus-5-5` 的生产适配器 JPEG/WebP/静态 GIF 识图与冷恢复回忆已通过。随后补严 Claude 单帧 GIF 首次/恢复校验，保留 Codex 原行为；该后续修正在 `84102bb1c` 的本地门禁及静态 GIF 在线窄复验通过，另有此提交 GUI JPEG 原字节与识色正例；Linux／Windows 对应在线链仍待补，旧 WIP 收据不重标。
- **当前替代方式**：使用本次已接通的明确格式；其他格式仍须先转换，跨平台通过范围不外推。
- **源码／证据**：[格式门禁](../../app/src/ai/cli_agent_runtime/managed_input.rs#L42)、[已通过的 PNG GUI 收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/gui-claude-composer-recovery-v7.safe.json)。
- **关闭条件**：对计划支持的其他格式逐项验证原生合同、声明 MIME 与真实格式、大小限制、字节投递、图片回答及恢复；能力矩阵列明已通过格式，不使用笼统“所有图片”。

### G05 — Claude 托管纯图片及图片与技能混用

- **状态／优先级／范围**：模式限制，中；P1 组合输入、P4 继续与恢复。
- **实际情况与影响**：本次输入增量已接通纯图片，以及精确注册单技能加图片的原生 Skill 工具指令，不注入任意技能正文或绕审批。macOS `2.1.280/claude-opus-5-5` 生产适配器纯 PNG（零文字图片块）识图与冷恢复回忆通过；同轮图片加单技能已有本次整合工作区生产适配器新建与同会话冷恢复正例，两次精确 Skill AllowOnce、图片原始字节、独立识色及自然退出清理分别计证；后续 Mac GUI 已有 PNG加单技能及修复后应用重启重关联正例，三条消息和原生历史未重投；不同构建分别绑定，跨平台仍待补。更早原生 stream-json 正例的审批次数未单独计证，不被新收据回填。原先裸 slash 数组未执行技能的失败及探针指令冲突失败仍保留。
- **当前替代方式**：纯图片已有 84102 GUI 和生产适配器正例；图片加单技能已有 Mac 生产适配器和实际 GUI 正例，其他平台仍待补验。
- **源码／证据**：[托管组合输入](../../app/src/ai/cli_agent_runtime/managed_input.rs#L46)、[分轮 GUI 收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/gui-claude-composer-recovery-v7.safe.json)。
- **关闭条件**：分别完成纯图片和图片＋单技能的原生编码、接收、执行和恢复验收；多技能组合另受 G06 约束。拒绝时不得丢弃输入或部分派发。

### G06 — Claude／Grok 每轮单技能及会话启动登记限制

- **状态／优先级／范围**：模式限制，中；P1 技能、P4 会话继续。
- **实际情况与影响**：当前工作区已接通固定 Claude `2.1.280` 的无图多技能与会话内新增，整合后本地门禁通过；Grok 仍保留每轮单技能及启动登记限制；Grok 还要求继承模式，固定策略禁用技能见 G10。新增 macOS Claude `2.1.280/Opus 5.5` 原生 PTY/stream-json 证据已证明同轮顺序调用两技能、会话中新增及 reload_plugins 注册、冷恢复读取新标记；每次 Skill 单独审批。另有实现来源 `3b4d8e8a3` 的生产适配器三轮正例：多技能、新增技能、同会话冷恢复，6 次真实 Skill 单次审批，重复消息未重复执行。实现采用准确 reload_plugins 确认、失败回滚、旧回调隔离及接收后持久化；固定文件策略仍禁技能。Claude macOS GUI 首轮 alpha/beta、同会话新增后 alpha/gamma 均逐次 AllowOnce 并返回独立标记；同原生会话和 runtime、逻辑代次 1→2，SQLite 累积三项 selected_skills。第三轮 PNG加alpha 已有原生字节/识色/Skill正例；热技能导致的重启身份误判已修，GUI重关联同宿主/原生进程、同代3和三条输入，未重投，正常断开清理通过。中文说明/技能列表/历史可读；真正身份冲突另保留终态并禁恢复，技能回执先于清单落盘，ACK或清单写入失败后的幂等重放已通过真实SQLite回归，零重复发送。跨平台未完成。Grok 默认 profile、显式 leader 的原生校准已证明同轮首技能正文展开、后技能由 read_file 完整读取，以及新增后显式 reload 才可见；同 leader 两个已信任目录均被刷新，sessionId 不提供局部作用域。该探针零审批请求，不是产品多技能／热新增接通、严格串行调用或父权限上限证明。
- **当前替代方式**：Grok 产品仍一轮使用一个启动登记技能；需要其他技能时新建并绑定，不伪装为原会话继续。Claude 新增能力按已验证的无图、继承模式范围使用。
- **源码／证据**：[每轮数量限制](../../app/src/ai/cli_agent_runtime/local_skills.rs#L209)、[启动登记检查](../../app/src/ai/cli_agent_runtime/task_manager_input.rs#L287)、[Grok 单技能与恢复收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/grok-selected-skill-v7.safe.json)。
- **关闭条件**：验证多技能与会话中新增技能的真实接口及调用顺序；补齐路径与权限边界、准确注册确认、历史恢复和失败原子性。若原生不支持，按具体 CLI／模式记录证据，不从文件出现在目录推断技能已可调用。

### G07 — 三款普通终端的文件附件卡片

- **状态／优先级／范围**：功能缺项，高；P1 附件和文件上下文。
- **实际情况与影响**：本次输入增量把本地文件卡片转为明确路径引用：整批校验绝对路径、普通文件及当前账号可读性，以 JSON 保留中文/空格/引号边界，交由 CLI 原生读取与审批，失败保留草稿；不发送任意二进制内容，远程文件卡仍拒绝。三款 macOS 原生 PTY 手动路径投递均有读取独立标记正例，Claude 另有原生拒绝；尚非修改后的 GUI 链；随后 macOS Codex 实测确认“附加文件”选择成功仅插入路径正文，没有生成文件卡片，入口接线待补。原点击未选中的类型过滤误判已通过键盘复核撤回，两次均零模型输入。Grok 产品发送仍受 G01 阻止。
- **当前替代方式**：使用 CLI 可访问的文件路径或现有文件上下文入口，明确确认引用的文件；不承诺二进制文件由模型直接理解。
- **源码／证据**：[文件附件投递](../../app/src/terminal/view/use_agent_footer/mod.rs#L694)。
- **关闭条件**：界定支持的文件种类和投递语义，完成三款普通 PTY 的附件转换、正确路径／字节接收、空格与中文路径、失效文件和权限拒绝验收；不能只删除拒绝分支。

### G08 — 远程 CLI 图片传输

- **状态／优先级／范围**：功能缺项，中；P1 附件、P5 SSH／tmux。
- **实际情况与影响**：普通终端只要存在 `remote_host` 即拒绝图片粘贴；本地图片没有自动传到远端 CLI 可访问位置。三款均受此远程门禁约束。
- **当前替代方式**：用户先将图片放到目标环境，再使用该 CLI 已验证的远程文件路径读取方式；本机附件路径不能直接当远程路径。
- **源码／证据**：[远程图片门禁](../../app/src/terminal/view/use_agent_footer/mod.rs#L972)、[SSH 现有覆盖](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md#普通终端ssh-与-tmux-原生通知)。
- **关闭条件**：实现明确目标主机与会话绑定的图片传输、引用和清理，在实际 SSH／tmux 中证明远端收到原始内容；覆盖断连、重连、权限拒绝、重复投递及旧会话回调。

### G09 — 包管理器安装的自动升级

- **状态／优先级／范围**：功能缺项，高；CLI_AUTOUPDATE、P0 来源识别、P5 平台维护。
- **实际情况与影响**：提交 `b6f93f6627738fb83b6be776dd225666a900ac90` 已把 Codex/Claude 的 npm 管理器包清单、CLI 包登记、真实命令入口和安装前缀进行绑定，拒绝同名伪入口；npm 执行仍为 `ManualOnly`，Node/依赖树/lifecycle script 的执行闭包和真实升级恢复尚未完成。Homebrew 仍手动，WinGet 尚未可靠绑定。此来源识别增量不等于包管理器自动升级完成。
- **当前替代方式**：使用原安装包管理器手动升级，或由用户明确选择官方原生安装；不得自动迁移或猜测同名命令的所属安装。
- **源码／证据**：[npm 手动计划](../../app/src/terminal/cli_agent_updates/sources.rs#L1330)、[Homebrew 手动计划](../../app/src/terminal/cli_agent_updates/sources.rs#L1438)、[WinGet 未识别边界](../../app/src/terminal/cli_agent_updates/sources.rs#L1105)、[自动升级记录](CLI_AUTOUPDATE.md)。
- **关闭条件**：按包管理器分别完成来源／入口／依赖身份绑定、渠道解析、忙碌延期、实际升级与降级、失败回滚和应用重启后的中断恢复；验证不修改其他前缀或用户安装，并覆盖对应平台真实事务。

### G10 — Claude／Grok 子任务限固定权限策略

- **状态／优先级／范围**：模式限制，高；P3 权限、P4 子任务与双向消息。
- **实际情况与影响**：Claude 原有子任务要求 `ClaudeRestrictedFilesV1`；当前工作区新增独立 `ClaudeRestrictedFilesV2`，仅固定 `2.1.280` 开放受审 `Write` 创建或覆盖项目内文件，V1 父任务不可扩为 V2；macOS `claude-opus-5-5` 的真实生产父子链已证明精确 Write 允许生效、拒绝不改文件、双向 ACK／结果回收及两代正常清理，另有真实 V1 父扩为 V2 在发送原生输入前拒绝的独立链；Grok 子任务仅在相应固定读取／文件策略和已核验 SDK 合同下开放，继承设置不能证明完整父权限上限。固定策略禁用原生 shell、hooks 和技能；Claude V1 工具限 `Read`／`Edit`，V2 仅增加 `Write`；V1 的 `Write` 及两策略的 `Bash`、`Skill` 等仍被拒绝。新 V2 的待审批取消、活跃重关联、冷恢复、GUI 和跨平台尚未在线验证。现有父子消息、结果回收真实通过，不等于已提供可任意运行命令和技能的通用编码子任务。**固定策略不是操作系统文件或网络沙箱。**
- **当前替代方式**：在固定策略内拆分允许的文件任务；需要原生 shell 或技能时使用用户明确启动的继承设置根任务，不把它称为具有同等父权限保证的受控子任务。
- **源码／证据**：[子任务派发限制](../../app/src/ai/cli_agent_runtime/coordinator_tools.rs#L590)、[Claude 固定工具与参数](../../app/src/ai/cli_agent_runtime/claude_profile.rs#L13)、[Claude 空 hooks 验证](../../app/src/ai/cli_agent_runtime/claude_profile.rs#L612)、[Grok 固定工具与空技能／hooks](../../app/src/ai/cli_agent_runtime/grok_profile.rs#L316)、[Grok 父子实链](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/grok-parent-child-v8.safe.json)。
- **关闭条件**：先明确允许扩展的子任务工具范围，验证真实可约束的父权限上限，再逐项接入命令／技能等能力；审批允许／拒绝、越界拒绝、取消后进程清理、双向 ACK、冷恢复及结果必须有真实链。不得以固定绕过审批或修改用户全局配置换取可用性。

## 控制语义与持续兼容

### G11 — 运行中追加的原生语义不一致

- **状态／优先级／范围**：原生语义差异，中；P0 接口、P2 状态、P4 托管控制。
- **实际情况与影响**：Codex 活跃回合使用 `Steer`；Claude／Grok 使用 `Submit`。Grok 仅开放后续回合排队，`steer` 为 false；Claude 保留原生合并或后续轮次语义，并拒绝未验证的同回合 `Steer`。追加指令不是全部缺失，但不能向用户保证三者都即时修改当前回合。
- **当前替代方式**：使用各自已开放的追加入口，并依据排队／接收／执行状态判断；必要时先取消再提交明确的新回合。
- **源码／证据**：[统一入口分流](../../app/src/ai/cli_agent_runtime/task_manager_view.rs#L2344)、[Claude 拒绝同回合 steer](../../app/src/ai/cli_agent_runtime/claude.rs#L1286)、[Grok 未验证扩展](../../app/src/ai/cli_agent_runtime/grok.rs#L965)、[追加与 ACK 支持说明](RELEASE_SUPPORT.md#审批取消与消息确认)。
- **关闭条件**：按 CLI 固定版本保留真实追加合同与双语提示，实际验证追加顺序、合并／排队关系、ACK、取消和结果关联；只有独立验证上游同回合接口后才开放对应操作，不能把排队改名为即时 steer。

### G12 — 普通 Codex Escape 不保证工具退出

- **状态／优先级／范围**：原生语义差异，高；P2 可信终态、P4 取消、P5 验收。
- **实际情况与影响**：普通 PTY 实测 Escape 中断对话后，测试 shell 仍运行至自然结束。因此普通终端不能仅凭按键或 Stop hook 宣称工具已取消、进程树已清理；当前保持未知并提供显式关闭终端。
- **当前替代方式**：用户显式关闭终端；需要可核验的托管进程树清理时使用托管任务。二者收据不能相互替代。
- **源码／证据**：[普通 PTY 生命周期与取消记录](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md#普通终端ssh-与-tmux-原生通知)、[原始普通终端索引](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/ordinary-pty/archive-index.safe.json)。
- **关闭条件**：普通终端若提供“停止工具”能力，须实际证明所拥有工具及后代退出，并处理旧进程身份、重连和退出失败；否则持续清楚标明仅中断对话及未知状态，不计为完整工具取消通过。

### G13 — 升级发现与新版本托管适配分离

- **状态／优先级／范围**：模式限制，中；CLI_AUTOUPDATE、P0 能力判断、P5 持续兼容。
- **实际情况与影响**：官方渠道发现和安装新版本，不会自动证明该版本协议、插件、权限、继续及恢复已兼容；未验证托管版本应保持受限。固定版本本轮的通过不能外推到后续所有发布。
- **当前替代方式**：使用已验证的固定版本与模式；新版本尚未接入托管时保留普通终端和历史记录。用户渠道偏好不等于适配验收。
- **源码／证据**：[托管版本检查入口](../../app/src/ai/cli_agent_runtime/task_manager_view.rs#L2365)、[Grok 版本判断](../../app/src/ai/cli_agent_runtime/grok.rs#L230)、[版本与自动升级边界](RELEASE_SUPPORT.md#cli-自动升级与渠道)。
- **关闭条件**：对每个新增支持版本分别记录真实接口探针、受影响回归、插件兼容、权限与生命周期验收，并同步支持矩阵；本条是持续维护边界，不能要求或宣称一次验收覆盖未来版本，也不因此自动扩大当前固定版本范围。

## 验收覆盖

### V01 — Linux／Windows 完整在线模型生命周期

- **状态／优先级／范围**：验收缺口，高；P3、P4、P5。
- **实际情况与影响**：已有工作区测试、原生安装／升级、通知、真实 ACP 清理与部分 GUI，尚不构成两平台三款 CLI 各自完整在线模型链；当前主要完整生命周期证据来自 macOS。真实 Grok 四场景清理未提交模型输入，退出清理成功不能计作模型任务成功。
- **当前替代方式**：只引用已通过的实际场景和平台，明确保留其余未验；不能由 Mac 成功或协议夹具推定两平台成功。
- **源码／证据**：[同提交补验及无模型清理边界](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/FINAL_SAME_COMMIT_VERIFICATION_20260925.md)、[固定版本真实链对应](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/ACCEPTANCE_FIXED_VERSIONS_20260924.md)。
- **关闭条件**：Linux／Windows 分别用三款固定 CLI 在实际应用完成新建、两轮、允许／拒绝、追加、取消、继续、应用重启、恢复和结果回收，并记录原生 ID、消息状态、文件效果与进程清理；缺失或失败步骤不能以其他平台替代。

### V02 — Linux／Windows 物理中文输入法

- **状态／优先级／范围**：验收缺口，高；P1 中文／多行输入、P5 平台验收。
- **实际情况与影响**：Linux／Windows 现有系统剪贴板、中文字形和截图不能证明输入法组合输入、候选选择、提交键处理正常；macOS 的实际拼音输入法证据只属于 macOS。
- **当前替代方式**：已有剪贴板路径仍按其范围可用；不标记两平台实际 IME 验收通过。
- **源码／证据**：[Mac IME 收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/ime.safe.json)、[Linux GUI 收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/final-same-commit-20260925/run-36103244022/linux/linux-gui.safe.json)、[Windows GUI 收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/final-same-commit-20260925/run-36103244022/windows/gui-review.safe.json)。
- **关闭条件**：在两平台实际输入法中验证 preedit、数字／空格选词、中英文混排、多行和 Enter 不误提交，覆盖普通终端富输入与托管输入并保留真实界面及输入结果。

### V03 — SSH／tmux 远端组合与 Codex 关闭透传负例

- **状态／优先级／范围**：验收缺口，高；P2 通知、P4 重连、P5 SSH／tmux。
- **实际情况与影响**：当前固定版本三款链主要覆盖 Mac 到本机隔离 OpenSSH，并包含 tmux 断连重接；不覆盖远端 Linux、Windows、WSL 等组合。Codex 关闭 tmux 透传补测虽观察到产品接收 0，但未独立证明内层原生 hook 实际发送，不能仅用 Claude／Grok 的公共解析器正例补齐 Codex 原生证据。
- **当前替代方式**：按已测 Mac→本机组合说明支持，不把本机 CLI 安装检出当作远端就绪。
- **源码／证据**：[三款 SSH／tmux 场景表](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md#普通终端ssh-与-tmux-原生通知)、[Codex 关闭透传原收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/codex-tmux-off-one-input-v2.safe.json)。
- **关闭条件**：明确后续目标远端组合，在真实目标 CLI、插件与 PTY 上验证通知、双向交互、重复／乱序、断连恢复和取消；Codex 关闭透传须同时独立观察内层原生发送与外层无接收，不能把未触发误判为成功拦截。

### V04 — 图片投递与模型理解分别验收

- **状态／优先级／范围**：验收缺口，高；P1 附件、P5 实际结果。
- **实际情况与影响**：Codex GUI 已证明类型化图片进入原生请求，但该轮颜色回答错误，不能计作图片理解成功；这也不足以单独断言投递代码出错。Claude 本次增加 JPEG/WebP/静态 GIF/纯 PNG 的 macOS 生产适配器正确识色与字节回放；Grok 固定模型 PNG 的生产适配器及后续 GUI 均有原生字节与两张独立识色正例，GUI 冷恢复和活跃重关联分别计证。后续收据为 WIP 源码摘要绑定；最终提交、其他平台及其余格式/组合仍分别欠验，原 Codex 错误回答保留；独立原生 exec 的 gpt-6-sol 同图回答经目视语义核对正确（LIME 对应亮绿），原 strict_match=false 保留，不计 GUI/产品适配器已复验。
- **当前替代方式**：分别呈现投递通过和理解未通过，保留原回答，不以重试成功覆盖原失败。
- **源码／证据**：[Codex 原收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/gui-codex-composer-v2.safe.json)、[Claude PNG 原收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/gui-claude-composer-recovery-v7.safe.json)、[图片结果说明](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md)。
- **关闭条件**：使用明确可判定的图片夹具，逐次核对原生请求字节、实际模型与回答，记录原失败的排查结果及仍未确定的原因，并取得独立正例；格式、组合和平台分别列证，不能把标签可见或接收 ACK 当作读图正确，也不要求或承诺模型在任意图片上的回答百分之百正确。

### V05 — Linux／Windows Grok 专属双语视口

- **状态／优先级／范围**：验收缺口，中；P1 入口、P3 权限、P5 本地化布局。
- **实际情况与影响**：两平台现有英文／简体中文截图主要选择 Codex，证明所见按钮、输入与附件标签；未完整覆盖 Grok 固定权限说明、技能禁用提示、滚动后区域及相应交互状态。本轮 macOS 已检查 Grok 英中权限、附件、技能限制、消息与历史结果完整滚动区，无截断遮挡；不能替代两平台布局或未执行的审批交互。
- **当前替代方式**：保留现有可见范围通过，同时明确未检查视口，不能用 Fluent 键一致性代替实际布局。
- **源码／证据**：[Grok／Claude 排队与模式提示视图](../../app/src/ai/cli_agent_runtime/task_manager_view.rs#L1790)、[Linux GUI 原记录](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/final-same-commit-20260925/run-36103244022/linux/linux-gui.safe.json)、[Windows GUI 原记录](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/final-same-commit-20260925/run-36103244022/windows/gui-review.safe.json)。
- **关闭条件**：两平台分别启动实际英文和简体中文界面，选择 Grok 并覆盖继承、固定读取／文件策略、技能禁用、审批及完整滚动区域；按支持窗口尺寸检查截断、换行、控件与焦点，并保留原图和构建来源。

## 阶段合并与完整验收的区别

本表不要求为了文档整理再次运行全部测试，也不将阶段性合并等同于发布或 Goal 完成。合并说明应公开这些条目、已实现范围与现有证据；原始失败被保留是可追溯性要求，不表示修复后独立通过记录无效。完整 Goal 的后续关闭应逐项解决功能缺项与验收缺口，并明确处理真实原生差异，不能再以“已可靠拒绝／已记录不支持”代替实现与验收。

本次输入增量包含用户功能变化，英文与简体中文图片/文件说明已同步；macOS 当前 WIP 布局范围见验证结论。整合工作区本地门禁已通过，最终提交绑定与相关平台验收仍待补；本段所述产品增量不能沿用此前“仅文档、无需本地化变更”的描述。本次仅更新五份状态文档，无需本地化变更。
