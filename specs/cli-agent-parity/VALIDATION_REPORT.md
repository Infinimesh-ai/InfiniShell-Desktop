# CLI 支持与能力对齐：验证结论

本报告保留截至 2026-09-26 的固定版本验收结论、源码来源和未验边界，不保存逐轮日志或截图。**这是阶段交付，完整 Goal 尚未完成。** 当前状态以 [CURRENT_STATUS](CURRENT_STATUS.json) 为准；剩余功能、优先级和关闭条件统一维护在 [KNOWN_GAPS](KNOWN_GAPS.md)。

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

## 2026-09-26 Claude npm 单包更新增量

以 `fe135e3436c22eaa7857c31bfaf94969eb1aa11f` 为基线的工作区接入 Claude npm 官方完整包树事务，目标限定 `2.1.280`，旧安装验收起点为 `2.1.278`。独立核对 wrapper/platform 归档 SRI、完整树及公共 native 入口，不执行安装脚本；原有忙碌预约、配置约束及监督清理保持。候选使用私有空配置、固定 `--version` 和拒绝网络；新增删除前逐成员校验及保留外部变化的回归。Codex npm、Windows npm、musl、额外 ACL、Homebrew、WinGet 和合法降级仍未完成，G09 不关闭。

macOS arm64 本地 `cargo check -p warp`、i18n 11 项、更新模块 182 项（4 跳过）、监督模块 85 项（17 跳过）、Python runner 16 项及应用构建通过。首两轮测试编译的导入错误已修，原日志保留。工作流 actionlint 在显式登记现有自托管 runner label 后通过。英中版本探测失败提示已在同一签名 GUI 构建实际显示，完整文本、渠道说明、按钮与下拉框可读；合成无效版本只计布局，不能替代更新事务。应用、测试程序、监督程序及缓存均从内置盘运行，两种语言启动均未等待人工授权。

第一轮真实 npm 事务在 `/private/tmp` 私有子目录中构造官方包后，被已有 macOS 原子执行祖先检查拒绝：`managed_process.atomic_macos_ancestor_unsafe`。原始 native exit 1、cleanup_confirmed 收据保留，旧公共程序摘要不变，暂存树已回收。未放宽权限检查。第二轮在用户私有内置目录通过祖先检查，但 15 秒输出等待到期：原子镜像验证发生在监督者就绪之后，空输出和 `stdio_closed` 收据不能证明已进入 CLI 主程序。该轮同样确认清理、旧公共摘要不变和暂存树回收。候选版本探针改用既有更新事务的有界 300 秒预算，并单独返回 `TimedOut`；新源码门禁及后续五个真实场景均已通过，旧失败保留。

本轮 macOS arm64 在私有前缀物化官方包，实际执行 npm 只读来源查询、生产 Rust 更新后端及受监督候选 `--version`：正常更新到 `2.1.280`、交换后缺收据的冷进程恢复到原 `2.1.278`、外部改动及 journal 保留、候选改写后未启动和未审核 `2.1.278` 降级拒绝，五场景均通过。独立 Python 检查逐份核对 SRI、归档成员与最终目录；公共入口链接保留，所有已启动候选正常退出并确认清理。版本拒绝不能外推为合法降级已实现。

最终监督程序 SHA-256 为 `63c34a50b1ce86d3bfdd8774aee3ee9ab070bef348c9d3bc8cda2e9e5c30499b`，测试程序为 `ebc56ba3d43ccb0f8b92838cd777831e4ea8d484b6fcf68aeaebd4eb7f041a96`，零模型输入。忙碌范围仅为另行运行的既有产品状态机测试，未覆盖 GUI 更新点击或插件重检。GUI 文案与资源未随超时修复变化，复用 v3 的独立布局原图，不冒充同一程序全量重跑。349 个原始记录、旧失败、源码与截图保存在仓外 `validation/claude-npm-20260926/`，索引 SHA-256 为 `7f0a70056a1edfafd746a962699d8f2ae70685ce909658bf745c217b7a2ec022`；实现提交为 `529512a354d66fdcf48f70db1bbe63216c36a3da`，独立提交绑定 SHA-256 为 `6d2b4d5c7836803e3d1890998d924edf6af214761145f1ba42475b784ed51be8`。[Actions 36217724224](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36217724224) 已通过同提交 Linux/Windows 定向门禁，Linux 官方 Claude npm 五场景原始收据逐份核对通过，零模型输入。云端 253 份记录归档于仓外 `validation/cloud-g09-529512a35/`，索引 SHA-256 为 `97ca36b14ecbd9eecf2d6313ab4a3fc51a662a26c27953934d0ba2b207a2e991`。本次仅补验收结论，无需本地化变更；G09 的其余安装来源仍未关闭。

## 2026-09-26 文件卡片同提交门禁

文件选择器增量已提交为 `83e5115839b301697f3322092f487ddce9d72dfe` 并推送。本地 `cargo check -p warp`、i18n 11 项、editor 6 项、footer 46 项与应用构建通过；实际 macOS Codex 两张卡片投递、原生 shell 读取及双语布局沿用该增量的原收据。Grok 自动提交、Claude 首次向导后的普通终端链仍待补，G07 不关闭。

[Actions 36210156960](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36210156960) 绑定同一提交，Linux x64 与 Windows x64 定向门禁均通过，Intel 作业未选择。它覆盖相关编译、回归、无认证原生协议/通知/清理；不证明已认证模型生命周期、真实中文输入法或 Grok 完整双语 GUI。47 个原始日志和产物保存在仓外 `validation/cloud-g07-83e511583/`，索引 SHA-256 为 `20373845cd0a8228ae23fca12f57db91aec0e8bb6dc62986d462fa302678a364`。此前 `6790b185a0c0bc581f804edf975960047df85db7` 的 Linux/Windows 整合门禁亦通过，归档索引为 `8239de43dd32999ba0348146d923df701e9d7b97e0b5e079022beff9d2abcdc3`。

## 2026-09-25 继续实现增量

实现来源标记为**输入实现提交 `84102bb1c687f87a2425bc1937784e77250c416c`**。G02 接通固定 Grok PNG ACP 图片；G04/G05 扩展 Claude 静态格式、纯图与精确注册单技能的图片编码；G07 接通普通终端本地文件卡片到明确路径引用。G09 的 Codex/Claude npm 管理器/包清单/前缀/真实入口绑定已提交为 `b6f93f6627738fb83b6be776dd225666a900ac90`，但 npm 更新仍为 `ManualOnly`，尚未完成真实升级、降级及恢复事务。以下均不关闭 Gap。

### 图片生产适配器与原生合同

| CLI、版本、模型、平台与模式 | 本次实际结果 | 证据边界 |
| --- | --- | --- |
| Claude `2.1.280`、`claude-opus-5-5`、macOS arm64、Inherit 生产适配器 | JPEG、WebP、静态 GIF 分别新建识别随机象限色；原生 user 回放的 MIME、字节和数组摘要与持久图片一致；关闭后同原生 ID 冷恢复正确回忆，均自然退出并清理。每用例 2 次模型输入 | WIP 二进制 `6e3e13f07aef46612c90c35b0c67c84163313f00f5e594701746f1f06215d284`；不是 GUI、应用重启或跨平台链 |
| 同上，纯 PNG | 先文字定义后续图片回答格式，再提交没有文字块的纯图片；实际识色、字节回放、持久引用恢复及冷恢复回忆通过，共 3 次模型输入 | 不外推所有纯图片格式；不是图片加技能的生产验收 |
| Grok `1.0.41`、`grok-4.7`、macOS arm64、Inherit、无已选技能的真实生产 connect | 新建识别红/蓝 PNG，关闭后同原生 ID 冷恢复，提交独立绿/黄 PNG 并正确识别。真实最终历史 user_message_chunk 的图片字节、MIME、typed content 摘要与持久输入一致，promptIndex 为 0/1，代次和回合准确关联 | WIP 二进制 `ff821a38c0260c35e9a3210f39090156c6814d2b1327499bb8992669b2b3f8b0`，未用 candidate flag；两代各重发同一消息 ID 共 3 次，仍各仅执行 1 个原生回合。两次退出均 `stdio_closed`、exit 0 且 cleanup_confirmed；不是 GUI/应用重启或其他策略/平台 |
| Claude `2.1.280`、`claude-opus-5-5`、macOS arm64、直接原生 stream-json 图片加单技能 | v5 新建与冷恢复各有精确注册命令的真实 Skill tool_use 及 `SKILL_OK LEFT=RED RIGHT=BLUE` 正例 | 原生合同；保留原生权限流程，审批次数未单独计证，不是完整生产适配器/GUI链。裸 slash 数组未触发技能的失败和 v4 探针“禁止工具”指令冲突导致的失败原字节保留 |

两款生产用例使用同一内置盘监督者，摘要 `a8aae1653910a4625c879caa62f730e5131207eb08bfb68877334f4eb3b1d4b8`。收据明确记录基线 HEAD、工作区脏状态、关键源文件和运行器摘要及运行期间源码不变；**HEAD 只是基线，不能把 WIP 二进制记为该提交已验证**。其后又补严 Grok 已选技能图片门禁，以及仅 Claude 的静态 GIF 首次/恢复校验；上述原始收据没有覆盖这两项后续修正，不回填为修正后通过。最终重建与受影响模块回归已通过（见下节），静态 GIF 后续已用 `84102bb1c` 对应 libtest 完成在线窄复验；20 个构建源文件均与此提交 Git blob 一致，旧 WIP 记录不回填。 Grok Python 运行器后来增加 Linux 临时目录回退，但没有本次 Linux 在线结果；Windows 运行器明确拒绝执行，认证副本 ACL 与在线验收另待实现验证。Rust ignored 测试可跨平台编译不等于 Windows 产品链可用。

### 普通终端、技能与独立图片判断

本次普通终端探针均为 macOS 手动驱动的原生 PTY，产品源码改动未在该探针执行：Codex `0.156.1/gpt-6-luna`、Claude `2.1.280/Opus 5.5`、Grok `1.0.41/grok-4.7`。

- G07：三款分别通过原生 exec/read 工具读取中文与空格路径的独立标记；Claude 第二文件的 No 拒绝未读取。它证明路径引用合同，不证明修改后的文件卡片 GUI 链、拒绝后草稿恢复或 Linux/Windows；Grok 产品仍受 G01 自动发送守卫阻止。
- G01：新空会话超过 66 秒没有 idle_prompt；help 模态、未提交中文草稿及后台 sleep 仍活跃时却可能出现 idle_prompt。审批等待 76.7 秒只有 permission_prompt。该通知不能作为 Enter 安全或编辑器为空的证明，守卫继续保留。
- G03：Ctrl+V 后 bracketed text 粘贴造成同一回合两张图片，负例保留；Ctrl+V 后普通 UTF-8 两行配合 Alt+Enter 得到恰好一段文字和一张图片，并正确识别红蓝。无剪贴板消费 ACK 与可靠输入就绪，尚未接通自动链。
- G06：Claude `2.1.280/Opus 5.5` 的原生 PTY 与 stream-json 取得同轮两技能依次审批/调用、运行中新增技能、reload_plugins 返回准确命令列表及冷恢复读取新技能标记的正例。只限无副作用技能原生合同；产品实现由后续增量处理，注册事务、并发、父权限上限、Grok 多技能及两平台均未计通过。
- V04：Codex `0.156.1` 独立原生 exec 的 `gpt-6-luna` 位置错误保留；同图 `gpt-6-sol` 返回 `YELLOW BLUE RED LIME`，独立目视确认 LIME 对应亮绿且顺序正确。原收据 strict_match=false 不修改，语义判断单列；该正例不是 GUI/产品适配器链，也不解释原 GUI 错误的全部原因。

### 当前门禁与双语布局

集中定向测试曾为 1,600 通过、16 失败、60 跳过；不能称全绿。Grok 旧“所有图片拒绝”断言已修正，内置临时目录复验 atomic 15/15、runtime host 31 通过/1 跳过。2026-09-26 最终输入源码通过 `cargo check -p warp`、`cargo build -p warp --bin infinishell` 与 `cargo test -p warp --lib i18n::tests`（11/11）。复制到内置盘并核对 SHA-256 后，受影响模块通过：managed_input 27、Claude 169、Grok 383、普通终端 footer 45，合计 624；另有 21 个在线用例跳过，不能计通过。该轮覆盖新 GIF 边界和 Grok 已选技能门禁，libtest SHA-256 为 `e2ab62925d6aae589eaf00d6d3d1f3b4feb4056f3b242551b42ec35c5fe0fe44`；构建的 20 个源文件与 `84102bb1c` Git blob 一致，绑定证据见 `input-final-local/commit-binding.safe.json`；Linux／Windows 相关验证运行 [36158594977](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36158594977) Linux 成功，Windows 两项新增附件投递测试失败，整体失败，不计作在线模型验收。Windows 的 `cargo check` 通过；定向测试 2,778 通过／2 失败／4,769 未选或跳过。两失败把原始 Windows 路径与 JSON 转义路径比较；工作区已改为准确数组/字符串断言，本地 footer 45 项复验通过，Windows 同提交复验待补。Linux 主应用定向 2,916 项通过，未选、零模型和旧版 Codex 原生脚本均在各自 scope 中明确区分。

macOS 重启各语言进程后，Claude 图片格式说明与 Grok 顶部新说明的 en/zh-CN 原图均可读，完整换行且相关按钮可见，原图 2560×1600、逻辑窗口 1280×800；只验此视口布局，无模型输入。GUI 使用上述 WIP 二进制（签名后摘要 `8f42725d3ab4c9f4c2ea8d4efb7e71edb29010abf3f0303e3edc3cbbe4d73715`）动态读取当前 Fluent，早于最后两项 Rust 门禁修改。旧 live-switch 混语言截图保留且不计中文完整布局；不计最终 SHA、错误提示/审批全视口、Linux/Windows 或 V05 完成。

### 2026-09-26 后续实现与窄复验

- `84102bb1c`：Claude `2.1.280/claude-opus-5-5` 静态 GIF 新建和冷恢复窄复验通过，原收据因无关未跟踪探针记 dirty=true；另附关键源码与 Git blob 的逐字节绑定，不改原收据。
- 同提交 macOS GUI：JPEG 原始 MIME／字节和四象限回答正确；第二轮 READY 后应用退出再启动，重新关联原进程，消息仍为两条。断开确认原 PID 消失后，历史继续使用新进程和同原生 ID；此时尚未新增消息，随后显式发送纯 PNG，原生内容仅一个 image 块、无 text，独立象限回答正确，持久消息变为三条。区分重关联、冷恢复、发送，不把恢复按钮当作新输入投递。范围为 Inherit 根任务，非父权限证明或三平台完整生命周期。
- 同提交 Grok GUI 选择 PNG 被旧通用选择器拒绝的负例已保留；托管运行时已支持图片，但入口遗漏。当前工作区已修正入口，仍要求固定版本、继承模式、无已选技能和原生模型再次核验，后续修正后的双图 GUI 与恢复正例见下文，旧拒绝不改记通过。
- G06 实现来源 `3b4d8e8a3`：空插件槽、无图多技能、严格 reload_plugins 确认、事务回滚、旧回调隔离及接收后持久化已接入。该来源的 macOS 生产适配器三轮在线通过（多技能→新增→冷恢复，6 次真实 Skill 单次审批），两代正常退出。原模块首轮 1,022 通过／3 失败／52 跳过，单线程复核其中两项通过，一项监督控制通道 os35 仍失败；不称全模块全绿。后续整合门禁已通过；Claude Mac GUI 双技能及同会话热新增实际 native Skill 逐次允许通过，后续热技能恢复修复与 GUI 复验见下文，跨平台仍待补。
- G10 实现来源 `9f5967f62`：独立 V2 文件创建策略保持父权限版本匹配，逐次审批、路径范围和允许时再次校验；固定 `2.1.280` 的原生 Write 允许／拒绝／越界／符号链接探针已通过。V2 父子与 V1 禁升级的生产协调器验收夹具来自 `de135f92c`，后续两条生产协调器链已通过，范围见下文；原生探针与真实父子链分别保留来源。

- 整合工作区内置盘首轮：i18n 11 项、运行时 1,038 项通过／53 跳过，`cargo check -p warp` 与应用构建通过；之前 os35 的恢复场景本轮单线程通过，原失败不改写。外置盘断连导致的编译 SIGBUS/os5 和主动中止重试仍保留；经用户授权改用内置缓存，并仅清理已核对无占用的 3.18 GiB 旧运行程序。后续修正须另补门禁。
- G05 整合工作区生产链：固定 Claude `2.1.280/claude-opus-5-5`、macOS arm64、Inherit 根任务的图片＋单技能新建与同会话冷恢复通过。两次精确 Skill `AllowOnce`、两组独立象限颜色、原生图片字节/工具身份、两代 `stdio_closed/exit 0/cleanup_confirmed` 对应完整；libtest `ec718473…`、监督者 `48018a00…`。不外推多技能图片、父权限上限、GUI、应用重启或跨平台。
- G10 首轮 V2／V1-ceiling 生产父子链均失败：G06 新逻辑把正常邮箱 `MessageAccepted` 当作 self `user_input`，返回错误的启动不匹配。子进程未取得原测试完整清理收据，之后按精确身份补救清理另记；已修正为只有自身用户输入登记技能，其他邮箱保留原回执校验；后续还修正已持久化邮箱 joined 重放的同类误判，两条新构建生产链已独立通过，见下文；原失败和当时缺失的子清理收据不回填。补救前仅确认 launchd 项已登记但 not running，不能推断原生进程当时仍存活或由补救导致退出。
- Grok 整合 GUI 已能选择 PNG 并保留图片卡片，但新建时出现 `post-response setup phase arrived before session/new response`，没有进入模型或确认原生会话，缺项仍开放。原始 journal、未确认输入和清理收据保留；不能把卡片显示算作图片投递成功。原生正负对照已定位为缺省配置走 direct 模式：仅有 `--leader-socket` 不强制 leader。工作区对固定 1.0.41 根任务显式传 `--leader`，SDK／固定策略仍为 `--no-leader`，协议阶段检查不变；后续新构建双图 GUI 已通过，见下文；原始握手失败不改记通过。

### 2026-09-26 整合修正后的限定验收

以下仍为 `84102bb1c` 基线上的工作区摘要绑定，**不是最终提交或三平台完整验收**。内置盘 `fixed` 轮来源摘要为 `b8c832473e548e8213eb83cd29422f6ecebf8879e8ed1718677dda1073e92632`，i18n 11 项、runtime 1,044 项通过／53 跳过、check 与应用 build 通过。随后邮箱 joined 重放修正的 `replay-fixed` 轮来源摘要为 `d5a8790a63fdcf7c267dd8bf99c525f59781f9ca85102226d550c066478177bc`，coordinator 91 项通过／4 跳过，i18n 11 项、check 与应用 build 通过；未无变化重跑整套 runtime。该轮 libtest SHA-256 为 `c7cf03f14893dd82eceb80cdf39fabf1cfe2f2e155076f27f2d86f2922a2f595`，监督者为 `8484a0c900e2c596f95d3d2ff4c473e1814f18713540c8fef8e0191e8e405e95`。跳过项、Windows 原失败与外置盘构建失败保持原结论；同提交相关平台验证待补。

恢复清单、身份隔离及 ACK 顺序修正后的源码快照为 `05e7fcbb8d4e8ef98e924ccf1b17426564239b3b2313639ab34a4fc100306461`（36 个构建源文件）。i18n 11、coordinator 101 通过／4 跳过、`cargo check -p warp`、应用 build 全部通过；此前未再修改的 task-manager 46 项通过。新增真实 SQLite 故障测试验证 ACK 失败不改技能、ACK 成功但清单失败后重放零发送、错误会话不提前 ACK。libtest `7a3955070f01470a00cb2fb970ed2b0757c14325384ac82b6d94b84c3021e05d`，监督者 `db931fe468a13b7a7d30d4850ddee763606f7527b15d24e87af24bd8b6c1f884`。较早恢复构建被主动中止以整合隔离补丁的记录保留，不记作编译失败或通过；GUI 和在线结果仍使用下列各自原二进制来源。

- **G02／V04 Grok GUI**：macOS arm64、`1.0.41/grok-4.7`、Inherit 无技能，两张独立 PNG 的原始字节和实际象限色回答分别通过。首次断开并退出应用后，历史继续使用新宿主/原生进程和同原生 ID，发送新输入前消息仍为 1 条；第二轮完成后退出应用，宿主仍存活，重启精确关联同宿主、同原生进程和代次 2，消息仍为 2 条，没有重投。两代均 `stdio_closed/exit 0/cleanup_confirmed`。两轮分别绑定对应源快照与签名后应用摘要，不能把第一轮改记为后一轮构建。英文／简体中文权限、附件、技能限制、消息及历史结果完整滚动区无截断遮挡；不计 Linux/Windows、真实 IME 或未执行的审批交互。
- **G05／G06 Claude GUI**：固定 `2.1.280/claude-opus-5-5`、macOS arm64、Inherit 根任务，三轮真实 GUI（双技能、热新增、PNG加单技能）共5次精确Skill AllowOnce，原生历史模型为claude-opus-5-5且图片字节摘要匹配；恢复修复前错误及原始记录保留。修复构建重关联同宿主/原生进程、同会话和代次3，仍3条输入，原生历史字节不变；正常断开exit0并确认清理。英文和简体中文相关说明、技能列表、消息与历史结果可读。模型轮次与重关联程序分别绑定各自源码/二进制，不宣称新构建重跑模型。 旧宿主签名后摘要 `b014a98f…`，恢复应用为 `8dd332a3…`（源码快照 `e01dee11d49901d81b1a427916aef5d87e78709546847a2de30f140615694f76`，未签名程序 `b4f56cb5…`）；持久 OwnerClaimed epoch 1→2，原生会话 `33ae1e46-47b6-4865-bf8d-008d6630e0b9` 不变。恢复后通过显式“刷新”加载 alpha/beta/gamma 目录。原生历史摘要 `c6668675b2002c0e53d61c457643d6cbda70375497da4dbf9223551a6fd013af`；原 PNG 与原生图片块同为 `ea9dc58742e14577738c00e60ab53214bf9863b7a0ee6b57545426d79d742c9f`。不证明父权限上限、图片加多个技能或跨平台。
- **G07 GUI 入口复核**：macOS arm64、Codex `0.156.1`、应用摘要 `8dd332a3…`，真实“附加文件”可以选择文本文件，但只把路径插入正文，没有生成 PendingFile 卡片。旧点击未生效时 Open 禁用的截图保留；随后键盘选中并点击 Open 成功，撤回“文本文件不可选”的误判。两会话均零模型输入，清空草稿后正常退出；不计文件读取、投递或 G07 完成。
- **G06 Grok 原生合同**：默认 profile、显式 `agent --leader`、固定 `1.0.41/grok-4.7`、macOS arm64，2 次模型输入证明同轮首技能原生展开、后技能通过 read_file 读取完整正文，以及同会话新增技能须显式 reload 后才注册/展开。另一次零模型双目录探针证明 reload 会刷新同 leader 两会话，即使参数仅写一个 sessionId；不能当作局部刷新。两轮 client 自然 exit 0，按精确归属清理各自 leader。零审批请求，不计 InfiniShell 多技能/热新增产品接通、严格串行多技能执行、父权限上限或 GUI。未信任第二目录的早期作用域观察和离线校验器失败均保留。
- **G10 Claude V2／V1-ceiling 生产链**：固定 `2.1.280/claude-opus-5-5`、macOS arm64，使用上述最终内置 libtest/监督者。V2 精确 Write 允许生效、拒绝后文件不变；实际 5 次允许／1 次拒绝。另一条真实 V1 父记录请求 V2，因 `claude_profile_parent_mismatch` 在建立原生连接和发送输入前拒绝，父记录不变；该链保留 V1 Edit 效果。两链分别 4 次 MCP 工具、5 次输入、3 次原生执行、2 次 joined，双向消息和自动结果 ACK 完整；四代共 12 张原始清理收据齐全，host/native PID 全部消失，launchd 项均注销，无补救清理。
- **G10 运行器修正范围**：V2 Rust 测试及完整私有树已通过，但首版公开投影误把 inspect 明确标记截断的嵌套 JSON 当完整 JSON 解析，wrapper 失败。修正后只对布尔 `body_truncated/evidence_truncated=true` 的有界副本保留原字节摘要，未标记的坏 JSON 和损坏完整证据仍拒绝。原失败 metadata/原始树保持不变，同一次 V2 原生执行重新投影通过，没有新增模型执行；两个 Python 脚本 before/after 摘要另记，Rust blob 未变，离线测试 25+70 项及 py_compile 通过。待审批 Write 取消、活跃宿主重关联、冷恢复、GUI、Linux/Windows 及命令/技能扩展不在本轮范围，G10 仍开放。

### 6790b185 提交绑定与文件选择器后续增量

技能、V2 文件策略和恢复修正已提交并推送为 `6790b185a0c0bc581f804edf975960047df85db7`。最终快照 `05e7fcbb…` 的 36 个构建源文件与此提交 Git blob 一致；绑定收据摘要 `613cb19968667392a08829f4f831030af6da4a2fca7d9fa58ef538e4e6b80060`。相关平台运行 [36176479030](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36176479030) Linux x64／Windows x64 两项作业均成功，Intel 作业跳过。该定向工作流不包含已认证在线模型、物理 IME 或 Grok 专属双语完整布局，不将相关门禁外推为完整验收。

随后 G07 四文件增量接通展开富输入的文件选择器与 PendingFile 卡片，保留 CLI 锁定输入模式、取消和旧会话回调边界。源码快照摘要 `e02d7239b30e43019a209688348eb23bf7b76246b0798267d291d00ebb73477e`，基线为 `6790b185`、工作区 dirty；内置盘本地 i18n 11、editor 6、footer 46、check 和应用构建通过。libtest `a2de24af1e607ae5b6a479fb100cc61521a27cd4e0e85fa8480f675b414670e8`，签名后应用 `d383765f2e24f0f49747e25a1ec3f8d69a9f2237bd9b0586c8a986eeb17d2688`。无需本地化变更，既有“附加文件”语义适用。

- macOS arm64、Codex `0.156.1/gpt-6-luna`、普通终端 read-only/on-request：真实选择器添加两卡片，中文/空格路径以一个 JSON 数组随正文一次到达原生请求。中文首轮模型转向 TextEdit，拒绝后未读取内容，原失败保留。英文独立正例仅关闭官方 computer_use/browser_use 工具功能，保留原安全策略；模型调用原生 shell，工具输出及最终回答均匹配两份文件的独立标记。会话分别为 `01a0d9f8-a040-7950-b03e-82202114b1ec`、`01a0d9fd-cb30-7bf3-8a07-b4d1aef1b6b7`。两语言卡片和正文布局可读，不计物理 IME、允许/拒绝完整生命周期或跨平台。
- Grok `1.0.41/grok-4.7`、普通终端 default：实际创建两张文件卡片，Enter 后未发送提示完整且卡片/正文不丢；零模型回合。该结果仅证明选择器和保留行为，G01 未接通前不算 G07 投递成功。
- Claude `2.1.280` 认证状态仍 loggedIn=true，但普通 TUI 首次启动向导未完成；本轮未发送模型输入。未改全局 Claude 配置跳过向导，托管模式已有证据范围不变。
- 本轮所有执行程序和缓存均在内置盘；应用 PID 55371 及受控进程树未发现打开的外置卷路径。旧实例误选截图、一次命令输入丢下划线导致的零模型启动错误分别保留操作边界，不能算新构建缺陷或有效验收。

### 仓外证据入口

本机持久归档根为 `/Users/zhishi/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/`。以下为根下相对路径；原件逐字节保留，认证工作目录未归档，跨机器不能仅凭本机路径认定已验。

| 证据 | SHA-256 与范围 |
| --- | --- |
| `g07-picker-gui-20260926/index.safe.json` | `1432fbbf90e6b9b0e82a2cffb6826e6082f112c705c74d07eb872e6ca6d59162`；43 文件，GUI 原图、Codex 原始失败/成功会话、内置执行路径与本地门禁 |
| `input-final-local/commit-binding.safe.json` | `84102bb1c` 的 20 个构建源文件与 Git blob 一致；本地门禁原件与运行程序摘要同目录保存 |
| `claude-gif-84102bb1c/receipt.json` | 收据 `8de6ba3946bfb5b4108516898990d49670e55f7712400b3ec46a69283538f5e6`；同目录源码绑定 `88935b4822d11f13a5c0579be07390ef0fda7751e82f83fd9fc64625f95df5d7` |
| `gui-input-84102/index.safe.json` | `53054d274bee66768574f1a556d1c6cd4b3270992e740b57f968bed2cb8a8ef5`；27 个文件，含 JPEG／纯 PNG 原生行、SQLite 投影、英文 GUI 和独立恢复证据；scope `1b7f054843239e966dcc9c6da9a58c93d6e1d4b55432583ee6525856882a2b88` |
| `claude-skills-adapter-3b4d8e8a3/index.safe.json` | `fb448a1a509ec50871993eda2c65c74dd5ff2c5e5c72fd839de244270379b0ba`；12 个原始文件，三轮真实适配器及模块失败/复核日志；scope `6f016b661f7ff58eedabb154b36fcd6abd2098e64cbe68f56a9f979ee818b8ed` |
| `claude-image-skill-integrated-wip-v1/index.safe.json` | `24a2b2367eb5125b05842e1793a11eda54129a8854b6c279492487660f4c7b85`；G05 20 文件，收据 `a1a015dec9718047f26cea2d4cffa0819e3a752c4c43527a548ff18b27a8feb1` |
| `cloud-input-84102/index.safe.json` | `abe10089eb66d5518a7f49bf572e669bd4e4aa1665519bb913c9df70e7b6f282`；Linux 23 文件；采集时 Windows 尚在运行，旧索引不回填 |
| `cloud-input-84102/windows-job-108149210250/index.safe.json` | `181418248d53df176440faabc6a56c74341ee53511451ef58e007ea12e830384`；Windows 46 文件、8 个 Actions 产物摘要一致，整体失败 |
| `internal-integrated-gates-20260926/index.safe.json` | `785f001b60cbb04bbf243261b6981c436e3be6f91809d19d3f7654e1f733933a`；29 文件，fixed 与 replay-fixed 的各自源码、构建和门禁；不是最终提交/云端验收 |
| `g07-picker-negative-20260926/index.safe.json` | `f41dad825484fc447a7fe03a534808011341eaa97b1143d591da4e2475ae5ea1`；原截图/AX保留，类型过滤判断已撤回 |
| `g07-picker-recheck-20260926/index.safe.json` | `653e648cd37a62a1fa1b74d19f726d3edde34af6a20788c0e097b70cbc926b7e`；键盘可选的反证及路径正文无卡片的真实入口缺口 |
| `claude-recovery-local-gates-20260926/index.safe.json` | `c446536dc2eb113f75941666bc562963619aabc48ec975799ba4064e0a9baa0a`；36文件，三轮各自源码/门禁、主动中止、最终ACK故障回归及程序摘要 |
| `gui-claude-hot-skills-20260926/index.safe.json` | `a13e13a571adec83aae6361398313dda3e7cf47342301c091f1840d70079d737`；70文件，三轮技能/图片原生历史、重启原失败、修复后零重投重关联、正常清理及英中原图 |
| `gui-grok-two-images-20260926/index.safe.json` | `64726f010ae77e59c480df88efc704fb5597b984832807c94c5bc755135bd735`；47 文件，双图、两个独立恢复模式、原生字节、两代清理与 macOS 双语滚动区 |
| `grok-g06-leader-default-20260926/index.safe.json` | `c9f7a13d0ca0c63fe4b22d4f78254c390b2dbcce494641b82b11390871bc6413`；368 文件，默认 leader 多技能原生调用及跨会话 reload 作用域；不计产品接通 |
| `g10-coordinator-post-fix/index.safe.json` | `8d30cf54cc0fa3d6c39a0037a020787229364200950aea3d28dd7408f718b42c`；41 文件，V2／V1 父上限两条生产链、投影原失败及零新增模型重验、四代完整清理 |
| `g10-coordinator-pre-fix/index.safe.json` | `6e7921157149f50062b7e39c76c36cebf86142149a8a2e54fb2c7b853dbd6a3e`；V2／V1-ceiling 原失败、源码绑定和补救清理分别保留 |
| `gui-grok-pre-fix-20260926/index.safe.json` | `abaf004d0efefb645718add14529c0575696f087ec8d25ea103414ffdff1a1e7`；GUI 图片卡片正例及握手负例、原始未确认输入与已确认清理，零模型输入确认；不计物理 IME |
| `grok-setup-mode-1041-20260926/index.safe.json` | `08a82a439f5e6cf1e85e912f0e07b9d41da7d5eec73e5fea2318ecb48f3d74cc`；8 次零模型原生模式探针，缺省 direct 负例与显式 leader 正例，56 个索引文件 |
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

## 2026-09-26 Grok 多技能与会话内新增增量

本增量基于 `83e5115839b301697f3322092f487ddce9d72dfe`，仅开放 Grok `1.0.41/grok-4.7`、Inherit、无 profile/model 覆盖／本地父子工具的私有独占 leader，最多 32 项本地技能。首技能原生展开，其他技能由原生工具读取；不保证工具严格串行。热新增等待准确 reload ACK、同会话目录及路径／内容核验，原生接收与技能清单落盘分别持久化。

macOS arm64 生产适配器 v3 三轮通过；真实 GUI v3 首轮双技能、v4 同会话新增 gamma、两次应用重启重新关联活跃任务、停止旧进程后新进程继续同原生会话并执行 gamma/beta 均通过。三轮原生输入、技能正文或 read_file 原字节、模型最终标记、NativeProtocol ACK 与产品结果逐项核对；零自动重投，两代退出码 0、资源组和后台任务清理已确认。v4 原始构建 SHA-256 `29278201780c87a6869efc34729d16e40d71025ea2995bdd0093568df6e3c599`，内置盘签名运行文件 `9ff3a223d5d45610f9ade22e4836c4c2576708aecfcb7d783e3c7c17827fb939`。两次重启无权限弹窗；英文与简体中文长说明、权限、技能卡、消息及结果完整滚动区检查可读。

本地通过：i18n 11、Grok 454（15 个在线专项默认忽略，三轮专项另有真实运行）、协调器 88、本地技能 12、宿主 31（1 个在线专项忽略）、任务面板 59，`cargo check -p warp` 与应用构建。v4 仅修正界面将已完成输入关联误判为忙碌；未变协议／持久化测试按源码摘要复用。v3/v4 的源码、二进制和各自运行范围独立绑定，不冒称单一二进制全量重跑。

保留旧握手阶段顺序失败、未信任项目技能目录失败、热新增入口隐藏、输入工具丢中文的未发送草稿及第三轮尚在运行的中间快照；最终成功不覆盖这些记录。原始日志、截图、负例、审计和源码索引共 284 文件位于仓外 `resume-20260925/validation/grok-g06-20260926/`，`index.safe.json` SHA-256 为 `3606b77b9c1e716e1d85f4a1705213dea5170c8b4ca694ea029635482acb3710`；对应实现已提交为 `fe135e3436c22eaa7857c31bfaf94969eb1aa11f`，独立提交绑定 SHA-256 为 `3419077aa75a1af26e05c9f5114c43cf8190ac179df1ae11ff27b7937dc2e9d2`。[Actions 36213944594](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36213944594) 的同提交 Linux/Windows 定向门禁均通过，47 个原始日志及产物归档于 `validation/cloud-g06-fe135e343/`，索引 SHA-256 为 `323bb6af8d3df103331f0069657e2c6c30bafeee60e7b54e0a85ee4f4403fcfb`。两平台已认证模型与 GUI、用户级来源和固定权限模式仍未完成；G06 及原 Goal 保持未完成。

## G01 owned 普通终端基础增量（2026-09-26）

固定 macOS arm64 Grok `1.0.41/grok-4.7/default`：应用隐藏入口在真实 shell PTY 核对设备、父进程和前台进程组后 exec；生产 leader 侧车和 SQLite 一次领取完成两轮中文输入，每轮均有独立原生 prompt ID、精确最终 ACK 和 hook 模型标记。再次提交同一消息被持久层拒绝，恢复旧启动记录不能再次派发。PTY 身份直接从现有 master 文件描述符查询，不改变启动服务的二进制消息格式。插件 `0.1.5` 增加原生权限观察；通知本身不认证进程、不承担审批。

原始证据归档 `~/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/grok-owned-20260926`，146 文件索引 SHA-256 `d49082ee7e6290bf007f2b316c3db5c96b80967fcc40377e38df7cf407fbec33`。v1 两轮输入通过，但运行器关闭 PTY master 的顺序导致 shell 回收卡住；代理收尾及原失败均保留。仅修改验收运行器，v2 57.04 秒完成两轮和 TUI／leader／shell 回收，私有认证副本已移除。独立检查确认恰好两个原生输入和两个正确标记，SQLite 两条消息均已 ACK。复用相同程序和 libtest 摘要，未改产品源码来放行复验。

本地 v7 `cargo check -p warp`、i18n 11、PTY 内核身份 2、CLI 会话 413（6 项专门环境测试跳过）及应用构建通过；独立运行随后显式执行其中一项原生在线测试。未变更的持久化模块复用 v4 78 项，修复后的运行器 6 项通过。构建源文件摘要已逐文件绑定实现提交 `922308af60cbdd7f84ea69fb3e465ae08b303213`，独立提交绑定 SHA-256 `7e88b8fea31de65fa5d58639f8295083cebb78d67b706349d4b3c17579432ed5`；[Actions 36220234098](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36220234098) Linux 通知/安装验收因脚本仍要求 0.1.4、实际插件为 0.1.5 失败；Windows 仍在执行。旧失败另存，修复后同提交复验。**不是 GUI、真实 IME、工具审批或跨平台在线生命周期验收。G01 保持开放，尚不能解除普通终端无条件拒绝。**

无需本地化变更：该增量尚未改变消费者入口和现有显示文案；后续 GUI 接线须同步英文／简体中文并分别检查实际布局。

本次恢复增量在上述基础上把侧车事件改为异步接收；真实退出先留清单再释放占用，SQLite 读盘前同步置忙，后台恢复仅关联已有启动占用，并隔离旧回调和重复定时器。普通 TUI 记录禁止进入托管新建、继续、自动重连及未确认宿主统计，避免第二个原生进程或重复模型输入。

macOS arm64 v4 `cargo check -p warp`、i18n 11、PTY 身份 2、CLI 会话 423（6 忽略）、持久化 78、完整协调器 111（4 忽略）、运行器 6 和应用构建通过。恢复专项原生两轮 53.09 秒通过；独立审计核对恰好 2 次 prompt_submit、2 次 stop、独立 prompt ID、正确模型标记及 SQLite 两条 NativeProtocol ACK。TUI、leader、shell 退出和私有认证副本删除确认。构建二进制 SHA-256 `f630b6362389f021b9b1c98578378adfc2e7767e220fc039572707d3ef606e02`；libtest `5f3238e4d6b34777deff6b08384d0b38e2ebb8118f514800b17dccf0cbdd07bf`。

内置盘签名后 GUI SHA-256 `0e5d6f237062bbd99a3b810e33a9db8f4b47a205c47f9d8c2ccf67664ca6e174`，独立空白 profile 英文/简体中文设置与 Grok 版本区域实际可读，未出现人工授权弹窗；两测试 GUI 进程均正常退出。无需本地化变更：内部恢复和通道调整复用既有文案。本次不是 GUI 输入、真实 IME、工具审批或有模型会话的应用重启验收；GUI 新启动登记/派发/握手、启动前期异常回收及平台验收仍欠。

原始证据 159 文件存于仓外 `validation/grok-owned-recovery-20260926/`，索引 SHA-256 `0c1fbc47dec6f0a7cd9c2bc7c8c6a6cc59c73a707dc0c95518096cb4386f8402`，含各轮源码和二进制；提交后追加独立 Git blob 绑定。v1 Timer 返回类型编译失败、v2 门禁通过后主动中止构建、v3 未确认宿主统计遗漏的失败均保留，v4 修复后复验。首张名为 chinese 的图实际仍为英文，重启后的中文正例另存，未覆盖原图。G01 与原 Goal 均未完成。

恢复实现提交 `044263a61b38fa95af32de2b758c1101aae08226` 已逐文件绑定上述 11 个编译源文件，提交绑定 SHA-256 `3491a456e67a69582ee2333d8314a78bdb7dca4e46c5f6a0deef4a0d0f1b6338`。基础提交 Linux 的两个失败门禁已定位为 Python 仍硬编码插件 0.1.4：实际安装步骤退出 0，原始 `passed=false` 收据保留。`4ed5b3d80` 同步严格目标至 0.1.5，旧版/未知版/缺失/混合版本均拒绝；本地验证器 15 项、真实原生 worker 17 项中 15 通过/2 个缺少 shell 场景跳过。稳定步骤键沿用历史名称，真实目标由 `current_plugin_version` 核对。没有重跑 Linux 安装或回填旧失败；整合后同提交跨平台复验。无需本地化变更。

G01 与原分支 G09 整合后再次通过本地 `cargo check -p warp`、i18n 11、PTY 身份 2、会话 423/6 忽略、协调器 111/4 忽略、升级 182/4 忽略、受监督版本探测 7/1 忽略、持久化 78、运行器 6、插件验证器 15 及应用构建。105 文件索引位于仓外 `validation/g01-g09-integration-20260926/`，SHA-256 `67b875a435b5408373166375e16adbe24b65cdeaa485b8f72146491666f18c64`；没有重跑在线模型，原先各轮来源不变。整合提交将从原分支执行同 SHA 跨平台门禁。
