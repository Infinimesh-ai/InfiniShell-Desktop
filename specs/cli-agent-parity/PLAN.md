# Codex CLI、Grok Build、Claude Code 支持优化与能力对齐计划

> **最终状态（2026-09-21）**：Goal 已完成。产品与跨平台验收代码 SHA 为 `38a611773b8ee53860f9ab731b2476b9a40d1819`；macOS 精确复跑、Linux／Windows 最终 full workflow、真实 CLI、双语 GUI 与 SSH／tmux 的结果和边界见 [最终验收](FINAL_20260921_CLI_PARITY.md)。下文“未完成”均为历史快照在当时的真实状态，不再代表当前结论。

阶段性收尾与新会话入口：[2026-09-19 续接交接](HANDOFF_20260919_CLI_PARITY.md)。source56 已统一 Claude 固定策略的受支持版本判定，修复 2.1.278 父任务被旧 2.1.273 字面量阻断的问题，并以官方 Claude Code 2.1.278 完成固定策略恢复、真实父子派发／双向 ACK／结果回收、运行批量取消和待编辑审批取消；原父子失败与批量验收脚本误判均保留。Grok 1.0.34 P0 生产链、GUI 原生读取审批及本机 SSH／tmux worker 已按各自快照通过；最终 Goal 仍未完成。
[自动升级与最新版当前验证](OFFICIAL_42_AUTOUPDATE_VERIFICATION.md)：source50 同源测试程序与签名监督程序已通过 Claude 2.1.278 Latest → Stable 2.1.267、反向 Stable → Latest 及 2.1.278 同版本渠道同步；source56 又以精确测试程序补齐最新版固定权限、父子和两类取消原生链，并以启用 `local_cli_managed_tasks` 的 source56 签名应用核对官方 2.1.278 检出、固定审批入口及英文／简体中文布局。source45 的配置污染失败、source50 首次冷探测失败及 source56 两个失败边界均原样保留。source56 仍没有 GUI 任务运行／应用重启、Linux／Windows 或最终同提交复验，不能回填为最终通过。Codex／Grok 已有渠道结果、最新版协议任务链、完整工作区及最终跨平台仍按各自证据边界继续验收。

2026-09-19 用户追加范围：三款 CLI 必须为消费者提供自动升级，开发与最终验收同步对齐各厂商最新正式版本。此项属于当前 Goal，不取代 P0–P5，也不因旧固定版本已验证而完成。具体实现与新增门禁见 [CLI 自动升级](CLI_AUTOUPDATE.md)；原固定版本的清单与证据仅作为历史回归基线保留，后续不再将旧版本号作为最终目标。

[source41候选](OFFICIAL_40_RECOVERY_AND_INPUT_GUARDS.md)已通过check、i18n11项、受影响Rust1826项和桌面构建；Grok文件允许／拒绝及待审批取消五阶段通过，冷继续恢复与输入ACK已到达，但测试代理连接预算耗尽，整体仍失败。普通Grok复合启动工具栏、英文与简体中文完整提示、两种语言多行草稿显式复制往返均通过；自动PTY输入仍为明确降级。source41a另修父子自动结果的时序及跨代恢复，已通过 check、i18n 11 项及受影响 Rust 1834 项；尚未运行该快照的原生链。新增[消费者自动升级与开发最新版对齐](CLI_AUTOUPDATE.md)属于当前Goal；历史固定版本证据保留，不能替代最新版或最终同提交三平台、完整工作区及SSH／tmux验收。

更新日期：2026-09-19（北京时间）。当前基线提交为 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`，source39 及其后修复为后续未提交候选；P0–P5 仍在实施与验收中。下文旧提交和快照仅保留各自受测范围，不代表当前最终验收。

[source26 同提交本地门禁](OFFICIAL_26_LOCAL_GATES.md)已通过 check、i18n11、受影响 Rust1393、Python410、main构建与严格签名，完整英文／简体中文FTL嵌入已核对。[source26 原生部分验收](OFFICIAL_26_NATIVE_PARTIAL_VERIFICATION.md)中，Claude 等待 Edit 取消2通过完整取消聚合及同原生会话继续；Grok SDK9和无输入policy1均失败，原始结论保留，权限上限及子任务仍关闭。[source26 实际GUI](OFFICIAL_26_GUI_PARTIAL_VERIFICATION.md)已观察三图标、安装版本、双语目标控件布局，Claude同会话两轮、审批允许／拒绝、追加接收并随后执行、取消与文件不变、显式历史继续，以及应用重启后的记录恢复和不自动重发；重启后新输入的最终记忆结果也已回收。上述不是三方全部最终验收。

[CI12](CI_12_INITIAL_VERIFICATION.md)的Actions实际head为同一436提交，`full_workspace_tests=false`。目前两平台Python步骤失败，固定原生准备跳过；Windows check、SSH worker及受影响Rust等步骤已通过，但整次已观察failure；Linux真实日志定位Grok coordinator离线夹具的macOS临时目录未隔离，Windows真实日志另定位同一临时目录、SQLite连接占用以及平台路径哈希断言三项问题，不能计平台整体通过。为定位Grok真实协议失败，另以436为父提交冻结source27的9路径诊断候选；其精确私有响应ID／事务账本与只公开摘要的接线不放宽产品协议或子任务门禁，[本地门禁](OFFICIAL_27_LOCAL_GATES.md)已通过check、i18n11、Rust1410和Python426，不能冒充已提交或同提交平台通过。

检查点 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04` 已实际提交、推送并核对远端一致。该提交干净验证树的 [source23 门禁](OFFICIAL_23_LOCAL_GATES.md)通过 check、i18n11、Rust1301、Python363、main构建、严格签名及完整双语资源嵌入。[source23 在线验收](OFFICIAL_23_NATIVE_LIVE.md)：Claude PNG3三输入、两个正常退出及同原生会话历史继续通过；Grok SDK8整体失败，但现代2026-07-28 MCP发现、工具列表及原生ready实际已观察；Claude等待Edit取消1整体失败，原生interrupt ACK、审批撤销、错误result及cancelled生命周期均已观察，未取得应用可信取消聚合。[CI11](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35248824124) 同SHA：Linux的Python超时夹具失败、Rust未执行；Windows的Python与Rust定向步骤失败，check和SSH worker构建通过；真实日志已取得，分别定位为纯测试的路径分隔符断言和私有JSON反斜线编码断言，原生准备因前置失败跳过、证据上传缺失为继发失败。两平台均不能计完整通过。后续修复不在该提交：[source24候选门禁](OFFICIAL_24_LOCAL_GATES.md)已冻结111路径并通过check、i18n11、Rust1378、Python410；包含Claude真实取消枚举补丁、Grok测试夹具对生产传输真实错误的诊断、标题栏普通终端启动及本地安装路径回退。只读预检已预编译并核对唯一ignored入口，准备标记已解除，原生接口尚未执行；该候选验证树dirty、未提交，不计新的同提交／跨平台／GUI通过。同一source24快照另行补跑实际`terminal::model::session::test`模块15/15通过，含新增4项，主1378计数不回填。source25已以同一111路径冻结Windows两处测试修复与两平台正确Session筛选，[正式候选门禁](OFFICIAL_25_LOCAL_GATES.md)已通过check、i18n11、Rust1393、Python410；该候选修改后来形成436提交，source26与CI12分别独立复验，不回填当时候选。

历史 source20 为 83 路径的中间脏快照：[本地门禁及构建](OFFICIAL_20_LOCAL_GATES.md)通过 check、i18n 11 项、定向 1248 项、Python 329 项、main 构建、严格签名和完整双语资源嵌入。两个 [source20 真实运行](SOURCE20_NATIVE_DISCOVERY.md)均整体 FAILED：Claude PNG 恢复身份判据提前失败；Grok SDK 仅进入首次发现回调。source21 已冻结 87 路径并通过 check、i18n 11、定向 1270、Python 363 项；未构建 main 或运行真实 CLI。随后发现旧独立 Claude AgentDriver 的固定绕过与全局配置写入，source22 已关闭旧入口并移除相关实现，以93路径 [source22 新快照](OFFICIAL_22_LOCAL_GATES.md)通过 check、i18n 11、定向1301、Python363；此修复不是沿用 source21 结论。

[source28候选门禁](OFFICIAL_28_LOCAL_GATES.md)已将source27的9路径与CI12两处coordinator Python修复合并冻结：111项输入、恰好11项修改，check、i18n11、Rust1410、Python426全部真实通过。SQLite读写连接显式关闭、离线夹具隔离macOS临时目录、路径哈希断言沿用实际平台语义；未改生产原生探测目录。[source27真实诊断](OFFICIAL_27_NATIVE_DIAGNOSTIC.md)的policy2、SDK10仍失败；[协议差异复核](GROK_27_PROTOCOL_DIFFERENCE_REVIEW.md)将SDK响应ID摘要精确匹配为固定二进制的 `skills-reload`，尚未证明可安全接纳。

实际436的[Grok GUI续验](OFFICIAL_26_GROK_GUI_PARTIAL_VERIFICATION.md)已有同会话两轮、Write允许与拒绝、追加指令、待Write主动取消和后续轮结果回收证据；第7轮历史继续已接收，但观察窗内没有终态。正常应用退出重开后恢复为连接中断，消息未重放；第8代明确启动历史连接并发送新输入，取得72字节完整记忆结果。第9代独立复验显式历史继续也已取得71字节完整记忆结果，结束事件与本代原生回合匹配。第7代中断记录保持原结论，不以其他代结果覆盖；网络隧道及认证副本均已清理。

source28的11路径代码已形成实际e687提交；[CI13第三窗口](CI_13_THIRD_WINDOW_VERIFICATION.md)针对e687的Linux／Windows两个job已completed/success，所选门禁通过。full_workspace_tests=false、GUI编译跳过，不能计完整工作区或三方在线链；终态日志一次取得失败，精确case计数未回填。已核实全局通知 `_x.ai/mcp/servers_updated` 的25字节UTF-8摘要及官方schema；[新修复](GROK_POLICY_GLOBAL_CATALOG_FIX.md)仅接受实际New／Load注册的封闭空目录，不绑定原生会话、不消费pending RPC、不证明权限。[source29门禁](OFFICIAL_29_LOCAL_GATES.md)的check、i18n11、Rust1420（含新增10）、Python436真实通过；四路径已形成实际0059提交，111项源码字节与候选及Git对象一致，[干净0059的policy3](OFFICIAL_30_GROK_POLICY_VERIFICATION.md)整体FAILED：初始化及封闭空目录通知兼容成功，New收到关联RPC错误，未进入历史继续或取得完整原生退出收据；没有模型输入。探针漏掉生产适配器已有的cached_token认证步骤，后续四路径候选补齐两阶段握手及14RPC预算，不能仅凭这一缺口推断错误原因或授权失效。同0059平台仍待验。

[真实SDK11与后续兼容修复](GROK_INTERNAL_MAINTENANCE_COMPATIBILITY_FIX.md)：source31唯一一次在线探针仍FAILED（Rust101／runner1），但严格成功形状及私有账本对应校验真实为true，认证副本与隧道已清理。确认固定skills-reload双层封闭成功结构后，后续两路径生产候选仅作内部维护空操作，不消费数值pending，不发出生命周期或提升子任务／权限门禁；新增6项Rust回归未执行，不将原失败追认为修复后成功。

当前真实证据与未满足组合统一见 [能力矩阵](CAPABILITY_MATRIX.md) 和 [验证报告](VALIDATION_REPORT.md)。历史检查点、原始失败与独立审计均保留在原专报，不改写为当前或最终同提交通过。

## 1. 目标与范围

让用户在 InfiniShell 中使用三款 CLI 时，可以通过一致的入口完成启动、输入、查看状态、处理审批、停止任务、继续会话和查看结果。各 CLI 的模型、权限和协议差异由适配层处理；尚未验证或上游不提供的能力应明确显示为不可用，不能通过猜测终端文本伪造支持。

本计划已进入实施阶段。目标覆盖 P0–P5，全部验收满足前保持 Goal active；具体版本证据见 [PROTOCOL_EVIDENCE.md](PROTOCOL_EVIDENCE.md)。

- 初始评估日期为 2026-09-16，仓库起点为 `6921a9925`；这是历史起点，不是当前实现状态。
- 已固定并取得真实接口证据的版本为 Codex CLI `0.147.0`、Claude Code `2.1.273`、Grok `1.0.30 (04b7ffed98c6)`。
- 本地安装、登录、受测版本及最低兼容版本分别判断；其他版本不得凭版本号或 PATH 命中自动放行。
- 本期覆盖桌面 GUI、其终端中的三款 CLI，以及本地子任务。平台目标为 macOS、Linux、Windows；SSH/tmux 单独验收。
- 模型、MCP、各 CLI 自己的多代理和沙箱能力按各自配置与协议保留差异；本次统一应用集成体验，不跨 CLI 自动复制这些配置。
- InfiniShell 自身的 `warp_tui` 前端、云端任务、模型 API/BYOP、SuperGrok OAuth 重接属于独立工作，不作为本次 CLI 对齐的前置条件。

## 2. 初始基线（历史）

下表及问题清单保留 2026-09-16 的初始审计，不描述当前缺失项。当前代码与验收分别见 [能力矩阵](CAPABILITY_MATRIX.md) 和本文第 10 节；“已有”仅表示当时存在代码路径。

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

### 初始必须处理的问题（历史）

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

### 最终完成门槛

分阶段交付不缩小本次 Goal；只有以下全部成立才标记完成：

1. P0–P5 的代码、随附插件、协议转换、测试夹具、发布所需文件、更新文档、三方能力矩阵及验证报告全部交付。
2. 三款 CLI 分别通过“新建 → 两轮交互 → 审批允许／拒绝 → 追加指令 → 取消 → 继续 → 应用重启 → 恢复 → 结果回收”，普通终端与应用托管任务分别留证。
3. 插件缺失、版本不兼容、CLI 崩溃、重复／乱序事件、旧回调、消息重投及恢复失败都有实际范围相符的测试；可信终态和真实进程清理不能互相替代。
4. 权限策略与审批可用，既有全局配置无启动副作用；父子派发、双向消息、原生接收确认、进度、最终结果和持久恢复完成。
5. 英文与简体中文同步、布局检查完成；`cargo test -p warp --lib i18n::tests`、`cargo check -p warp`、受影响模块及仓库要求的完整门禁通过。
6. 包含实际修改的同一提交完成 macOS、Linux、Windows 的相关平台验证，SSH／tmux 单独覆盖，保留真实 CLI 证据；选定门禁不能冒称全工作区通过。
7. 所有外部能力限制、失败和未执行项准确列出且不计完成；不存在未满足验收。

## 9. 依据

### 仓库代码

以下定位和行号来自初始审计，随实现可能变化；“权限跳过”“配置写入”“未导出”等描述保留原问题来源，不表示当前仍存在这些行为。

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

## 10. 当前执行看板

| 阶段 | 已有代码或范围限定的证据 | 尚未满足 |
| --- | --- | --- |
| P0 | 固定三方版本、认证及结构化根任务均有真实证据；Claude 2.1.278 固定策略恢复已通过，Grok 首次固定策略读取允许与冷恢复已到真实审批 | Grok SDK 父子整链、Claude hooks 兼容完整链及其他版本兼容仍待验 |
| P1 | 三方身份／版本／启动入口；Codex macOS 图片／文件／技能／评审；Claude source37 普通 PTY 双语、多行、文件及技能；Grok source35 普通 PTY 整链和 source36 非 Git 技能目录 | Grok 托管 selectedSkills 精确路径及组合输入待验；固定 1.0.30 ACP 声明 image:false，托管图片未开放；真实 IME、其余组合及最终三平台仍待验 |
| P2 | 插件事务、来源绑定及重复／旧事件回归；Grok source37 0.1.1→0.1.2 真实升级、两轮应用通知消费；Windows 历史 ConPTY 通知证据 | 当前同提交复验、插件故障完整组合及三方 SSH／tmux 整链；普通 PTY Stop 仅显示状态未知 |
| P3 | 已移除固定审批绕过；source56 的 Claude 2.1.278 固定策略父子派发、双向 ACK、自动结果、运行批量取消和待编辑审批取消均通过，签名 GUI 已核对版本检出及固定审批入口；source37 普通启动及完整交互后六项全局配置保持 | source56 尚无 GUI 原生任务运行／应用重启证据；Grok 固定读写策略、拒绝／取消完整链及 SDK 父子回收待验；source39 Files1 的允许与拒绝已实际执行，取消输出夹具误判，剩余四阶段尚未发送 |
| P4 | SQLite 代次／父子／消息／结果持久化；三方显式历史继续有 GUI 证据；Claude source37 应用重启不重发、同原生 ID 继续并回收结果 | 最终三方活跃重关联、故障恢复及父子 GUI；历史失败不因后续继续成功而改写 |
| P5 | source56 的 `cargo check -p warp`、i18n 11 项、受影响 Rust 模块与 Python 28 项通过；启用 `local_cli_managed_tasks` 的 source56 main 构建、严格签名、官方 Claude 2.1.278 检出及英文／简体中文布局通过；source38 三策略双语 GUI 已通过；CI14 对 af1040 的 Linux 所选门禁通过 | source56 未提交，尚无 Linux／Windows workflow、GUI 任务运行／重启或完整工作区；最终实际修改同提交三平台、完整功能布局和 SSH／tmux 未完成 |

近期顺序：source38 已完成本地门禁、main 和固定只读允许／冷恢复／拒绝验收。source39 已冻结 188 项输入，合入默认入口 selectedSkills 路径绑定、固定文件六输入夹具及目录／info 安全诊断，先执行本地门禁和 main，再进行分别限定输入预算的在线验证。随后完成普通终端 Grok 的实际 UI、双语布局和故障恢复。最后将实际修改形成最终提交，针对该同一提交执行 macOS／Linux／Windows、完整工作区及 SSH／tmux 验收。旧失败和中间快照分别保留，任何缺失均不计完成。

[内部维护兼容修复](GROK_INTERNAL_MAINTENANCE_COMPATIBILITY_FIX.md)、[缓存认证握手修复](GROK_POLICY_AUTH_HANDSHAKE_FIX.md)及[Grok 0.1.1 插件升级修复](GROK_PLUGIN_011_MIGRATION_FIX.md)已合入新的 source32 候选快照：父提交0059，119项输入、18处变更，其他101项逐父Git对象保持一致，新增38项Rust回归包含8项tokio异步测试。诊断测试同步为合法内部维护帧无业务效果且不消耗pending，未知ID、错误帧及夹带会话身份仍拒绝；原source31快照和失败记录保持不变。source32独立门禁已真实全部通过：check退出0／140.859秒、i18n11／320.445秒、Rust1458／25.620秒、Python15组453项无跳过；38项新增Rust（含8项tokio）逐项各1PASS，119项输入与850624232字节测试库前后保持一致。main已真实构建退出0／329.100秒，严格签名退出0、完整中英文资源逐字节嵌入，测试库身份保持一致。[source32原生部分复验](OFFICIAL_32_NATIVE_PARTIAL_VERIFICATION.md)：SDK12（29.281秒）与policy4（13.074秒）均Rust101／runner1，原失败保持；SDK12已实际走通现代发现、tools/list和一次inspect业务调用，2次原生审批允许、transport正常结束及清理收据通过，但六种carrier均无原生S/P/T，身份关联验收未通过，子任务和父权限门禁保持关闭。policy4实际仅发3RPC，initialize及cached_token authenticate成功，session/new关联rpc_error，未完成两阶段接口验收；失败取消的null退出码投影缺口另行修复，不补成退出0。认证副本／连接／目录已清理，配置与策略夹具保持不变，不能将14RPC预算写成已发出计数。原生六阶段插件迁移尚未执行；另发现0.1.1配方的通知仍上报0.1.0，后续候选须同步脚本版本及严格历史夹具，并独立验证真实安装后的通知。[SSH／tmux执行计划](SSH_TMUX_FINAL_ACCEPTANCE_EXECUTION_PLAN.md)仅盘点可复用入口和待验边界，没有新增真实SSH链通过。

根代理最新 API 核对为 Linux／Windows runner 均 online、busy=false；此前 Linux 离线观察仅保留历史。派发前须重新核对并留证，可用性不等于平台验收通过，不构成整个 Goal 的实质阻塞。用户的 `web_runtime.rs`、`websearch_tests.rs` 及 `validation/gui-7e065085/` 全目录保留并排除本目标暂存。source22 新增旧入口不可用提示，英文与简体中文同步，i18n门禁11项已通过，最终双语检查待验。
