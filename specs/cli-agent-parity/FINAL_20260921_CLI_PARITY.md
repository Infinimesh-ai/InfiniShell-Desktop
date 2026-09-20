# CLI 对齐 Goal 最终验收（2026-09-21）

结论：Codex CLI、Claude Code 与 Grok Build 的 CLI 对齐 Goal 已完成。产品与跨平台验收代码提交为 `38a611773b8ee53860f9ab731b2476b9a40d1819`；本文件及历史证据归档位于其后的纯文档提交。旧报告中的 FAILED、未执行项和能力限制继续描述各自快照，不被本结论回填或改写。

## 最终受测对象

| 项目 | 固定值 |
| --- | --- |
| 分支 | `codex/cli-agent-parity` |
| 产品与验收代码 SHA | `38a611773b8ee53860f9ab731b2476b9a40d1819` |
| Codex CLI 正式版 | `0.155.1` |
| Claude Code 正式版 | `2.1.278` |
| Grok Build 正式版 | `1.0.34`；Alpha `1.0.38` 仅用于渠道往返验收 |
| 最终文档与证据 | 本文件所在提交 |

版本来源分别为 [Codex GitHub release](https://api.github.com/repos/openai/codex/releases/latest)、[Codex npm latest](https://registry.npmjs.org/@openai%2Fcodex/latest)、[Claude release latest](https://downloads.claude.ai/claude-code-releases/latest)、[Claude npm latest](https://registry.npmjs.org/@anthropic-ai%2Fclaude-code/latest)、[Grok stable](https://x.ai/cli/stable) 与 [Grok alpha](https://x.ai/cli/alpha)。版本探测不替代产品协议和生命周期验收；对应真实运行证据仍以本目录的逐阶段报告及 `validation/` 收据为准。

## 最终同提交门禁

### macOS arm64

在精确 SHA `38a611773b8ee53860f9ab731b2476b9a40d1819` 上执行：

- `cargo check -p warp`：通过。
- `cargo test -p warp --lib i18n::tests`：11 通过、0 失败、7346 filtered out。
- `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2 --exclude integration --exclude warp_tui`：第二次精确执行 `10548/10548` 通过、83 skipped、2 slow，245.046 秒。第一次同 SHA 执行最终返回 100，但当前 PTY 未保留失败明细；该次不计通过，也不用于推断具体失败原因。立即以只输出失败项并落盘的模式复跑后全绿。
- 最终代码之前的同产品快照已构建并严格签名主程序；二进制 SHA-256 为 `d4d1367f4c223f07485ca83a862724125b9a4cfab91494733326ebe4e66ebe13`，大小 746249456 字节。其后代码增量仅涉及验收稳定性、监督器 EOF／优雅退出顺序、测试同步和 CI 清理；最终精确 SHA 的完整 Rust 套件覆盖这些增量。

仓库原始全工作区命令还包含 `integration` 与 `warp_tui`。`integration` 是需要图形／SSH 环境的手工集成框架，不能在无相应会话的本地 nextest 中冒充通过；最终 workflow 编译该 crate，并单独执行 TUI、SSH worker、原生探针与桌面套件。上述拆分是明确的环境边界，不把跳过项记成通过。

### Linux 与 Windows

[最终跨平台 run 35513649252](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35513649252) 的 head SHA 精确为 `38a611773b8ee53860f9ab731b2476b9a40d1819`，两平台均 `completed/success`：

| 平台 | 重点门禁 | 完整桌面工作区 |
| --- | --- | --- |
| Linux | CLI 重点 Rust 2561、AI 75、`warp_cli` 129、TUI 9、command 1、监督器 live 4 均通过 | 10557 通过、88 skipped |
| Windows | CLI 重点 Rust 2457、AI 75、`warp_cli` 129、TUI 9、command 2、监督器 live 4 均通过 | 10318 通过、94 skipped；`local_control::handlers::layout::tests::tab_create_rejects_shell_parameter` 第 2/3 次通过，nextest 标记 1 flaky |

Windows job 在开始时停止可复用 sccache 并精确清理 `C:\infinishell-ci\cargo-target` 与 `C:\infinishell-ci\sccache`，清理后 C 盘可用 279845486592 字节；随后从冷状态完成全部构建和测试。workflow 对“没有运行中的 sccache server”显式归一为成功，避免把工具的陈旧退出码带入清理步骤。

[重点跨平台 run 35499678157](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35499678157) 绑定 `ca3c2f830`，Linux 与 Windows 的 CLI 重点门禁均通过。它作为较早重点收据保留，不替代上面的最终精确 SHA 全量 run。

## 产品、GUI、原生 CLI 与远程链

历史收据按受测快照共同组成验收链，最终同提交回归负责证明后续增量没有破坏能力：

- 三款 CLI 的新建、两轮交互、允许／拒绝审批、追加、取消、继续、应用重启、记录恢复和最终结果回收，分别保存在 Codex、Claude 与 Grok 的原生／GUI 专报及 `validation/` 安全收据中。
- 普通 PTY 与应用托管任务保持不同合同：普通终端没有可信结构化终态时显示“结果未知”，不会把 Stop 或进程清理冒充任务成功；托管任务只在匹配原生会话、回合、审批和结果账本后发布可信终态。
- 插件缺失、版本不兼容、CLI 崩溃、重复或乱序事件、旧回调、消息重投、来源漂移、升级事务回滚与恢复失败均有离线或真实范围相符的回归；原失败收据保留，不以之后成功覆盖。
- Claude `2.1.278` 固定策略、父子派发、双向 ACK、批量取消、待 Edit 审批取消和结果回收通过；Codex app-server 链与插件缓存恢复通过；Grok `1.0.34` ACP、插件通知、固定策略、租约和恢复链按开放能力通过。上游未提供或没有可靠来源的扩展能力继续关闭，不为了表面对齐而伪造支持。
- 英文与简体中文消息键、变量和硬编码扫描门禁通过；历史签名 GUI 已检查两种语言的版本、权限、安装／更新和普通终端布局。本轮最终修复没有新增或改变用户可见文案，**无需本地化变更**。

SSH／tmux 最终安全收据为 [macos-official-60-ssh-tmux-replay.safe.json](validation/macos-official-60-ssh-tmux-replay.safe.json)，SHA-256 `1fa7f3020210be70004e9b507ef950dc0ea449271639fc96cde643350ecb29d3`。它确认 `passed=true`、direct transport、tmux passthrough、默认 tmux blocking 与远端上下文；固定版本为 Codex `0.155.1`、Claude `2.1.278`、Grok `1.0.34`。范围是本机 OpenSSH 回环加真实 tmux 传输，不等于另一个操作系统主机，也没有从产品 SSH GUI 发起连接；这两项不改变远端传输合同的完成结论，平台特定逻辑由最终 Linux／Windows 门禁覆盖。

## PLAN 最终完成门槛

| 门槛 | 最终结论 |
| --- | --- |
| 1. P0–P5、插件、协议、夹具、发布文档与矩阵 | 已交付；本目录归档完整历史材料，最终代码已推送 |
| 2. 三款 CLI 完整生命周期，普通终端与托管任务分别留证 | 已满足；历史原生／GUI收据加最终回归共同覆盖，能力差异没有被抹平 |
| 3. 故障、乱序、重投、恢复失败与真实清理 | 已满足；负向用例与原始失败继续保留 |
| 4. 权限、父子双向消息、进度、结果与持久恢复 | 已满足；未知来源或不可靠上游能力保持关闭 |
| 5. 双语、check、i18n、受影响及仓库门禁 | 已满足；最终修复无需新本地化，精确 SHA 的门禁见上文 |
| 6. macOS、Linux、Windows、SSH／tmux 与真实 CLI | 已满足；平台与远程证据的明确边界见上文 |
| 7. 限制、失败、未执行项准确列出 | 已满足；没有把 skipped、历史失败、原生初始化或进程退出扩大为产品成功 |

## 保留限制

- 三款上游 CLI 的协议、模型、审批形状和扩展能力本来就不同；“对齐”是共同动作有可靠完成路径及能力差异可见，不是强行声明功能完全相同。
- 普通 PTY 缺少原生终态时，Stop 后仍可显示 Unknown；这是正确降级，不是遗漏的可信成功路径。
- Windows 完整套件的一项无关布局测试通过 nextest 重试，已如实标记 flaky；最终 job 与测试总数均成功。
- macOS 完整套件首次执行没有保留失败明细，不能推断原因；同一 SHA 的立即完整复跑全绿。两次结果均在本报告中保留。
- SSH／tmux 收据证明回环 OpenSSH、远端上下文和真实 tmux 字节路径，不声称完成异机 GUI 操作演示。
- `cargo fmt --all -- --check` 会报告仓库中本 Goal 范围外的既有格式差异；本次变更的 `git diff --check` 通过，未顺手格式化无关文件。

本报告取代 [2026-09-19 阶段性交接](HANDOFF_20260919_CLI_PARITY.md)作为当前入口。交接文件和此前各阶段“Goal 未完成”的文字是当时真实状态，继续作为历史证据保留。
