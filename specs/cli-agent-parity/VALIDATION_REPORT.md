# CLI 支持与能力对齐：验证结论

本报告保留截至 2026-09-25 的固定版本验收结论、源码来源和未验边界，不保存逐轮日志或截图。**这是阶段交付，完整 Goal 尚未完成。** 当前状态以 [CURRENT_STATUS](CURRENT_STATUS.json) 为准；剩余功能、优先级和关闭条件统一维护在 [KNOWN_GAPS](KNOWN_GAPS.md)。

本次目录整理没有修改产品，也没有重新执行下列测试。历史通过、失败与跳过继续按原提交和实际场景计证，不能外推到整理后的新提交、其他版本或全部平台。

## 验收基线与追溯

| 项目 | 固定来源与解释 |
| --- | --- |
| CLI 版本 | Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41` |
| 产品行为冻结 | `ee839b4fc7dacd58bac0806aa549500af5776b30`；包含桌面 Grok 默认开放、启动取消修复和双语说明修正 |
| 最新相关验证提交 | `50e1515bcd3f20cf120edf5565766975b6a68d5d`；只完善测试与运行器，未再次修改产品行为 |
| Mac 模型与 GUI 实链 | 来自 V2–V13 等实际构建批次及源码摘要；后续按变化域复验，不能重标为 `ee839b4fc` 或 `50e1515bc` 全量重跑 |
| 历史档案固定提交 | `e3ef39689cd8b686dfe040b90217ed1067b4d268`；保存原过程专报、失败、通过、截图与摘要索引 |

原始材料从当前目录移出后，仍可从[固定历史目录][history]追溯，或使用 `git show e3ef39689cd8b686dfe040b90217ed1067b4d268:specs/cli-agent-parity/<历史路径>` 读取。历史总体“全部完成”判断已被当前状态撤回；保留原文件不表示继续认可其总体判断。

关键来源是[固定版本验收表][acceptance]、[Mac 交付记录][mac]和[同提交补验报告][final]。这些固定提交链接用于复核旧结论，不承担当前待办管理。

## 三平台相关门禁

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

## 固定版本产品链已验范围

以下主要来自 Mac 的已认证原生执行及真实 GUI，均保留原构建与模型来源。Grok 模型为 `grok-4.7`，Claude 父子链为 `claude-sonnet-4-6`；不同模式不能相互替代。

| 能力 | 已验结论 | 范围限制 |
| --- | --- | --- |
| Codex 托管父子任务 | V9 双向原生 ACK、父权限上限、自动结果 ACK、显式继续后持久结果回收通过 | 不是完成态父任务自动唤醒证明，也没有该用例的子文件效果或 GUI 证明 |
| Claude 托管父子任务 | 审批、工具、子文件效果、双向消息 ACK、inspect、结果 ACK 与清理通过 | 仅相应固定权限策略，不代表任意工具或原生 shell 可用 |
| Grok 托管根与子任务 | 根任务两轮、允许／拒绝、排队、取消和同会话恢复通过；V8 父子 ACK、结果回收及跨宿主冷恢复通过 | 固定读取／文件策略与继承设置模式分别验收，不能混称操作系统沙箱 |
| 三款 GUI 恢复 | 存活宿主重新关联、同原生会话冷恢复及结果回收有实链；重启不重投旧输入 | 源码代次与宿主身份必须核验；不等于所有平台均完成该链 |
| Grok 多轮重启 | V10 两轮后应用重启，输入数未变，第三轮正确回忆；V11 空闲目录刷新不新增回合 | 目录刷新不等于任意新技能均能在运行中使用 |
| Codex 富输入 | 附件类型化通道、技能、评审意见及文件引用实际进入原生请求 | 图片颜色回答错误，不能计图片理解成功 |
| Claude 富输入 | PNG 原始字节摘要一致，图片回答正例、单技能执行及重启恢复通过 | 纯图片、其他格式、图片与技能混用仍有缺项 |
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
