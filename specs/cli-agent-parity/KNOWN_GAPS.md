# CLI 能力缺项与验收缺口

更新日期：2026-09-29。历史核对基线为 `3c5910678`，原产品冻结于 `ee839b4fc`、相关原生验证提交为 `50e1515bc`；输入实现提交 `84102bb1c687f87a2425bc1937784e77250c416c`已继续实现图片与文件卡片，npm 来源绑定另见 `b6f93f662`。本轮 Claude 图片双技能在线收据已绑定提交 `2383428dac785fc34ed44120226b596d592ad191`；其他旧在线收据仍按各自源码摘要绑定，不回填最终提交验收。本表固定讨论 Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41`，不将历史版本或未登录探针外推为所有版本、账号与模型的能力。

当前交付具有阶段性合并基础，但完整体验对齐仍有下列缺项。最新相关产品源码提交为 `64bb5f6c3988a9ca463d2d8a6d79a81eb543b447`；本机定向证据只覆盖下文所述范围。含该源码的精确分支提交 `301040fe3c18c3ffc806f1b048f2e448bdc81ca5` 已通过 [Linux／Windows 普通门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36503995096)，不含在线模型或 G09 原生失败专项。**安全拒绝、明确降级、原生安装成功或入口可见均不等于对应功能完成。** 原先“P0–P5 全部完成、剩余项为空”的汇总结论不能作为完整 Goal 的验收结论；具体通过记录仍按其实际范围有效。

本表集中记录已确认的边界。目录精简后，历史原始通过、失败、截图与摘要由固定 Git 提交链接追溯，当前树只保留验证结论和必要夹具。功能证据见[验证结论](VALIDATION_REPORT.md)，平台与源码对应见[验证结论](VALIDATION_REPORT.md)，当前用户可用范围见[支持说明](RELEASE_SUPPORT.md)。当前状态和后续工作以本表为准，历史报告中的阶段性“完成”不关闭这些条目。

2026-09-28 主线整合提交 `e5f50d9fc` 已保留main与CLI Agent分支历史；同提交CI36411332886为Linux成功、Windows失败（AI OAuth取消监听后重绑端口返回10048），历史失败保留。后续7dd68cc9b整合Homebrew回滚预检与OAuth测试端口隔离，CI36419778583两平台官方成功；Windows desktop/TUI组660通过含1条LEAK（上下文菜单引用用例），句柄来源仍待查，不称全体普通PASS。main已从693172a26快进至70f65a996；测试精确绑定7dd68，之后仅三份验收文档变化。实际计数、推广及证据见[验收结论](VALIDATION_REPORT.md)。该阶段整合及固定临时目录清理约定均不关闭下列14项必要缺口，也不改变G09或Windows Node/npm既有失败结论。

## 记录规则与优先顺序

- **功能缺项**：当前应用没有接通所列能力；拒绝路径已实现不算该能力已实现。
- **模式限制**：仅部分权限策略、输入组合或安装来源可用，必须标出范围。
- **原生语义差异**：保留各 CLI 真实协议差异；不能以实现统一按钮冒充统一执行语义，也不要求虚构上游不存在的接口。
- **验收缺口**：已有实现或局部证据不足以证明指定场景通过；不直接推定功能故障。
- **高**：直接影响原始输入、附件、子任务、升级或平台验收目标，应先处理；**中**：组合输入、扩展场景或持续兼容维护，随后处理。这里的优先级不对应 [PLAN](PLAN.md) 的 P0–P5 阶段编号。

以下条目均未关闭。“替代方式”只是当前可行操作，不是用户已接受的范围删减。关闭功能缺项需要实现和实际验证，并在验证结论中记录来源；若真实上游接口限制无法实现，需要当前认证、版本、模型和模式的证据，并由用户确认范围调整，不能自行把“不支持”改记为完成。原生语义差异应以真实合同和准确 UI 呈现验收，不强求不存在的同回合控制。后续每次关闭还应记录源码提交、平台、版本、模式、正例和失败边界，并同步能力矩阵。

## 功能与模式

2026-09-26 按用户要求形成的**功能代码优先**检查点已进入无人值守回归阶段；当时真实 GUI／模型验收后置，后续范围以各项注明的最新实测为准。下列“未验证代码”不关闭缺项，也不改变已有原始成功／失败的范围；G01/G03 的 macOS／Linux／Windows 专属后端已合入当前主工作区，原 `.worktrees/grok-owned-terminal` 工作树后来已由用户清理；其他增量也已整合。代码检查点为 `704bb33bb2943f8a686b24b843c3b3a1ba656c19`，已通过 macOS arm64 编译及 i18n 门禁；后续修正提交 `b78b62a48223235e8c29157e3786747c8db2ff68` 的 Linux／Windows 同提交 `cargo check` 均已通过。最新定向回归与夹具复验见[最新门禁](VALIDATION_REPORT.md)，这些门禁不代表完整产品验收。以下未验证均指真实功能／目标平台验收尚未完成。

### G01 — Grok 普通终端富输入自动提交

- **状态／优先级／范围**：功能缺项，高；P1 富输入、P2 可信状态、P5 验收。
- **实际情况与影响**：普通未绑定 PTY 仍拒绝 Grok 自动提交；专属 TUI 入口已接线，Mac 限定 GUI 单图加中文正例见下文。历史 `1.0.41/grok-4.7` 探针确认 `idle_prompt` 在 help 模态、已有未提交草稿及后台工具仍存活时也可能出现，不能证明编辑器为空或 Enter 安全；新启动空会话超过66秒又未产生该事件。因此专属路径使用原生侧车及精确回执，不以该事件或延时授权 PTY Enter。
- **本轮实现**：新增固定 macOS arm64 owned TUI 启动、真实 PTY 内核身份、原生 leader 侧车、SQLite 一次领取和精确 ACK；通知插件 `0.1.5` 观察原生权限，缺失／变化即撤销。生产路径在真实 shell PTY 完成两轮中文输入、独立模型标记、重复拒绝和进程清理；未经过 GUI，审批、编辑／重连、冷恢复及 Linux／Windows 尚待接通验收，不能关闭本项。原始证据与范围见验证报告。
- **恢复增量**：本次侧车改为异步接收；退出清单保留，SQLite 恢复前同步置忙，旧回调及定时器隔离，普通 TUI 禁止进入托管新建/恢复。macOS 本地回归与两轮原生输入通过；空白 GUI 双语启动不代替真实会话重启验收，未接通消费者自动发送。
- **持久目录增量**：新清单与临时 socket 分离，绑定目录内核身份；两轮原生输入通过，首轮退出恢复的 `leader.lock` 残留失败保留，修复后在原退出现场独立复验清理通过。旧启动不可重派，GUI、真实重启与其余关闭条件仍未完成。
- **未验证代码增量**：macOS arm64 与 Linux x86_64 的 GUI 启动、原生侧车输入、精确回执、恢复占用及菜单已合入主工作区。Linux 通过真实 PTY、pidfd 和 SO_PEERPIDFD 绑定进程，缺少内核能力时不使用裸 PID 替代；不能声明全部 Linux 版本可用。Windows 已合入原始 ConPTY shell 句柄、创建时原子 Job 归属、命名管道侧车、PowerShell 启动及恢复接线；原生 Job／控制台／管道链仍须真实校准。任务列表历史继续也已接同UUID --resume、新generation CAS及活跃原PTY只读关联；最后实机链仍欠。之前“GUI 尚未接线”描述对应已提交基线。
- **上下文补接**：`debed8c51` 已将有效本地／远端专属 Grok 的代码／评审／diff 上下文接入恢复后的同代草稿队列，已打开会话也保持连续追加顺序；新增四项回归本地通过，英中操作提示已同步。普通未绑定 PTY 的拒绝不变，三平台真实发送与双语布局待验，G01 不关闭。
- **2026-09-27 `e2eba8596` 实际 GUI 增量**：Mac ARM64 固定 `1.0.41/grok-4.7/default` 专属入口已通过中文加单图一次发送、原生同 prompt ACK/end_turn、精确图片字节与识色回答；ACK 清草稿、应用重启不自动重投及局部英中布局通过。普通未绑定 PTY 拒绝保持；多行长文本、审批、活跃／冷恢复和其他平台仍待验，不关闭本项。
- **2026-09-29 `2383428da` 当前 GUI 增量**：Mac 独立签名版专属 TUI 真实完成英文与中文两行文本各一轮，原文进入同一原生会话、取得回执及回复；普通未绑定 TUI 拒绝并保留草稿。重启后从任务面板继续同一原生 ID，历史两条各一次，新 run 2 第三条回答 `4`。第二轮模型精确标记答错；旧会话关闭的 hook 警告和任务面板未保存最终结果保持未解。图片粘贴、权限审批、Linux／Windows 和普通未绑定 PTY 安全自动提交仍待验，G01 不关闭。
- **2026-09-29 `bd4612887` 失败提示修复**：专属会话消失或输入租约登记失败时，提交不再静默返回，复用既有双语提示并保留草稿；会话消失与 PTY 零写入的定向回归通过。普通未绑定 PTY 的可信提交与旧 run 结果持久化没有因此完成，G01 不关闭。
- **2026-09-29 当前分支结果持久化增量**：固定 Grok `1.0.41/grok-4.7` 的独立 Mac 专属 PTY 两轮在原生 ACK 后保存完整历史核验的最终正文及完成水位，任务结果按代次持久化；另一次真实链在两轮 ACK 后让应用侧测试进程退出，保持同一原生 TUI/leader 存活，由第二进程只读补查旧代和当前代，消息与原生用户回合均保持 2→2，第三进程在原生退出后重新读取两代结果。44 项定向回归、i18n 11 项和 `cargo check -p warp` 通过；安全归档 `g01-native-cold/r-G5hcBT9a` 索引 SHA-256 `6264fbdc909f5c25392e33211abd7a1114187f33bc570b5a3808f5f220335bd7`。这是专属本机原生会话的结果补写，不覆盖普通未绑定 PTY、图片、审批、GUI 双语新提示或 Linux／Windows，G01 不关闭。
- **普通未绑定 PTY 可行性复核**：固定 `1.0.41` 的 `idle_prompt` 既可出现在帮助模态、未提交草稿与后台工具运行时，也可能在空会话长期不出现；`permission_prompt` 不能证明 Enter 不会批准。当前侧车仅绑定从启动时受控的 `--leader-socket`／原生会话，已运行的普通 TUI 没有可认证的连接或编辑器状态查询。安全覆盖该入口需要原生提供经进程与会话身份核验的接入，以及带编辑器修订、模态和审批状态检查的原子 `submit_if_idle` 与一次性回执；仅增加通知或发送前检查仍有状态变化竞态。现有无条件拒绝必须保留，直至该合同与三平台实际回合验收完成或用户明确调整范围。
- **旧 run 结果落盘诊断**：旧私有 profile 中第 1 代两条、第 2 代一条输入均已获 `NativeProtocol/end_turn` ACK，但 task `result` 仍为 `None`。专属 TUI 从 TerminalView 直接建任务，未经过常规 pane 的 `bind_local_task`；旧实现的 ACK 只保存投递状态，不含模型正文。直接补绑定还会与逐次输入 checkpoint 争用 revision。当前分支已按上文实现同 leader 完整历史、水位和身份核验后的旧代结果 CAS，以及应用侧退出后的只读补写；旧 profile 的结果并未据此回填，仍按原件记为缺失。
- **当前替代方式**：保留草稿，明确点击复制，关闭富输入后由用户粘贴到原生 CLI；也可使用已验证的托管文本输入。
- **源码／证据**：[普通终端提交与拒绝路径](../../app/src/terminal/view/use_agent_footer/mod.rs#L1153)、[历史明确复制验收](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/OFFICIAL_40_RECOVERY_AND_INPUT_GUARDS.md)。
- **关闭条件**：固定版本的真实普通 PTY 能在可信输入就绪时自动发送中文、多行和长文本；审批、旧会话、重连与重复点击不得误批准、丢失或重投。取得实际回合接收与结果证据后，才可移除无条件拒绝。

### G02 — Grok 托管图片输入

- **2026-09-27 组合代码增量 `ba73c4df3`**：在固定版本／模型与 Inherit 根会话下接通 PNG＋单／多技能，组合额外要求私有独占 leader、逐项目录来源和完整注册确认；准备、排队及刷新后均重新校验。Mac ARM64 原生两轮已验证单技能＋图片及同历史冷加载双技能＋图片的原字节、技能读取和独立识色；应用适配器、GUI、热新增组合和其他平台仍待验，不关闭本项。

- **状态／优先级／范围**：功能缺项，高；P0 真实能力判断、P1 附件、P4 托管输入。
- **实际情况与影响**：本次输入增量已接通固定 `1.0.41/grok-4.7`、Inherit、无已选技能根会话的 PNG ACP 内容，保持未知版本、模型及固定策略拒绝。macOS 生产适配器取得新建图片、关闭后同原生 ID 冷恢复并提交独立图片的正例，真实历史图片字节与持久引用一致，重复消息未重复执行。原生仍声明 `image:false`，已认证正例否定此前未登录探针的能力外推。84102 GUI 图片选择器遗漏及缺省 direct 握手失败均已保留；工作区显式选择 `--leader`，保留协议阶段校验，后续 macOS 真实 GUI 已取得两张独立 PNG 字节/识色正例、同原生 ID 新进程冷恢复、应用重启后同宿主重关联且未重投，两代自然退出并清理。两轮各自绑定源码与签名后应用摘要；最终提交、跨平台及其余要求仍待验，条目不关闭。
- **当前替代方式**：满足固定版本、模型与 Inherit 条件时可走 PNG 路径；组合技能另需上述私有 leader 与目录确认；其他组合保留草稿或选用另一个已验入口。文字替代不计本功能完成。
- **源码／证据**：[输入准备](../../app/src/ai/cli_agent_runtime/managed_input.rs#L35)、[ACP 图片适配](../../app/src/ai/cli_agent_runtime/grok.rs#L2217)、[明确未登录的固定版本收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/final-same-commit-20260925/run-36116931025/windows/grok-fixed-acp.json)。
- **关闭条件**：先在已授权账号及明确模型下验证原生图片合同；若具备接口，完成类型化输入、持久引用、继续／恢复及失败保留，证明原生接收字节与实际图片回答。若接口确实缺失，保留具体证据并由用户决定范围，不能仅凭未登录声明关闭。

### G03 — Grok 普通终端图片粘贴

- **状态／优先级／范围**：功能缺项，高；P0、P1、P5。
- **实际情况与影响**：普通未绑定终端图片门禁仍保留；专属 TUI 选择器路径已有下文 Mac 单图 GUI 正例，粘贴／拖放仍欠验。历史原生 Ctrl+V 后再粘贴 bracketed text 会重复附图；改用普通 UTF-8 输入及 Alt+Enter 换行虽得到单图识色正例，却没有剪贴板消费 ACK 或可信输入就绪，不能用该手工操作代替应用自动链。G02 的托管 ACP 正例也不替代本项。
- **未验证代码增量**：专属 TUI 会话新增 PNG 类型化输入，图片先校验后持久化为当前数据库 scope 的哈希文件；独立富消息主题保存文本和图片引用，worker 重读核验后发送同一 session/prompt，沿用单次领取和原生回执。纯图、文字加多图、粘贴／拖放进富输入框及失败保留已接线；不再依靠剪贴板时序。macOS arm64、Linux x86_64 与 Windows x86_64 已共用固定 1.0.41/grok-4.7/default 输入；Windows ConPTY 入口已整合。已开展的自动化门禁见[最新门禁](VALIDATION_REPORT.md)，图片模型及真实 GUI 验收仍待补。
- **2026-09-27 `e2eba8596` 实际 GUI 增量**：修复选择图片误用内置模型能力及其他面板切换模型清除草稿。Mac 专属 TUI 的选择器单图加中文已取得产品收据、原生图片原字节及识色正例；英中单图卡片与输入控件可读。证据属于选择器路径，不替代粘贴／拖放、纯图、多图、审批失效或其余平台验收。
- **当前分支拖放修复与双图 GUI 正例**：专属 Grok 富输入打开时，双图依序进入附件，混合或纯非图片整批拒绝；会话身份失配时也先拦截，不能退回 PTY 路径写入。失配路径修复前定向测试捕获 1 次 PTY 写入，修复后双图／混合／纯非图／失配零写入 1 项通过，`cargo check -p warp` 通过。首次私有 Mac GUI 双图卡片试验因人工提前按 Return 启动普通未绑定 TUI，随后发送被保护分支拒绝，任务仍 queued 且原生消息数为零；此负例不回填。重新签名并确认专属绑定后，固定 `1.0.41/grok-4.7` 的 GUI 富输入按顺序粘贴两张不同图片、一次提交、取得原生 ACK；原生请求为文字＋图片＋图片，inline 字节与产品持久附件及原生资产两两一致，模型精确回答 `RED BLUE GREEN YELLOW`，同一最终结果及完成水位落盘。此正例属于 GUI 粘贴，不等于 Finder 拖放事件、普通未绑定 PTY、审批失效或 Linux／Windows 验收；新英中文案虽同步，真实双语拒绝提示布局仍待验，G03 不关闭。
- **Finder 拖放独立补验**：新私有签名 Mac GUI 已精确绑定固定 Grok `1.0.41` 的专属会话，富输入打开后以 CUA 从 Finder 向应用拖动两次，均只改变 Finder 选择，应用没有可观察的 drop event、图片卡或提示；任务仍 queued，未发送模型请求。该轮只能证明自动化手势没有交付，不能判定产品拖放成功或失败，也不能充当英中拒绝提示布局检查。安全索引 `g03-finder-drop/r-d_2ajoia/index.json` SHA-256 `c870674c58bfe26b5e0067591db47f82178928782d95f4b17825b316fdcda5d6`；私有进程和短目录已核验清理，G03 不关闭。
- **真实事件路由缺口与修复 `64bb5f6c3`**：只读审查发现富输入 editor 作为 child 会先消费拖放，把混合批次分成图片附件和非图片路径，从而绕过顶层的 owned 身份与整批拒绝守卫。现仅在专属 Grok 富输入打开时让外层先接收完整文件批次，再走原有守卫；该外层不发送 terminal resize。Mac arm64 真实渲染树的模拟窗口 `DragAndDropFiles` 用例 1／1 通过：焦点明确在 editor，混合批次出现整批拒绝且附件／草稿不变，纯图片附加一张，身份失配出现不可用提示且第二张未附加，PTY 写入计数零；既有 owned drop 1／1、i18n 11／11、`cargo check -p warp` 通过。安全收据 `resume-20260929/g03-event-routing/r-c4ddbd5f.safe.json` SHA-256 `66786d39135ed3a2a09fde8c947e6e0204cc4b42020644e839eeb95d62e28560`，短 TMPDIR 经进程、launchd、打开文件核验后清理。两条既有英中拒绝文案语义已复核，无需本地化变更；模拟 Event 不等于 Finder 原生手势，真实双语提示布局与 Linux／Windows 仍待验，G03 不关闭。
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

- **2026-09-29 同提交在线正例 `2383428da`**：Claude `2.1.280/claude-opus-5-5` macOS 生产适配器图片加双技能新建和同原生会话冷恢复均通过；两张不同 PNG 的原生字节和结果、每轮两个 Skill 的精确审批及执行、自然退出与清理按收据分别计证。随后同一签名二进制的 Mac GUI 新建任务也验证一张 PNG 加 alpha/beta 两技能独立审批、原生执行顺序与结果；GUI 冷恢复因同 bundle ID 的归属不明旧实例无法精确绑定而在发送前停止。证据见[验证结论](VALIDATION_REPORT.md)。这些正例不覆盖其他平台、父权限上限或这一构建的 GUI 会话内热新增／冷恢复，不关闭 G06。

- **2026-09-27 Grok 组合增量 `ba73c4df3`**：图片与技能已接同一类型化输入帧，单技能 slash 在图片前，多技能按所选顺序保留独立引用；首技能原生展开、其余原生文件读取的真实语义不改变。新增十项适配器回归，原无技能图片范围不收紧。原生两轮合同不能代替产品组合及跨平台验收，固定策略未扩权。

- **2026-09-27 代码增量 `3dbe79e58`**：固定 Claude 图片与多技能已接通，共用全部技能名称预算和类型化图片编码；原生注册完整确认、固定父权限与独立审批均保留。新增回归覆盖顺序、原字节、重投、部分注册、未知版本、注册副本改变及父集合。本地编译、i18n及定向模块门禁通过，真实组合和双语布局未验；普通 Inherit 零／单技能编码保持，固定技能的单技能图片前缀已变，须另验。
- **状态／优先级／范围**：模式限制，中；P1 技能、P4 会话继续。
- **实际情况与影响**：当前工作区已接通固定 Claude `2.1.280` 的无图多技能与会话内新增，整合后本地门禁通过；Grok 工作区已接入固定 1.0.41/grok-4.7、Inherit、私有独占 leader 的多技能及空闲热新增；固定策略禁用技能仍见 G10。新增 macOS Claude `2.1.280/Opus 5.5` 原生 PTY/stream-json 证据已证明同轮顺序调用两技能、会话中新增及 reload_plugins 注册、冷恢复读取新标记；每次 Skill 单独审批。另有实现来源 `3b4d8e8a3` 的生产适配器三轮正例：多技能、新增技能、同会话冷恢复，6 次真实 Skill 单次审批，重复消息未重复执行。实现采用准确 reload_plugins 确认、失败回滚、旧回调隔离及接收后持久化；固定文件策略仍禁技能。Claude macOS GUI 首轮 alpha/beta、同会话新增后 alpha/gamma 均逐次 AllowOnce 并返回独立标记；同原生会话和 runtime、逻辑代次 1→2，SQLite 累积三项 selected_skills。第三轮 PNG加alpha 已有原生字节/识色/Skill正例；热技能导致的重启身份误判已修，GUI重关联同宿主/原生进程、同代3和三条输入，未重投，正常断开清理通过。中文说明/技能列表/历史可读；真正身份冲突另保留终态并禁恢复，技能回执先于清单落盘，ACK或清单写入失败后的幂等重放已通过真实SQLite回归，零重复发送。跨平台未完成。Grok 默认 profile、显式 leader 的原生校准已证明同轮首技能正文展开、后技能由 read_file 完整读取，以及新增后显式 reload 才可见；同 leader 两个已信任目录均被刷新，sessionId 不提供局部作用域。早期探针零审批请求，不作父权限上限证明。后续 macOS 生产适配器 v3 已通过双技能→热新增→同原生 ID 冷恢复三轮，按实际原生展开、read_file 字节和结果分别计证；调用不保证串行。GUI 首轮双技能、v4 同会话热新增、两次应用重启同宿主重关联、新进程同原生 ID 冷恢复第三轮均通过；三条 NativeProtocol ACK、零自动重投，两代原生退出 0 并清理。未信任目录、握手顺序和热新增入口旧失败保留，未改变全局审批。macOS 中英文完整滚动区域可读；v3/v4 分别绑定源码与二进制，未变模块按摘要复用，不宣称同一二进制全量重跑。同提交云端门禁与两平台已认证模型／GUI、Grok 用户级技能来源扩展及固定权限模式仍欠验，G06 不关闭。仓外 `validation/grok-g06-20260926/index.safe.json` 摘要 `3606b77b9c1e716e1d85f4a1705213dea5170c8b4ca694ea029635482acb3710`。
- **未验证代码增量**：固定 Grok 的技能目录新增严格的 `user:<name>` 来源绑定，与既有 `local:<name>` 一起核对唯一名称、规范路径与文件摘要；初始选择、热新增和冷恢复统一接线。原生用户级目录及模型链待最终验收，固定策略仍禁技能。
- **当前替代方式**：两款按固定版本、无图和继承模式的已验证范围使用；Grok 热新增必须在空闲时等待原生确认，不能把目录刷新视为调用成功。未信任项目需通过原生项目信任流程处理，应用不自动写入信任。
- **源码／证据**：[每轮数量限制](../../app/src/ai/cli_agent_runtime/local_skills.rs#L209)、[启动登记检查](../../app/src/ai/cli_agent_runtime/task_manager_input.rs#L287)、[Grok 单技能与恢复收据](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/validation/macos-fixed-versions-20260924/grok-selected-skill-v7.safe.json)。
- **关闭条件**：验证多技能与会话中新增技能的真实接口及调用顺序；补齐路径与权限边界、准确注册确认、历史恢复和失败原子性。若原生不支持，按具体 CLI／模式记录证据，不从文件出现在目录推断技能已可调用。

### G07 — 三款普通终端的文件附件卡片

- **状态／优先级／范围**：功能缺项，高；P1 附件和文件上下文。
- **实际情况与影响**：本次输入增量把本地文件卡片转为明确路径引用：整批校验绝对路径、普通文件及当前账号可读性，以 JSON 保留中文/空格/引号边界，交由 CLI 原生读取与审批，失败保留草稿；不发送任意二进制内容，远程文件卡仍拒绝。三款 macOS 原生 PTY 手动路径投递均有读取独立标记正例，Claude 另有原生拒绝；此前 GUI 入口只插入正文路径的负例已保留，后续四文件增量已接通选择器 PendingFile 卡片并保留 CLI 锁定输入模式与旧回调校验。macOS Codex `0.156.1/gpt-6-luna` 真实 GUI 两卡片一次发送，原生 shell 实际读取中文/空格路径的两份文件并返回独立标记；英文和简体中文布局可读。中文首轮模型转向 TextEdit、审批被拒的读取失败仍保留，不冒充正例。Grok `1.0.41` 两卡片可创建，发送仍受 G01 阻止并完整保留草稿；Claude 日常配置曾有认证，但本轮独立 GUI 配置未登录，首次向导停在登录方式选择，文件卡链待补。失效文件、拒绝链和 Linux/Windows 完整验收仍欠。
- **2026-09-29 `bd4612887` Shell 模式防护**：Codex／Claude 锁定 Shell 模式与本地文件卡片组合时，`!` 前缀会把路径说明带入原生命令模式；现于写入 PTY 前拒绝并保留草稿及卡片。含中文、英文空格路径的双卡片零写入回归通过。实际 GUI 的收起入口、Claude 首次向导和三平台原生读取仍待验，G07 不关闭。
- **2026-09-29 Mac 私有 GUI 补验**：独立签名应用从收起态打开系统文件选择器，中文路径和英文空格路径各形成一张文件卡；锁定 Shell 模式的 `!printf` 提交出现拒绝提示，草稿与双卡保留。原生输入框未见新文本或输出块；同域既有单测验证 PTY 零写入，但本轮 GUI 未取得精确 PTY 写入计数。原生 Codex 为 `0.155.1`，不计固定 `0.156.1` 文件读取正例；GUI 截图仅在 CUA 回执，未归档原图。私有进程与短测试目录已精确清理，收据 `g07-gui-r-hq0ITed6/receipt.json` SHA-256 `71d082c6866d8c0e8b5a9b317939863054b946ad10da0dc30bbf6e86356b2a5b`。Claude 首次向导、Grok 真实文件卡投递与 Linux／Windows 仍待验，G07 不关闭。
- **同日固定 Codex 复验**：从官方归档私有复制签名 `0.156.1`，核二进制 SHA 与独立 `CODEX_HOME` 登录状态后，以本分支签名 Mac GUI 从收起态系统选择器生成英文空格和中文路径双卡。一次提交在同一原生回合产生两个独立 `exec_command` 读取，工具结果各含对应合成文件标记，最终回答逐行匹配；卡片与草稿清空。正例脱敏收据 `g07-fixed-codex-r-fdZbGmBG/positive.safe.json` SHA-256 `ea732f8c52e6b277b5b6a45b5c09ffae00dd3310e053545c4be828b72932b062`。同会话锁定 Shell `!printf`＋双卡被拒，草稿与卡片保留，原生 rollout 摘要、用户消息、工具调用和任务启动计数均不增加；负例收据 `negative.safe.json` SHA-256 `b3ab89b9a16ddac5f4666f6e18d2c0661e78aca6ca5ccd3072e555fdbe18aee2`，整轮索引 `index.safe.json` SHA-256 `139bf593d75de9bdf57e8340721f702689d72e77f9ab58ab2524b541e78ef95a`。私有进程、launchd 和文件占用精确核对后短目录已清理。该链不含 GUI 精确 PTY 字节数、Claude 首次向导、Grok 投递、失效文件／权限拒绝与 Linux／Windows，G07 不关闭。
- **固定 Claude 私有 GUI 首次向导**：官方签名 `2.1.280` 在独立配置中可由本分支签名 GUI 启动，首次向导到达登录方式选择；原生 `auth status` 为未登录，因此没有选择登录方式、创建文件卡或发送模型输入。日常配置大小及修改时间未变，私有进程、launchd、打开文件与短目录已核验清理；仓外安全索引 `g07-claude-r-5d36ab53/index.safe.json` SHA-256 `d95d5da8e32099fb27df77fd383965735ca34b245b00428a5cd9a4f11ecebf1f`。Claude 双文件读取与拒绝链仍须在独立测试配置登录后实测，G07 不关闭。
- **成卡后文件失效回归 `611aac0a4`**：Codex／Claude 本地双卡先成功建立，再删除第二文件；同类权限用例在成卡后将第二文件改为 mode 000。Mac `file_submission_tests` 6／6 通过，提交时整批拒绝，草稿与两卡保留，`WriteBytesToPty` 为零；`cargo check -p warp` 通过。脱敏收据 `resume-20260929/g07-dfaec474/receipt.safe.json` SHA-256 `9eaec29c252d67428dbeb0a84eea77cb321e1f4004c5b44baae56fe78a80ffbf`，测试日志 SHA-256 `5764ee7e7d6947aed36c6ff6c67643b106bb3ffc8884b04a87bd4579db09b315`；私有进程与短 TMPDIR 经记录核对后清理。仅补强既有提交前复核的时序，不改变用户可见文案，无需本地化变更。路径引用从验证到原生 CLI 实际读取仍可能被外部修改，不能把此回归计为三款 CLI 文件读取验收，G07 不关闭。
- **当前替代方式**：使用 CLI 可访问的文件路径或现有文件上下文入口，明确确认引用的文件；不承诺二进制文件由模型直接理解。
- **源码／证据**：[文件附件投递](../../app/src/terminal/view/use_agent_footer/mod.rs#L694)。
- **关闭条件**：界定支持的文件种类和投递语义，完成三款普通 PTY 的附件转换、正确路径／字节接收、空格与中文路径、失效文件和权限拒绝验收；不能只删除拒绝分支。

- **入口增量**：CLI 富输入收起时点击选择文件，现改为携带当前输入代次打开富输入，待草稿恢复事件完成后复用附件选择器，生成文件卡片。新入口不再直接插入裸路径；旧异步路径回调仍可处理。Mac GUI 该入口的固定 Codex 正例见上，Claude／Grok 及其他平台仍待补，已开展的平台自动化回归见[最新门禁](VALIDATION_REPORT.md)。

### G08 — 远程 CLI 图片传输

- **2026-09-29 回环 SSH 暂存验收**：签名 `2383428da` 产品二进制经私有 macOS 回环 sshd 的二进制 stdio proxy 上传 70 字节 PNG，远端 Verify 和独立 SSH 读取均与源图 SHA-256 一致；并发连接不能读取另一连接的未发布传输，半图断连后清理，同 host 重连可按凭据查询、旧 epoch／错误 key 被拒，显式 release 删除原图。三条 SSH 代理及私有 sshd／daemon 退出，七轮临时目录经身份和打开文件核验后清理。验收工具 `eaee5261e` 的 4 项离线测试通过；执行时工作树另含未提交产品增量，因此只绑定上述旧签名二进制和限定传输行为。不含 GUI、PTY 中的原生 CLI 图片消费或模型回合，G08 不关闭，收据见[验证结论](VALIDATION_REPORT.md)。
- **隔离认证前置核查**：固定 Claude `2.1.280` 在全新私有 `HOME`／`CLAUDE_CONFIG_DIR` 且不继承 API 环境时，原生 `auth status --json` 返回未登录；进程与短临时目录清理确认。该轮未发送模型输入，也未取得远端原生 `Read` 或历史图片字节。继续实测须先让私有测试配置完成原生登录，不借用日常配置推定认证。G08 不关闭。
- **固定 Grok 隔离认证复核**：固定 `1.0.41` 在新私有 `HOME`／`GROK_HOME`、独立 leader socket 且不继承认证环境时，单轮模型标记探针退出 1、没有标记或 `auth.json`；没有发送图片。脱敏收据 `resume-20260929/g08-isolated-grok-r-6574681e/receipt.safe.json` SHA-256 `f640ca4ad9bded5aad4ca880cb3959869f3e8e1ad847950c50f41bfdb6ba5e76`，私有进程与短目录已核验清理。离线审查未发现可在无认证条件下确定复现的远端消费缺陷；这只说明本轮认证前置不满足，G08 仍需专用登录及交互式 SSH／tmux 的原生消费验收。

- **状态／优先级／范围**：功能缺项，中；P1 附件、P5 SSH／tmux。
- **实际情况与影响**：远程图片的上传、会话绑定和三款 CLI 原生消费者均已接线，未满足来源／会话合同的输入仍拒绝；真实 SSH／tmux 投递、消费及恢复验收尚未完成。
- **未验证代码增量**：已整合分块图片 RPC、SSH 连接／终端代次绑定、引用账本、发布／释放和断连撤销。固定 Codex `thread/queue/add` 已接一次 Unknown 派发与图片快照回执，语义为后续排队；固定 daemon hook 的环境不证明发起 TUI 身份，目前额外限定同 CODEX_HOME 唯一前台客户端、完整分页唯一 loaded thread，现已另接每窗格独立 app-server/显式 Unix socket/当前 TUI 一次票据、内核身份、GUI入口及查询恢复，供多 pane 独立绑定；原默认路径仍保留唯一性约束。Grok 远端专属 ticket、真实 PTY wrapper、持久状态查询、同连接撤销、类型化图片服务、当前 SSH/tmux pane 双语菜单、GUI 发送及应用重启关联已整合。Claude 固定 2.1.280 已接正式插件进程/历史候选、TTY/内核peer/映像绑定、上传原图后原生 next 消息触发 Read；写出文本不算图片消费，只有同请求后代的 Read tool_use 与最终历史 typed image 原字节匹配才清理，Unknown 持久保留且只查不重发。Claude 合计原图32MiB、单图20MiB、最多20图，原生图片重编码或历史替换的未确认引用仍保留。三款 CLI 的远端原生图片消费仍未实测，不以可靠拒绝或上述暂存正例代替完成。
- **恢复补接**：`5aac3f7d0` 增加同会话 Claude 原生事件后的有限只读补查，并在 Grok 确认未进入 Submit 时保存精确未派发终态，允许用户主动重试；未知 RPC 不重投、不清新草稿。新增十项回归本地通过，英中提示同步；真实远程延迟审批／回收及布局待验，G08 不关闭。
- **当前替代方式**：用户先将图片放到目标环境，再使用该 CLI 已验证的远程文件路径读取方式；本机附件路径不能直接当远程路径。
- **源码／证据**：[远程图片门禁](../../app/src/terminal/view/use_agent_footer/mod.rs#L972)、[SSH 现有覆盖](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md#普通终端ssh-与-tmux-原生通知)。
- **关闭条件**：实现明确目标主机与会话绑定的图片传输、引用和清理，在实际 SSH／tmux 中证明远端收到原始内容；覆盖断连、重连、权限拒绝、重复投递及旧会话回调。

### G09 — 包管理器安装的自动升级

- **2026-09-28 Mac Claude cask后端恢复 `ae37daf19`**：修复非Committed恢复先回退入口、后发现目录外改而断链；回滚写前检查新树/旧backup和公共链接，三项回归通过。本地Python15/i18n11/更新207及check/build通过，新worker和supervisor执行固定`2.1.278→2.1.280`四场景均独立通过：正常更新、跨PID冷恢复、外部改动保留、候选改动启动前拒绝；三次真实候选清理、一项零generation分别计证。官方Ruby字段派生且明确标记的元数据与人工私有登记不等于真实消费者来源或brew安装验收；实时渠道、GUI、忙碌/插件、其他平台及Windows正式npm失败仍欠。205份小原件和完整APFS映像保留并正常卸载；无需本地化变更。同提交CI36403642011的Linux/Windows定向门禁均通过；Linux新brew三项逐项通过，Windows本轮不编译/运行这些Unix测试。本项及14项必要缺口不关闭。

- **状态／优先级／范围**：功能缺项，高；CLI_AUTOUPDATE、P0 来源识别、P5 平台维护。
- **实际情况与影响**：提交 `b6f93f6627738fb83b6be776dd225666a900ac90` 已把 Codex/Claude 的 npm 管理器包清单、CLI 包登记、真实命令入口和安装前缀进行绑定，拒绝同名伪入口；该来源识别提交的 npm 执行为 `ManualOnly`。后续工作区已接入 Claude 目标 `2.1.280` 的 Unix 官方单包事务：完整归档/成员核验、原子目录交换、私有无网络版本探针、POSIX 权限及中断 journal；不执行 npm install 或 lifecycle script。本地门禁和英中错误布局已过；macOS 官方包私有安装的真实更新、交换后冷恢复、外部改动保留、候选改动拒绝及未审核降级拒绝五场景通过，原失败保留。同提交 Linux/Windows 待验；不是 GUI 更新或模型生命周期验收。Codex Node/launcher 闭包、Windows npm、musl、额外 ACL、合法降级、Homebrew 和 WinGet 均未完成；此增量不关闭包管理器自动升级。
- **未验证代码增量**：Homebrew Claude 固定 cask、WinGet portable 和 Codex Homebrew macOS ARM64 的来源绑定、候选探针、发布及恢复已写入；Codex 三种 shell 补全纳入交换和回滚。Codex npm macOS ARM64／Linux x64 已补官方 wrapper、完整平台资源、实际 Node 公共入口、依赖快照及恢复身份绑定；Homebrew Node 使用私有 dylib 副本，不改原安装，Linux 探针要求 Landlock ABI 3。Windows x64 Codex npm 的 cmd/PowerShell → Node → 完整平台包、AppContainer 双探针、两步无覆盖发布及恢复已合入；Claude Windows npm 的精确包内硬链接和双入口探针已接；Grok 三平台 npm 的完整三包/原生解压、包目录与用户 bin 多位置事务，以及 Mac ARM Homebrew 双别名/三补全也已接。Codex WinGet完整目录/登记/依赖事务也已接入；Claude Unix npm 2.1.280→2.1.278受限主动降级合同也已接，但要求显式Stable且官方实时指针恰为2.1.278；当前2.1.274仍拒绝，不算当前渠道可降级。三款 Linux x64 Homebrew cask 和 Grok WinGet 固定来源事务现也已接入，Linux 来源发现与执行总入口已接；Claude 三探针、Codex 完整 musl 包及补全、Grok 双别名及补全分别绑定。macOS Intel 已按用户明确范围排除。用户安装未运行升级；实际公共入口、隔离、回滚与恢复仍待验，未知版本门禁保留。
- **本批真实验收入口**：`39941b281` 接入 Codex/Grok 私有 npm 四场景入口；Mac ARM Grok 正常升级及后续 `2b59c8c62` Codex 正常升级已按各自源码独立复核；`39941b281` 的 Linux Grok 四场景也已通过并核验清理／冷恢复，不外推其他平台或来源。旧 Codex/Grok 失败和清理未知原件保留；旧 CI 的 Linux glibc ELF 失败与 ABI1 能力事实分开，Windows 一项 `LEAK` 仍待明确。详细正常结果与旧失败统一见[验证结论](VALIDATION_REPORT.md)。
- **隔离前置增量**：`2b59c8c62` 在生成 Linux Codex npm／三款 Linux Homebrew 版本变更计划前核验 Landlock ABI≥3，并同步英中不可用原因；原 worker 门禁不变。Grok npm、其他来源、Mac／Windows与同版本无候选执行不误限。本地检查通过；相关平台同提交 CI、实际双语布局和真实来源事务仍待补，不关闭 G09。
- **当前解析与入口增量**：`56f6da217` 修正 Linux 官方 Node 大 ELF 字符串表误拒，并补 Windows Codex npm 真实登记、双入口及五场景实测入口；本地门禁及同提交两平台编译／定向 Rust 通过，Linux 原文件依赖闭包通过；Windows 私有 npm 登记失败，产品事务尚未执行，G09 不关闭。无需本地化变更。
- **真实 npm 模板补接**：`97649deb2` 修运行器的 Windows 路径表示；`bd80cc9ed` 精确支持 cmd-shim 6.0.1 与既有 8 的整组三入口，原三 SHA 固定身份贯穿候选和恢复，不接受未知或事务内切换模板。本地九项新增合同回归通过；同提交 Windows 真实事务仍待验，不关闭 G09。
- **Windows 控制台依赖补接（`43c9709d2`）**：精确 `conhost.exe` 租约、Job／AppContainer／零 capability 和真实退出句柄已接，同提交 Windows 编译、978项主回归、5项 command 和7项原生边界回归通过，原 `unbound.exe` 拒绝及清理反例通过。真实 Codex npm 首个 CMD 候选仍在 `Prepared` 返回 `RecoveryRequired`，CMD 报 verbatim 工作目录不兼容及拒绝访问，清理未确认、缺 AppContainer 清理收据；PowerShell／发布／恢复未执行。后续须保留目录身份门禁核对 cwd 规范化，并另查清理失败；当前不外推唯一根因、不改旧失败。本地 check、i18n 重试02（11项）和 actionlint 通过，首次空间不足原件保留；按用户要求修复／完整联测后置，G09 不关闭，见[验证结论](VALIDATION_REPORT.md)。

- **Windows npm 工作目录后续增量**：已在 `05b0b8faa` 提交（基线 `6408131e2`），将已核 canonical／卷号／FileID 等价的 DOS cwd 仅传 CreateProcessW，保持 canonical ACL／readonly 门禁。新增路径边界回归和无敏感数据的阶段诊断；本地 check、i18n 11项、actionlint及差异检查通过，同提交 Windows [CI 36280073722](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36280073722) 已结束：主组981项、command7项、显式atomic7项通过（含新增5项），真实更新首个CMD候选仍失败。旧UNC提示消失，但原生“拒绝访问”后缺完整清理收据；剩余进程和拒绝对象尚未定位，PowerShell、发布与恢复未执行。旧 `stop_requested` 来自上层300秒 stdout 超时，退出码是外层worker；清理卡点仍未证明，不闭合G09。

- **Windows npm 协作取消增量**：`0525c9439` 的协作取消已接入原认证通道，本地check、i18n 11项及control6项通过；同提交 [Windows CI 36282335500](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36282335500) 编译通过，普通回归986项通过/1项失败，失败为Drop的2秒回收等待（nextest既有3次自动尝试）；新增原生取消2项均通过、atomic9项和command7项通过。真实npm首updated返回Network、未形成journal/generation或候选执行/清理收据，不能继承上一轮cleanup=false，发布/恢复未验。后续修正 `49ea40561` 已提交推送，监听改为非阻塞读取及可唤醒等待，保留6项契约和2秒断言；独立审查、本地02 check、i18n 11项、control6项及actionlint通过。同提交 [Windows CI 36283908796](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36283908796) 已结束：编译通过，主组987项、control6项、command7项及原生atomic9项通过；Drop首次0.112秒通过，原生取消2项也首次通过。真实npm首updated仍返回ProbeFailed，原生拒绝访问后因取消中断；本轮已取得Job／ACL／profile清理标记及cleanup_confirmed=true。无原生根退出码，外层worker码1不代替它；PowerShell／发布及后续恢复场景未验。无需本地化变更，G09及Goal保持开放。

- **Windows npm 诊断增量 `ae4279e48`**：固定下载角色与失败阶段、已核验 CREATE/EXIT 角色和清理模式已接入，授权、隔离及失败判定不变。本地 check、i18n 11项及 actionlint 通过；同提交 [Windows CI 36285301933](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36285301933) 已结束：Windows 编译、主组990项、command7项、原生atomic9项通过，真实 npm 首个 updated 返回 ProbeFailed。正常阶段仅记录 root／console CREATE；取消清理阶段记录 root／console EXIT1，另有尚未分类的 unknown CREATE／EXIT3221225738，不能据此认定 Node 从未启动或定位拒绝对象。最终取得 cleanup_confirmed=true 及 AppContainer 清理标记；PowerShell、发布与独立恢复未验。CREATE/EXIT 与 root_exit 使用同一 session 时钟，probe 阶段仍用自身起点，不直接相减。按用户代码优先、修复与联测后置要求封存本轮，不自动重试或追加猜测性诊断；G09和Goal保持开放。无需本地化变更。
- **空闲通知补接**：`debed8c51` 将升级器的会话模型观察对齐三支持平台，避免 Linux／Windows 恢复清理后仍沿用旧 busy 状态；本地门禁通过，目标平台真实更新链待验。`c7ef8d95b` 仅修跨 Python 版本的验收夹具，不改生产校验。
- **2026-09-27 `e2eba8596` 原生诊断**：同提交 Windows 编译及504项回归通过，atomic为11通过／1失败；TITLE与官方GOTO/NUL/TITLE顺序均启动绑定CMD子映像、退出0、清理确认，原NUL输出对照仍为空且失败。无重试、未跑真实npm更新，不能把此NUL失败当作旧Node/Codex链挂起的唯一根因；下一步需核取消阶段unknown子进程真实身份，隔离和官方shim保持不变。
- **2026-09-27 `b3908488c` 真实 npm 复验**：新增清理CREATE租约身份及首次终止前创建时序诊断，Windows编译、995项普通回归、command7项通过；显式原生11通过／原NUL对照1失败。真实首个CMD候选约241秒取消后清理确认，仍为ProbeFailed；本轮正常和清理只见root／console，没有重现旧unknown CREATE，不能给旧事件补造Node身份。PowerShell／发布／恢复未到达，G09不关闭；继续比较完整shim选路与stdin EOF/open，保留隔离和官方入口。
- **2026-09-27 `a0e6662b2` 选路／stdin 对照**：四组完整shim控制流均成功执行绑定CMD副本，显式runtime路径与裸node PATH在EOF／开放stdin下均退出0并清理确认。Windows504项普通回归及原生15项通过，原NUL对照仍失败；未执行真实Node或重跑npm。两变量单独不足以复现挂起，继续固定Node与真实stdio布置对照，不能因CMD副本成功关闭G09。
- **2026-09-27 `4c4b73c1d` 固定Node标准流对照**：Windows编译及504项普通回归通过；原生16通过／2失败（原NUL和新增Node）。绑定CMD在pipe／pipe／disk下退出0；固定Node20.9.0正常CREATE后自然退出`0xc0000142`，无版本输出、无取消，shim标记及AppContainer／Job清理确认。只证明本对照中的初始化失败，不能等同真实npm超时根因；继续有界loader观察与同一Node根／子进程对照，保留全部成功断言和隔离，G09不关闭。
- **2026-09-27 `70c172560` loader／根进程对照**：Windows编译、普通504项与loader3项通过，原生16通过／3失败。固定Node根／子均到达首断点后退出`0xc0000142`，shim不是必要条件；三组清理确认且无诊断丢弃。conhost首机会异常在成功CMD基线同样出现，不能定为根因。下一步仅测试隐藏独立控制台，保持生产默认和隔离，G09不关闭。
- **2026-09-27 `e3f44c29c` 隐藏控制台对照**：Windows编译、普通504项、command11项和loader3项通过；原生16通过／6失败。新增三组均在根进程初始断点前自然退出`0xc0000142`，CMD launcher尚未执行shim或创建Node；旧NoWindow的CMD基线仍成功，Node根／子则在初始断点后失败，两种阶段不得混同。六组Job／AppContainer清理确认、无取消或日志丢弃。runner收尾另清理一个conhost（PID2004），现有日志不能归属至具体对照，不能据六组收据宣称机器级无残留。隐藏模式不能接入生产；下一步先只读核对runner窗口站／桌面与隔离令牌访问边界，不修改全局ACL或弱化隔离。真实npm未重跑，G09仍开放。
- **2026-09-28 `5ea2614e0` 环境实测**：Windows编译、普通504项、command11项、loader3项及新增环境4项通过；原生仍16通过／6失败，不重试。六组14次观察一致：runner为Session0／高完整性／已提升、不可见非WinSta0窗口站；候选为同会话低完整性AppContainer，Win32k禁用策略为0、严格句柄策略为3。目标线程桌面查询无有效值且hresult为0，不能记作已证实拒绝访问；未评估DACL或完整运行时访问。失败阶段和六组清理结果与上轮一致，runner收尾本轮仅记录vctip，不能回填旧conhost归属。下一步只读模拟实际token对runner窗口站／桌面描述符的访问；真实npm未重跑，G09不关闭。
- **2026-09-28 `f7e0801f1` DACL实测**：Windows编译、普通504项、command11项和诊断10项通过；原生仍16通过／6失败。六组14次实际token观察的窗口站／桌面固定掩码及MAXIMUM_ALLOWED共56次模拟全部拒绝，成功CMD基线也相同；仅检查runner当前对象，MIC未评估，不能据此确定候选实际目标或Node根因。目标线程桌面仍为query_failed（hresult=0），不是已证实拒绝。NoWindow Node根／子首断点后失败，隐藏三组root首断点前仅3条DLL即退出0xc0000142；旧NUL失败保留。六组内外层清理确认，runner另清理的conhost PID31528无法归属。真实npm未重跑；私有窗口站／桌面对照仅为计划，G09及14项必要缺项保持开放。
- **2026-09-29 受限令牌 NUL 诊断 `97a647351`**：同提交 Windows [run36498757238](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36498757238) 编译通过，私有 warp libtest 辅助映像经 SHA／大小绑定并进入零 capability AppContainer／严格 Job，却在 `CreateFileW` 调用前以 `0xc0000142` 退出；`helper_report_present=false`，没有普通文件或 NUL 的实际令牌结果。Job／profile 清理及收据匹配均为 true。外层 nextest 的既有 60 秒限额将该失败记为 TIMEOUT，额外标准流工件上传因本轮无输出文件而失败；不能将其解释为 NUL 拒绝。完整 Windows 日志和两个成功上传的官方工件归档，日志清单 SHA-256 `df8d021420a14d971e44a3e7be090821628490c45f0f20e515d1c8962bf5d30f`。后续提交 `c31d61fa7` 仅对这项大型诊断设置 150 秒 nextest 预算，并新增测试专用现有非交互窗口站私有桌面对照；真实原生结果见下项同提交运行，生产隔离默认路径不变。G09 不关闭。
- **2026-09-29 私有桌面实测 `c31d61fa7`**：同提交 Windows [run36499906695](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36499906695) 编译及前置 508／24／1／14 项检查通过，两个显式原生诊断均失败且零重试。大 helper 仍于 `CreateFileW` 前退出 `0xc0000142`，现记为 FAIL 而非 TIMEOUT，实际 NUL 访问仍无收据。固定 Node `20.9.0` 的现有非交互窗口站私有 desktop 已创建并读回核验，持有对象关闭和 Job／profile 清理确认；Node 仍以 `0xc0000142` 退出、无版本输出，故单独私有 desktop 不足以修复初始化。未修改现有窗口站 ACL 或生产默认隔离，真实 npm 发布／恢复未运行。三份官方工件及完整日志归档，清单 SHA-256 `bdd5d3509e7b37c7e67eb5433bedfb92b9931eda61532a981899225949508544`；G09 不关闭。
- **2026-09-29 小型 CreateFile 夹具首次运行 `10f99d6df`**：同提交 Windows [run36501584604](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36501584604) 中，仓库内 Kernel32-only C 源由 runner 的 MSVC 编译，普通 runner 令牌基线实际运行并验证固定 120 字节收据；随后 `warp` libtest 的新解析闭包以 E0277（`Vec<u32>` 被 `i32` 索引）编译失败，实际 AppContainer 进程没有启动。Agent 生命周期与 Grok 安装步骤亦因同一编译错误失败，Grok 工件未生成；不能从这轮推断受限 NUL 访问。完整日志与唯一成功上传的官方工件已归档，日志清单 SHA-256 `1b84e9652376a3c5270be33f57b195cc1737100b51137a0bd2006bdced922fa7`。后续 `2d3297089` 修正了两个 `usize` 调用索引，首次运行的失败结论保留。
- **2026-09-29 实际受限令牌 NUL 结果 `2d3297089`**：同提交 [run36502343517](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36502343517) Windows job success，前置主回归 508／508、command 24／24、未命名站 1／1、环境 14／14、原生对照 1／1，未见重试。小型 C 夹具由本提交源编译并通过普通 runner 令牌基线；私有复制映像在零 capability AppContainer／严格 Job 内正常退出 0，120 字节收据有效。`GENERIC_WRITE` 与 `FILE_GENERIC_WRITE` 下同目录普通文件均打开为磁盘文件并正常关闭；`NUL` 与 `\\.\NUL` 均未打开，立即 `GetLastError` 为 5（访问被拒）。Job／profile 清理和匹配收据确认。完整日志与两份官方工件已归档，清单 SHA-256 `d6e57a4c32313b37cf913407ce328194c5b9fe251df807f59ed304e4810890f8`。这直接定位本轮受限令牌的按名 NUL 访问失败；既有 CMD 控制流正例与本轮受限调用不同，不能直接断言官方 shim 必因该拒绝停止，也不能解释固定 Node 的 `0xc0000142`。真实 npm 发布／恢复仍未完成；G09 不关闭。
- **当前替代方式**：使用原安装包管理器手动升级，或由用户明确选择官方原生安装；不得自动迁移或猜测同名命令的所属安装。
- **源码／证据**：[npm 手动计划](../../app/src/terminal/cli_agent_updates/sources.rs#L1330)、[Homebrew 手动计划](../../app/src/terminal/cli_agent_updates/sources.rs#L1438)、[WinGet 未识别边界](../../app/src/terminal/cli_agent_updates/sources.rs#L1105)、[自动升级记录](CLI_AUTOUPDATE.md)。
- **关闭条件**：按包管理器分别完成来源／入口／依赖身份绑定、渠道解析、忙碌延期、实际升级与降级、失败回滚和应用重启后的中断恢复；验证不修改其他前缀或用户安装，并覆盖对应平台真实事务。
- Mac ARM Codex npm 补验：`e3a2e5654` 的 run-01 候选改写在启动前拒绝；`bb78f3ac5` 的 run-03 两项冷恢复通过：缺收据后实际 inspect 为旧版 0.155.1 且完整树回滚，外部改动分支返回 RecoveryRequired 并保留 marker、旧备份和 journal。两项交换前候选 Node→codex.js 探针均原生 exit 0、cleanup true；external 的结果版本 0.156.1 为固定字段，非恢复后再次执行。历史正常升级仍单独属于 `2b59c8c6`，未形成同 SHA 四场景验收。证据：`macos-codex-npm-recovery/run-01-review.safe.json`、`run-03/summary.safe.json`、`run-03-index.safe.json`。G09 保持未关闭，14 项必要缺口及消费者发现/GUI/模型/其它平台边界不变。
- **Mac 同冻结产物追加回归 run-04**：复用bb78的worker和签名监督程序，在干净14fb检出下补正常更新与候选篡改拒绝，19份相关源码均与bb78字节相同。公开入口实际分别为0.156.1/0.155.1，正常完整新树与候选清理通过，篡改场景保留完整旧树且无候选启动；独立复核通过。与run-03共同形成同冻结产物的四场景分批证据，不外推14fb重建或整SHA全验收，G09仍开放。
- **Windows 私有对象对照 `14fb66d99`**：同提交 [CI 36342378730](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36342378730) 编译、504项普通回归、command21项及诊断13项通过；原生16通过/7失败，不重试。新增对照在创建窗口站时被拒（0x80070005），尚未验证对象或启动Node，私有/AppContainer清理未获完整收据，外层Job清理确认；原六项失败保持，六组已有对象对照清理确认。runner另清理conhost PID22732但归属未知。下一步只读核验失败调用线程的有效管理员成员前提，不改全局ACL或提权；真实npm未重跑，G09及14项必要缺口保持开放。无需本地化变更。

- **Windows 命名站前提实测 `eb2b271e3`**：同提交CI36344378073编译及504/22/14项通过，原生仍16通过/7失败。创建调用线程的CheckTokenMembership(NULL, BuiltinAdministrators)成功返回member=false，随后CreateWindowStation仍0x80070005；未建立私有环境或执行Node，内层清理未知、外层Job清理确认。这只确认该对照的必要成员前提未满足，不解释原Node DLL失败。等待独立管理员验收runner信息，当前不提权/改全局权限；继续Mac独立工作，G09仍开放。

- **Windows NUL实际令牌DACL增量 `11f278091`**：[CI36389274428](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36389274428)编译及504/22/14项通过，原生仍1通过/2失败。7条CREATE观察中，同NUL SD的READ/WRITE/MAX模拟为bound 21拒绝、driver 21允许、无查询失败；成功CMD也在拒绝组，不能认定Node/npm唯一根因。NUL仍空文件，Node记录29条DLL并到初始断点后退出0xc0000142，内外清理确认；MIC及真实CreateFile未观测。上游MXC/libuv的设备ACL准备未执行，现有隔离不放宽；普通交互待用户、NULL+CWF_CREATE_ONLY仅方案，RDP未启、14项缺口保留。该诊断轮Mac Claude cask当时尚未执行，后续四场景结果见本项ae37daf19增量。主源、索引与独立审查见验证报告，无需本地化变更。

- **Windows 服务stdio三项复验 `730b63b6a`**：修正stdout保留名为redirected.txt后，[CI36384245535](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36384245535)编译及504/22/14项通过，原生1通过/2失败；CMD通过，NUL仍为空文件，固定Node根仍0xc0000142，清理确认不等于功能通过。实际runner trace为Session0/high/TokenIsElevated=1，不能称未提升普通交互用户；也不据此推断管理员成员。RDP未启用、回环选择读回All，普通交互会话仍待用户确认。下一步仅测试的实际令牌NUL DACL观察尚无运行结论，不改ACL/capability或官方shim；历史原件、G09及14项必要缺口保留，详见验证报告，无需本地化变更。

- **Windows 管理员对照身份已解决（2026-09-28）**：用户授权后临时以Administrator复用原runner领取一次同提交61d0747b4的[CI36379989244](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36379989244)。编译及504/22/14项通过，原生17通过/6失败；实际创建线程member=true，私有窗口站/桌面核验通过，固定Node真实输出v20.9.0、原生退出0，内外Job/AppContainer和自有对象句柄清理确认。原NUL及五个普通/隐藏对照失败保持；同批普通对象失败而私有对象成功不等于普通用户产品或真实npm修复。原低权限服务已恢复，.runner文件与五个目录根权限保持，缓存/工作目录读写已核验；无需继续等待管理员凭据信息。后续定位普通身份实现与正式npm链，G09及14项必要缺口仍开放，详见验证报告。

- **Mac Grok 486330824增量**：外置卷显式策略及17份worker源码绑定完成，Python41、check、i18n11通过。新worker与bb78监督程序15份等价源码组合的run-01首个正常更新被SanDisk卷根0775祖先门禁拒绝；Grok候选未exec，子worker退出1及Job/coalition清理确认。单独run-02候选篡改拒绝通过，实际公开入口仍1.0.40、完整旧树/用户镜像/配置保留，候选未启动。原失败保留；正常更新与两项冷恢复仍欠验，待独立私有映像环境授权，不改原卷权限或放宽门禁。不是当前SHA整包四场景通过，G09仍开放；无需本地化变更。

- **Mac Grok 私有APFS补验 run-03**：复用冻结worker与bb78监督程序，在318c干净检出的17/15关联源码等价边界下，正常更新、缺失完成记录冷回滚、外部变更保留、候选篡改拒绝四项同批通过，实际公开版本依次1.0.41/1.0.40/1.0.41/1.0.40，独立复核全部通过。新卷0700、原SanDisk0775保持，noowners未改，不声称原布局修复或多用户隔离。172份收据/日志及完整映像已留存，普通卸载确认；本轮cargo check通过，无需本地化变更。原失败保留，G09/14项必要缺口仍开放。

- **最新实测**：`bd80cc9ed` 的 Windows 真实 npm 登记与固定三入口模板通过；首个 CMD 候选探针因未绑定子映像被拒，`cleanup_confirmed:false`，尚未进入 PowerShell／发布／恢复。Linux 与 Windows 编译及定向回归结果见[最新门禁](VALIDATION_REPORT.md)，不据此关闭 G09。后续 `ca37b9cbf` 已补脱敏诊断及失败清理证明，本地门禁通过；Windows 在事件码字段类型编译错误处失败，原生场景未运行，字段在 `3dbe79e58` 修正且编译通过。新日志定位System32 conhost，实际更新仍拒绝；本轮清理确认成功，旧未知不回填。`2383428da` 的 [Windows CI](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36453314472) 原子进程专项 3 项中 2 项仍失败：NUL 重定向输出为空但前后普通命令成功，固定 Node 根进程在首断点后以 `0xc0000142` 退出，均确认严格 Job 清理；未命名窗口站探针 `created=false`，未建立对象。现有 NUL DACL 仿真不能证明真实 CreateFile 或 Node 根因，不能据此改 ACL 或隔离；真实 npm 仍未通过，G09 保持开放。
- **当前分支诊断对照**：在上述同一 CMD／目录／令牌的 NUL 失败用例前增加普通文件 stderr 重定向正对照，并逐项核对输出、错误流与独立文件身份；原 NUL 失败断言保留。同提交 [Windows 专项 36497300508](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36497300508) 中，普通 CMD 正例、普通文件 stderr 和四个普通文件的独立 FileID 均通过；`2>NUL` 的输出仍为空且脚本错误级别为 0，固定 Node `20.9.0` 根进程在初始断点后自然退出 `0xc0000142`、无版本行。三项 1 通过／2 失败，严格 Job／AppContainer 清理确认；两项失败不能据此归为同一根因，真实 npm 事务仍未通过。G09 继续开放，本增量无需本地化变更。

### G10 — Claude／Grok 子任务限固定权限策略

- **2026-09-29 V2 Write 待审批取消正例**：固定 Claude `2.1.280/claude-opus-5-5` 的 Mac 生产父子链在父 `run_agents` 单次允许后，子原生精确 `Write` 停在待审批且目标文件不存在；子回合 `Interrupt` ACK 后审批撤销，同一审批 ID 的迟到 `AllowOnce` 被拒，目标始终未创建。持久父子身份及权限上限核对通过，父子原生进程分别退出，Job／资源域清理均确认。最初从外置盘启动的监督 worker 在 dyld 装载阶段超过握手预算，后续从外置盘启动的 Claude CLI 也出现初始化超时；将同 SHA、签名和固定版本的测试程序、worker、CLI 放到内置私有测试盘后，本链独立通过。原失败及无模型正对照保留，不因环境对照修改产品超时。此正例仅关闭 V2 待审批取消这一子项；G10 其余工具／技能权限、GUI、冷恢复及 Linux／Windows 仍未完整在线验收。

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
- **实际情况与影响**：当前固定版本三款链主要覆盖 Mac 到本机隔离 OpenSSH，并包含 tmux 断连重接；不覆盖远端 Linux、Windows、WSL 等组合。旧 Codex 关闭 tmux 透传补测仅观察到产品接收 0，缺少内层发送证据。2026-09-29 独立本机回环补测在固定 `0.156.1` 的原生 `SessionStart`／`UserPromptSubmit` hook、私有 tmux `3.7c` 且 `allow-passthrough=off` 下，确认 pane 内两条通知与外层 SSH 输出零通知；该双端负例补齐这一窄范围，不代表远端组合、产品 UI 的双向交互或断连取消验收。
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
