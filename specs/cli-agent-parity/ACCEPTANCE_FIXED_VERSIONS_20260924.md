# 固定版本验收对应表（2026-09-24）

本表把原 PLAN P0–P5、REOPEN R1–R8 与本轮实际证据对应。版本固定为 Codex `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41`，不因上游发布新版本重新开始本轮。当前仍在最终验证，不能据此标记 Goal 完成。

代码沿用 `codex/cli-agent-parity`，开发基线 `104f5b8a1e857c49c5bbb782c78ddbfd89eef628`。Mac 实链按各自二进制和源码域留证；后续改动通过相应回归补验，不把旧收据改写成最终提交实测。详细来源、失败和平台边界见 [Mac 交付记录](MAC_FIXED_VERSION_DELIVERY_20260924.md)。

| 要求 | 主要代码 | 版本／模式／平台及现有证据 | 当前结论 |
| --- | --- | --- | --- |
| P0 身份、协议和能力；R1 Grok 扩展 | `app/src/ai/cli_agent_runtime/{codex,claude,grok}.rs`、`grok_profile.rs` | 固定三款，Mac 正式托管；Grok 固定读取、文件、单技能和原生目录均有实链；[目录刷新 V11](validation/macos-fixed-versions-20260924/gui-grok-idle-catalog-v11.safe.json)覆盖最终目录字段修复 | Mac 对应能力通过；其他平台等集中 CI |
| P1 富输入、附件、上下文、技能、评审；R6 | `task_manager_input.rs`、`task_manager_view.rs`、终端 `use_agent_footer` | [Codex GUI](validation/macos-fixed-versions-20260924/gui-codex-composer-v2.safe.json)、[Claude 技能／图片／恢复](validation/macos-fixed-versions-20260924/gui-claude-composer-recovery-v7.safe.json)、[Grok 技能](validation/macos-fixed-versions-20260924/grok-selected-skill-v7.safe.json)、[真实 Mac IME](validation/macos-fixed-versions-20260924/ime.safe.json) | Mac 输入路径通过；图片字节投递与模型理解分别记录；跨平台系统剪贴板 GUI 待 CI |
| P2 事件和通知 | `app/src/terminal/cli_agent_sessions`、随附三款插件 | 固定三款普通 PTY 与产品接收收据；Grok 0.1.4 增加原生权限通知分类及已知旧版迁移 | [V13 审批索引](validation/macos-fixed-versions-20260924/ordinary-pty/grok-permission-v13/index.safe.json)：真实审批提醒出现、单次允许后替换为 Unknown、文件效果和退出清理通过；不伪造回合阻塞状态 |
| P3 权限和配置保全；R5 默认入口 | `permissions.rs`、`grok_profile.rs`、`app/src/features.rs` | 固定三款明确审批、父权限上限、默认构建 GUI；[V11 默认构建](validation/macos-fixed-versions-20260924/build-default-goal-v11.safe.json)没有托管编译 feature | Mac 正式入口和相应策略通过；Grok 继承设置与固定策略边界保留 |
| P4 父子任务和双向 ACK；R2 | `coordinator.rs`、`runtime_host.rs`、持久任务和信箱 | [Codex V9 双向 ACK](validation/macos-fixed-versions-20260924/codex-parent-child-duplex-v9.safe.json)、[Claude 父子](validation/macos-fixed-versions-20260924/native-summary.safe.json)、[Grok V8 父子冷恢复](validation/macos-fixed-versions-20260924/grok-parent-child-v8.safe.json) | 三款对应原生链通过；显式继续、自动结果入队／ACK、持久 inspect 分别计证 |
| P4 普通终端与托管生命周期；R4 | 三款适配器、`runtime_host.rs`、`coordinator.rs`、终端会话模型 | [普通 PTY 16 输入](validation/macos-fixed-versions-20260924/ordinary-pty/archive-index.safe.json)、三款 GUI 重启／同会话冷恢复、[V10 多轮重启](validation/macos-fixed-versions-20260924/gui-grok-two-turns-restart-v10.safe.json) | 对应链通过；普通 Codex Escape 不保证工具退出，保留未知状态和显式关闭终端，不冒充托管清理 |
| SSH／tmux；R3 | shell hook、通知 worker、终端可信输入绑定 | 固定三款真实 CLI 触发、真实本机隔离 OpenSSH、产品 PTY 接收、tmux 断连重接；各自收据见交付记录 | 对应 Mac→本机 SSH 场景通过；不是其他 OS 远端或所有组合；关闭透传复用公共解析器风险验证 |
| CLI_AUTOUPDATE；R7 | `cli_agent_updates`、`managed_process_atomic_*` | [Mac 三款正式升级](validation/macos-fixed-versions-20260924/formal-updates-v4.safe.json)、[Claude Latest→Stable 降级](validation/macos-fixed-versions-20260924/claude-stable-downgrade-v10.safe.json)、Busy／回滚／恢复回归 | Mac 已识别原生来源通过；Linux／Windows 真正更新事务待集中 CI；未知来源降级保持 |
| P5 双语和本地门禁 | `app/i18n/{en,zh-CN}`、Rust／Python／Node 回归 | [V11 本地索引](validation/macos-fixed-versions-20260924/mac-v11-local-index.safe.json)：check、libtest、i18n 11、定向 1645、原子 15；[双语布局](validation/macos-fixed-versions-20260924/gui-bilingual-final-v11.safe.json) | 上述通过；[完整桌面首次结果](validation/macos-fixed-versions-20260924/goal-v11-workspace-result.safe.json)为 10858 通过、3 前置超时、109 跳过，[3 项同二进制串行复验](validation/macos-fixed-versions-20260924/workspace-host-serial-v11.safe.json)通过，原失败保留 |
| P5 平台、提交和报告；R8 | `.github/workflows/cross-platform-preflight.yml`、本表和验证收据 | 产品提交 `2c483322f00d569bc126bfa96616e2920083f872` 已推送；测试及 CI 修正 `faab52d8a41f9d47530f55c9bc73d13f1c115803` 的集中复验已取得 Linux 全量通过；`cbdd0254e4f0439ff16ac06bb8e8712490c0f876` 的[剩余项集中复验 35992533669](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35992533669)在持久化探针处失败，修复后重验 | 待剩余平台结果和报告闭环；[当前源码对应](validation/macos-fixed-versions-20260924/source-binding-cbdd0254e.safe.json)单列 Windows 终端修复，Mac 产品行为保持 |

验证范围明确区分：真实 GUI 运行、GUI integration 编译、系统剪贴板、真实 IME、模拟协议、原生 CLI 执行和模型结果。跳过项不计通过，已记录失败不删除或追改。Grok 托管图片不支持，固定策略禁用原生 shell／hook／技能；这些限制应由界面与能力矩阵明确表达。

产品源码已冻结并推送为 `2c483322f00d569bc126bfa96616e2920083f872`。[来源对应记录](validation/macos-fixed-versions-20260924/final-source-binding-2c483322f.safe.json)确认 V13 的 125 个变更源文件和历史插件编译夹具均与提交一致，不重标旧构建收据；[提交后 Mac 门禁](validation/macos-fixed-versions-20260924/mac-commit-gates-2c483322f.safe.json)中的 check／i18n 亦通过。统一 CI 启用 Linux x64、Windows x64、完整桌面集合、固定原子更新与 Windows 调试监督器门禁；没有启动 Mac Intel 备用验证。

渠道切换按实际变化域关联证据：[源码对应审计](validation/macos-fixed-versions-20260924/channel-source45-current.safe.json)核对了 source45 的精确源码，渠道解析、配置发布／恢复和 Codex 更新标记等 22 个函数段与当前逐字相同。历史 Codex latest↔alpha、Grok stable↔alpha 及同版本渠道同步保留原版本／源码范围；后续启动绑定、journal 和原子监督变更由 V4 三款正式升级及本轮平台事务验证覆盖，Claude 命令与私有配置变化另由 V10 真实降级覆盖。现行渠道、恢复和 CAS 回归补验变化，不把旧渠道收据重标为最终提交实测。

运行中升级保护由三款真实会话模型事件→更新管理器延期→最后会话退出后单次派发的回归，以及启动保留期间禁止升级的回归覆盖。原计划要求该行为成立，没有额外要求通过模型输入或 GUI 点击触发。普通 Codex PTY 的 Escape 只计原生轮次中断；工具自然完成，不能计工具取消或进程树清理，继续保留 Unknown 与显式关闭终端的降级边界。

首次统一 CI 的失败按原样保留：[Linux](validation/macos-fixed-versions-20260924/ci-35973064369-linux-final.safe.json)定向 2,846 项通过、1 项旧候选权限预期失败；真实 GUI 在创建窗口前缺少 `libXcursor`，没有进入剪贴板断言。Windows 的 [PATH 测试](validation/macos-fixed-versions-20260924/ci-35973064369-windows-grok-runner.safe.json)与 [Codex 临时目录清理](validation/macos-fixed-versions-20260924/ci-35973064369-windows-codex-hooks.safe.json)失败分别修复，原生通知 case 与 ConPTY 的通过保持各自边界。当前修正只涉及测试与 CI 环境，没有改变已验收的 Mac 产品行为；[本地权限 12 项、i18n 11 项和 check](validation/macos-fixed-versions-20260924/ci-fixes-local-gates.safe.json)、[清理脚本回归](validation/macos-fixed-versions-20260924/ci-fixes-codex-cleanup-local.safe.json)及 [workflow 静态检查](validation/macos-fixed-versions-20260924/ci-fixes-workflow-static.safe.json)通过。依赖步骤被跳过的结果不计通过，修正后的远端复验仍待执行。

Windows 首次定向为 2,716 项通过、1 项同样的旧候选权限预期失败。真实 GUI 编译通过，运行在 DXGI swapchain 创建时报 `0x887A0022`，没有窗口、剪贴板断言或截图通过证据。[私有 Mesa WGL 环境脚本](validation/macos-fixed-versions-20260924/ci-fixes-windows-gui-mesa-static.safe.json)已通过本地语法及固定包摘要检查，远端验证仍待执行：它复制同一构建的 EXE 和运行库，使用固定 Mesa `26.2.1` 的两个 DLL，不改产品或系统安装；必须同时取得实际 GL/llvmpipe 窗口渲染日志、同二进制剪贴板收据和两张非空截图才计通过。

修正已推送为 `faab52d8a41f9d47530f55c9bc73d13f1c115803`。集中复验使用指向同一提交的 `codex/cli-agent-parity-verify-faab52d8a` 引用，Linux 利用空闲 runner 先执行，Windows 排队接续首轮全量；独立并发组保留首轮作业，不将其取消。两次 run 的失败和通过分别留证。

首轮 Windows [最终回执](validation/macos-fixed-versions-20260924/ci-35973064369-windows-final.safe.json)为全量 10,659 项通过、1 项同样的旧候选权限断言失败、110 项跳过。5 项 nextest leaky 表示测试退出后的输出句柄迟闭，保留其名称及边界，不作为进程树清理通过证据。

集中复验中 Windows 暴露两项脚本边界：[私有配置 LF／CRLF](validation/macos-fixed-versions-20260924/ci-35985989728-windows-offline-step9-original.safe.json)与 [Codex 主进程自然退出后输出 reader 未全结束](validation/macos-fixed-versions-20260924/ci-35985989728-windows-codex-01561-partial.safe.json)。前者显式写入 LF，并把 35 个独立离线脚本改为全部执行后统一报错；固定 CLI 准备和原生探针只依赖各自真实前置，失败状态仍保留。本机 PowerShell 批次 [797 项通过、7 项平台跳过及 cargo check](validation/macos-fixed-versions-20260924/ci-followup-offline-batch-local.safe.json)不能替代 Windows 实测。后者保留完整原生 hook 终态、配置恢复与 ConPTY 通过证据，EOF 超时仍按失败处理；没有依据将它认定为后代已清理。上述变化无需本地化变更。

EOF 补验改用两路共享 5 秒事件截止时间，记录通道、真实 EOF、线程存活和耗时；读取或解码失败、超时仍严格失败，始终不宣称 Job 后代清理。[真实短命子进程回归](validation/macos-fixed-versions-20260924/ci-followup-codex-eof-local.safe.json)覆盖继承 stdout 延迟关闭、超时后的失败快照不变、非法 UTF-8 三个边界。改动后的 [最终 35 脚本批次](validation/macos-fixed-versions-20260924/ci-followup-final-local-gates.safe.json)为 800 项通过、7 项按平台跳过；当前源摘要已附，Windows 原生结果仍须后续复验。

集中复验的 [Linux 最终结果](validation/macos-fixed-versions-20260924/ci-35985989728-linux-final.safe.json)已确认全量 10,899 项通过、98 项跳过，定向 2,847 项通过，Grok 0.1.4 六阶段正式安装／升级及 Codex 原生恢复通过。原子更新 12 项均在夹具准备时报 `binary_not_regular`，没有执行产品升级；[原失败分析](validation/macos-fixed-versions-20260924/ci-35985989728-linux-atomic-failure.safe.json)保留大小与普通文件合并报错的不确定性。[运行器修正](validation/macos-fixed-versions-20260924/ci-followup-linux-atomic-local.safe.json)将构建产物摘要上限与正式 CLI 的 1 GiB 限制分开，下一轮先记录构建文件类型和大小。真实 Linux GUI 已创建窗口，但中文草稿断言失败；测试补齐输入框焦点前置，仍要求真实 Ctrl-V、中文精确一致、PNG 像素和零提交。

Windows 的 [三款正式更新原失败](validation/macos-fixed-versions-20260924/ci-35985989728-windows-atomic-failure.safe.json)保留不变：Codex／Grok 返回 `CommandFailed`，Claude 已达到目标摘要／版本但原生退出收据判定失败，三款配置保全、journal 清除和严格 Job 清理均通过。Claude 收据改为允许先观察到 `StdioClosed`，同时仍强制实际退出码 0、严格 Job 与清理确认；停止／宿主断开不接受。失败的产品执行另用独立私有副本诊断 loader 与原生退出，不把诊断成功替代产品失败，不放宽 DLL 信任检查。

Windows Mesa 已实际创建 GL/llvmpipe 窗口，失败发生在终端引导、尚未开始剪贴板。测试补齐私有 profile 目录，并强制配置路径位于该目录；引导仍保留原 20 秒与全部断言，失败只增加四项状态诊断。原始 InitShell 直接进入父 stdout 的现象与 [Microsoft ConPTY 维护者说明](https://github.com/microsoft/terminal/discussions/15814)一致：恢复 `STARTF_USESTDHANDLES` 并保持三项标准句柄为 NULL，避免 shell 继承父进程重定向输出；该 Windows 产品修正仍须真实 GUI 复验，不能由 Mac check 代替。

下一轮集中 CI 单独启用 GUI 剪贴板和原子升级，关闭完整工作区集合；新增 `run_gui_clipboard` 仅用于独立复验真实窗口，既有 `full_workspace_tests` 行为保留。当前修正无需本地化变更；原失败、跳过和不同提交来源继续分别保留。

本轮冻结后的 [本地门禁](validation/macos-fixed-versions-20260924/ci-followup-frozen-local-gates.safe.json)为 35 个脚本 811 项通过、7 项平台跳过，cargo check、i18n 11 项、构建摘要边界 1 项及静态检查通过。[Windows 安全诊断回归](validation/macos-fixed-versions-20260924/ci-followup-windows-atomic-diagnostics.safe.json)单列阶段、系统数值、原生退出与 Job 清理，原生 Rust 分支仍待 CI。外置盘一度权限失败的原日志保留，后续检查已恢复通过。

35992533669 的 Linux 在共享 `NativeRecorder.readers` 改为字典后，持久缓存探针仍按列表遍历，发生 `TypeError`／`AttributeError`；这是本次验收脚本回归，不是原生插件或产品能力失败。修复全部三处迭代并用真实父类和合成子进程验证双流 EOF；回归 25 项通过、5 项 Windows 条件跳过。本轮 Windows 尚在队列时取消，以修复后的提交替代，避免消费一次已知失败的运行；前一轮 Windows 全量继续。Linux 工作流也改为按各项真实前置执行独立检查，保留每项失败但不再让单一探针阻断编译、升级和 GUI。
