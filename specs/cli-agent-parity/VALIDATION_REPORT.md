# CLI 对齐验证记录

更新日期：2026-09-18（北京时间）。Goal 未完成。工作分支 `codex/cli-agent-parity`；本轮实际修改提交前的 HEAD 与根代理当时实际远端核对均为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`。本轮修改与证据将形成新检查点，实际平台运行与SHA另存；不将历史检查点或中间 dirty 快照计为最终同提交验收。

## 当前快照与门禁

source21 已冻结87路径，manifest SHA `966dfd9ff2bced152808fb6ee75bb2a7b41e6f87eabb57157fdd0a947f24dff0`，通过 check（63.194秒）、i18n11（178.715秒）、定向1270／5534 skipped（18.283秒）及 Python14组363（4.794秒）；lib SHA `10755f06d9e0d004c99551f24063660139f34bc8e9f07d08f3f9dd328d81a09e`。没有 source21 main／签名／真实CLI，见 [独立归档](OFFICIAL_21_LOCAL_GATES.md)。随后发现的旧独立Claude AgentDriver 已在主工作区关闭，固定绕过与全局写入实现已移除，普通终端和托管任务保留各自入口；source22 的93路径新快照（manifest SHA `2179e1a3add0f5507440f565b9fe2b4382ab68112555c313d9fc758073cb1482`）已通过 check（85.881秒）、i18n11（169.478秒）、定向1301／5493 skipped（18.693秒）及 Python363（4.831秒），lib SHA `752f2bd512cbe2bc874904a0b647350979a2b46c702254c43e37beecc96f450a`。五项新增配置与分派/auth回归全部实际PASS；编译有两项测试桥导出unused_import警告，退出0。见 [source22 独立归档](OFFICIAL_22_LOCAL_GATES.md)；没有22main、实际CLI或同提交平台结论。

历史已完成构建的 source20 为83路径清单 SHA `b3b3e7a61530b1314ef5e4f8de5f2c2b5bfb62f883e7b3e6109d42d2590fc693`。实际结果见 [独立本地归档](OFFICIAL_20_LOCAL_GATES.md)。

| 实际门禁 | 结果 | 墙钟时间／范围 |
| --- | --- | --- |
| `cargo check -p warp` | 退出0 | 60.240秒 |
| `cargo test -p warp --lib i18n::tests` | 11通过、0失败 | 146.350秒，内部4.45秒，6770 filtered out |
| 受影响模块 nextest | 1248通过、5533 skipped | 17.632秒，内部14.685秒，不等于全工作区 |
| Python离线回归 | 13组／329项通过 | 4.565秒；SDK组77项，Codex准备器27项 |
| source20 main构建 | 退出0 | 196.531秒 |
| `codesign --verify --deep --strict` | 退出0，双输出0字节 | 0.480609秒，不证明发布者／公证／公开发布 |
| 双语完整资源嵌入 | 原构建报告记录两份完整FTL存在 | en 374753字节／zh-CN 361658字节；不等于布局检查 |

source20 lib SHA 为 `c5819a0d7e73031b3ce19716e27e143b3188e03a5c6e7b0bc5e1a8b2a88c7716`，main／监督worker为 `8b1ad22b400b71b13e08ca6d9f8fdcc3b78da69c965d3fd9366a8cfa1ed3ddea`。身份来自原运行／构建报告，独立归档核对报告及日志，没有重算二进制或重新查询内核。当前文档整理也未重跑门禁。

source21 已编译现代MCP、PNG Resume就绪／身份分离及待定Edit取消夹具并通过本地门禁，真实验证仍未进行。旧独立Claude入口修复将作为source22新增验证；它不属于source21通过范围。

## 当前真实CLI证据

| 原报告／实际来源 | 已证明步骤 | 未扩大为通过的范围 |
| --- | --- | --- |
| [Codex完整包原生复验](CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION.md) | 37b0bc732干净macOS适配器的新建／两轮／审批／追加／取消／同ID进程历史继续、工具恢复、localImage | 当时提交的原生适配器证据，不替代当前GUI／三平台／最终提交 |
| [Codex隔离GUI](validation/macos-gui-report.md) | 中间bundle两轮、允许／拒绝、真实Steer与取消、应用退出重启、原ID显式继续、技能／图片／评审、父子消息和实际回收 | bundle3–6各自边界保留；候选列表、普通其他CLI、完整故障组合及最终提交待验 |
| [Claude API适配器](CLAUDE_API_ADAPTER_VERIFICATION.md) | 固定2.1.273真实两轮、精确文件审批、排队确认、取消与原ID新进程恢复标记 | dirty中间快照；不替代完整父子GUI和最终提交 |
| [Claude固定策略GUI](CLAUDE_GUI_RUNTIME_CHAIN.md) | Edit允许／拒绝、实际合并追加、source8原ID／同策略历史继续与回收 | source6第一精确夹具及第五取消失败保留；英文source8／9动态FTL边界保留，最新完整双语未验 |
| [Claude父子协调器](CLAUDE_COORDINATOR_ACCEPTANCE.md) | 第2轮生产执行5输入／3执行／2合并，双向原生ACK、inspect与自动结果，修正独立审计通过 | 原libtest退出0，但旧wrapper退出1／metadata=false不改写；修正审计不是新真实重跑，不覆盖GUI／全重启／跨平台 |
| [Grok source11 GUI](GROK_GUI_RUNTIME_CHAIN.md) | 根任务9消息／9代、7 Completed／2 Cancelled，Write允许／拒绝、排队、正文后取消、同连接继续、实际应用重启、原ID历史继续、无自动重投及显式记忆回收 | 固定1.0.30、Inherit根任务、source11 dirty；36张实际JPEG与对应EN／ZH布局，不覆盖最新全界面、SDK／子任务或其他平台 |
| [Grok source13生产coordinator](GROK_COORDINATOR_ACCEPTANCE.md) | 生产runtime commands与SQLite实际8输入／ACK／Started／完整结果，6 Completed／2 Cancelled；原提交5到执行6关联、原ID新进程load及记忆回收、两次正常清理 | App::test不是GUI；不是完整桌面重启／活跃重关联，不验证父子权限或SDK；审批运输回执不冒充原生审批ACK |
| [Claude PNG2／SDK7](SOURCE20_NATIVE_DISCOVERY.md) | 下列source20真实改善与失败均原字节保存 | 两次test101／wrapper1、整体FAILED；不计任何整轮成功 |

### source20 PNG2：恢复判据提前失败

PNG识色与中文多行两次完整结果及原生来源摘要通过，首代明确 `stdio_closed`、exit0、Job／CID清理。之后磁盘附件哈希／大小和typed引用一致，`image_replayed_to_native=false`；只是文件引用恢复，不是原生图片重投或全继续链通过。

Resume最新startup的原生ID为null／关联false，夹具以 `resume_ready_identity_failed` 提前拒绝，第三input未发。第二代回执也是stdio／0／清理确认，但不能据此确认恢复到同一原生历史或回收成功；metadata中的整体acceptance和durable restore仍为false。新候选须在后续实际ACK／输入／结果确认同ID，不能把启动就绪自身当成会话身份。

### source20 SDK7：首次发现回调，不是业务工具结果

首次真实 `_x.ai/mcp/sdk_call → server/discover`，outer／inner ID为0、SDK请求1；SDK参数键为message／serverId，内层只有_meta。公开版本投影null不能证明_meta内没有版本；三个散列键的标准发现关联来自根代理官方接口核对，实际值没有在独立归档读取。

六个carrier的session／prompt／tool来源字段均缺失或null，但这是发现阶段，不推断后续业务工具也无来源。应用submitted1，原生输入确认0，initialize／tools-list／inspect／审批0，没有业务ACK／Started／Result。来源unknown、产品SDK／子任务gate关闭。退出回执 `exit_code=null`、原始wait9、cleanup=true；不可描述为原生退出0。SDK6旧失败保留，direct首次回调不证明leader根因。

## 完整验收矩阵

以下是用户规定的最终门槛。“中间证据”仅证明原快照相应步骤；三款CLI均未在最终实际修改提交满足全部组合。

| 验收 | Codex | Claude | Grok |
| --- | --- | --- | --- |
| 新建／两轮／完整结果 | 原生与macOS GUI中间通过 | API与GUI有阶段证据，PNG2前两轮通过 | source11 GUI及source13 coordinator根任务通过 |
| 审批允许／拒绝 | 中间GUI与实际文件效果通过 | 固定策略API／GUI通过；完整父子GUI待验 | 根任务精确Write通过；子任务上限未验 |
| 运行中追加／原生接收／模型采用 | GUI真实Steer及父→子采用有证据 | API排队、GUI／父子实际合并与完整UUID结果有证据 | 根任务后续回合排队有证据，不宣称steer |
| 取消／继续 | 中间真实取消与继续有证据 | 批次三证据取消有证据；等待Edit取消旧GUI失败、新真实夹具未运行 | source11正文后取消与同连接继续通过；SDK7清理不替代业务取消 |
| 应用重启／恢复／回收 | bundle历史与活动退出后显式继续有证据 | 固定策略GUI历史继续有证据；PNG2第3输入未发／恢复未通过 | source11真实应用重启与历史记忆通过；最终提交整链待验 |
| 父子双向消息／进度／结果 | macOS中间GUI有实际闭环 | 固定策略独立协调器执行与修正审计通过，完整GUI待验 | SDK发现尚未进入业务工具，未开放子任务 |
| 插件缺失／版本／禁用／更新失败恢复 | 各原生／GUI／平台子集有记录，完整故障组合待验 | 固定缓存与升级／强杀重试有记录，完整组合待验 | 安装器事务子集有记录，完整平台组合待验 |
| 崩溃／重复／旧回调／重投／恢复失败 | 定向故障与macOS真实崩溃清理有证据，最终提交待验 | 共享回归不能替代真实异常工具崩溃；完整组合待验 | 共享回归及根任务正常清理不替代真实崩溃／SDK故障整链 |
| 双语布局 | 旧bundle布局有证据，最新全功能待验 | 固定策略阶段布局有证据，动态FTL边界及最新GUI待验 | source11内嵌EN／ZH实际布局通过，最新完整布局待验 |
| 同提交macOS／Linux／Windows、全工作区、SSH／tmux | 最终组合未完成 | 最终组合未完成 | 最终组合未完成 |

## 平台、发布与实质限制

[CI10 b3d8](TENTH_PLATFORM_RUN_B3D8.md)的Linux x64／Windows x64所选门禁通过，`full_workspace_tests=false`；不包含其后固定策略、完整历史及布局修改。Windows实际五项Codex hook注册、两项ConPTY通知通过，另外三项实效不能从注册推定。固定完整运行包的新准备器27项离线回归与异平台静态提取见 [准备说明](CODEX_FIXED_RUNTIME_PREPARATION.md)；异平台提取不是异平台执行，Windows ACL及ARM64执行边界仍按原报告保留。

根代理最近实际API核对Linux／Windows runner均online、busy=false；00:01的Linux离线观察保留为 [历史可用性](validation/runner-availability-20260918-0001.json)。真正dispatch前须重新核对留证，当前可用性不计新的平台门禁通过。不存在整个Goal因此无法独立推进的结论。

SSH／tmux的 [构造通知](SSH_TMUX_VERIFICATION.md) 与 [真实Codex原生TUI传输](CODEX_NATIVE_SSH_TMUX_VERIFICATION.md)分开记录；模型HTTP为零的探针不等于三方完整在线交互、输入、审批、重启和恢复。远端只通过仓库cross-platform-preflight workflow，在本地新门禁通过、实际修改提交推送且headSha一致后验证。最终按仓库要求执行全工作区门禁，不能以选定筛选覆盖替代。

Codex默认shell_snapshot在含引号与命令替换字符CODEX_HOME中的上游路径限制仍有 [独立原记录](validation/macos-codex-shell-snapshot-path-limitation.json)，隔离探针关闭特性的正向结果不改变默认限制。Claude固定文件策略不是OS文件／网络沙箱，Inherit观察不能证明任意完整父权限上限。Grok SDK业务来源、现代发现版本、子任务权限及兼容Claude hooks完整链仍待验；旧额度／未登录失败保留历史，不再把已完成的官方在线授权和根任务成功说成当前无法使用。

## 下一轮交付必须补齐

1. source21本地门禁通过后发现遗留Claude入口，修复和source22新check／i18n／focused（包含旧harness1301项）／Python363已通过，接着精确commit／push／CI11；不做dirty21 main或native。随后隔离验证树检出同一新实际提交的干净源码，复验门禁并构建main／严格签名，再运行PNG3／SDK8／待Edit取消原生首轮；不能改写source20结果。
2. PNG恢复第3输入的原生同ID／无重投／完整记忆结果，现代SDK发现与业务工具来源账本，等待Edit审批真实取消及随后继续。
3. 普通三方终端、附件／上下文／技能／评审、插件负向与故障、父子GUI与恢复的完整验收；最新英文／简体中文布局分别留证。
4. 包含实际修改的同一提交macOS／Linux／Windows相关验证、完整工作区与独立SSH／tmux链；真实CLI证据与Actions head SHA相符。
5. 完整能力矩阵、插件／发布配套、文档和验证报告；所有外部限制及失败准确列出，只有全部门槛满足才完成Goal。

source22 新增旧独立入口不可用提示，英文与简体中文同键、同 `$cli` 变量同步；i18n11与编译门禁已通过，最终双语布局待验。用户排除的两个网页搜索文件与整个validation/gui-7e065085目录继续保留并排除本目标暂存。

## 历史原始报告索引

历史检查点和中间快照只保留当时结论，不作为当前HEAD或最终验收。下列独立专报、原始JSON、失败和修正审计均留在原文件；本次未修改或重新运行它们。具体受测源码、二进制、错误及未覆盖项以原报告为准，不能因为存在文档链接而自动视为已审。

### 平台历史

[TENTH_PLATFORM_RUN_B3D8](TENTH_PLATFORM_RUN_B3D8.md)、[NINTH_PLATFORM_RUN_E4738E1EA](NINTH_PLATFORM_RUN_E4738E1EA.md)、[EIGHTH_PLATFORM_RUN_494569582](EIGHTH_PLATFORM_RUN_494569582.md)、[SEVENTH_PLATFORM_RUN_37B0BC732](SEVENTH_PLATFORM_RUN_37B0BC732.md)、[THIRD_PLATFORM_RUN_328D5ED35](THIRD_PLATFORM_RUN_328D5ED35.md)、[WINDOWS_CONPTY_NOTIFICATION_PROBE](WINDOWS_CONPTY_NOTIFICATION_PROBE.md)、[SIXTH_PLATFORM_RUN_7E0650855](SIXTH_PLATFORM_RUN_7E0650855.md)、[FIFTH_PLATFORM_RUN_94A412EB](FIFTH_PLATFORM_RUN_94A412EB.md)、[WINDOWS_VERIFICATION_FIXTURES](WINDOWS_VERIFICATION_FIXTURES.md)。

### 本地冻结与构建历史

[OFFICIAL_11_LOCAL_GATES](OFFICIAL_11_LOCAL_GATES.md)、[OFFICIAL_12_LOCAL_GATES](OFFICIAL_12_LOCAL_GATES.md)、[OFFICIAL_13_LOCAL_GATES](OFFICIAL_13_LOCAL_GATES.md)、[OFFICIAL_20_LOCAL_GATES](OFFICIAL_20_LOCAL_GATES.md)、[OFFICIAL_21_LOCAL_GATES](OFFICIAL_21_LOCAL_GATES.md)、[OFFICIAL_22_LOCAL_GATES](OFFICIAL_22_LOCAL_GATES.md)、[OFFICIAL_18_LOCAL_GATES](OFFICIAL_18_LOCAL_GATES.md)、[OFFICIAL_19_LOCAL_GATES](OFFICIAL_19_LOCAL_GATES.md)、[OFFICIAL_15_LOCAL_GATES](OFFICIAL_15_LOCAL_GATES.md)、[OFFICIAL_14_LOCAL_GATES](OFFICIAL_14_LOCAL_GATES.md)、[OFFICIAL_16_LOCAL_GATES](OFFICIAL_16_LOCAL_GATES.md)、[OFFICIAL_17_LOCAL_GATES](OFFICIAL_17_LOCAL_GATES.md)。

### 真实任务与GUI历史

[GROK_GUI_RUNTIME_CHAIN](GROK_GUI_RUNTIME_CHAIN.md)、[GROK_OFFICIAL_ONLINE](GROK_OFFICIAL_ONLINE.md)、[CLAUDE_GUI_RUNTIME_CHAIN](CLAUDE_GUI_RUNTIME_CHAIN.md)、[CLAUDE_BATCH_CANCEL](CLAUDE_BATCH_CANCEL.md)、[GROK_COORDINATOR_ACCEPTANCE](GROK_COORDINATOR_ACCEPTANCE.md)、[SOURCE13_NATIVE_CALIBRATION](SOURCE13_NATIVE_CALIBRATION.md)、[SOURCE20_NATIVE_DISCOVERY](SOURCE20_NATIVE_DISCOVERY.md)、[CLAUDE_APPROVAL_CANCEL](CLAUDE_APPROVAL_CANCEL.md)、[SOURCE18_NATIVE_FAILURES](SOURCE18_NATIVE_FAILURES.md)、[CLAUDE_GUI_E473_A245](CLAUDE_GUI_E473_A245.md)、[CLAUDE_API_ADAPTER_VERIFICATION](CLAUDE_API_ADAPTER_VERIFICATION.md)、[CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION](CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION.md)。

### 协议、权限、插件和恢复边界

[SDK_ORIGIN_PROBE](SDK_ORIGIN_PROBE.md)、[GROK_SDK_ROUTING_DIAGNOSTIC](GROK_SDK_ROUTING_DIAGNOSTIC.md)、[CLAUDE_QUEUE_AND_CODEX_REV4](CLAUDE_QUEUE_AND_CODEX_REV4.md)、[PROTOCOL_EVIDENCE](PROTOCOL_EVIDENCE.md)、[STOP_HOOK_COMPLETION_AUDIT](STOP_HOOK_COMPLETION_AUDIT.md)、[UNCONFIRMED_TASK_STATE](UNCONFIRMED_TASK_STATE.md)、[OZ_LOCAL_MESSAGE_DELIVERY](OZ_LOCAL_MESSAGE_DELIVERY.md)、[CODEX_HOOK_TRANSPORT_VERIFICATION](CODEX_HOOK_TRANSPORT_VERIFICATION.md)、[CODEX_NATIVE_SSH_TMUX_VERIFICATION](CODEX_NATIVE_SSH_TMUX_VERIFICATION.md)、[macos-gui-bundle7-plugin-report](validation/macos-gui-bundle7-plugin-report.md)、[CLAUDE_NO_CREDENTIALS_VERIFICATION](CLAUDE_NO_CREDENTIALS_VERIFICATION.md)、[DARWIN_COALITION_LAUNCHD_AUDIT](DARWIN_COALITION_LAUNCHD_AUDIT.md)、[DARWIN_LAUNCHD_COALITION_PROTOTYPE](DARWIN_LAUNCHD_COALITION_PROTOTYPE.md)、[DARWIN_COALITION_PRODUCT_CONTRACT](DARWIN_COALITION_PRODUCT_CONTRACT.md)、[GROK_BYOK_AUTH_VERIFICATION](GROK_BYOK_AUTH_VERIFICATION.md)、[GROK_TITLEBAR_ENTRY](GROK_TITLEBAR_ENTRY.md)、[CODEX_FIXED_RUNTIME_PREPARATION](CODEX_FIXED_RUNTIME_PREPARATION.md)、[macos-gui-bundle8-report](validation/macos-gui-bundle8-report.md)、[GROK_FIXED_ACP_BOUNDARIES](GROK_FIXED_ACP_BOUNDARIES.md)、[CLAUDE_PLUGIN_UPGRADE_TRANSACTION](CLAUDE_PLUGIN_UPGRADE_TRANSACTION.md)。
