# CLI Agent Parity 阶段检查点交接（2026-09-21）

## 1. 阶段结论

- 总 Goal 仍为 **active / 未完成**。本轮仅冻结当前实现、真实验证结果和失败收据，不得据此恢复此前“全部完成”的结论。
- 工作分支：`codex/cli-agent-parity`。
- 本轮代码与证据检查点：`c90d19ebd`（`完善 CLI 代理对齐与原子升级边界`），已推送到 `origin/codex/cli-agent-parity`。
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
- 已补充当前认证形状、设置/目录/MCP 通知和固定平台夹具；候选能力仍只在测试门内，生产能力没有被错误放开。
- 多次全新官方进程观察到 setup 通知顺序跨运行变化。最终严格运行在 `session/new` 响应前收到 `_x.ai/models/update`，以 `unexpected Grok models update position` 失败，模型输入数为 0。
- 这证明当前按固定全序列绑定握手存在竞态。运行中追加、固定权限、允许/拒绝、技能、本地工具、子任务、双向原生 ACK、进度、结果回收和持久恢复仍未验收。
- 当前安全收据：`validation/macos-working-tree-103-grok-1040-root-lifecycle-current.safe.json`。

### 2.3 Windows 原子升级与真实 CLI

- 产品仍保持 `ManualOnly`，在产生副作用前 fail closed；候选 PE/调试器代码保留，但未接入生产自动替换路径。
- Windows 上三款当前 CLI 均完成真实二进制身份、签名和 `--version` 验证：Codex `0.155.1`、Claude Code `2.1.278`、Grok Build `1.0.40`。
- PE 结构与程序/祖先租约检查通过，但持有 cwd 句柄时叶子目录仍可改名；调试器进程树超过 30 秒不退出；三次 updater 参数运行均超时，且终止调试器后 Claude/Grok 根进程曾存活，已显式清理。
- Codex 官方 Windows 资产已漂移：当前下载与 GitHub API digest 一致，但仓库冻结的旧 size/SHA 不匹配；在重新冻结前不得宣称安全自动升级通过。
- 当前安全收据：`validation/windows-working-tree-104-atomic-real-cli-failclosed.safe.json`。

## 3. 当前提交已通过的定向门禁

- Python：Claude 适配器 `20/20`、Claude 协调器 `64/64`、Grok 官方运行器 `23/23`、自动升级验证器 `28/28`。
- Rust：Claude profile `18/18`、Claude adapter profile `14/14`、Grok `137/137`、managed process `46/46`、i18n `11/11`。
- `cargo check -p warp --features local_cli_managed_tasks` 通过。
- `cargo fmt --all -- --check`、`git diff --check` 通过。
- 本轮最终修改没有新增用户可见文案；已有相关本地化修改已通过 i18n 单测。
- **未执行**同一冻结提交的最终 macOS/Linux/Windows 云端矩阵；也未完成 SSH/tmux 产品接收、真实 GUI IME/双语布局和全范围 P0–P5 生命周期，因此不能标记完成。

## 4. 本地磁盘与外置磁盘

- 已将约 10 GiB 可迁移的任务缓存、旧 worker、GUI/IPC/自动升级临时目录、Claude 隔离配置和 Grok 旧临时目录迁往：`/Volumes/ORICO/InfiniShell-Desktop-local-offload-20260921`。
- 当前外置临时目录：`/Volumes/ORICO/InfiniShell-Desktop-tmp`；Cargo target：`/Volumes/ORICO/CargoTarget/InfiniShell-Desktop`。
- 内置磁盘可用空间从约 1.8 GiB 恢复到约 14 GiB；没有移动认证材料，也没有广泛清理用户数据。
- 详细记录见 `WORKSPACE_CLEANUP_20260921.md`。

## 5. 新会话的严格入口

1. `git fetch origin`，确认当前分支为 `codex/cli-agent-parity`、工作树干净，且本交接提交与远端 SHA 一致。
2. 依次阅读 `AGENTS.md`、`HANDOFF_20260921_REOPEN.md`、本文、`PLAN.md`、`CAPABILITY_MATRIX.md`、`VALIDATION_REPORT.md` 和收据 102–104。
3. 先做定向闭环，不要因接手而立即重复全量构建。
4. Grok：把 setup 绑定重构为基于相关原生事件的偏序/并发状态机，不再假定唯一通知顺序；先完成根生命周期，再开放固定审批、技能、本地工具和父子能力。
5. Claude：先解决 macOS launchd + 外置 debug supervisor 的原生 I/O 启动问题，或生成更小的 release supervisor；随后用手动权限配置完成真实父子全链。不得退回计划模式，也不得放宽工具权限来换取通过。
6. Windows：补齐 cwd 身份绑定、调试进程树退出和残留清理收据，重新冻结 Codex 官方资产；在这些条件完成前继续保持 `ManualOnly`。
7. 继续剩余原范围：Codex 当前版完整生命周期与取消、Linux 实际原子替换、SSH/tmux 产品接收、GUI IME 与双语布局、异常矩阵，以及最终冻结提交的跨平台验证。
8. 只有“要求→实现→CLI 版本→源码提交→模式/平台→收据→结果”矩阵中所有必需项在同一当前提交上通过，才可把 Goal 标记为 complete。

## 6. 安全边界

- Claude 官方在线账户已获授权；兼容 API 凭据仅存于系统钥匙串，不得写入仓库、收据、日志或聊天。
- 用户曾在聊天中提供兼容 API 密钥；项目最终验收后应轮换该密钥。
- 历史失败必须保留；回放、模拟、编译、skipped、租约注册或单次 inspect 均不能替代真实交互通过。
