# CLI 集成的支持与回退说明

**2026-10-04 当前 Goal 范围**：用户将其他平台实机验收移交后续执行，继续补齐原生能力和 Mac 未验项。当前 G01／G02／G03／G04／G05／G06／G07／G08／G10／V03 共 10 项在本次范围关闭，G09 仍开放，V01／V02／V05 移交用户；PR 保持草稿，最终同提交相关源码门禁仍须完成。G02 已由固定 Grok `1.0.41/grok-4.7`、Inherit 的基本图片历史正例及 `d290fd713` 图片＋单技能／热新增双技能／冷恢复组合真实链满足 Mac 条件；G04 已按固定 Claude `2.1.280/claude-opus-5-5`、Inherit 的真实 Mac 证据关闭；G05 已按原条件补齐纯图片、同轮图＋单技能及恢复／保稿原件并独审通过，历史失败保留；其他平台未改记为通过。G04 支持 PNG、JPEG、静态 GIF、WebP；G05 的纯 PNG 与 PNG＋单技能按新建、同会话冷恢复及 GUI 恢复分别计证，不外推所有格式排列或固定策略。其余状态见 [当前范围](KNOWN_GAPS.md#2026-09-30-当前范围)；以下早期记录保留各自源码和边界。

**2026-09-30 G06 当前实现与验收**：固定 Grok `1.0.41/grok-4.7` 的 Inherit 私有 leader 已通过 local/user 混合多技能、会话内 user 技能热新增和同原生 ID 冷恢复的 Mac 生产适配器验收；多技能按原生已确认目录附带所选来源限定名和原路径，保持原文本／图片顺序，不复制技能正文。独立 `GrokRestrictedSkillsV1` 已通过精确 Skill 拒绝、允许、同会话冷恢复与未选技能零派发；固定读取／文件策略仍禁技能，原生 shell／hooks 不扩权。两条链各两代自然退出 0；固定受审 no-leader stdio 的 EOF 收尾现有专用有界宽限，取消与断连沿用原合同。英中说明和布局已审，`625ffcb98` 两平台源码门禁成功，G06 在本次 Mac 范围关闭；Windows 桌面用例 1 条 LEAK 警告保留。此处不声称真实父子子任务、命令／邮箱整链或 OS 沙箱通过。

2026-09-26 用户明确目标平台为 macOS Apple Silicon、Linux x64 和 Windows x64；macOS Intel（包括 Codex CLI）不属于当前开发或验收范围。历史 Intel 记录保留原范围。

> **2026-09-25 完成结论更正：阶段交付，完整 Goal 尚未完成。** 已通过的固定版本功能、三平台相关回归及原始收据按固定 Git 提交追溯；功能缺项、模式限制与未覆盖验收不计为完成。统一待办见[已知缺项](KNOWN_GAPS.md)，当前机器可读状态见[CURRENT_STATUS](CURRENT_STATUS.json)，阶段合并范围见[当前状态](CURRENT_STATUS.json)。本说明依据固定基线的实际支持范围，不因目录精简扩大能力，也不表示已经合并或发布。

本文件说明当前分支的产品支持范围与回退行为，不表示已经发布。固定版本的功能证据见 [验收对应表](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/ACCEPTANCE_FIXED_VERSIONS_20260924.md) 和 [Mac 交付记录](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md)；历史对应提交的三平台必要补验已通过；输入实现提交 `84102bb1c687f87a2425bc1937784e77250c416c`尚未取得最终 SHA 门禁，实际范围与来源以 [验证结论](VALIDATION_REPORT.md) 为准。不同提交的历史通过不能改记为最终提交通过。

2026-09-26 功能代码检查点为 `704bb33bb2943f8a686b24b843c3b3a1ba656c19`：三平台专属 Grok 普通会话 PNG 输入、Codex 远端图片队列、收起编辑器时的文件卡片入口、Codex 三目标平台 npm／Homebrew／Claude WinGet 来源升级、子任务搜索与 Claude/Grok 受限技能已写入代码，Grok 远端专属会话/图片/重启关联已接，Claude 远程原图经原生 Read 和历史消费回执的链路已接；Codex每pane独立app-server/明确socket票据绑定、Claude/Grok Mac/Linux受审命令及命令+技能组合、Claude Windows npm、Grok三平台npm和Mac ARM Homebrew双别名也已接入；Codex WinGet完整目录/安装登记/依赖事务已接；Windows受审命令MCP与Job清理也已接；Grok普通终端同UUID历史继续和Claude Unix npm固定版本受限降级合同也已接；当前渠道不匹配仍拒绝。三款 Linux x64 Homebrew cask 与 Grok WinGet 固定来源事务现也已接入；G10 同会话顺序多命令和运行中原生追加也已接入；三平台命令各用独立监督代，真实清理后才能继续。该检查点已通过 macOS arm64 编译与 i18n 门禁；后续修正提交 `b78b62a48223235e8c29157e3786747c8db2ff68` 的 Linux／Windows 同提交 `cargo check` 均已通过。最新定向回归与夹具复验见[最新门禁](VALIDATION_REPORT.md)，这些门禁不代表完整产品验收。真实功能、GUI／模型与双语布局验收仍待补，不扩大以下支持承诺；最终仍需绑定提交、平台、版本及模式验收。

2026-09-27 G09 增量：`39941b281` 已接私有 npm 四场景验收入口；后续 `2b59c8c62` 新增 Linux 候选隔离能力前置与英中提示。本地门禁通过，Mac ARM Codex `0.155.1→0.156.1` 与 Grok `1.0.40→1.0.41` 的私有正常升级分别按各自源码收据通过；另有 `39941b281` 的 Linux Grok 正常升级、交换后恢复、外部改动保留、候选改动拒绝四场景通过。其他平台恢复／故障、消费者渠道、GUI 与 Homebrew／WinGet 真实事务仍待验，G09 和 Goal 不关闭。详细结果、原失败与收据集中见[验证结论](VALIDATION_REPORT.md)及[当前状态](CURRENT_STATUS.json)。

`56f6da217` 已修正 Linux 官方 Node 大 ELF 字符串表误拒，同提交 Linux 原文件依赖闭包通过；新增 Windows Codex npm 五场景入口仍在真实私有 npm 登记阶段失败，尚未进入产品事务。两平台编译与定向 Rust 通过，离线夹具修复已另提交；G09 未关闭，详细范围见[验证结论](VALIDATION_REPORT.md)。

`5aac3f7d0` 已补远程 Claude 延迟图片回收与 Grok 明确未派发后的主动重试；`bd80cc9ed` 已接 Windows Codex npm 两组固定入口模板。新增 19 项回归及本地编译／英中门禁通过，同提交 [验证 36266539274](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36266539274) Linux 通过，Windows 编译／定向回归通过但真实 npm 首个 CMD 候选探针拒绝未绑定子映像，后续场景未执行；实际远端／GUI／模型链仍待验，G08/G09 不关闭。

`ca37b9cbf` 已补 Windows npm 拒绝后的显式 Job／ACL／profile 清理与脱敏映像诊断；本地编译和 i18n 门禁通过，仅 Windows 的[同提交验证 36270700886](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36270700886) 因 Win32 事件码字段类型错误在编译阶段失败，原生场景未运行；字段修正待新增量复验。候选拒绝仍需定位，未扩大白名单或宣称真实升级完成。

`43c9709d2` 的 `console-binding-04` 已补 Windows Codex npm 精确控制台依赖，文件租约、同 Job、同 AppContainer SID、零 capability 与真实退出确认仍强制。同提交 Windows 编译及普通／原生边界回归通过，但真实 npm 首个 CMD 候选返回 `RecoveryRequired`，stderr 报工作目录不兼容及拒绝访问，清理未确认；发布、PowerShell 与恢复未到达，不扩大消费者支持承诺。本地 check、i18n 重试02（11项）和 actionlint 通过，首次空间不足失败保留。复用英中探针失败提示，无需本地化变更；修复及 GUI／在线联测按用户要求后置，G09 不关闭。

Windows npm 工作目录后续增量已在 `05b0b8faa` 提交（基线 `6408131e2`），仅修执行路径表示，授权对象和隔离门禁保持；本地 check、i18n 11项、actionlint及差异检查通过，同提交 Windows [CI 36280073722](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36280073722) 已结束：主组981项、command7项、显式atomic7项通过（含新增5项），真实更新首个CMD候选仍失败。旧UNC提示消失，但原生“拒绝访问”后缺完整清理收据；剩余进程和拒绝对象尚未定位，PowerShell、发布与恢复未执行；不改变消费者支持承诺。无需本地化变更，完整 GUI／在线验收仍后置。

`0525c9439` 的协作取消已接入原认证通道，本地check、i18n 11项及control6项通过；同提交 [Windows CI 36282335500](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36282335500) 编译通过，普通回归986项通过/1项失败，失败为Drop的2秒回收等待（nextest既有3次自动尝试）；新增原生取消2项均通过、atomic9项和command7项通过。真实npm首updated返回Network、未形成journal/generation或候选执行/清理收据，不能继承上一轮cleanup=false，发布/恢复未验。后续修正 `49ea40561` 已提交推送，监听改为非阻塞读取及可唤醒等待，保留6项契约和2秒断言；独立审查、本地02 check、i18n 11项、control6项及actionlint通过。同提交 [Windows CI 36283908796](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36283908796) 已结束：编译通过，主组987项、control6项、command7项及原生atomic9项通过；Drop首次0.112秒通过，原生取消2项也首次通过。真实npm首updated仍返回ProbeFailed，原生拒绝访问后因取消中断；本轮已取得Job／ACL／profile清理标记及cleanup_confirmed=true。无原生根退出码，外层worker码1不代替它；PowerShell／发布及后续恢复场景未验。无需本地化变更，G09及Goal保持开放。

**Windows npm 诊断增量 `ae4279e48`**：固定下载角色与失败阶段、已核验 CREATE/EXIT 角色和清理模式已接入，授权、隔离及失败判定不变。本地 check、i18n 11项及 actionlint 通过；同提交 [Windows CI 36285301933](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36285301933) 已结束：Windows 编译、主组990项、command7项、原生atomic9项通过，真实 npm 首个 updated 返回 ProbeFailed。正常阶段仅记录 root／console CREATE；取消清理阶段记录 root／console EXIT1，另有尚未分类的 unknown CREATE／EXIT3221225738，不能据此认定 Node 从未启动或定位拒绝对象。最终取得 cleanup_confirmed=true 及 AppContainer 清理标记；PowerShell、发布与独立恢复未验。CREATE/EXIT 与 root_exit 使用同一 session 时钟，probe 阶段仍用自身起点，不直接相减。按用户代码优先、修复与联测后置要求封存本轮，不自动重试或追加猜测性诊断；G09和Goal保持开放。无需本地化变更。

`debed8c51` 补接有效专属 Grok 的上下文草稿队列及 Linux／Windows 升级空闲通知，英中提示已同步，本地门禁通过；真实三平台／远端发送、双语布局及同提交平台结果仍待补，G01/G09 不关闭。

仅 Linux Codex npm 和三款 Linux Homebrew 在确需更换版本时要求 Landlock ABI≥3；不足时不生成更新计划，原 worker 硬门禁保留。Grok npm、其他来源、macOS／Windows与同版本无候选执行的检查／配置同步不受此门槛影响。`2b59c8c62` 相关平台 CI 和实际双语布局待补；旧 `39941b281` CI 的 Linux glibc ELF 失败与 ABI1 事实分开记录，Windows 作业成功但保留一项 `LEAK`，均不外推为完整通过。

Homebrew 范围仅为已登记默认前缀的固定 cask（Mac ARM `/opt/homebrew`、Linux x64 `/home/linuxbrew/.linuxbrew`）；当前没有可据以开放 formula 的固定官方合同。`unshare` 缺失与 `WinGet.exe` 不在 PATH 均不能代替内核能力或 portable 来源事务验证。

Mac ARM64 历史联测包为内置盘 `InfiniShellParity-2d159360a.app`，产品源码与 `ae4279e48` 一致；英文／简体中文资源内嵌、独立测试 profile、签名及摘要已核验。该构建记录时应用尚未启动，不作为 GUI／模型验收，也不表示当前最新验收程序。构建、缓存归档与原始失败范围见[验证结论](VALIDATION_REPORT.md)，路径和摘要见 `CURRENT_STATUS.json.current_macos_joint_test_bundle`。

2026-09-27 `ba73c4df3` 的 Grok 图片与技能组合已接线：固定 `1.0.41/grok-4.7`、Inherit 根会话、私有独占 leader，按当前会话目录核验所选技能；排队及刷新后再次核对并保留失败草稿。Mac ARM64 原生两轮仅证明单技能＋PNG及冷加载同历史后的双技能＋PNG，图片原字节、技能读取与识色分别通过；适配器、GUI、热新增组合、权限交互及 Linux／Windows 仍待验。英中说明同步，布局后置。

## 版本与运行方式

本次固定受测版本为 Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41`，本轮不因上游继续发布而更换验收目标。这些版本不是所有未来版本的兼容保证。应用分别检测命令、安装和版本；安装成功不能证明已经登录或获得模型额度。

普通终端使用 CLI 自己的交互界面。应用托管任务使用本地任务窗口，记录输入、接收确认、审批、结果和历史。两种方式分别验收；普通终端插件的通知不能替代托管任务的完整结果。

InfiniShell 在 macOS、Linux、Windows 的桌面入口默认启用 `LocalCLIManagedTasks`，三款 CLI 共用本地任务入口，Grok 无需额外编译开关。共用库和 TUI 保持独立开关；默认开放入口不改变版本、权限模式、原生能力及来源核验。

CLI 升级后若版本或能力没有验证，应保留普通终端入口和本地记录，只开放已经证明的操作。Grok `1.0.41` 已进入正式托管路径，按权限模式区分能力：

- 继承 CLI 设置：支持固定 `1.0.41/grok-4.7` 根任务，私有独占 leader 下最多 32 项技能；原生会话、目录及路径核验后才能发送，热新增须在空闲时完成。Mac 已验多技能、local/user 来源、热新增、同原生 ID 冷恢复及 PNG 与单／多技能组合；GUI、模型轮次和恢复程序分别绑定原构建，不称当前程序全量重跑。其他平台实机验收移交，G06 本次 Mac 范围关闭，源码门禁及其 Windows LEAK 警告见验证报告。不开放父子任务工具。
- 固定读取／文件策略：支持对应文件操作、审批、恢复，以及满足父权限上限和 SDK 合同的父子任务与双向消息；禁用原生 shell、hooks 和技能，不提供任意工具或操作系统沙箱保证。
- PNG 托管图片支持已核验 `1.0.41/grok-4.7` 的 Inherit 根会话；与单／多技能组合时还要求私有独占 leader 和准确目录确认。Mac 生产适配器组合真实链及基本 PNG GUI／恢复分别通过，G02 本次范围关闭。未知版本／模型与固定权限图片组合仍拒绝，其他平台实机移交；目录、技能、权限或会话身份未通过核验时保持不可用。

## 输入与技能的实际限制

- Grok 专属普通会话的富输入自动提交与 PNG 图片投递已有各自历史证据；Mac 普通 TUI 的定制 `.3` 文本桥另有标准首页真实 GUI 正例，未核验工件仍拒绝。关闭富输入后的文本粘贴最新修复仅打开并恢复／追加本地草稿，不插入原生编辑器、不自动发送，英中合并草稿明确发送已有正例；中文回退提示缩短后的最终布局也已通过；不能使用桥时可关闭富输入后在 Grok 中直接键入。普通 PTY 图片按下述固定 `.5` 合同支持，托管 PNG 按 G02 的独立范围支持。
- G01 原生桥已基于 [官方 grok-build 源码](https://github.com/xai-org/grok-build) 实现，仅用于从普通 shell 启动的独立受审定制工件，官方发行版不含该桥；此前“无可修改源码、需等待上游”的判断已更正。该仓库按 Apache-2.0 支持本地构建，不接外部 PR 不阻止本地补丁。当前固定公开 `1.0.41` 快照 [07e35a3dfeed2f200d319ef6c893b5ea286d9a51](https://github.com/xai-org/grok-build/tree/07e35a3dfeed2f200d319ef6c893b5ea286d9a51)，`SOURCE_REV=84745de98b3d3996729aefcefd518890ffb73930`，不同于已验发行版 `1.0.41 (4220f3b224a6)`。[原生提交事务补丁](../../native/grok-build/README.zh-CN.md) 的 Mac `.3` 独立构建已有 74 项原生回归通过；Mac 普通标准首页 GUI 已取得英文、中文长文本、审批保护、明确同文新一轮及同会话冷恢复的八条真实输入与结果证据。该能力仅对应已绑定定制工件，不能宣称官方发行版已支持。随后文本粘贴修复本地门禁及英中各一条真实提交通过；中文手动回退提示截断已通过单行缩短及新工件布局复验解决，Mac 功能与英中审计满足；Linux 普通桥宿主接入、Windows 原生传输与宿主接入均已实现，三平台分别绑定已核验 Mac `.3`、Linux `.6`、Windows `.11` 工件。最终两平台宿主源码门禁 `36796894945` 已逐名覆盖账本各 19 项及必要桥组，G01 在本次 Mac 范围关闭；用户仅后置其他平台实机验收，不免除实现与最终源码门禁，详见 [当前缺口](KNOWN_GAPS.md)。
- Grok 普通 PTY 图片要求已核验的定制 `1.0.41+infinishell.session-notifications.5` 工件。Mac 纯 PNG 粘贴、拖入富输入或上方终端、多附件顺序、焦点／审批保护及失败保稿已验；新会话未先打开富输入时，首拖即可展开并保留图片卡，已由用户确认。图片先进入富输入，明确发送后通过原子桥投递到正确原生会话；拖放本身不会自动提交。三平台原生实现与工件、相关源码门禁及 Mac 英中布局齐备，G03 在本次 Mac 范围关闭；不将官方未绑定工件或其他平台实机改记通过。
- Claude `2.1.280` 托管输入已扩展 PNG、JPEG、静态 GIF、WebP，允许纯图片及精确登记的单技能组合。格式及纯 PNG 已有 macOS/claude-opus-5-5 生产适配器字节、识图和冷恢复正例；图片加单技能已有当前工作区生产适配器新建与冷恢复正例，两次精确 Skill AllowOnce 与图片字节/识色分别计证；后续 Mac GUI 的 PNG加单技能、应用重启同宿主重关联及清理已有正例，原生历史未重复；跨平台仍待验。GIF 单帧校验也约束旧引用，不改变 Codex 原格式行为（G04／G05）。
- 固定 Claude `2.1.280` 当前增量已接通无图多技能与会话内新增；必须等原生注册确认后才派发，失败保留输入并隔离旧回调。`3dbe79e58` 另接图片与已注册多技能，完整名称共用预算和编码；`2383428da` 的 Mac 生产适配器新建／冷恢复及签名 GUI 新建任务已取得 PNG 加双技能逐次审批、执行和结果正例；该 GUI 冷恢复未验。该历史提交保持普通 Inherit 单技能编码；当时固定技能画像的单技能图片前缀变化另待验，现有 Inherit 图片收据不证明固定权限图片组合。Grok 多技能按上述固定范围接入；固定 V1/V2 文件策略不能选择技能。Claude Mac 适配器有正例，Mac GUI 双技能及同会话热新增均实际 Skill 逐次允许通过，原生会话/runtime不变且累积技能持久化；热技能历史导致的重启误判已修，真实 GUI 重关联同宿主与原生进程且消息仍三条，正常断开清理；不同源码/程序分别计证。Grok 原生默认 leader 已取得多技能和显式 reload 后新增技能正例，但刷新影响同 leader 多会话，产品已改为仅使用私有独占 leader，真实适配器已通过三轮；GUI 双技能、修复后的热新增、应用重启重关联及冷恢复第三轮均有独立正例。不把零审批的原生只读探针当作父权限证明。跨平台仍待验（G06／G10）。
- 普通终端本地文件卡片只传绝对路径引用；发送前整批检查普通文件及当前账号可读性，失败保留草稿和卡片，由原生 CLI 按自身工具能力与审批读取。Codex／Claude 锁定 Shell 模式下的文件卡在写入 PTY 前拒绝，避免路径说明被当作命令执行。三款固定 CLI 的 Mac 普通 GUI 已有实际读取与原生审批拒绝证据；实际模型验收使用 UTF-8 文本，不承诺任意二进制传输或所有格式解释。三款 CLI 的 Mac SSH／tmux 图片消费、原消费者断连恢复、权限拒绝、重复／旧回调保稿及引用清理已按各原构建实测，G07／G08 在本次 Mac 范围关闭；远程普通文件卡仍不自动传输，其他平台实机不外推为已验。

各项源码依据、替代方式和完成标准见[统一清单](KNOWN_GAPS.md)。这些限制不能用“支持附件／技能”的概括性表述省略。

## 通知插件与授权

通过第三方 CLI 设置页安装或更新插件。自动安装只接受受控版本及来源；缺少运行时、插件禁用、自定义文件、未知版本或恢复失败都不能报告就绪。插件不可用时仍可使用普通终端。

| CLI | 本次受控配套 | 使用时的区别 |
| --- | --- | --- |
| Codex | `codex-warp 0.4.0` 与随附持久来源、固定通知修补 | 安装与原生 hooks 信任分别检查；在 Codex 内使用 `/hooks` 审核并决定信任当前定义。应用不代写信任摘要 |
| Claude | `warp 2.2.0` 与固定通知修补 | `2.1.0` 仅是已审计的升级来源；禁用状态和无关用户设置应保留 |
| Grok | 随附 `infinishell-grok 0.1.6` 与原生通知桥接 | 需要 Node.js 18 或更高版本；桥接补齐 `1.0.41` 初始 hook 加载，按事件去重，无需手动 reload；原生 `plugin update` 成功不证明实际文件已更新，应用另核对安装缓存完整性 |

Grok 安装状态与启用状态分开。已禁用的插件不会因修复而自动重新启用。受控旧版 `0.1.0`–`0.1.5` 可迁移到 `0.1.6`；同版本受控文件损坏可以由用户触发修复，未知来源或并发编辑不能被直接覆盖。升级失败时仅回退属于此次操作的文件和配置，无法确认所有权时保留恢复资料并报告失败。详细操作与各阶段证据见 [通知兼容说明](PLUGIN_COMPATIBILITY.md)、[Codex 持久来源](CODEX_PLUGIN_CACHE_REFRESH.md)、[Claude 升级事务](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/CLAUDE_PLUGIN_UPGRADE_TRANSACTION.md) 和 [Grok 完整性](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/GROK_PLUGIN_INTEGRITY_VERIFICATION.md)。

Grok 原生审批通知有时只有会话标识、没有回合标识。应用仅在可信的当前会话上显示“需要操作”提醒，不据此宣称当前回合阻塞或成功；工具完成或后续输入会清除提醒。插件通知不授予工具权限。

Codex、Claude 的独立通知修补可以导出供目标主机安装：

```sh
python3 script/cli-agent-parity/apply_notification_patch.py --export /absolute/path/to/new-export
```

导出目录必须保持完整，并核对 `SHA256SUMS.json`。SSH、容器和 WSL 应使用目标环境自己的 CLI 与配置目录，本机安装检测不证明远端已安装。Windows 的文件交付与原生通知另列验证范围；`--files-only` 不证明控制终端、Git Bash、审批或完整交互可用。

## CLI 自动升级与渠道

CLI 可执行文件升级与通知插件升级分别管理。三款 CLI 的自动升级默认开启，默认跟随已识别安装的渠道；用户可关闭自动升级或手动切换该 CLI 实际提供的渠道，偏好在应用重启后保留。

| CLI | 可选渠道 | 跟随安装的含义 |
| --- | --- | --- |
| Codex | `latest`、`alpha` | 保留已识别的正式／alpha 安装渠道 |
| Claude | `latest`、`stable` | 读取已有 `autoUpdatesChannel`；未配置时使用 `latest` |
| Grok | `stable`、`alpha` | 以原生检查返回的渠道为准 |

渠道名称保留各 CLI 差异，不把 Codex 的 `latest` 改名为 `stable`。显式切换渠道可能需要降级或同步同版本渠道配置；Claude `2.1.280` → `stable 2.1.267` 已有实际事务证据。切换与升级操作按用户选择修改相关渠道设置，不是启动 CLI 时改写用户全局配置。

macOS、Linux、Windows 已识别且通过来源与身份核验的三款原生安装接入正式升级事务；当前平台证据覆盖 macOS arm64、Linux x64、Windows x64。`b6f93f662` 已加强 Codex/Claude 的 npm 包登记、管理器、安装前缀与真实入口绑定，后续工作区已接入 Claude 固定目标 `2.1.280` 的 Unix npm 官方单包事务，本地门禁与双语错误布局、macOS 五个真实后端事务场景通过，同提交云端仍待验。该范围不包含 GUI 更新点击和模型生命周期。固定审核范围内的 Codex／Claude／Grok npm、Mac ARM／Linux x64 Homebrew、Windows x64 WinGet 事务已接线；未知安装／helper 来源仍手动处理，这些历史接线记录不等同新增来源验收完成。额外 ACL 仍拒绝；不能仅凭同名命令或安装目录猜测升级方式。版本兼容门禁与升级发现分别判断，发现更新不表示新版本托管能力已经受测。

当前 Mac 已按各原构建完成三款 CLI 的 npm／Homebrew 六类消费者更新、Claude 合法渠道降级、忙碌延期、配置失败回滚与重试，以及实际应用中断后正常重启恢复和插件复核；英中布局已验。Windows 原生 npm 的 CMD／PowerShell 五场景和三次独立冷恢复尚未通过；EOF 修复后的 run37200286598 已确认 PS 后代退出0但版本输出为空，真实更新失败，后四场景／三冷恢复未执行。G09 保持开放，不把后置实机验收等同实现通过。

Claude Linux x64 musl 的固定 `2.1.278/280/285/287` 安装合同、原生加载器绑定及 npm 平台恢复路径已实现并通过本机静态和回归门禁；真实 Linux 执行、更新、显式降级及新平台捕获的 Mac 事务尚待验证，暂不据此宣称新增来源可用。

活跃会话或启动保留期间延期升级，退出后再执行。事务核对来源、目标版本和摘要，保留无关配置，并记录失败回滚及中断恢复；无法确认所有权或清理结果时报告失败，不把进程退出码单独作为升级成功。已有正式三平台事务与历史负例见 [自动升级记录](CLI_AUTOUPDATE.md)，最终提交补验另见 [验证结论](VALIDATION_REPORT.md)。

## 审批、取消与消息确认

Claude `2.1.280` 的独立 V2 文件策略允许逐次审核项目内文件创建／覆盖 Write，禁止扩展 V1 父任务上限；历史 macOS arm64／claude-opus-5-5 的 V2 生产父子链已验证精确 Write 允许、拒绝、结果回收及清理，真实 V1 父任务扩为 V2 会在建立原生连接前拒绝，父记录不变。该轮没有覆盖待审批 Write 取消、活跃重关联、冷恢复、GUI 或跨平台，不把后续其他策略的正例改记为这一轮通过。

根任务默认继承该 CLI 的设置。需要审批时，允许、拒绝和取消作用于当前任务的当前请求；关闭连接后旧审批不能继续使用。Claude 固定文件策略是另一个显式选项，不是操作系统文件或网络沙箱；继承设置也不提供可以证明的完整子任务权限上限。

三款 CLI 已有各自限定模式的父子链证据；子任务仍必须满足创建时保存的父权限上限和来源约束。Claude／Grok 固定读取或文件策略禁用原生终端命令、自动 hooks 和技能；独立所选技能与受审命令策略另按登记集合、逐次审批及原生 SDK 合同判断。Grok 继承模式不开放父子工具。G06 的技能正例不证明 G10 完整父子、命令与邮箱链已验，不能当作任意配置下的完整编码子任务。不能为了避免隐藏审批而固定开启跳过权限参数，也不能通过修改用户全局 Claude 配置来免除授权。

G10 的父权限、命令／技能逐次审批与拒绝、越界、运行取消及清理、双向 ACK、冷恢复和结果回收已由各自原构建的真实链满足，受影响宿主审批英中布局已验。`04ac0d46f` 的两平台相关最终源码门禁成功，Windows 菜单测试 1 条 LEAK 单列保留；G10 在本次 Mac 范围关闭，不承诺任意模型／策略的全排列或其他平台实机已验，具体证据见[验证结论](VALIDATION_REPORT.md)。

“接收确认”只说明对应接收阶段。`native_protocol` 表示原生协议接收；`application_history` 表示记录已进入应用历史。二者都不独立证明模型执行完成。Codex 运行中追加、Claude 输入合并或后续轮次、Grok 后续回合排队分别保留原生语义。

取消请求发出或 interrupt 回复不等于取消完成。完整结果、可信取消、失败、断线和未知状态分别展示；普通终端 Stop 通知仅作为候选响应，缺少可信终态时保持降级状态。普通 Codex PTY 的 Escape 不保证工具进程退出，应用保持未知状态并提供显式关闭终端；托管进程树清理另以退出收据判断。Grok 审批拒绝后可能由原生返回 Cancelled，不能把这个差异改写成成功。

Windows 固定 Grok 曾在标准输入关闭五秒后仍存活，随后才自然退出；原生五秒 EOF 不作为可靠能力。应用托管结束以当前代次的监督回执和进程树清理为准，不能把关闭输入或最终根进程退出码单独当作清理成功。历史补验新增的真实 ACP 监督清理四场景已在三平台通过，结果见同提交验证报告；Windows 无 leader 两场景的退出码为 1，清理通过不计为模型任务成功，原五秒失败记录保留。

## 继续、重启与恢复

重新选择仍有活跃连接的任务应复用当前连接。连接已经结束时，历史继续会启动新进程，并明确使用已保存的原生会话 ID。应用不能同时为同一活跃会话启动第二个恢复进程。

应用重启后先恢复任务、父子关系、消息状态、结果和附件引用，并可重新关联身份核验通过的存活宿主，包括已完成当前回合的宿主。恢复记录不会自动重投旧输入；用户明确提交的新输入才进入后续执行。历史会话不存在、项目路径失效、版本不兼容或原生拒绝恢复时，应保留记录与错误，不能悄悄创建新会话来冒充恢复成功。

普通终端展开富输入后的“附加文件”以及从收起态打开选择器均已在 Mac 普通 GUI 实际验收，保留 CLI 输入模式和异步回调代次校验。固定 Codex `0.156.1`、Claude `2.1.280`、定制 Grok `.3` 均已通过文件卡投递、原生实际读取与独立原生审批拒绝；拒绝以精确工具结果和原生终态判断，不把 UI 标签或安全拒绝单独当作成功投递。Claude 的 `142c635412` 新构建另完成共用文件错误 Toast 英中完整提示、长文件名、草稿与双卡保留实窗验收；Grok 和旧 Codex 正例仍按各自原构建计证，旧入口及失败原件保留。`e3d5e58a0` 的 Linux／Windows 相关最终源码门禁已逐项通过，G07 在本次 Mac 范围关闭；其他平台实机由用户后续验收，详细收据见[验证结论](VALIDATION_REPORT.md)。

固定 Claude 热新增技能只凭同代原生确认扩展恢复清单；先持久化原生回执再保存清单，崩溃后的重放不重新发送输入。真正的任务或原生会话身份不匹配会保留已完成结果并隔离连接，旧进程后来退出也不会自行解除隔离。

发送未确认、排队待执行和已执行的消息需要分别处理。重试不能把已执行的副作用再次发送；取消后的旧回调和消息也不能进入新代次。结果回收要求当前任务与原生回合对应的完整结果，不能仅用进程退出码、最后一段文本或目录注册代替。

## 平台与验证边界

macOS 已保留三款固定版本的普通终端、托管生命周期、父子消息、真实 GUI 恢复、附件与技能以及中英文布局证据。Linux／Windows 已有桌面工作区单元测试、相关原生安装与升级；历史 `ee839b4fc` 的两种语言独立 GUI 进程、系统剪贴板及原图目视也已通过，所见三款选择按钮、中文字形和附件标签可读。截图选择 Codex，不证明 Grok `1.0.41` 全部权限说明或未滚动区域；也不等于在每个平台重跑全部模型交互。该历史默认入口改动与对应提交门禁共同计证，不能替代本次输入增量验证。

- 实际中文输入法组合输入在 macOS 验证；Linux／Windows 的系统剪贴板、中文字形及截图不代替物理输入法验收。
- Codex GUI 已证明类型化图片进入原生请求，该轮模型颜色判断错误不能计作理解正确；Claude 历史 GUI PNG 有字节和识色证据；本次 Claude 其他静态格式/纯 PNG 的生产适配器、84102 GUI JPEG/纯 PNG，以及后续 Grok 双图 GUI 正例按各自源码列证，不外推其余格式或平台。
- SSH／tmux 是 macOS 到本机隔离 OpenSSH、固定三款 CLI 与实际 PTY 的验证，包含断连再接回；不覆盖其他远端操作系统、容器或 WSL 的所有组合。关闭 tmux 透传的公共解析器风险验证复用 Claude／Grok 正例，Codex 原生发送未独立观察的边界保留。
- 历史失败、跳过项和旧构建来源均保留。历史产品冻结提交的相关三平台门禁见 [同提交补验](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/FINAL_SAME_COMMIT_VERIFICATION_20260925.md)。未变功能保留分阶段实际来源，跳过或不支持的能力不计为通过，也不据此宣称已经发布。

历史默认开放增量曾随三平台桌面入口，同步修正英文和简体中文的 `cli-task-manager-claude-verification` 文案：移除功能开关及待验证表述，明确托管任务支持 Claude Code `2.1.273`、`2.1.278` 和 `2.1.280`。这替代此前“无需本地化变更”的结论。真实 GUI 用例按 `en`／`zh-CN` 分别启动独立进程，分目录保存截图和实际界面语言收据。macOS 已核对默认入口、新说明和 Grok 权限按钮的双语可见布局；Linux／Windows 四张原图分别通过上述视口范围的目视检查。其后的历史进程清理补验没有修改产品文案；这不适用于本次输入增量。

本次输入增量已同步英文与简体中文图片/文件说明；macOS 重启后两种语言的 Claude 格式说明及 Grok 顶部说明均可读。后续 Grok 已检查英中权限、附件、技能限制、消息与历史结果完整滚动区，另有独立双图 GUI 在线链。整合工作区内置门禁已通过，按各轮源码快照和二进制计证；最终提交绑定与 Linux/Windows 验证仍待补。本次状态文档更新无需本地化变更。详见 [当前增量验证](VALIDATION_REPORT.md#2026-09-25-继续实现增量)。

**以下三段保留 owned Grok 的早期历史阶段状态与证据，不代表当前代码状态；当前接线范围以本文顶部代码检查点为准。**

在该历史阶段，G01 的 owned 普通 Grok 路径正在接线：macOS arm64 固定版本原生两轮与 SQLite 去重已验证，入口与恢复 UI 尚未交付。该内部增量不解除普通 Grok 富输入现有门禁，不能解释为三平台消费者验收通过。

在该历史阶段，Grok owned 普通终端基础仍未开放消费者 GUI 自动发送。该阶段的恢复实现仅核对已知启动清单和真实进程退出，保留升级占用，不把历史改为托管 ACP 启动，也不自动重发输入。macOS 内置盘空白 profile 的英文/简体中文启动冒烟已通过，无需人工授权；不代表有模型会话的应用重启或跨平台生命周期验收。

该历史阶段的 owned Grok 新启动清单使用持久数据目录和独立短 socket 目录；退出只清理与已确认结束的 leader 对应的锁与 socket，目录被替换或出现未知文件时保留现场。该阶段实现限定 macOS arm64，不能据此推定其他平台或 GUI 冷恢复已经可用。
