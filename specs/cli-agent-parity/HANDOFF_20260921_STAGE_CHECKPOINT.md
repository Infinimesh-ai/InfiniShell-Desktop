# CLI Agent Parity 阶段检查点交接（2026-09-21）

## 1. 阶段结论

- 总 Goal 仍为 **active / 未完成**。本轮仅冻结当前实现、真实验证结果和失败收据，不得据此恢复此前“全部完成”的结论。
- 工作分支：`codex/cli-agent-parity`。
- 当前代码检查点：`ac0fa70eea4372675367dfa59ac0226396c613df`（`修复 Grok 恢复握手回放`），已推送到 `origin/codex/cli-agent-parity`；receipt106 的 supervisor、libtest 与真实 Grok root 候选链均绑定该干净提交。
- 基线归档仍为 `e6e9d619318e87d0bb889df344c8570f97977e90`；此前受测代码提交为 `38a611773b8ee53860f9ab731b2476b9a40d1819`。
- 后续必须继续使用同一工作树和当前分支，不得从 `main` 重来，不得 `reset/clean` 根目录，也不得直接弹出整理前的完整 stash。

## 2. 本轮冻结的当前事实

### 2.1 Claude Code 官方在线账户

- 默认在线账户认证已验证为可用；安全收据只记录登录状态、账户来源和套餐类型，不记录身份、邮箱、组织或令牌。
- 适配器与协调器增加了显式 `--use-authorized-default-account` 路径：该模式不注入 `CLAUDE_CONFIG_DIR`，并与隔离配置模式互斥。
- 当前固定权限配置已收紧到 `restricted + manual + host permission prompts + Read/Edit`，拒绝用户、项目、本地设置源，并对管理策略异常 fail closed。
- 计划模式实测已到达根会话、两轮交互、ACK、结果与清理，但计划约束拒绝了父子派发和双向消息，因此不能作为父子验收。
- 手动权限模式的当前提交实测在 macOS 外置磁盘 debug supervisor 经 launchd 接管原生 I/O 时触发 `EAGAIN`，在 `SessionReady`、模型输入和父子派发前退出；此项属于 **证据不足 / 启动阻塞**，不是父子功能通过或失败。
- 当前安全收据：`validation/macos-working-tree-102-claude-authorized-parent-child-current.safe.json`。

### 2.2 Grok Build 1.0.40

- 当前实测正式版为 `1.0.40 (eb1a2256660d)`；macOS arm64 二进制 SHA-256 为 `3f2aef9618191a2c60d18a5044fa462c9c77bdc4187b02ed716b0394e8d4fef2`。
- receipt103 的固定全序握手失败保留；后续实现已按相关原生事件改为有界偏序，接受动态且有界的官方模型目录，并只为“当前版本、正在恢复、session 精确匹配、`isReplay=true`、已审核方法”放行 `session/load` 响应前历史回放，其他错误 session、非 replay 与未知方法继续 fail closed。
- 干净提交 `ac0fa70e…` 的真实生产 supervisor／ACP root 候选链已通过：官方 `grok-4.7` 完成同一原生会话两轮、允许写入、拒绝无文件效果、运行中排队输入的原生 ACK 与下一轮结果、原生取消终态、完整历史核对，以及新进程恢复原会话和排队标记。两代 supervisor 均有 `cleanup_confirmed=true`，认证副本、内部 state、隧道与 staged 进程残留均已清理。
- 能力仍由 test-only candidate gate 隔离，`public_product_gate_open=false`；same-turn steering、技能、本地工具、子任务、父权限上限、App 重启／GUI、产品网络隔离和其他平台没有由本收据证明。
- 当前安全收据：[receipt106](validation/macos-working-tree-106-grok-1040-root-lifecycle-clean-commit.safe.json)；历史失败继续见 [receipt103](validation/macos-working-tree-103-grok-1040-root-lifecycle-current.safe.json)。

### 2.3 Windows 原子升级与真实 CLI

- 产品仍保持 `ManualOnly`，在产生副作用前 fail closed；候选 PE/调试器代码保留，但未接入生产自动替换路径。
- Windows 上三款当前 CLI 均完成真实二进制身份、签名和 `--version` 验证：Codex `0.155.1`、Claude Code `2.1.278`、Grok Build `1.0.40`。
- PE 结构与程序/祖先租约检查通过，但持有 cwd 句柄时叶子目录仍可改名；调试器进程树超过 30 秒不退出；三次 updater 参数运行均超时，且终止调试器后 Claude/Grok 根进程曾存活，已显式清理。
- [receipt105](validation/macos-working-tree-105-codex-01551-windows-asset-correction.safe.json) 已纠正资产结论：receipt104 错把 `0.155.1` 官方包与 legacy `0.147.0` 清单比较；版本选择后的仓内 `0.155.1` size/SHA 与 GitHub API digest 一致，无需修改产品摘要。cwd、debugger、退出收据和清理缺口不受此纠正影响。
- 当前安全收据：`validation/windows-working-tree-104-atomic-real-cli-failclosed.safe.json`。

## 3. 当前提交已通过的定向门禁

- 当前精确提交：Grok Rust `139/139`、Grok／supervisor 定向 Python `37/37`、i18n `11/11`。
- `cargo check -p warp --features local_cli_managed_tasks` 和 `release-tui-debug-assertions` feature supervisor build 通过；同提交无模型 supervisor 探针 `2/2` 通过。
- `cargo fmt --all -- --check`、`git diff --check` 通过。
- 本轮最终修改没有新增或变动用户可见文案，无需本地化资源变更。
- receipt102／104–105 的 Claude、Windows 和升级阶段门禁仍按各自快照保留，未冒充为 `ac0fa70e…` 上已重跑。
- **未执行**同一冻结提交的最终 macOS/Linux/Windows 云端矩阵；也未完成 SSH/tmux 产品接收、真实 GUI IME/双语布局和全范围 P0–P5 生命周期，因此不能标记完成。

## 4. 本地磁盘与外置磁盘

- 已将约 10 GiB 可迁移的任务缓存、旧 worker、GUI/IPC/自动升级临时目录、Claude 隔离配置和 Grok 旧临时目录迁往：`/Volumes/ORICO/InfiniShell-Desktop-local-offload-20260921`。
- 当前外置临时目录：`/Volumes/ORICO/InfiniShell-Desktop-tmp`；Cargo target：`/Volumes/ORICO/CargoTarget/InfiniShell-Desktop`。
- 内置磁盘可用空间从约 1.8 GiB 恢复到约 14 GiB；没有移动认证材料，也没有广泛清理用户数据。
- 详细记录见 `WORKSPACE_CLEANUP_20260921.md`。

## 5. 新会话的严格入口

1. `git fetch origin`，确认当前分支为 `codex/cli-agent-parity`、工作树干净，且本交接提交与远端 SHA 一致。
2. 依次阅读 `AGENTS.md`、`HANDOFF_20260921_REOPEN.md`、本文、`PLAN.md`、`CAPABILITY_MATRIX.md`、`VALIDATION_REPORT.md` 和收据 102–106。
3. 先做定向闭环，不要因接手而立即重复全量构建。
4. Grok：receipt106 已完成当前版干净提交 root 候选链，不要无代码变化地重复消费模型额度；下一步保持产品入口关闭，分别补技能、本地工具、子任务／父权限上限、App 重启／GUI和网络隔离的独立真实收据，再决定是否开放对应能力。
5. Claude：先解决 macOS launchd + 外置 debug supervisor 的原生 I/O 启动问题，或生成更小的 release supervisor；随后用手动权限配置完成真实父子全链。不得退回计划模式，也不得放宽工具权限来换取通过。
6. Windows：补齐 cwd 身份绑定、调试进程树退出和完整 Job 残留清理收据；沿版本选择后的 `0.155.1` 清单复核官方资产，不再使用 legacy `0.147.0` 条目比较。在这些条件完成前继续保持 `ManualOnly`。
7. 继续剩余原范围：Codex 当前版完整生命周期与取消、Linux 实际原子替换、SSH/tmux 产品接收、GUI IME 与双语布局、异常矩阵，以及最终冻结提交的跨平台验证。
8. 只有“要求→实现→CLI 版本→源码提交→模式/平台→收据→结果”矩阵中所有必需项在同一当前提交上通过，才可把 Goal 标记为 complete。

## 6. 安全边界

- Claude 官方在线账户已获授权；兼容 API 凭据仅存于系统钥匙串，不得写入仓库、收据、日志或聊天。
- 用户曾在聊天中提供兼容 API 密钥；项目最终验收后应轮换该密钥。
- 历史失败必须保留；回放、模拟、编译、skipped、租约注册或单次 inspect 均不能替代真实交互通过。
