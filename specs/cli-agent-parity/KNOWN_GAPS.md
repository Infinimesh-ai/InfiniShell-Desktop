# CLI 能力缺项与验收缺口

更新日期：2026-09-27。历史核对基线为 `3c5910678`，原产品冻结于 `ee839b4fc`、相关原生验证提交为 `50e1515bc`；输入实现提交 `84102bb1c687f87a2425bc1937784e77250c416c`已继续实现图片与文件卡片，npm 来源绑定另见 `b6f93f662`。新的在线收据为工作区源码摘要绑定，不是最终 SHA 验收。本表固定讨论 Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41`，不将历史版本或未登录探针外推为所有版本、账号与模型的能力。

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

2026-09-26 按用户要求形成的**功能代码优先**检查点现已进入无人值守回归阶段；真实 GUI／模型验收仍后置。下列“未验证代码”不关闭缺项，也不改变已有原始成功／失败的范围；G01/G03 的 macOS／Linux／Windows 专属后端已合入当前主工作区，原 `.worktrees/grok-owned-terminal` 工作树保留；其他增量也已整合。代码检查点为 `704bb33bb2943f8a686b24b843c3b3a1ba656c19`，已通过 macOS arm64 编译及 i18n 门禁；后续修正提交 `b78b62a48223235e8c29157e3786747c8db2ff68` 的 Linux／Windows 同提交 `cargo check` 均已通过。最新定向回归与夹具复验见[最新门禁](VALIDATION_REPORT.md)，这些门禁不代表完整产品验收。以下未验证均指真实功能／目标平台验收尚未完成。

### G01 — Grok 普通终端富输入自动提交

- **状态／优先级／范围**：功能缺项，高；P1 富输入、P2 可信状态、P5 验收。
- **实际情况与影响**：普通 PTY 仍拒绝 Grok 自动提交。本次 macOS 原生 `1.0.41/grok-4.7` 探针确认 `idle_prompt` 在 help 模态、已有未提交草稿及后台工具仍存活时也可能出现，不能证明编辑器为空或 Enter 安全；新启动空会话超过 66 秒又未产生该事件。负例已保留，GUI 自动发送接线尚未完成。
- **本轮实现**：新增固定 macOS arm64 owned TUI 启动、真实 PTY 内核身份、原生 leader 侧车、SQLite 一次领取和精确 ACK；通知插件 `0.1.5` 观察原生权限，缺失／变化即撤销。生产路径在真实 shell PTY 完成两轮中文输入、独立模型标记、重复拒绝和进程清理；未经过 GUI，审批、编辑／重连、冷恢复及 Linux／Windows 尚待接通验收，不能关闭本项。原始证据与范围见验证报告。
- **恢复增量**：本次侧车改为异步接收；退出清单保留，SQLite 恢复前同步置忙，旧回调及定时器隔离，普通 TUI 禁止进入托管新建/恢复。macOS 本地回归与两轮原生输入通过；空白 GUI 双语启动不代替真实会话重启验收，未接通消费者自动发送。
- **持久目录增量**：新清单与临时 socket 分离，绑定目录内核身份；两轮原生输入通过，首轮退出恢复的 `leader.lock` 残留失败保留，修复后在原退出现场独立复验清理通过。旧启动不可重派，GUI、真实重启与其余关闭条件仍未完成。
- **未验证代码增量**：macOS arm64 与 Linux x86_64 的 GUI 启动、原生侧车输入、精确回执、恢复占用及菜单已合入主工作区。Linux 通过真实 PTY、pidfd 和 SO_PEERPIDFD 绑定进程，缺少内核能力时不使用裸 PID 替代；不能声明全部 Linux 版本可用。Windows 已合入原始 ConPTY shell 句柄、创建时原子 Job 归属、命名管道侧车、PowerShell 启动及恢复接线；原生 Job／控制台／管道链仍须真实校准。任务列表历史继续也已接同UUID --resume、新generation CAS及活跃原PTY只读关联；最后实机链仍欠。之前“GUI 尚未接线”描述对应已提交基线。
- **上下文补接**：`debed8c51` 已将有效本地／远端专属 Grok 的代码／评审／diff 上下文接入恢复后的同代草稿队列，已打开会话也保持连续追加顺序；新增四项回归本地通过，英中操作提示已同步。普通未绑定 PTY 的拒绝不变，三平台真实发送与双语布局待验，G01 不关闭。
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
- **未验证代码增量**：专属 TUI 会话新增 PNG 类型化输入，图片先校验后持久化为当前数据库 scope 的哈希文件；独立富消息主题保存文本和图片引用，worker 重读核验后发送同一 session/prompt，沿用单次领取和原生回执。纯图、文字加多图、粘贴／拖放进富输入框及失败保留已接线；不再依靠剪贴板时序。macOS arm64、Linux x86_64 与 Windows x86_64 已共用固定 1.0.41/grok-4.7/default 输入；Windows ConPTY 入口已整合。已开展的自动化门禁见[最新门禁](VALIDATION_REPORT.md)，图片模型及真实 GUI 验收仍待补。
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

### G06 — Claude／Grok 多技能与会话内新增的实现及验收

- **状态／优先级／范围**：模式限制，中；P1 技能、P4 会话继续。
- **实际情况与影响**：当前工作区已接通固定 Claude `2.1.280` 的无图多技能与会话内新增，整合后本地门禁通过；Grok 工作区已接入固定 1.0.41/grok-4.7、Inherit、私有独占 leader 的多技能及空闲热新增；固定策略禁用技能仍见 G10。新增 macOS Claude `2.1.280/Opus 5.5` 原生 PTY/stream-json 证据已证明同轮顺序调用两技能、会话中新增及 reload_plugins 注册、冷恢复读取新标记；每次 Skill 单独审批。另有实现来源 `3b4d8e8a3` 的生产适配器三轮正例：多技能、新增技能、同会话冷恢复，6 次真实 Skill 单次审批，重复消息未重复执行。实现采用准确 reload_plugins 确认、失败回滚、旧回调隔离及接收后持久化；固定文件策略仍禁技能。Claude macOS GUI 首轮 alpha/beta、同会话新增后 alpha/gamma 均逐次 AllowOnce 并返回独立标记；同原生会话和 runtime、逻辑代次 1→2，SQLite 累积三项 selected_skills。第三轮 PNG加alpha 已有原生字节/识色/Skill正例；热技能导致的重启身份误判已修，GUI重关联同宿主/原生进程、同代3和三条输入，未重投，正常断开清理通过。中文说明/技能列表/历史可读；真正身份冲突另保留终态并禁恢复，技能回执先于清单落盘，ACK或清单写入失败后的幂等重放已通过真实SQLite回归，零重复发送。跨平台未完成。Grok 默认 profile、显式 leader 的原生校准已证明同轮首技能正文展开、后技能由 read_file 完整读取，以及新增后显式 reload 才可见；同 leader 两个已信任目录均被刷新，sessionId 不提供局部作用域。早期探针零审批请求，不作父权限上限证明。后续 macOS 生产适配器 v3 已通过双技能→热新增→同原生 ID 冷恢复三轮，按实际原生展开、read_file 字节和结果分别计证；调用不保证串行。GUI 首轮双技能、v4 同会话热新增、两次应用重启同宿主重关联、新进程同原生 ID 冷恢复第三轮均通过；三条 NativeProtocol ACK、零自动重投，两代原生退出 0 并清理。未信任目录、握手顺序和热新增入口旧失败保留，未改变全局审批。macOS 中英文完整滚动区域可读；v3/v4 分别绑定源码与二进制，未变模块按摘要复用，不宣称同一二进制全量重跑。同提交云端门禁与两平台已认证模型／GUI、Grok 用户级技能来源扩展及固定权限模式仍欠验，G06 不关闭。仓外 `validation/grok-g06-20260926/index.safe.json` 摘要 `3606b77b9c1e716e1d85f4a1705213dea5170c8b4ca694ea029635482acb3710`。
- **未验证代码增量**：固定 Grok 的技能目录新增严格的 `user:<name>` 来源绑定，与既有 `local:<name>` 一起核对唯一名称、规范路径与文件摘要；初始选择、热新增和冷恢复统一接线。原生用户级目录及模型链待最终验收，固定策略仍禁技能。
- **当前替代方式**：两款按固定版本、无图和继承模式的已验证范围使用；Grok 热新增必须在空闲时等待原生确认，不能把目录刷新视为调用成功。未信任项目需通过原生项目信任流程处理，应用不自动写入信任。
- **源码／证据**：[每轮数量限制](../../app/src/ai/cli_agent_runtime/local_skills.rs#L209)、[启动登记检查](../../app/src/ai/cli_agent_runtime/task_manager_input.rs#L287)、[Grok 单技能与恢复收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/grok-selected-skill-v7.safe.json)。
- **关闭条件**：验证多技能与会话中新增技能的真实接口及调用顺序；补齐路径与权限边界、准确注册确认、历史恢复和失败原子性。若原生不支持，按具体 CLI／模式记录证据，不从文件出现在目录推断技能已可调用。

### G07 — 三款普通终端的文件附件卡片

- **状态／优先级／范围**：功能缺项，高；P1 附件和文件上下文。
- **实际情况与影响**：本次输入增量把本地文件卡片转为明确路径引用：整批校验绝对路径、普通文件及当前账号可读性，以 JSON 保留中文/空格/引号边界，交由 CLI 原生读取与审批，失败保留草稿；不发送任意二进制内容，远程文件卡仍拒绝。三款 macOS 原生 PTY 手动路径投递均有读取独立标记正例，Claude 另有原生拒绝；此前 GUI 入口只插入正文路径的负例已保留，后续四文件增量已接通选择器 PendingFile 卡片并保留 CLI 锁定输入模式与旧回调校验。macOS Codex `0.156.1/gpt-6-luna` 真实 GUI 两卡片一次发送，原生 shell 实际读取中文/空格路径的两份文件并返回独立标记；英文和简体中文布局可读。中文首轮模型转向 TextEdit、审批被拒的读取失败仍保留，不冒充正例。Grok `1.0.41` 两卡片可创建，发送仍受 G01 阻止并完整保留草稿；Claude 认证有效但普通终端首次向导未完成，本轮 GUI 链待补。失效文件、拒绝链和 Linux/Windows 完整验收仍欠。
- **当前替代方式**：使用 CLI 可访问的文件路径或现有文件上下文入口，明确确认引用的文件；不承诺二进制文件由模型直接理解。
- **源码／证据**：[文件附件投递](../../app/src/terminal/view/use_agent_footer/mod.rs#L694)。
- **关闭条件**：界定支持的文件种类和投递语义，完成三款普通 PTY 的附件转换、正确路径／字节接收、空格与中文路径、失效文件和权限拒绝验收；不能只删除拒绝分支。

- **未验证入口增量**：CLI 富输入收起时点击选择文件，现改为携带当前输入代次打开富输入，待草稿恢复事件完成后复用附件选择器，生成文件卡片。新入口不再直接插入裸路径；旧异步路径回调仍可处理。该入口真实 GUI 验收仍待补，已开展的平台自动化回归见[最新门禁](VALIDATION_REPORT.md)。

### G08 — 远程 CLI 图片传输

- **状态／优先级／范围**：功能缺项，中；P1 附件、P5 SSH／tmux。
- **实际情况与影响**：远程图片的上传、会话绑定和三款 CLI 原生消费者均已接线，未满足来源／会话合同的输入仍拒绝；真实 SSH／tmux 投递、消费及恢复验收尚未完成。
- **未验证代码增量**：已整合分块图片 RPC、SSH 连接／终端代次绑定、引用账本、发布／释放和断连撤销。固定 Codex `thread/queue/add` 已接一次 Unknown 派发与图片快照回执，语义为后续排队；固定 daemon hook 的环境不证明发起 TUI 身份，目前额外限定同 CODEX_HOME 唯一前台客户端、完整分页唯一 loaded thread，现已另接每窗格独立 app-server/显式 Unix socket/当前 TUI 一次票据、内核身份、GUI入口及查询恢复，供多 pane 独立绑定；原默认路径仍保留唯一性约束。Grok 远端专属 ticket、真实 PTY wrapper、持久状态查询、同连接撤销、类型化图片服务、当前 SSH/tmux pane 双语菜单、GUI 发送及应用重启关联已整合。Claude 固定 2.1.280 已接正式插件进程/历史候选、TTY/内核peer/映像绑定、上传原图后原生 next 消息触发 Read；写出文本不算图片消费，只有同请求后代的 Read tool_use 与最终历史 typed image 原字节匹配才清理，Unknown 持久保留且只查不重发。Claude 合计原图32MiB、单图20MiB、最多20图，原生图片重编码或历史替换的未确认引用仍保留。各路径均未运行本轮实测，不以可靠拒绝代替完成。
- **当前替代方式**：用户先将图片放到目标环境，再使用该 CLI 已验证的远程文件路径读取方式；本机附件路径不能直接当远程路径。
- **源码／证据**：[远程图片门禁](../../app/src/terminal/view/use_agent_footer/mod.rs#L972)、[SSH 现有覆盖](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md#普通终端ssh-与-tmux-原生通知)。
- **关闭条件**：实现明确目标主机与会话绑定的图片传输、引用和清理，在实际 SSH／tmux 中证明远端收到原始内容；覆盖断连、重连、权限拒绝、重复投递及旧会话回调。

### G09 — 包管理器安装的自动升级

- **状态／优先级／范围**：功能缺项，高；CLI_AUTOUPDATE、P0 来源识别、P5 平台维护。
- **实际情况与影响**：提交 `b6f93f6627738fb83b6be776dd225666a900ac90` 已把 Codex/Claude 的 npm 管理器包清单、CLI 包登记、真实命令入口和安装前缀进行绑定，拒绝同名伪入口；该来源识别提交的 npm 执行为 `ManualOnly`。后续工作区已接入 Claude 目标 `2.1.280` 的 Unix 官方单包事务：完整归档/成员核验、原子目录交换、私有无网络版本探针、POSIX 权限及中断 journal；不执行 npm install 或 lifecycle script。本地门禁和英中错误布局已过；macOS 官方包私有安装的真实更新、交换后冷恢复、外部改动保留、候选改动拒绝及未审核降级拒绝五场景通过，原失败保留。同提交 Linux/Windows 待验；不是 GUI 更新或模型生命周期验收。Codex Node/launcher 闭包、Windows npm、musl、额外 ACL、合法降级、Homebrew 和 WinGet 均未完成；此增量不关闭包管理器自动升级。
- **未验证代码增量**：Homebrew Claude 固定 cask、WinGet portable 和 Codex Homebrew macOS ARM64 的来源绑定、候选探针、发布及恢复已写入；Codex 三种 shell 补全纳入交换和回滚。Codex npm macOS ARM64／Linux x64 已补官方 wrapper、完整平台资源、实际 Node 公共入口、依赖快照及恢复身份绑定；Homebrew Node 使用私有 dylib 副本，不改原安装，Linux 探针要求 Landlock ABI 3。Windows x64 Codex npm 的 cmd/PowerShell → Node → 完整平台包、AppContainer 双探针、两步无覆盖发布及恢复已合入；Claude Windows npm 的精确包内硬链接和双入口探针已接；Grok 三平台 npm 的完整三包/原生解压、包目录与用户 bin 多位置事务，以及 Mac ARM Homebrew 双别名/三补全也已接。Codex WinGet完整目录/登记/依赖事务也已接入；Claude Unix npm 2.1.280→2.1.278受限主动降级合同也已接，但要求显式Stable且官方实时指针恰为2.1.278；当前2.1.274仍拒绝，不算当前渠道可降级。三款 Linux x64 Homebrew cask 和 Grok WinGet 固定来源事务现也已接入，Linux 来源发现与执行总入口已接；Claude 三探针、Codex 完整 musl 包及补全、Grok 双别名及补全分别绑定。macOS Intel 已按用户明确范围排除。用户安装未运行升级；实际公共入口、隔离、回滚与恢复仍待验，未知版本门禁保留。
- **本批真实验收入口**：`39941b281` 接入 Codex/Grok 私有 npm 四场景入口；Mac ARM Grok 正常升级及后续 `2b59c8c62` Codex 正常升级已按各自源码独立复核；`39941b281` 的 Linux Grok 四场景也已通过并核验清理／冷恢复，不外推其他平台或来源。旧 Codex/Grok 失败和清理未知原件保留；旧 CI 的 Linux glibc ELF 失败与 ABI1 能力事实分开，Windows 一项 `LEAK` 仍待明确。详细正常结果与旧失败统一见[验证结论](VALIDATION_REPORT.md)。
- **隔离前置增量**：`2b59c8c62` 在生成 Linux Codex npm／三款 Linux Homebrew 版本变更计划前核验 Landlock ABI≥3，并同步英中不可用原因；原 worker 门禁不变。Grok npm、其他来源、Mac／Windows与同版本无候选执行不误限。本地检查通过；相关平台同提交 CI、实际双语布局和真实来源事务仍待补，不关闭 G09。
- **当前解析与入口增量**：`56f6da217` 修正 Linux 官方 Node 大 ELF 字符串表误拒，并补 Windows Codex npm 真实登记、双入口及五场景实测入口；本地门禁及同提交两平台编译／定向 Rust 通过，Linux 原文件依赖闭包通过；Windows 私有 npm 登记失败，产品事务尚未执行，G09 不关闭。无需本地化变更。
- **空闲通知补接**：`debed8c51` 将升级器的会话模型观察对齐三支持平台，避免 Linux／Windows 恢复清理后仍沿用旧 busy 状态；本地门禁通过，目标平台真实更新链待验。`c7ef8d95b` 仅修跨 Python 版本的验收夹具，不改生产校验。
- **当前替代方式**：使用原安装包管理器手动升级，或由用户明确选择官方原生安装；不得自动迁移或猜测同名命令的所属安装。
- **源码／证据**：[npm 手动计划](../../app/src/terminal/cli_agent_updates/sources.rs#L1330)、[Homebrew 手动计划](../../app/src/terminal/cli_agent_updates/sources.rs#L1438)、[WinGet 未识别边界](../../app/src/terminal/cli_agent_updates/sources.rs#L1105)、[自动升级记录](CLI_AUTOUPDATE.md)。
- **关闭条件**：按包管理器分别完成来源／入口／依赖身份绑定、渠道解析、忙碌延期、实际升级与降级、失败回滚和应用重启后的中断恢复；验证不修改其他前缀或用户安装，并覆盖对应平台真实事务。

### G10 — Claude／Grok 子任务限固定权限策略

- **状态／优先级／范围**：模式限制，高；P3 权限、P4 子任务与双向消息。
- **实际情况与影响**：Claude 原有子任务要求 `ClaudeRestrictedFilesV1`；当前工作区新增独立 `ClaudeRestrictedFilesV2`，仅固定 `2.1.280` 开放受审 `Write` 创建或覆盖项目内文件，V1 父任务不可扩为 V2；macOS `claude-opus-5-5` 的真实生产父子链已证明精确 Write 允许生效、拒绝不改文件、双向 ACK／结果回收及两代正常清理，另有真实 V1 父扩为 V2 在发送原生输入前拒绝的独立链；Grok 子任务仅在相应固定读取／文件策略和已核验 SDK 合同下开放，继承设置不能证明完整父权限上限。固定策略禁用原生 shell、hooks 和技能；Claude V1 工具限 `Read`／`Edit`，V2 仅增加 `Write`；V1 的 `Write` 及两策略的 `Bash`、`Skill` 等仍被拒绝。新 V2 的待审批取消、活跃重关联、冷恢复、GUI 和跨平台尚未在线验证。现有父子消息、结果回收真实通过，不等于已提供可任意运行命令和技能的通用编码子任务。**固定策略不是操作系统文件或网络沙箱。**
- **未验证代码增量**：ClaudeRestrictedFilesV3／GrokRestrictedFilesV2 的文件与搜索策略已接线。新增 ClaudeRestrictedSkillsV1 固定创建时技能及完整资源树，子任务仅取父集合子集；原生插件注册确认后逐次 Skill 审批，禁技能自行授权、隐式文件引用及动态 shell。旧策略不扩权。GrokRestrictedSkillsV1 也已接原生 user 技能目录与逐次 Skill 审批，旧策略不扩权。Claude/Grok ReviewedCommandsV1 及 ReviewedCommandsSkillsV1 已接精确 argv/cwd/timeout、父集合子集、逐次审批及整代清理后恢复，三目标平台现统一接固定 SDK MCP 的宿主受审命令，原生 shell 保持关闭。每条命令有独立监督代、稳定调用身份、审批和真实清理回执；前条清理后可在同一 CLI 会话继续下一条。运行中 Submit 已接回原生合并/后续排队语义；取消先结束未执行请求，已运行命令仍需真实清理。命令使用当前系统账号，不宣称操作系统沙箱。技能与命令英中说明已同步；本地资源门禁、同提交 Linux／Windows 编译及定向回归状态见[最新门禁](VALIDATION_REPORT.md)。真实命令／模型链和双语布局验收仍待补，不计验收完成。
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
