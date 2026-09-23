# CLI Agent Parity 阶段检查点交接（更新至 2026-09-23）

## 1. 阶段结论

- 总 Goal 仍为 **active / 未完成**。本轮仅冻结当前实现、真实验证结果和失败收据，不得据此恢复此前“全部完成”的结论。
- 工作分支：`codex/cli-agent-parity`。
- 当前产品代码检查点：`c25221a22b03c28ca8c8538bee9229ca1bb0ec87`（`修复 Windows Codex 安装器固定版本`），已推送到 `origin/codex/cli-agent-parity`；receipt108 的 Linux／Windows 聚焦预检绑定该精确提交。receipt107 的 Claude 父子链仍绑定 `45ba0d11…`，receipt106 的 Grok root 候选链仍绑定 `ac0fa70e…`，没有冒充在当前提交重跑真实模型链。
- 基线归档仍为 `e6e9d619318e87d0bb889df344c8570f97977e90`；此前受测代码提交为 `38a611773b8ee53860f9ab731b2476b9a40d1819`。
- 后续必须继续使用同一工作树和当前分支，不得从 `main` 重来，不得 `reset/clean` 根目录，也不得直接弹出整理前的完整 stash。

## 2. 本轮冻结的当前事实

### 2.1 Claude Code 官方在线账户

- 默认在线账户认证已验证为可用；安全收据只记录登录状态、账户来源和套餐类型，不记录身份、邮箱、组织或令牌。
- 适配器与协调器增加了显式 `--use-authorized-default-account` 路径：该模式不注入 `CLAUDE_CONFIG_DIR`，并与隔离配置模式互斥。
- 当前固定权限配置已收紧到 `restricted + manual + host permission prompts + Read/Edit`，拒绝用户、项目、本地设置源，并对管理策略异常 fail closed。
- receipt102 的计划模式拒绝与外置 debug supervisor `EAGAIN` 启动失败继续保留，未被后续成功改写。当前验收明确使用系统 `/private/tmp`、签名 release supervisor 和隔离 `WARP_DATA_PROFILE`，不再把外置临时目录的 launchd／Unix socket 问题混入产品结论。
- 干净提交 `45ba0d11…` 的真实生产 runtime-host 父子链已通过：官方 `claude-sonnet-4-6` 完成子任务创建、5 次逐次审批、4 次原生本地工具、5 个输入／3 个执行、2 次原生输入合并、父子双向原生 ACK、自动子结果投递与最终 inspect。三条只在隔离验收标记启用时产生的原生结果关联分别覆盖 2／2／1 个输入；普通会话事件序列不变。
- 父子 runtime-host 均产生 v2 密封退出账本，`last/ack` 分别为 `23/22`、`36/35`，原生进程退出、adapter 成功／终止和 event journal 完成都已核对；runner 退出 0，私有与公开双重审计及敏感值扫描均通过。
- 本收据仍不证明真实 GUI 父子操作、IME／双语布局、SSH／tmux、Linux／Windows、完整异常生命周期或全量 P0–P5。
- 当前安全收据：[receipt107](validation/macos-working-tree-107-claude-21278-parent-child-clean-commit.safe.json)；历史失败继续见 [receipt102](validation/macos-working-tree-102-claude-authorized-parent-child-current.safe.json)。

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

### 2.4 同提交 Linux／Windows 聚焦预检

- [run 35746148046](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35746148046) 在精确提交 `c25221a22…` 上结算成功：Linux x64 为 34 个成功步骤、2 个按 `full_workspace_tests=false` 跳过；Windows x64 为 45 个成功步骤、2 个同样跳过。两个 job 的 check、IPC frame-limit 恢复、生命周期／取消／Responses、双语 TUI、进程所有权、supervisor、宿主崩溃清理、无凭据原生恢复和 rust-genai 均通过。
- Windows 同一 job 同时验证 Codex `0.155.1` 当前运行时与专用于生产通知安装器合同的官方 `0.147.0` 包；生产 Rust 安装器 metadata 为 `accepted=true`、退出 0、无超时、无凭据、0 个模型命令、migration 通过、EOF 与私有根清理确认。它没有验证原生 hook 实际执行、完整受监督进程树清理或 App 重启／UI，不能回填为三款自动原子升级通过。
- 6 个 artifact 共 37 个文件已下载到仓库外临时目录并逐份核对；JSON／NDJSON 解析失败为 0，邮箱与常见凭据形态扫描均为 0，只把大小、SHA／manifest 摘要和允许字段写入 [receipt108](validation/cross-platform-preflight-108-c25221a22.safe.json)。
- 该 run 是聚焦跨平台边界证据，不含同提交 macOS、GUI integration 或 full workspace；因此总 Goal 继续 active。
- 复核该 Linux job 原始日志后，[receipt109](validation/linux-atomic-execveat-109-c25221a22.safe.json) 确认密封 memfd 的真实 `execveat` 参数／环境／cwd 夹具及失败后禁止 pathname 回退均在 Linux x64 实机通过；此前“Linux `execveat` 完全未运行”的阶段表述已过时。三款 CLI 的 Linux 产品升级事务、退出回执与恢复仍未运行。
- 2026-09-23 的 [receipt110](validation/macos-working-tree-110-grok-1040-selected-skill-gate-failed.safe.json) 保留 Grok `1.0.40` 默认入口单技能候选的负证据：用户重新登录后原生会话创建成功，但缺少输入前的技能目录更新，30 秒后按请求超时失败；0 次产品输入，正式技能门禁未开放。私有认证副本已移除；仅测试夹具接受固定 marketplace 初始化或单字段 purge。

## 3. 当前提交已通过的定向门禁

- `c25221a22…` 本地隔离 target 的 `cargo check -p warp --features local_cli_managed_tasks` 通过；Codex source runner Python `16/16`、准备器 Python `48/48`、YAML 解析、actionlint（仅允许仓库自定义 runner label）和 diff check 通过。
- receipt107 的 Claude 原生结果证据 `2/2`、Claude Rust `117/117`、协调器 Rust `63/63`、Claude 外层审计 Python `66/66`、i18n `11/11`、feature supervisor build、无模型探针 `2/2` 与真实父子链仍绑定 `45ba0d11…`。
- `cargo fmt --all -- --check`、`git diff --check` 通过。
- 本轮最终修改没有新增或变动用户可见文案，无需本地化资源变更。
- receipt106 的 Grok `139/139` 与 Python `37/37` 仍按 `ac0fa70e…` 快照保留；receipt102／104–105 的历史 Claude、Windows 和升级阶段门禁也只属于各自快照。
- `c25221a22…` 的 Linux／Windows 聚焦矩阵已通过，但同提交 macOS、full workspace、SSH/tmux 完整产品接收、真实 GUI IME／双语布局和全范围 P0–P5 生命周期仍未完成，因此不能标记完成。

## 4. 本地磁盘与外置磁盘

- 已将约 10 GiB 可迁移的任务缓存、旧 worker、GUI/IPC/自动升级临时目录、Claude 隔离配置和 Grok 旧临时目录迁往：`/Volumes/ORICO/InfiniShell-Desktop-local-offload-20260921`。
- 当前 `.envrc` 外置临时目录：`/Volumes/ACASIS/InfiniShell-Desktop-tmp`；默认 Cargo target：`/Volumes/ACASIS/CargoTarget/InfiniShell-Desktop`。本轮定向构建使用 `/Volumes/ACASIS/CargoTarget/InfiniShell-Desktop-cli-agent-parity-e846c137d`。
- launchd／Unix socket 真实探针显式覆盖 `TMPDIR`／`TEMP`／`TMP=/private/tmp`；默认外置临时目录会导致同一二进制启动失败，不能用该环境失败否定产品链。
- 当前内置磁盘约 105 GiB 可用、ACASIS 约 1.6 TiB 可用；没有移动认证材料，也没有广泛清理用户数据。
- 详细记录见 `WORKSPACE_CLEANUP_20260921.md`。

## 5. 新会话的严格入口

1. `git fetch origin`，确认当前分支为 `codex/cli-agent-parity`、工作树干净，且本交接提交与远端 SHA 一致。
2. 依次阅读 `AGENTS.md`、`HANDOFF_20260921_REOPEN.md`、本文、`PLAN.md`、`CAPABILITY_MATRIX.md`、`VALIDATION_REPORT.md` 和收据 102–108。
3. 先做定向闭环，不要因接手而立即重复全量构建。
4. Grok：receipt106 已完成 `ac0fa70e…` 的当前版干净提交 root 候选链；receipt110 的单技能候选在新登录后仍缺输入前目录更新而失败，0 次模型输入。不要绕过唯一原生路径校验或盲目重复模型探针；先定位当前版技能发现与 setup 时序，再补技能、本地工具、子任务／父权限上限、App 重启／GUI和网络隔离的独立真实收据，最后决定是否开放对应能力。
5. Claude：receipt107 已完成 `45ba0d11…` 的 release supervisor 真实父子全链，不要重复消费相同模型链；下一步补真实 GUI 父子操作／重启、SSH／tmux、完整异常生命周期和同提交跨平台证据。运行 launchd 夹具时继续显式使用系统 `/private/tmp`，不得退回计划模式或放宽工具权限换取通过。
6. Windows：补齐 cwd 身份绑定、调试进程树退出和完整 Job 残留清理收据；沿版本选择后的 `0.155.1` 清单复核官方资产，不再使用 legacy `0.147.0` 条目比较。在这些条件完成前继续保持 `ManualOnly`。
7. receipt108／109 已完成 `c25221a22…` 的 Linux／Windows 聚焦预检及 Linux `execveat` 机制实测，不要无代码变化地重复派发相同矩阵。继续剩余原范围：Codex 当前版完整产品生命周期与取消、Linux 三款产品原子升级事务、SSH/tmux 产品接收、GUI IME 与双语布局、异常矩阵，以及功能冻结后的同提交 macOS 与 full workspace 验证。
8. 只有“要求→实现→CLI 版本→源码提交→模式/平台→收据→结果”矩阵中所有必需项在同一当前提交上通过，才可把 Goal 标记为 complete。

## 6. 安全边界

- Claude 官方在线账户已获授权；兼容 API 凭据仅存于系统钥匙串，不得写入仓库、收据、日志或聊天。
- 用户曾在聊天中提供兼容 API 密钥；项目最终验收后应轮换该密钥。
- 历史失败必须保留；回放、模拟、编译、skipped、租约注册或单次 inspect 均不能替代真实交互通过。
