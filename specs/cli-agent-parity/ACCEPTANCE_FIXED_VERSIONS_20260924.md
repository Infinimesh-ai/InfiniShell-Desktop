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
| P5 平台、提交和报告；R8 | `.github/workflows/cross-platform-preflight.yml`、本表和验证收据 | 最终源码提交与 Linux／Windows 集中验证尚未结算 | 待最终 SHA、平台结果和推送闭环 |

验证范围明确区分：真实 GUI 运行、GUI integration 编译、系统剪贴板、真实 IME、模拟协议、原生 CLI 执行和模型结果。跳过项不计通过，已记录失败不删除或追改。Grok 托管图片不支持，固定策略禁用原生 shell／hook／技能；这些限制应由界面与能力矩阵明确表达。
