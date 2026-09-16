# CLI 能力矩阵

更新日期：2026-09-17。此表区分代码实现、真实接口证据与产品验收；任何“待验”项均不计入 Goal 完成。

受测接口为 Codex CLI 0.147.0、Claude Code 2.1.273、Grok Build 1.0.30。原始协议、隔离方法和官方来源见 [PROTOCOL_EVIDENCE.md](PROTOCOL_EVIDENCE.md)。不根据较新网页推定其他 CLI 版本兼容。

| 能力 | Codex CLI | Claude Code | Grok Build |
| --- | --- | --- | --- |
| 命令、别名、路径与管理命令识别 | 已实现，待当前提交回归 | 已实现，待当前提交回归 | 新增专用身份与规则，待当前提交回归 |
| 安装路径和真实版本检测 | 3 秒受限探测，安装不等于登录 | 同左 | 同左，补充 `.grok/bin` 路径 |
| 普通终端工具栏与富输入 | macOS 隔离 GUI 普通 PTY 两轮多行通过；bundle7 持久来源及五项原生授权经重启保持；bundle8 当前真实通知和英文 Unknown 已验证 | 文本后延迟 Enter | 括号粘贴候选；真实完整输入待验 |
| 连续发送、取消后旧输入 | 提交流程互斥；旧代际不可写入 | 同左 | 同左 |
| 中英文、多行、长文本 | macOS GUI 中英多行、真实拼音组合通过；候选列表选择和其他平台待验 | UTF-8 与控制序列夹具；真实完整输入待验 | 同左 |
| 图片 | 既有终端粘贴、托管 localImage；macOS Rust 适配器与中文 GUI 真实读图通过；其他平台待验 | 既有终端粘贴；托管支持需单列验证 | 未验证时拒绝并保留草稿和附件 |
| 文件与评审意见 | macOS 中文 GUI 已实际读取所选文件并传递有效评审的路径、行号与中英内容 | 复用上下文构造，真实完整路径待验 | 同左 |
| 技能 | 保留 `$` 与 Agents 来源；新项目真实路径/符号链接发现已修，GUI 刷新、选择和原生 skill 输入通过 | 保留 `/` 调用及 Claude 来源 | Grok/Claude 来源；Agents 仅 Home，项目 `.agents` 不冒充兼容 |
| 通知插件 | 旧 GUI 缓存修补曾被原生自动更新覆盖，失败证据保留；持久来源修复经 build17 回归及 bundle7 GUI 更新、CLI/应用重启保持验证，五项信任均由原生界面授权；bundle8 当前真实通知已确认；完整负向组合、其他平台仍未通过 | 2.2.0 固定来源关联修补；无模型原生重启/同版本更新保持修补及禁用，受控跨版本切换会失去修补，未知版本拒绝，详见 CLAUDE_PLUGIN_CACHE_AUDIT.md | 仓库随附插件；真实生产安装、同版本修复、禁用拒绝、文件事务回滚及恢复已通过，最终平台组合待验 |
| 基础通知 | OSC 9 只提供未知状态；普通 Stop 保存响应并显示 Unknown，同回合后续工具/审批/明确失败仍有效；相关回归在 build23 通过，bundle8 英文 Unknown 已验证 | Stop 不含全部 handlers 聚合后的最终确认；采用同样 Unknown 降级，共享回归通过，不替代真实模型验证 | 不把 PermissionDenied 当等待审批；shutdown Stop 不覆盖失败；成功 Stop 最终性未验证，降级 Unknown，共享回归通过 |
| 重复/过时事件 | 原生 turn_id、event ID/序号与 listener 实例检查；未关联终态降级，旧回合不能结束新输入 | 保留原生 prompt_id，与 Codex 不混用；同样过滤旧回合 | 独立 Grok 身份、兼容 hooks 去重及状态文件 |
| 普通本地子任务权限 | 移除固定绕过审批/沙箱，可见原生终端 | 移除固定绕过，不再修改全局信任/onboarding/config | 托管关键验收未过，暂不开放本地子任务 |
| 托管传输 | app-server 原生 JSON-RPC | 双向 stream-json/control；macOS 另有无凭据原生初始化/空闲 EOF 实证，不替代生产适配器完整生命周期 | ACP v1；已验证的连接能力与未验证的执行能力分开 |
| 托管运行中追加 | 原生 turn/steer；须已收到 turn/started | 运行中队列未通过验证，明确拒绝并保留草稿；不冒充同回合 steer | 未验证，明确拒绝 |
| 托管审批 | 原始请求 ID，单次允许或拒绝 | 单次响应及原生撤销；真实模型审批待登录验证 | 尚未通过真实审批验收，不发送未经验证的决定 |
| 托管取消 | interrupt 的 ACK 不等于已取消，等原生终态 | interrupt ACK 不等于终态，按原生回合确认 | 仅空闲 cancel 后连接存活有实证；运行中取消未通过 |
| 本地任务与消息记录 | SQLite 提交确认、父子约束、代际、revision、原生 ACK 与结果历史；build16 跨进程 Sent/结果唯一性故障注入通过 | 共用存储；协议排队差异保留 | 共用存储准备就绪，不代表执行已开放 |
| 已绑定 PTY 的结果待确认 | 兼容子 pane 保存为 Unconfirmed，继续占用原生会话/当前代，不生成最终结果；build23 SQLite 迁移和重启回归通过 | 共用状态；与托管原生 Completed 分开 | 共用存储能力，不代表托管已开放 |
| 应用重启 | macOS GUI 正常退出活动任务后重开为 Disconnected，明确继续才新建代次，同原生 ID 且未重发旧工具调用；新资源域下的真实 Codex 工具异常清理已独立通过，最终同提交重启整链待验 | 共用恢复逻辑，真实模型重启待验 | 共用恢复准备，托管未开放 |
| 历史继续 | 原生 thread/resume；macOS GUI 完成历史与活动退出后的继续均通过，缺失 ID 原生失败不会新建替代会话 | 缺失历史拒绝有实证，成功恢复待登录验证 | 空会话 load/resume 有实证，有模型历史的恢复未通过 |
| 当前应用活动连接 | 重新打开任务只选择现有连接，不再启动进程 | 同左 | 未开放托管任务 |
| CLI 崩溃后的进程清理 | 旧进程组残留负向保留；资源域 C 组 4 项、通用组同字节副本重验 4 项和真实 Codex 空闲崩溃通过；运行工具时宿主/CLI 两种崩溃均确认全部目标退出及完整清理证明，最终同提交待验 | 复用新资源域实现，真实 Claude 异常工具清理待验 | 复用监督基础设施，托管执行仍门控 |

## 产品开放边界

- `LocalCLIManagedTasks` 统一控制任务管理入口和托管启动，默认关闭；开发构建可通过 `local_cli_managed_tasks` 或运行时开关进行验收。
- 托管适配器启动前重新探测真实版本，不依赖 UI 缓存放行其他版本。受管任务默认继承 CLI 权限；Codex 经验证的只读/项目写入策略与 Claude 的权限模式不等价。
- Claude 缺登录、Grok 模型额度耗尽是当前真实模型验收的外部限制。完成基础协议、夹具或编译不能替代被阻塞的生命周期验收。
- 本机安装状态不能用来证明 SSH 远端安装或版本；普通 PTY 会话不转换成协议任务。
- 三款普通 PTY Stop 只提供候选响应，不能据此显示成功；托管原生 Completed 契约保持独立。降级和后续同回合事件规则见 [Stop 审计](STOP_HOOK_COMPLETION_AUDIT.md)。

## 新增验证范围

- [bundle7 插件验证](validation/macos-gui-bundle7-plugin-report.md)通过持久来源重启、五项原生信任及授权/安装/更新六张双语说明布局；布局夹具不等于六条功能流程全部通过。授权查询已不依赖进程内运行时热缓存，完整插件树按正常路径组件映射清单、保留 Unix 字面反斜杠；本地回归通过不代表 Windows 原生验证。bundle7 当时未确认 GUI 富通知；后续 bundle8 已确认当前原生通知受理与英文 Unknown，中文新事件及最终同提交复验仍待完成。
- [独立原生 hook 传输](CODEX_HOOK_TRANSPORT_VERIFICATION.md)在 macOS 控制 PTY 捕获未改参考脚本的完整 OSC，完整原生结果为四项 completed、一项 stopped。固定输入由原生 hook 阻断，本机模型 HTTP 请求为零；没有经过 GUI 接收链，不验证 Stop 最终成功。
- [Codex 原生 SSH/tmux](CODEX_NATIVE_SSH_TMUX_VERIFICATION.md)在 macOS 回环 SSH 直连及透传 on 各收到两条通知；off 的原生 hook 已执行但通知被阻断。三模式绑定各自原生 session/turn 和 stopped 显示，HTTP 为零、正常退出；没有完整 HookRunSummary。含最终 SetEnv 的版本已在干净 dbee1ecae 实跑通过；不计 GUI SSH、逐键输入或模型生命周期。
- [Unconfirmed 状态与迁移](UNCONFIRMED_TASK_STATE.md)用于已绑定本地任务的兼容 PTY 子 pane；未知结果仍占用当前代，明确断线或启动恢复才记录 Disconnected，不重放消息。真实 SQLite/Diesel 回归在 build23 通过；独立 GUI 库副本原三表为空，迁移 SQL 与索引夹具不能冒称有用户任务的升级或新 GUI 启动验收。
- [Claude 2.1.273 无凭据验证](CLAUDE_NO_CREDENTIALS_VERIFICATION.md)通过 macOS 初始化及空闲 EOF 自行退出；固定 Linux/Windows 文件已完整下载并核验大小/摘要。第三轮 Linux 原生获取与初始化/EOF 已通过，Windows 因前置夹具失败待验；不计完整平台或模型生命周期通过。

## 尚待收口

1. 最终代码与同提交证据。build23 相关 617/617（8.590s）、i18n 11 项（4.67s）、check20（2m26）和 Python gate5 76 项（5.114s）通过；gate6 全量 76 项（5.142s）再验通过。build19 旧取消断言失败、build20 相关测试编译前撤下、build21 非 UTF-8 文件名夹具失败均保留；后者尚未进入产品校验，现限定 Linux 待验。TUI 既有 9 项 36/80 列双语回归独立保留。上述历史快照均为 dirty 中间构建。bundle8 当前通知与任务详情双语检查已有实证；dbee1ecae 同提交门禁的进展和未通过项见文末，不能用中间快照替代最终验收。
2. 三方完整产品验收。Codex macOS GUI 已完成主要生命周期、父子双向消息与结果回收，Claude 登录和 Grok 额度仍阻塞其完整流程；macOS 真实 Codex 异常工具树已在资源域实现通过，最终同提交及其余 CLI 待验。
3. macOS/Linux/Windows 同一实际修改提交的构建与交互验证；SSH/tmux 单独记录。
4. 随附 Claude/Codex 关联修补的完整安装、升级、禁用、失败回滚与 SSH 手动发布验收。Codex bundle7 已通过持久来源与原生授权的特定路径，bundle8 已确认当前 GUI 富通知；其余负向组合及最终同提交复验仍待完成。

## 本地工具与消息的当前实现

- Codex dynamic tools 与 Claude SDK MCP 的无凭据注册已经实测；原生工具调用进入绑定连接身份的协调器，先记录调用，再派发子任务或父子消息。重复调用不能自动再次执行。
- 工具授权默认关闭，界面保存显式派发/消息权限。Codex 子任务固定保存创建时父任务已提交的完整权限快照，握手后、首条输入前逐字段校验；用户默认配置漂移、未知权限结构或跨 CLI 不等价均拒绝，恢复仍绑定创建时父代。Claude 的 permissionMode 不包含完整允许/拒绝规则，暂不能证明子任务权限上限，因此原生 Claude 父任务的托管派发保守拒绝，此项尚未完成。
- 已发送未确认消息在完成、断线、取消与换代后仍保留未确认。结果从完整任务记录产生有界摘录，原子领取后才能首次投递。`inspect_local_tasks` 支持指定一个关联任务的 `result_generation`，并按返回的 `result_next_offset` 继续读取完整结果；偏移按 UTF-8 字节计数，每页最多 8192 字节，非法字符边界明确拒绝。查询历史不切换当前执行代次，也不重新投递消息。旧原生会话保存的工具参数定义可能尚不包含分页字段，完整历史仍可由应用查询。
- Codex 0.147.0 已真实完成动态查询工具调用、进程关闭、同原生会话 `thread/resume` 及再次调用；恢复使用原生保存的工具定义，不向恢复接口添加未知字段。监督版证据见 `validation/macos-codex-supervised-tools-restore-1.ndjson`；macOS GUI 正常退出应用后的恢复与父子双向消息/结果回收已有 bundle4/5 证据，最终提交仍须复验。Claude 缺少成功模型登录，运行中重叠输入与真实审批仍未通过。
- 消息回执记录 `receipt_kind`：`native_protocol` 是 CLI 原生接收，`application_history` 是已写入应用会话历史、供下一次正常请求读取。后者不能描述为立即追加指令或模型已执行。没有可靠来源的旧回执不能冒充原生确认。
- 子任务固定保存创建时的父代数；旧结果保存在原父代，父任务进入新轮后不自动改投。显式查询仍可回收历史结果。

- Codex macOS Rust 适配器已实际通过两轮、允许/拒绝、追加应用、取消、同 ID 历史恢复与图片输入；监督版重验见 `validation/macos-codex-supervised-lifecycle-1.ndjson` 和 `validation/macos-codex-supervised-image-1.ndjson`。此证据仍为中间快照，不能覆盖其后发现的真实 CLI 崩溃清理缺陷；正常退出应用重启及双语 GUI 已有 bundle4/5 的独立证据，见 `validation/macos-gui-report.md`；最终提交与其他平台仍需验证。

本轮 [build23](validation/macos-local-gates-build23.json)及 [bundle8](validation/macos-gui-bundle8-build.json)仍含 dirty 构建边界；另一任务网页搜索修改已排除本目标暂存，需在独立工作树验证实际提交。[Darwin coalition / launchd 只读核验](DARWIN_COALITION_LAUNCHD_AUDIT.md)不等于实际清理修复，其后资源域产品实现已通过真实 Codex 两种崩溃清理，见文末当前结果。

## 提交验收进度

实现检查点 `dbee1ecae81da54a1749de88a7caffb3099d7eb7` 已推送。干净 macOS 的 cargo check、Python 76 项、Grok Node 11 项及两项无模型原生边界已有同提交证据；Linux/Windows [预检](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35100708171)因 Python 工具链不满足要求失败，Rust 与原生 CLI 尚未执行。详见 [当前验证记录](VALIDATION_REPORT.md#当前提交验证)。

bundle8 当前通知受理、英文 Unknown 和 Unconfirmed 双语任务详情通过；其输入法造成的固定输出偏差仍未通过，英文历史选择框裁切已作局部修复、等待新包，中文 Unknown 新事件和最终同提交 GUI 仍待验。上述独立结果均不替代三方完整生命周期；macOS 异常子树清理的后续实证见文末。

新增 [macOS 受控 launchd / coalition 原型](DARWIN_LAUNCHD_COALITION_PROTOTYPE.md)已验证专属域跨 setsid/双重 fork 保持、仅按已知身份清理后 CID 查询返回 ESRCH；根 SIGKILL 与 bootout 均不能单独证明完成。上述是原型阶段证据；后续生产接入、真实 worker 与插件事务的当前结果见能力表及验证报告。私有接口/内核版本的边界仍保留，不把原型、版本号或原生命令退出码当成完整验收成功。

后续检查点 `c55385a69` 的干净 macOS 构建、国际化 11 项及相关模块 617 项已通过。同 SHA 第二轮预检仍失败：Linux 已解决 Python 安装，传输夹具失败；Windows 下载解压后安装失败。两平台 Rust/原生验收未开始。后续 Windows 使用官方固定 NuGet 包作为作业私有解释器，Linux 夹具修复独立推进，详见 [当前验证记录](VALIDATION_REPORT.md#当前提交验证)。

插件追加验证已在隔离树通过编译、国际化 11 项与插件回归 163 项。Grok 1.0.30 真实生产安装器五阶段通过（安装、同版本修复、禁用拒绝、受控文件失败回滚、更新恢复），0 模型；见 [记录](validation/macos-grok-production-installer-1.json)。Claude 2.1.273 的生产升级、暂存修补失败保持旧版本、安装父进程强杀后新目录重试三场景各 1 项通过，边界见 [事务报告](CLAUDE_PLUGIN_UPGRADE_TRANSACTION.md)。两组代码已纳入 328d5ed352；这些冻结源代码实测不替代最终同提交平台验收。

328d5ed352 的[第三轮平台记录](THIRD_PLATFORM_RUN_328D5ED35.md)已越过旧 Python 环境失败：Linux 本轮所有步骤通过，GUI integration/full workspace 跳过；Windows 失败限定为当前记录的验证夹具，修复后必须原生复验。后续 macOS [专属资源域接入](DARWIN_COALITION_PRODUCT_CONTRACT.md)已通过本地 check、国际化与相关测试，新 worker 的 C 夹具组和 Codex 空闲崩溃通过，通用组同字节副本重验 4 项通过，首轮加载失败保留且原因未明；真实 Codex 运行工具的宿主/CLI 两种崩溃也已通过生产监督清理证明；没有据此开放 Grok 托管执行。

Grok 1.0.30 新增[固定 ACP 无凭据边界](GROK_FIXED_ACP_BOUNDARIES.md)：macOS initialize 通过，新建受认证阻断，load/resume 明确拒绝缺失历史；缺失会话 cancel 只有通知发送。stdio EOF 自行退出，leader 需使用自持句柄强制回收，不能计自然退出或运行中取消通过。固定 Linux/Windows 输入与相同探针已接 workflow，待新提交实跑。

最新资源域检查点 `6635f98690` 已推送；干净 macOS [同提交门禁](validation/macos-local-gates-6635f9869.json)通过 check、国际化 11 项、相关模块 650 项与 command 5 项，[脚本 75 项](validation/macos-python-gates-6635f9869.json)也通过。第四轮已完成：Linux 本轮全部通过，Windows 新路径夹具和候选启动器字节边界失败保留；新包已构建签名，GUI 待验。修复检查点 `94a412eb89` 的第五轮仅 Windows 已结束且失败：真实 Codex 候选 hook 的字节、参数和两组阻断路径通过，Grok 在 ACP 初始化前遇到锁读取权限错误，后续默认门禁跳过；不能合并为完整平台通过。后续 Grok 只读锁修复在 macOS 通过离线与无凭据边界，Windows 待复验。Oz 普通消息投递与请求正文接入已在第三个 25 文件冻结快照通过 check、18 项提供商回归、11 项国际化及 965 项相关测试，见 [消息交付说明](OZ_LOCAL_MESSAGE_DELIVERY.md)；真实 Oz 请求、GUI 双语布局及最终同提交平台仍待验。新增 Windows ConPTY 探针仅通过 9 项离线回归及工作流静态检查，尚未原生运行。
