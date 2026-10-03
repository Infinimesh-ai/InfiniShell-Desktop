# CLI 能力矩阵

**2026-09-30 当前 Goal 范围**：用户将其他平台实机验收移交后续执行，继续补齐原生能力和 Mac 未验项。G02 已由固定 Grok `1.0.41/grok-4.7`、Inherit 的基本图片历史正例及 `d290fd713` 图片＋单技能／热新增双技能／冷恢复组合真实链满足 Mac 条件；G04／G05 已按固定 Claude `2.1.280/claude-opus-5-5`、Inherit 的既有真实 Mac 证据关闭；其他平台未改记为通过。G04 支持 PNG、JPEG、静态 GIF、WebP；G05 已验纯 PNG 与 PNG＋单技能语义，不外推所有格式排列或固定策略。其余状态见 [当前范围](KNOWN_GAPS.md#2026-09-30-当前范围)；以下早期记录保留各自源码和边界。

**2026-09-30 G06 当前增量**：Grok 官方 `1.0.41 (4220f3b224a6)/grok-4.7` 的 local/user 混合多技能、同连接热新增 user 技能、同原生 ID 冷恢复与固定 `GrokRestrictedSkillsV1` 的逐次拒绝／允许／冷恢复已取得独立 Mac 生产适配器正例，两条链各三次原生输入、两代自然退出 0。所选技能限定名与目录原路径按原生确认，固定调用顺序保留；固定读取／文件策略继续禁技能，独立所选技能策略不继承该禁用泛称。当前英中 GUI 说明布局通过，真实模型、布局和最终收尾修复各按其工件计证。`625ffcb98` 的 Linux／Windows 源码门禁已成功，G06 在本次 Mac 范围关闭；Windows 桌面用例 1 条 LEAK 警告保留；新固定链未创建真实子任务，不计作 G10 完成。详见[本轮验证](VALIDATION_REPORT.md#2026-09-30g06-本次-mac-范围关闭)。

2026-09-26 用户明确目标平台为 macOS Apple Silicon、Linux x64 和 Windows x64；macOS Intel（包括 Codex CLI）不属于当前开发或验收范围。历史 Intel 记录保留原范围。

**阶段交付，完整 Goal 尚未完成。** 本表记录固定历史基线及输入实现提交 `84102bb1c687f87a2425bc1937784e77250c416c`，npm 来源绑定提交为 `b6f93f662`。早期在线收据按 WIP 摘要计证；`84102bb1c` 后续已有静态 GIF、GUI JPEG／纯 PNG 窄复验，当前技能／权限工作区增量已有内置门禁、G10 两条生产协调器链和 Grok 双图 GUI 正例，最终提交与相关平台验证仍待补。它说明当前产品能力及限制，不将代码路径存在等同于全平台验收；具体通过范围见[验证结论](VALIDATION_REPORT.md)，剩余项见[已知缺项](KNOWN_GAPS.md)，整体状态见 [CURRENT_STATUS](CURRENT_STATUS.json)。

2026-09-27 `e2eba8596` 新增已验范围：Mac ARM64 固定 Grok `1.0.41/grok-4.7/default` 的专属 TUI 经 GUI 选择器提交单图加中文，原生图片字节、同 prompt 接收与识色回答通过；局部英中布局及退出后重启不重投通过。未关闭 G01/G03，详细限制与同提交云端结果见[验证结论](VALIDATION_REPORT.md)。以下早期“待验”描述按原提交范围保留。

普通终端富输入的“附加文件”已接通本地文件卡片选择器。Mac 固定 Codex `0.156.1`、Claude `2.1.280` 与定制 Grok `.3` 均已有普通 GUI 卡片投递、原生实际读取及原生审批拒绝证据；Claude 的 `142c635412` 新构建另完成共用文件错误 Toast 英文／简体中文完整布局、草稿与双卡保留验收，Grok 与旧 Codex 正例仍按原构建计证。文件卡仅传本地绝对路径引用，发送前整批检查普通文件及当前账号可读性，由原生工具按自身能力与审批读取；实际模型验收为 UTF-8 文本，不证明任意二进制传输或所有格式解释。最终源码门禁已满足，G07 在本次 Mac 范围关闭，见[最新验证结论](VALIDATION_REPORT.md)。

`bd4612887` 将 Codex／Claude 锁定 Shell 模式与文件卡片的组合限制为发送前拒绝，草稿及卡片保留；该模式不能计作文件附件已投递。Grok 会话或普通桥身份校验失败时仍拒绝自动提交并保留输入，已有双语回退提示；仅已绑定定制工件按 G01 合同开放。

按用户要求形成的功能代码检查点现已进入无人值守回归阶段；真实 GUI／模型验收仍后置。代码检查点 `704bb33bb2943f8a686b24b843c3b3a1ba656c19` 中的 G01/G03 专属 TUI GUI／PNG 输入、G08 远端图片、G09 安装来源事务、G10 搜索与受限技能增量见 [CURRENT_STATUS](CURRENT_STATUS.json) 的 `current_work_order`；该检查点已通过 macOS arm64 编译和 i18n 门禁；后续修正提交 `b78b62a48223235e8c29157e3786747c8db2ff68` 的 Linux／Windows 同提交 `cargo check` 均已通过。最新定向回归与夹具复验见[最新门禁](VALIDATION_REPORT.md)，这些门禁不代表完整产品验收，也不扩大下表的真实功能验收范围。Codex 远端图片已接入[固定 0.156.1 的原生队列](https://github.com/openai/codex/blob/rust-v0.156.1/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L899)及[图片快照](https://github.com/openai/codex/blob/rust-v0.156.1/codex-rs/protocol/src/local_media.rs#L20)，按后续输入排队处理；仍须后续真实 SSH／tmux 接收证明。共享 daemon 默认路径仍只允许唯一前台客户端和唯一 loaded thread；新增每 pane 独立 app-server、明确 socket 和当前 TUI 票据绑定及 GUI入口；Grok 远端专属会话、当前 pane 入口、图片发送和查询恢复已接线；Claude 新增上传原图→原生 Read→历史图片字节证明及未知状态恢复，不能把文本入队计为图像消费。三 CLI 三目标平台 npm、三 CLI Mac ARM/Linux x64 Homebrew、三 CLI Windows x64 WinGet、Claude/Grok 受限 Skill 和 Mac/Linux 命令+技能组合已有未验证接线；三平台受审命令现统一通过宿主工具逐条独立监督、审批和清理，同会话多命令及原生追加均已接；Grok普通终端历史继续也已接同UUID恢复与新代次CAS。

2026-09-27 G09 增量：`39941b281` 已接私有 npm 四场景验收入口；后续 `2b59c8c62` 新增 Linux 候选隔离能力前置与英中提示。本地门禁通过，Mac ARM Codex `0.155.1→0.156.1` 与 Grok `1.0.40→1.0.41` 的私有正常升级分别按各自源码收据通过；另有 `39941b281` 的 Linux Grok 正常升级、交换后恢复、外部改动保留、候选改动拒绝四场景通过。其他平台恢复／故障、消费者渠道、GUI 与 Homebrew／WinGet 真实事务仍待验，G09 和 Goal 不关闭。详细结果、原失败与收据集中见[验证结论](VALIDATION_REPORT.md)及[当前状态](CURRENT_STATUS.json)。

`56f6da217` 已修正 Linux 官方 Node 大 ELF 字符串表误拒，同提交 Linux 原文件依赖闭包通过；新增 Windows Codex npm 五场景入口仍在真实私有 npm 登记阶段失败，尚未进入产品事务。两平台编译与定向 Rust 通过，离线夹具修复已另提交；G09 未关闭，详细范围见[验证结论](VALIDATION_REPORT.md)。

`5aac3f7d0` 已补远程 Claude 延迟图片回收与 Grok 明确未派发后的主动重试；`bd80cc9ed` 已接 Windows Codex npm 两组固定入口模板。新增 19 项回归及本地编译／英中门禁通过，同提交 [验证 36266539274](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36266539274) Linux 通过，Windows 编译／定向回归通过但真实 npm 首个 CMD 候选探针拒绝未绑定子映像，后续场景未执行；实际远端／GUI／模型链仍待验，G08/G09 不关闭。

`ca37b9cbf` 已补 Windows npm 拒绝后的显式 Job／ACL／profile 清理与脱敏映像诊断；本地编译和 i18n 门禁通过，仅 Windows 的[同提交验证 36270700886](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36270700886) 因 Win32 事件码字段类型错误在编译阶段失败，原生场景未运行；字段修正待新增量复验。候选拒绝仍需定位，未扩大白名单或宣称真实升级完成。

`43c9709d2` 的 `console-binding-04` 已补 Windows Codex npm 的精确 `conhost.exe` 租约、同 Job／AppContainer／零 capability 校验及跨失败退出句柄。本地 check、i18n 重试02（11项）和 actionlint 通过；同提交 Windows 编译、978项主回归、5项 command 与7项原生边界回归通过，真实 npm 首个 CMD 候选仍返回 `RecoveryRequired`，停在 `Prepared`，清理未确认，PowerShell／发布／恢复未执行。CMD 报 verbatim 工作目录不兼容，拒绝访问及清理原因仍待查；保留原始失败，不新增“升级通过”能力。无需本地化变更，G09 仍开放，详见[验证结论](VALIDATION_REPORT.md)。

Windows npm 工作目录后续增量已在 `05b0b8faa` 提交（基线 `6408131e2`），仅给 CreateProcessW 传经身份核验的 DOS cwd；ACL／readonly 和隔离门禁不变。本地 check、i18n 11项、actionlint及差异检查通过，同提交 Windows [CI 36280073722](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36280073722) 已结束：主组981项、command7项、显式atomic7项通过（含新增5项），真实更新首个CMD候选仍失败。旧UNC提示消失，但原生“拒绝访问”后缺完整清理收据；剩余进程和拒绝对象尚未定位，PowerShell、发布与恢复未执行；不新增已验能力，G09 保持开放。

`0525c9439` 的协作取消已接入原认证通道，本地check、i18n 11项及control6项通过；同提交 [Windows CI 36282335500](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36282335500) 编译通过，普通回归986项通过/1项失败，失败为Drop的2秒回收等待（nextest既有3次自动尝试）；新增原生取消2项均通过、atomic9项和command7项通过。真实npm首updated返回Network、未形成journal/generation或候选执行/清理收据，不能继承上一轮cleanup=false，发布/恢复未验。后续修正 `49ea40561` 已提交推送，监听改为非阻塞读取及可唤醒等待，保留6项契约和2秒断言；独立审查、本地02 check、i18n 11项、control6项及actionlint通过。同提交 [Windows CI 36283908796](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36283908796) 已结束：编译通过，主组987项、control6项、command7项及原生atomic9项通过；Drop首次0.112秒通过，原生取消2项也首次通过。真实npm首updated仍返回ProbeFailed，原生拒绝访问后因取消中断；本轮已取得Job／ACL／profile清理标记及cleanup_confirmed=true。无原生根退出码，外层worker码1不代替它；PowerShell／发布及后续恢复场景未验。无需本地化变更，G09及Goal保持开放。

**Windows npm 诊断增量 `ae4279e48`**：固定下载角色与失败阶段、已核验 CREATE/EXIT 角色和清理模式已接入，授权、隔离及失败判定不变。本地 check、i18n 11项及 actionlint 通过；同提交 [Windows CI 36285301933](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36285301933) 已结束：Windows 编译、主组990项、command7项、原生atomic9项通过，真实 npm 首个 updated 返回 ProbeFailed。正常阶段仅记录 root／console CREATE；取消清理阶段记录 root／console EXIT1，另有尚未分类的 unknown CREATE／EXIT3221225738，不能据此认定 Node 从未启动或定位拒绝对象。最终取得 cleanup_confirmed=true 及 AppContainer 清理标记；PowerShell、发布与独立恢复未验。CREATE/EXIT 与 root_exit 使用同一 session 时钟，probe 阶段仍用自身起点，不直接相减。按用户代码优先、修复与联测后置要求封存本轮，不自动重试或追加猜测性诊断；G09和Goal保持开放。无需本地化变更。

`debed8c51` 补接有效专属 Grok 的上下文草稿队列及 Linux／Windows 升级空闲通知，英中提示已同步，本地门禁通过；真实三平台／远端发送、双语布局及同提交平台结果仍待补，G01/G09 不关闭。

仅 Linux Codex npm 和三款 Linux Homebrew 在确需更换版本时要求 Landlock ABI≥3；不足时不生成更新计划，原 worker 硬门禁保留。Grok npm、其他来源、macOS／Windows与同版本无候选执行的检查／配置同步不受此门槛影响。`2b59c8c62` 相关平台 CI 和实际双语布局待补；旧 `39941b281` CI 的 Linux glibc ELF 失败与 ABI1 事实分开记录，Windows 作业成功但保留一项 `LEAK`，均不外推为完整通过。

## 固定版本与模式

| 能力 | Codex CLI 0.156.1 | Claude Code 2.1.280 | Grok Build 1.0.41 |
| --- | --- | --- | --- |
| 桌面入口、身份、安装与版本 | 默认开放；检查实际路径、来源、版本 | 默认开放；安装不等于登录 | **默认开放**；保留版本、来源、策略与能力门禁 |
| 托管协议 | app-server | 双向结构化流／control | ACP；继承设置与固定策略分别判断 |
| 托管文本与文件上下文 | 中英多行、文件上下文 | 中英多行、文件上下文 | 中英多行、文件上下文 |
| 托管图片 | 原生类型化输入；模型识图另见 V04 | 已接 PNG/JPEG/静态 GIF/WebP、纯图和单技能组合；格式/纯图有 Mac 适配器正例，84102 GUI JPEG/纯 PNG 正例；图加单技能已有工作区 Mac 适配器及 GUI字节/识色/逐次审批正例；`2383428da` 的 PNG 加双技能 Mac 生产适配器新建／冷恢复及 GUI 新建任务均通过原生字节、两次独立审批和结果验证；GUI 冷恢复、跨平台及其余格式组合未验（G04／G05／G06） | 已接固定 1.0.41/grok-4.7、Inherit PNG；单／多技能组合要求私有独占 leader 与目录确认；`d290fd713` 已通过 PNG＋单技能、热新增双技能及冷恢复换序的 Mac 生产链。基本 PNG 的 GUI 字节／识色、冷恢复、同宿主重关联与清理按原构建计证，G02 本次 Mac 范围关闭；其他平台实机移交 |
| 普通终端富输入与附件 | 文本可发送；本地双卡 Mac GUI 原生读取及独立原生审批 No 已验，按原构建计证；G07 在本次 Mac 范围关闭；远程图片队列已接线，真实 SSH／tmux 链待验（G08） | 文本可发送；本地双卡 Mac GUI 实际 Read、独立审批 No，以及共用错误 Toast 英中完整布局与失效文件保稿已验；G07 在本次 Mac 范围关闭；远程原图 Read／历史消费真实链待验（G08） | 普通文本桥三平台实现及固定工件齐备，Mac `.3` 功能与英中已验；普通双文件卡实际读取／拒绝已验，G01／G07 在本次 Mac 范围关闭。专属 PNG 已有选择器单图及剪贴板双图正例；普通未绑定图片、真实拖放及审批保护仍待补（G03）；远端图片仍待验（G08） |
| 技能来源与调用 | 保留 `$` 与原生技能调用 | `/` 与原生 Skill；多技能、会话内新增及图片组合已有 Mac 生产适配器／GUI 逐次审批和执行证据，冷恢复及同宿主重关联零重投按各自构建计证。固定读取／文件策略禁技能；独立所选技能策略按已登记集合与父权限上限处理，完整父子工具链另见 G10 | Inherit 的 local/user 来源、多技能、空闲热新增及同原生 ID 冷恢复已有 Mac 实际消费证据；reload 影响同 leader，仅用于私有独占 leader。固定读取／文件策略禁技能；独立 `GrokRestrictedSkillsV1` 已验拒绝、允许、冷恢复及未选技能零派发。G06 本次 Mac 范围关闭，源码门禁及其 Windows LEAK 警告见验证报告，外平台模型／GUI 移交；不外推 G10 父子链 |
| 文件引用与代码评审 | 复用路径、选区、评审上下文 | 同类上下文入口 | 同类上下文入口；文件卡为路径引用，不等于任意二进制内容传输 |
| 审批与配置 | 当前请求允许／拒绝；不固定跳过审批 | 当前请求允许／拒绝；启动不改全局配置 | 固定读取／文件策略分别验权；拒绝可能返回 Cancelled，不改写为成功 |
| 父子任务与消息 | 派发、父权限上限、分阶段 ACK、进度与结果回收 | 固定文件策略派发及双向 ACK／结果；非任意继承配置（G10） | 固定策略和 SDK 合同满足时派发；继承模式无父子工具（G10） |
| 子任务工具范围 | 按可验证的父权限快照约束 | 固定文件策略已有 Mac V2 Write 限定证据；后续搜索、固定技能及逐次审批的宿主命令／技能组合已接线，旧策略不扩权，真实新增工具链待验（G10） | 固定读取／文件策略，以及新增固定技能、逐次审批的宿主命令／技能组合已接线；原生 shell／hooks 保持关闭，新策略真实链待验。两款固定策略均非 OS 沙箱（G10） |
| 运行中追加 | 当前活跃回合 `Steer` | 原生合并或后续轮次，不是 Codex steer（G11） | 仅后续回合排队，不支持即时 steer（G11） |
| 持久化与恢复 | 任务、原生 ID、父子、消息／结果；活跃重关联与历史继续分开 | 同类持久化；继续受原生会话与权限约束 | 同类持久化；技能／策略合同须在恢复时继续成立 |
| 配套通知插件 | `codex-warp 0.4.0`；原生 hooks 信任单独审核 | `warp 2.2.0`；保留禁用与无关配置 | `infinishell-grok 0.1.6`；安装、启用及完整性分别核对，原生权限观察不得替代审批 |
| 自动升级渠道 | latest／alpha | latest／stable | stable／alpha |
| 自动升级来源 | 原生安装已有验收；三平台 npm、Mac ARM/Linux x64 Homebrew、Windows x64 WinGet 固定来源事务已接；Mac npm 私有正常升级通过，其余新增场景及路径待验（G09） | 原生安装及固定 Unix npm 单包事务已有各自限定验收；Windows npm、两平台 Homebrew、WinGet 代码已接，新增路径待验。主动降级仅条件合同，当前渠道不匹配仍拒绝（G09） | 原生安装已有验收；三平台 npm、两平台 Homebrew、WinGet 固定来源事务已接；Mac npm 私有正常升级通过，其余新增场景及路径待验（G09） |
| 完成与清理 | 可信原生终态；普通 PTY Escape 不证明工具退出（G12） | 可信原生终态及本代清理回执 | 原生五秒 EOF 不可靠；托管清理依赖当前代次监督回执 |

CLI 升级、插件更新与托管兼容分别判断；渠道与来源详细合同见 [CLI_AUTOUPDATE](CLI_AUTOUPDATE.md)，用户支持范围与回退见 [RELEASE_SUPPORT](RELEASE_SUPPORT.md)。本表中的能力不外推到未来 CLI 版本（G13）。

## 共同状态与消息合同

- 应用实例、代次、原生会话／回合 ID 和事件来源用于隔离重复、乱序及旧回调；插件文件来源不能替代真实 CLI 身份。
- 普通 PTY Stop／基础通知不证明完整终态，缺依据保持未知；已绑定任务的未确认结果保存 `Unconfirmed`。托管完成、取消、失败、断线与清理各有独立判据。
- `native_protocol` 只说明原生接收，`application_history` 只说明进入应用历史；ACK 不证明模型已执行完成。父子结果要与实际任务及回合关联。
- 重启恢复记录不自动重放输入；活跃重关联复用核验过的宿主，历史继续才新起进程；同一原生会话不得并发恢复，失败不能偷偷改为新建。
- 插件缺失或不兼容时可保留普通终端；禁用不自动启用，未知文件和并发编辑不覆盖。具体故障与降级不能被“默认开放”掩盖。

## 平台与验收边界

| 验收项 | 当前结论 | 仍需处理 |
| --- | --- | --- |
| macOS 固定版本真实链 | 有三款限定模式的托管、父子、GUI 恢复与普通终端证据 | 能力模式与附件限制仍按 G01–G10，不代表所有组合完成 |
| Linux／Windows | 已有工作区／相关回归、原生安装升级、通知、监督清理和部分 GUI | 三款完整在线模型生命周期未全部覆盖（V01） |
| 中文输入法 | macOS 实际拼音输入法已有证据 | Linux／Windows 剪贴板、中文字形不等于真实组合输入与候选选择（V02） |
| SSH／tmux | 主要为 macOS 到本机隔离 OpenSSH，包含断连重接 | 目标远端系统组合；Codex 关闭透传须独立证明内层发送与外层无接收（V03） |
| 图片理解 | Codex 原错误保留，独立 sol 原生语义正例不计产品链；新增 Claude 多静态格式/纯 PNG、Grok PNG 的 Mac 适配器识色与字节正例；Grok 双图 GUI 独立正例及两种恢复另计 | 保留失败并排查、取得独立正例；只认逐格式、模型和平台的原收据，WIP 不冒充最终 SHA，也不承诺任意图片回答准确（V04） |
| 英文／简体中文布局 | 历史选定视口；本次 Mac 英中 Claude 图片说明可读，Grok 权限、附件、技能限制、消息与历史结果完整滚动区无截断；142c635412 的 Claude 普通 GUI 已补共用文件错误 Toast 英中全文、长文件名及保稿双卡布局；分别按原构建计证 | Linux／Windows Grok 权限、技能禁用、审批及完整滚动区域（V05） |

G01–G10 是功能或模式缺项，V01／V02／V03／V05 是尚未满足的验收；G11／G12 是必须如实展示的原生语义，G13 是持续兼容边界，V04 将投递与模型理解分开。18 项的影响、替代方式与关闭条件只在[统一清单](KNOWN_GAPS.md)维护；安全拒绝、无模型清理或其他平台成功不关闭这些缺项。

G01 原生源码判断已于 2026-09-30 更正：[官方 grok-build](https://github.com/xai-org/grok-build) 按 Apache-2.0 公开 CLI／TUI／运行时并支持本地构建，不能继续将 npm 包不含源码等同于无可修改源码。已固定公开 `1.0.41` 快照 [07e35a3dfeed2f200d319ef6c893b5ea286d9a51](https://github.com/xai-org/grok-build/tree/07e35a3dfeed2f200d319ef6c893b5ea286d9a51)，`SOURCE_REV=84745de98b3d3996729aefcefd518890ffb73930`，与已验发行版 `1.0.41 (4220f3b224a6)` 分别绑定。[原生原子输入桥补丁](../../native/grok-build/README.zh-CN.md) 的 Mac `.3` 已通过独立构建和 74 项定向回归；宿主 check、48 项桥、11 项 i18n 及两项有效 peer 回归通过。`r-wudipm8z` 的普通标准首页 GUI 已取得八条真实输入与 ACK，包含中文 400 行长文本、草稿／审批保护、明确同文新一轮和同会话冷恢复；七个 `end_turn`、一个用户拒绝审批的 `cancelled`，局部英中布局通过。随后文本粘贴修复仅恢复／追加本地富输入草稿，本地七门禁与英中各一条明确提交通过；中文手动回退截断已由单行缩短与最终布局复验解决；Mac 功能及英中审计满足；Linux 普通桥宿主接入、Windows 原生传输与宿主接入均已实现，并分别绑定已核验 Mac `.3`、Linux `.6`、Windows `.11` 工件，仅供从普通 shell 启动的独立受审定制构建使用；官方发行版不含该桥。最终两平台宿主源码门禁 `36796894945` 已逐名覆盖账本各 19 项及必要桥组，G01 在本次 Mac 范围关闭；其他平台实机验收移交不免除实现或最终源码门禁，不向未核验工件或官方发行版外推。许可证、贡献政策及剩余条件见 [G01 清单](KNOWN_GAPS.md#G01--grok-普通终端富输入自动提交)。

G01 历史原生证据：固定 macOS arm64 `1.0.41/grok-4.7/default` 的 owned 普通 TUI 已有生产启动、leader 侧车和 SQLite 两轮真实 PTY 正例。当时 GUI 富输入尚未接线；后续三目标平台 GUI／PNG／文件卡与上下文草稿接线已完成，真实自动发送验收仍待补，不能以该历史正例扩大当前已验范围。`idle_prompt`、固定延时和 PTY Enter 都不作为这条路径的发送授权。 后续恢复增量只重新登记已知进程占用，不恢复输入授权、不启动进程；退出清单与托管历史隔离已覆盖本地回归，未计为真实 GUI 会话恢复。 新清单已改为持久数据域与独立短 socket；旧清单保持兼容，精确原生锁清理已在退出现场复验，不扩大消费者发送范围。
