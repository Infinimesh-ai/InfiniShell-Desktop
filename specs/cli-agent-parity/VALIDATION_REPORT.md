# CLI 支持与能力对齐：验证结论

本报告保留截至 2026-09-25 的固定版本验收结论、源码来源和未验边界，不保存逐轮日志或截图。**这是阶段交付，完整 Goal 尚未完成。** 当前状态以 [CURRENT_STATUS](CURRENT_STATUS.json) 为准；剩余功能、优先级和关闭条件统一维护在 [KNOWN_GAPS](KNOWN_GAPS.md)。

目录整理本身没有修改产品或重跑历史测试；此后继续实现的输入增量及新证据单列如下。历史通过、失败与跳过仍按原提交计证；新增工作区收据也不能外推到最终 SHA、其他版本或全部平台。

## 验收基线与追溯

| 项目 | 固定来源与解释 |
| --- | --- |
| CLI 版本 | Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41` |
| 历史产品行为冻结 | `ee839b4fc7dacd58bac0806aa549500af5776b30`；包含桌面 Grok 默认开放、启动取消修复和双语说明修正 |
| 历史最新相关验证提交 | `50e1515bcd3f20cf120edf5565766975b6a68d5d`；只完善测试与运行器，未再次修改产品行为 |
| Mac 模型与 GUI 实链 | 来自 V2–V13 等实际构建批次及源码摘要；后续按变化域复验，不能重标为 `ee839b4fc` 或 `50e1515bc` 全量重跑 |
| 历史档案固定提交 | `e3ef39689cd8b686dfe040b90217ed1067b4d268`；保存原过程专报、失败、通过、截图与摘要索引 |

原始材料从当前目录移出后，仍可从[固定历史目录][history]追溯，或使用 `git show e3ef39689cd8b686dfe040b90217ed1067b4d268:specs/cli-agent-parity/<历史路径>` 读取。历史总体“全部完成”判断已被当前状态撤回；保留原文件不表示继续认可其总体判断。

关键来源是[固定版本验收表][acceptance]、[Mac 交付记录][mac]和[同提交补验报告][final]。这些固定提交链接用于复核旧结论，不承担当前待办管理。

## 2026-09-25 继续实现增量

实现来源标记为**本次输入增量（提交绑定待补）**。G02 接通固定 Grok PNG ACP 图片；G04/G05 扩展 Claude 静态格式、纯图与精确注册单技能的图片编码；G07 接通普通终端本地文件卡片到明确路径引用。G09 的 Codex/Claude npm 管理器/包清单/前缀/真实入口绑定已提交为 `b6f93f6627738fb83b6be776dd225666a900ac90`，但 npm 更新仍为 `ManualOnly`，尚未完成真实升级、降级及恢复事务。以下均不关闭 Gap。

### 图片生产适配器与原生合同

| CLI、版本、模型、平台与模式 | 本次实际结果 | 证据边界 |
| --- | --- | --- |
| Claude `2.1.280`、`claude-opus-5-5`、macOS arm64、Inherit 生产适配器 | JPEG、WebP、静态 GIF 分别新建识别随机象限色；原生 user 回放的 MIME、字节和数组摘要与持久图片一致；关闭后同原生 ID 冷恢复正确回忆，均自然退出并清理。每用例 2 次模型输入 | WIP 二进制 `6e3e13f07aef46612c90c35b0c67c84163313f00f5e594701746f1f06215d284`；不是 GUI、应用重启或跨平台链 |
| 同上，纯 PNG | 先文字定义后续图片回答格式，再提交没有文字块的纯图片；实际识色、字节回放、持久引用恢复及冷恢复回忆通过，共 3 次模型输入 | 不外推所有纯图片格式；不是图片加技能的生产验收 |
| Grok `1.0.41`、`grok-4.7`、macOS arm64、Inherit、无已选技能的真实生产 connect | 新建识别红/蓝 PNG，关闭后同原生 ID 冷恢复，提交独立绿/黄 PNG 并正确识别。真实最终历史 user_message_chunk 的图片字节、MIME、typed content 摘要与持久输入一致，promptIndex 为 0/1，代次和回合准确关联 | WIP 二进制 `ff821a38c0260c35e9a3210f39090156c6814d2b1327499bb8992669b2b3f8b0`，未用 candidate flag；两代各重发同一消息 ID 共 3 次，仍各仅执行 1 个原生回合。两次退出均 `stdio_closed`、exit 0 且 cleanup_confirmed；不是 GUI/应用重启或其他策略/平台 |
| Claude `2.1.280`、`claude-opus-5-5`、macOS arm64、直接原生 stream-json 图片加单技能 | v5 新建与冷恢复各有精确注册命令的真实 Skill tool_use、单次审批及 `SKILL_OK LEFT=RED RIGHT=BLUE` 正例 | 原生合同，不是完整生产适配器/GUI链。裸 slash 数组未触发技能的失败和 v4 探针“禁止工具”指令冲突导致的失败原字节保留 |

两款生产用例使用同一内置盘监督者，摘要 `a8aae1653910a4625c879caa62f730e5131207eb08bfb68877334f4eb3b1d4b8`。收据明确记录基线 HEAD、工作区脏状态、关键源文件和运行器摘要及运行期间源码不变；**HEAD 只是基线，不能把 WIP 二进制记为该提交已验证**。其后又补严 Grok 已选技能图片门禁，以及仅 Claude 的静态 GIF 首次/恢复校验；上述原始收据没有覆盖这两项后续修正，不回填为修正后通过。最终重建与受影响模块回归已通过（见下节），在线静态 GIF 窄复验与提交绑定待补。 Grok Python 运行器后来增加 Linux 临时目录回退，但没有本次 Linux 在线结果；Windows 运行器明确拒绝执行，认证副本 ACL 与在线验收另待实现验证。Rust ignored 测试可跨平台编译不等于 Windows 产品链可用。

### 普通终端、技能与独立图片判断

本次普通终端探针均为 macOS 手动驱动的原生 PTY，产品源码改动未在该探针执行：Codex `0.156.1/gpt-6-luna`、Claude `2.1.280/Opus 5.5`、Grok `1.0.41/grok-4.7`。

- G07：三款分别通过原生 exec/read 工具读取中文与空格路径的独立标记；Claude 第二文件的 No 拒绝未读取。它证明路径引用合同，不证明修改后的文件卡片 GUI 链、拒绝后草稿恢复或 Linux/Windows；Grok 产品仍受 G01 自动发送守卫阻止。
- G01：新空会话超过 66 秒没有 idle_prompt；help 模态、未提交中文草稿及后台 sleep 仍活跃时却可能出现 idle_prompt。审批等待 76.7 秒只有 permission_prompt。该通知不能作为 Enter 安全或编辑器为空的证明，守卫继续保留。
- G03：Ctrl+V 后 bracketed text 粘贴造成同一回合两张图片，负例保留；Ctrl+V 后普通 UTF-8 两行配合 Alt+Enter 得到恰好一段文字和一张图片，并正确识别红蓝。无剪贴板消费 ACK 与可靠输入就绪，尚未接通自动链。
- G06：Claude `2.1.280/Opus 5.5` 的原生 PTY 与 stream-json 取得同轮两技能依次审批/调用、运行中新增技能、reload_plugins 返回准确命令列表及冷恢复读取新技能标记的正例。只限无副作用技能原生合同；产品实现由后续增量处理，注册事务、并发、父权限上限、Grok 多技能及两平台均未计通过。
- V04：Codex `0.156.1` 独立原生 exec 的 `gpt-6-luna` 位置错误保留；同图 `gpt-6-sol` 返回 `YELLOW BLUE RED LIME`，独立目视确认 LIME 对应亮绿且顺序正确。原收据 strict_match=false 不修改，语义判断单列；该正例不是 GUI/产品适配器链，也不解释原 GUI 错误的全部原因。

### 当前门禁与双语布局

集中定向测试曾为 1,600 通过、16 失败、60 跳过；不能称全绿。Grok 旧“所有图片拒绝”断言已修正，内置临时目录复验 atomic 15/15、runtime host 31 通过/1 跳过。2026-09-26 最终输入源码通过 `cargo check -p warp`、`cargo build -p warp --bin infinishell` 与 `cargo test -p warp --lib i18n::tests`（11/11）。复制到内置盘并核对 SHA-256 后，受影响模块通过：managed_input 27、Claude 169、Grok 383、普通终端 footer 45，合计 624；另有 21 个在线用例跳过，不能计通过。该轮覆盖新 GIF 边界和 Grok 已选技能门禁，libtest SHA-256 为 `e2ab62925d6aae589eaf00d6d3d1f3b4feb4056f3b242551b42ec35c5fe0fe44`；相关平台与最终提交绑定待补。

macOS 重启各语言进程后，Claude 图片格式说明与 Grok 顶部新说明的 en/zh-CN 原图均可读，完整换行且相关按钮可见，原图 2560×1600、逻辑窗口 1280×800；只验此视口布局，无模型输入。GUI 使用上述 WIP 二进制（签名后摘要 `8f42725d3ab4c9f4c2ea8d4efb7e71edb29010abf3f0303e3edc3cbbe4d73715`）动态读取当前 Fluent，早于最后两项 Rust 门禁修改。旧 live-switch 混语言截图保留且不计中文完整布局；不计最终 SHA、错误提示/审批全视口、Linux/Windows 或 V05 完成。

### 仓外证据入口

本机持久归档根为 `/Users/zhishi/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/`。以下为根下相对路径；原件逐字节保留，认证工作目录未归档，跨机器不能仅凭本机路径认定已验。

| 证据 | SHA-256 与范围 |
| --- | --- |
| `image-evidence-sha256.json` | `ad4bb6a4da73f41b44c25c08844a259386a63bbcacda304fd1bd02bd0d288308`；55 个图片证据文件的源/目标字节摘要，含两个完整证据目录 |
| `infinishell-claude-rich-images-evidence/production-image-evidence-wip-index.json` | `3ef53b8a10968af6a9549f4986b2a01228f38e88bb1ada109cd239639b90a39f`；四个格式/纯图生产用例；同目录 native-skill-v4/v5 保留失败/成功 |
| `infinishell-grok-managed-image-wip-v1/receipt.json` | `53f51826fc669de485ebc8a0a1c33f803feca9ea2ceaa0737b3caf8267421154`；原生图片投影文件摘要 `d0ba084e5c437f1b9383378dfdf3ee499861b3cfc7b5ed793ed1f0e4e2fcb05a` |
| `terminal-files-native/scope.safe.json` | `25285aecbd815b9cc3389acd6f22025a6b5491c296fd313013183c9ebcfb2987`；原生 G01/G03/G07 的来源和负例；原 index 为 `25a22584b2ec0cabc3f7143e035c866a188e6f6c5cb4aae9dfaa9bfa8e5e7f3f` |
| `claude-skills-native/scope.safe.json` | `10d7e33219112755c0c72a076ef5941ed48cd1daeb0aa8419686f560be45acf0`；原生 G06；原 index 为 `271c28ffb75cd75c46f09149ad5d4b57cda3eff4648ad62633e6bca8a1162f19` |
| `codex-image-luna-native.archive-index.json`、`codex-image-sol-native.archive-index.json` | 分别 `2c8c10286e4d581f7cbb390520481442ba40c90e143ef929820f02047d5e5259`、`47f7905eadcc9ce7e46c23b991597aa004cf1a4f88bcc88f993d753a88005e56`；原错误及独立语义正例 |
| `gui-layout-wip.archive-index.json` | `feea309ee4d7e61054d6987f17c4838fd8280af0b30dda0de6e8250dfe4bc90f`；四张通过原图、旧混语言负例及 layout-review.safe.json 的 WIP/资源来源边界 |

## 历史三平台相关门禁

`ee839b4fc` 的 macOS arm64 本地门禁与 [Linux／Windows 集中验证 36103244022](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36103244022)记录如下。该次 Windows 作业整体失败，不能由其他通过项覆盖。

| 范围 | macOS arm64 | Linux x64 | Windows x64 |
| --- | --- | --- | --- |
| 桌面工作区测试 | 10,895 通过、96 跳过 | 10,923 通过、100 跳过 | 10,680 通过、112 跳过 |
| `cargo check -p warp`、i18n、默认入口 | 通过；i18n 11 项 | 通过；i18n 11 项 | 通过；i18n 11 项 |
| 实际双语 GUI 目视 | 6 张有效原图通过 | 4 张原图通过 | 4 张原图通过 |
| 原生 CLI 原子升级 | 复用各自固定构建的三款实链 | 12 场景通过 | 三款正式升级通过 |
| 直接 Grok ACP 五秒 EOF | 不与托管清理混计 | 按原探针来源保留 | 5,005 ms 时未退出，之后自然退出 0；五秒观测仍失败 |

桌面工作区集合排除 `command-signatures-v2`、`integration`、`warp_tui`；GUI 与受影响 TUI 单列，不等于所有仓库测试。macOS 的 TUI 消息状态 9 项通过。Windows 4 项 leaky 属于通过子集，不证明后代进程已退出；macOS 3 项、Windows 1 项 slow 按原记录保留，跳过项不计通过。

双语 GUI 只覆盖原收据列明的视口。Linux／Windows 使用 Codex 页面与系统剪贴板，不证明 Grok 专属权限界面或物理中文输入法；Windows GUI 使用实际 Mesa GL/llvmpipe 窗口，不外推为 DXGI 全功能验收。

## 真实 Grok 进程清理补验

`50e1515bc` 的 macOS 提交后实测和 [Linux／Windows 集中补验 36116931025](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36116931025)均通过。该次只补相关原生模式；工作区、GUI、原子升级沿用上节的原来源，没有再次执行。

四场景分别在私有 leader／无 leader 两种拓扑执行显式结束与标准输入关闭后结束，经过生产 `ManagedChild`，核对真实进程身份、代次、stdout EOF、旧代次拒绝及已绑定进程退出。

| 平台 | 已通过结果 | 不能据此推定 |
| --- | --- | --- |
| macOS arm64 | 提交后四场景通过，源码干净；之后同提交 `cargo check` 也通过 | 二进制是核对摘要后复用的内置盘副本，不是四场景前重新编译 |
| Linux x64 | 定向 847 项、原监督 5 项、真实 Grok 四场景通过；取得 `linux_subtree` 清理回执 | 无 leader 场景退出码为空，不能称全部自然退出 0 |
| Windows x64 | 定向 747 项、原监督 5 项、真实 Grok 四场景通过；真实进程句柄与严格 Job 清理 | 两个无 leader 场景为退出 1，只证明清理，不是模型任务成功 |

此补验没有认证或模型输入，不替代审批、用户 Stop、历史继续与应用重启的完整在线链。独立直接 ACP 的自然 EOF 与 leader 强制清理也分别计证，不能冒充生产监督器证明。macOS Intel 备用作业未执行。

## 历史固定版本产品链已验范围

以下主要来自 Mac 的已认证原生执行及真实 GUI，均保留原构建与模型来源。Grok 模型为 `grok-4.7`，Claude 父子链为 `claude-sonnet-4-6`；不同模式不能相互替代。

| 能力 | 已验结论 | 范围限制 |
| --- | --- | --- |
| Codex 托管父子任务 | V9 双向原生 ACK、父权限上限、自动结果 ACK、显式继续后持久结果回收通过 | 不是完成态父任务自动唤醒证明，也没有该用例的子文件效果或 GUI 证明 |
| Claude 托管父子任务 | 审批、工具、子文件效果、双向消息 ACK、inspect、结果 ACK 与清理通过 | 仅相应固定权限策略，不代表任意工具或原生 shell 可用 |
| Grok 托管根与子任务 | 根任务两轮、允许／拒绝、排队、取消和同会话恢复通过；V8 父子 ACK、结果回收及跨宿主冷恢复通过 | 固定读取／文件策略与继承设置模式分别验收，不能混称操作系统沙箱 |
| 三款 GUI 恢复 | 存活宿主重新关联、同原生会话冷恢复及结果回收有实链；重启不重投旧输入 | 源码代次与宿主身份必须核验；不等于所有平台均完成该链 |
| Grok 多轮重启 | V10 两轮后应用重启，输入数未变，第三轮正确回忆；V11 空闲目录刷新不新增回合 | 目录刷新不等于任意新技能均能在运行中使用 |
| Codex 富输入 | 附件类型化通道、技能、评审意见及文件引用实际进入原生请求 | 图片颜色回答错误，不能计图片理解成功 |
| Claude 富输入 | PNG 原始字节摘要一致，图片回答正例、单技能执行及重启恢复通过 | 当时仅 PNG；本次静态格式/纯图的新增范围见上节，组合与完整验收仍有缺项 |
| macOS 中文输入 | 物理拼音、marked text、数字／空格选词、中英混输及多行保留通过 | 此 IME 用例未提交模型，系统候选浮窗未单独截图 |

普通 PTY 生命周期补验共 16 次输入，三款允许／拒绝、继续及应用重启后同会话恢复有证据。Claude／Grok 取消使测试进程退出；Codex Escape 仅中断对话，测试 shell 自然结束，因此保留 Unknown 与显式关闭终端，不能称工具取消清理通过。

插件与原生通知已有三款实链。Grok `0.1.4` 的通知桥接由产品安装器管理，保留配置禁用、冲突与去重门禁；Windows 安装、`0.1.3 → 0.1.4` 更新、同版本修复、禁用保全、失败回滚和再修复在 [36028161259](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36028161259)通过。插件安装测试为零模型请求，模块导出不证明普通 TTY／GUI 通知全链。

## SSH、tmux 与升级

三款固定版本已在 Mac 本地 PTY、Mac 到同机隔离 OpenSSH、tmux 透传开启且断连重接场景取得真实 hook 与产品会话配对。不是远端 Linux／Windows／WSL 验收；Grok 该批模型输入为 ASCII，不计中文输入通过。

关闭 tmux 透传时，Claude／Grok 已观察到内层 SessionStart 与外层零接收；Codex 补输入后外层零接收，但内层原生发送未独立观察，负例不完整。Codex 首轮 tmux 的 Return 换行疑点未独立确认原因，不宣称已修复。

三款官方原生安装的固定升级为 Codex `0.155.1 → 0.156.1`、Claude `2.1.278 → 2.1.280`、Grok `1.0.40 → 1.0.41`。Mac 正式事务及 Linux／Windows 后续原生事务按各自提交通过，包含目标版本／摘要、配置保全与清理；包管理器来源的缺项以 G09 为准。

Claude Mac `2.1.280 → 2.1.267` 的 Latest → Stable 真实渠道降级通过；其他渠道切换只保留各自历史版本和源码域结论。更新偏好在 GUI 重启后恢复，不能由偏好持久化推定升级事务成功。忙碌延期、启动预约、回滚及恢复有对应回归，不等于正式升级样本曾在运行中的模型会话内执行。

## 失败、修正与未验结论

| 原始问题 | 后续结论 |
| --- | --- |
| `9af6de393` Linux Grok 中断升级退出超时 | `ee839b4fc` 修复启动取消窗口并取得 12 场景独立通过；原超时未被追改，也未声称唯一因果已证明 |
| `ee839b4fc` Windows 直接 ACP 五秒 EOF 失败 | 迟到自然退出单列；随后新增真实监督清理门禁通过，仍不承诺原生五秒 EOF |
| `14e0d7d42` Linux 身份绑定／Windows 输入路径检查失败 | `50e1515bc` 修正线程 TID、路径与实际对象身份检查后通过；原失败不能计作已执行的模型场景 |
| Mac 外置盘 worker 启动受阻 | 栈采样显示未进入 Rust main；用户说明启动需要授权，之后核对摘要并从内置盘启动，不放宽超时或权限 |
| 早期 Windows 编译、更新和 Grok 迁移失败；Linux 中文方框 | 分别修正后取得窄范围原生补验及 GUI 正例；旧失败与其真实来源仍在固定历史提交 |
| Grok 技能旧 runner 为 false、早期冷恢复失败 | 技能原生两轮由独立离线复核确认；冷恢复另有 V8 实链。不能把原 runner 或旧构建改记通过 |

完整在线生命周期的 Linux／Windows 覆盖、两平台物理 IME、远端 SSH/tmux 组合及 Grok 专属双语布局仍未完整满足。原生图片、技能、普通终端富输入、远程附件、包管理器升级和子任务工具范围的当前缺项统一见 [KNOWN_GAPS](KNOWN_GAPS.md)，本报告不重复维护其关闭状态。

本报告不计为合并、发布或 Goal 完成。后续结论更新应同时写明提交、CLI 版本、平台、模式和实际通过／失败／未验范围；新增原始日志与截图置于仓库外，只将必要结论与可追溯引用留在专题目录。

[history]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/tree/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity
[acceptance]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/ACCEPTANCE_FIXED_VERSIONS_20260924.md
[mac]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md
[final]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/FINAL_SAME_COMMIT_VERIFICATION_20260925.md
