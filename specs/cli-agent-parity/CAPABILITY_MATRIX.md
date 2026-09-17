# CLI 能力矩阵

更新日期：2026-09-18（北京时间）。此表分别记录实现路径、真实接口证据与最终产品验收。Goal 未完成；固定受测版本为 Codex `0.147.0`、Claude `2.1.273`、Grok `1.0.30`。本轮实际修改提交前的已推送检查点为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`。

检查点 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04` 已实际提交、推送并核对远端一致。该提交干净验证树的 [source23 门禁](OFFICIAL_23_LOCAL_GATES.md)通过 check、i18n11、Rust1301、Python363、main构建、严格签名及完整双语资源嵌入。[source23 在线验收](OFFICIAL_23_NATIVE_LIVE.md)：Claude PNG3三输入、两个正常退出及同原生会话历史继续通过；Grok SDK8整体失败，但现代2026-07-28 MCP发现、工具列表及原生ready实际已观察；Claude等待Edit取消1整体失败，原生interrupt ACK、审批撤销、错误result及cancelled生命周期均已观察，未取得应用可信取消聚合。[CI11](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35248824124) 同SHA：Linux的Python超时夹具失败、Rust未执行；Windows的Python与Rust定向步骤失败，check和SSH worker构建通过；真实日志已取得，分别定位为纯测试的路径分隔符断言和私有JSON反斜线编码断言，原生准备因前置失败跳过、证据上传缺失为继发失败。两平台均不能计完整通过。后续修复不在该提交：[source24候选门禁](OFFICIAL_24_LOCAL_GATES.md)已冻结111路径并通过check、i18n11、Rust1378、Python410；包含Claude真实取消枚举补丁、Grok测试夹具对生产传输真实错误的诊断、标题栏普通终端启动及本地安装路径回退。只读预检已预编译并核对唯一ignored入口，准备标记已解除，原生接口尚未执行；该候选验证树dirty、未提交，不计新的同提交／跨平台／GUI通过。同一source24快照另行补跑实际`terminal::model::session::test`模块15/15通过，含新增4项，主1378计数不回填。source25已以同一111路径冻结Windows两处测试修复与两平台正确Session筛选，[正式候选门禁](OFFICIAL_25_LOCAL_GATES.md)已通过check、i18n11、Rust1393、Python410；新实际提交及同提交原生／平台复验仍待执行。

source21 的87路径中间快照已通过 check、i18n11、定向1270、Python363；没有 source21 main 或真实CLI。source20构建与严格签名保留历史。随后发现旧独立Claude AgentDriver 的绕过与全局写入，已关闭该旧入口并移除相关实现；source22 的93路径新快照通过 check、i18n11、定向1301、Python363。下表中间证据不代表最终实际修改提交、所有平台或整个能力通过。完整来源、失败与历史入口见 [验证报告](VALIDATION_REPORT.md)。

## 身份、普通终端与上下文

| 能力 | Codex CLI | Claude Code | Grok Build |
| --- | --- | --- | --- |
| 命令／管理命令、安装及版本识别 | 专用规则及受限版本探测；最终提交回归待验 | 同类专用规则，安装不等于登录 | 独立身份、规则及 `.grok/bin` 探测；默认标题栏已有双语 [入口实测](GROK_TITLEBAR_ENTRY.md) |
| 普通 PTY 工具栏与富输入 | macOS 隔离 GUI 两轮多行有 [证据](validation/macos-gui-report.md) | 延迟 Enter 策略；认证后完整普通 PTY 组合待验 | 括号粘贴候选；普通 PTY 完整链待验，托管 GUI 证据另列 |
| 中文输入法／中英多行 | macOS GUI 逐键拼音组合、混排与多行已有证据；候选列表及其他平台待验 | 托管 API／GUI 有多行输入阶段证据；普通 PTY 与三平台待验 | source11 托管 GUI 中英文多行通过；普通 PTY 输入法与三平台待验 |
| 图片 | `localImage` 原生适配器及 macOS GUI 已实际读图；最终提交与其他平台待验 | 同提交source23生产typed PNG3三输入、两代正常退出及同原生S历史继续通过；磁盘附件引用恢复通过，完整App重启／SQLite／GUI及其他格式仍待验 | 托管图片未验，拒绝并保留草稿／附件 |
| 文件与代码评审 | macOS GUI 文件实际读取及有效评审的路径／行号／内容传递有证据 | 复用上下文；普通 PTY 与完整组合待验 | 复用上下文；完整组合待验 |
| 技能来源与语法 | `$` 与 Agents 来源；新项目发现、选择及原生注入有 macOS GUI 证据 | `/` 与 Claude 来源；普通／托管组合分别验收 | Grok／Claude 来源，Agents 仅已支持的 Home 来源；托管技能未开放，不从菜单存在推断调用成功 |
| 原生模型、推理与登录 | 按实际版本／配置保留差异，不从安装推断已登录 | API 鉴权已可用于实测，不将 Inherit 模式视为固定文件策略 | 官方在线授权及根任务已有成功证据，旧额度失败不再作为当前阻塞 |

## 插件、事件与状态

| 能力 | Codex CLI | Claude Code | Grok Build |
| --- | --- | --- | --- |
| 插件安装／更新／禁用／恢复 | 固定 rev4、持久来源及原生信任有中间证据；完整负向组合与最终提交待验 | 固定插件版本及来源修补、禁用和事务恢复有阶段记录；未知版本不放行 | 随附独立插件；真实生产安装、同版本修复、禁用及事务回滚有阶段记录；全平台组合待验 |
| 普通 PTY 完成依据 | Stop／OSC 9 仅候选响应，显示 Unknown；同回合后续有效事件继续处理 | 同样降级 Unknown，不能仅由 Stop 推断聚合完成 | `PermissionDenied` 不表示等待审批；缺可信终态降级 Unknown |
| 托管终态 | 原生 turn 终态与任务结果分别核对 | 原生输入 UUID／执行归属与完整 result；取消要求完整证据聚合 | 完整原生历史与同回合 completion watermark 核对；不从 `end_turn` 单独推断正文完整 |
| 重复／乱序／旧回调 | turn／事件 ID、代次与实例约束，定向回归已有证据 | 保留 prompt／UUID 语义，不混用其他 CLI 身份 | 独立身份与来源，兼容 hooks 及乱序的完整真实负向链待验 |
| Windows hooks | CI10 五项原生注册通过；SessionStart／UserPromptSubmit 两项真实到达 ConPTY，其余三项实效待验 | 无凭据原生边界与事务各按原报告范围；完整模型／GUI 待验 | ACP／插件边界各按原报告范围；完整模型／GUI 待验 |

## 托管任务、权限与恢复

| 能力 | Codex CLI | Claude Code | Grok Build |
| --- | --- | --- | --- |
| 传输 | 官方 app-server JSON-RPC | 双向 stream-json／control | ACP；根任务 leader 与 SDK direct 对照分别记录 |
| 新建／两轮／允许与拒绝 | 完整固定包 [原生适配器](CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION.md)和中间 macOS GUI 有证据 | [API 适配器](CLAUDE_API_ADAPTER_VERIFICATION.md)与固定策略 [GUI 阶段](CLAUDE_GUI_RUNTIME_CHAIN.md)有证据，失败阶段保留 | [source11 GUI](GROK_GUI_RUNTIME_CHAIN.md)根任务两轮及精确 Write 允许／拒绝通过 |
| 运行中追加 | 原生 `turn/steer`，必须已有 started；macOS GUI 实际采用追加内容 | 原生可并入当前工具执行或独立下一轮；必须逐 UUID 核对 ACK、合并及完整结果，不宣称 steer | 后续回合排队；保留提交代与执行代，source11／13 已实际核对，不宣称同回合 steer |
| 取消 | interrupt ACK 不等于取消；原生 Cancelled 与下一轮完成已有中间证据 | 批次三证据取消有阶段证据；source23等待Edit取消1整体FAILED：原生interrupt ACK／approval取消／错误result／cancelled生命周期已观察，应用未确认可信取消聚合，继续输入未发送 | source11 GUI 正文输出后真实 Cancelled，随后同连接继续通过；SDK7 清理不等于取消或完成成功 |
| 默认权限及配置副作用 | 已移除固定绕过审批／沙箱；普通子 pane 使用可见原生终端 | 可见子 pane 与托管入口已移除固定绕过；旧独立 AgentDriver 已关闭并移除全局信任／onboarding 改写；source22 回归通过，真实全局配置保持仍须进程证据 | 根任务 Inherit；SDK 创建与权限上限尚未获证据，子任务不开放 |
| 父子派发／双向 ACK／回收 | macOS 中间 GUI 生产协调器有派发、子进度、父追加、原生 ACK 与实际结果回收 | 固定 `ClaudeRestrictedFilesV1` 的 [独立父子夹具](CLAUDE_COORDINATOR_ACCEPTANCE.md)第2轮生产执行和修正审计通过：5输入、3执行、双向 ACK、inspect 与自动结果；旧运行器误拒绝记录保留。完整 GUI／最终提交待验 | source23 SDK8实际现代discover／tools/list／native ready注册已观察；inspect0、业务来源与父权限上限未验，整体FAILED、子任务关闭 |
| 权限上限 | 保存创建时父代完整快照，首输入前核对；未知／漂移／跨 CLI 不等价拒绝 | Inherit 不提供可证明的完整规则上限，派发拒绝；固定策略只允许已验证的同策略继承，非 OS 沙箱 | 根任务的审批证据不能用于子任务权限上限；来源缺失时不派发 |
| SQLite 任务／消息／结果 | 父子、代次、revision、原生 ACK 与结果历史；中间故障注入有证据 | 固定策略 GUI 与父子协调器记录有证据；合并输入保留原提交代和完整 UUID 列表 | source11 GUI 9消息／9代（7 Complete、2 Cancel），source13 生产 coordinator／SQLite 8输入／8结果实际核对 |
| 当前连接重新选择 | 复用活跃连接，不启动第二进程 | 同类路径；完整故障恢复待验 | source11 取消后继续复用原连接；历史继续才创建新连接 |
| 历史继续／应用重启 | GUI 完成与活动退出后的原 ID 显式继续、无重复执行有证据 | 固定策略GUI同ID／同策略历史继续有阶段证据；source23 PNG3修正就绪与身份判据后同原生S实际继续并回收第三结果，完整桌面重启／SQLite／GUI恢复仍待验 | source11 实际应用退出重开、原 ID 新进程历史继续、无自动重投及显式记忆回收通过；source13 夹具不是完整桌面重启 |
| 已绑定 PTY 未确认结果 | 保存 Unconfirmed，占用原会话／代次，不能生成最终成功结果 | 共用约束，与托管 Completed 独立 | 共用存储约束不等于子任务已开放 |
| 真正进程清理 | macOS 资源域的真实 Codex 工具／空闲崩溃有独立证据，最终提交复验待验 | source23 PNG3两代stdio／0与资源域清理有收据；Edit取消1stdio／1、清理true保留失败，真实异常工具崩溃待验 | source11／13 根任务正常 stdio／0 清理有证据；SDK7 exit=null、原始 wait=9、cleanup=true，不能记原生退出0 |

## 产品开放边界与消息契约

`LocalCLIManagedTasks` 同时控制入口与托管启动，默认关闭；开发验收使用显式开关。启动前重新探测受测版本，未验证版本只开放已经证明的模式。普通 PTY 不转换成协议任务，本机安装状态不证明 SSH 远端状态。

Grok官方根任务已有阶段GUI与生产coordinator证据，SDK／子任务仍关闭。source23 SDK8已实际使用现代2026-07-28发现协议并加载工具列表、原生ready；六个注册carrier仍无业务S/P/T字段，注册本身不能证明业务来源。inspect未调用、完整结果未回收，保留整体失败，不追认leader根因。PNG就绪与身份分离修复已在source23原生三输入通过，产品完整GUI与重启仍须分别验收。

消息保存明确的 `receipt_kind`：`native_protocol` 表示原生接收；`application_history` 只表示进入应用历史供正常后续请求读取。两者都不能独立证明模型执行完成。发送未确认记录、创建时父代、历史结果和分页查询保持关联；恢复不能自动重投已经执行的输入。完整实现边界见 [消息交付](OZ_LOCAL_MESSAGE_DELIVERY.md)、[Unconfirmed](UNCONFIRMED_TASK_STATE.md) 和原协调器报告。

## 最终验收仍未满足

| 要求 | 当前边界 |
| --- | --- |
| 三款 CLI 各自完整规定链路 | 中间有成功与失败阶段，最终实际修改提交未完成整链 |
| 普通 PTY 与托管模式分别覆盖 | 普通 Claude／Grok 全输入上下文链、PNG GUI、Grok SDK／子任务待验 |
| 负向与故障组合 | 已有定向与部分真实记录，完整插件／崩溃／重投／恢复失败组合待验 |
| 英文／简体中文 | source23实际dec提交完整 FTL 嵌入及 i18n 通过；source25候选i18n11通过，source11 Grok 布局有实证，最新完整布局待验 |
| 同提交三平台／全工作区 | 当前实际修改dec的macOS本地门禁／构建／签名通过；CI11 Linux／Windows均有Python失败，完整平台门禁未通过；full_workspace=false |
| SSH／tmux | 既有原生通知与传输边界保留，三方完整交互和恢复链待验 |

根代理最近核对 Linux／Windows runner 均 online、busy=false，派发前重新留证；不把可用性当门禁成功。source22 不可用入口提示已同步英文与简体中文；i18n门禁11项通过，最终双语检查待验。
