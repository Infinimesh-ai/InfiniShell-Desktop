# CI13 第三观察窗口：选定检查点通过

同一运行 [35263203530](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35263203530) 已真实结束：Linux x64 和 Windows x64 均为 `completed/success`。受测提交严格为 `e6873e2cbb12a12ffce0b8283ee2ac96b940da39`，分支为 `codex/cli-agent-parity`。本报告只确认 `cross-platform-preflight.yml` 的选定检查点通过，`full_workspace_tests=false`，不作为整项 Goal 完成证明。

## 本窗口与历史边界

前两个窗口的九份档案逐 SHA 核对保持原字节，其中原 `pending` 结论保留。本窗口仅创建本文及 `third-window-run.json`、`third-window-jobs.json`、`third-window-audit.json`；不重新派发、取消或修改工作流，不以其他提交替代受测 HEAD。

- 第三窗口最早允许查询：2026-09-17 19:41:12.183109 UTC。
- 首轮实际查询：2026-09-17 19:42:30.456829 UTC；本窗口共一轮 run/jobs 查询。
- 两项 API 完成后开始十五分钟窗口：2026-09-17 19:42:32.866992 UTC；截止 19:57:32.866992 UTC。
- 首轮已观察终态，停止状态轮询，不开启第四窗口。
- Linux 结束时间：2026-09-17 19:38:33 UTC；Windows 结束时间：19:42:00 UTC。

run 原始 API 响应为 13540 字节，SHA256 `75237e7cf0da6e17d576b8e61d43220eeb68b750acfce8e2f0608c6ab8efc05d`；jobs 响应为 15533 字节，SHA256 `7cea8428c9084978eaf8d499cf1544585e245c34126f22d3144b74d6b9886af6`。公开 JSON 只保留白名单元数据，实际 API 请求时间分别为 19:42:30.457090 与 19:42:31.687897 UTC。调度参数依据初窗归档的实际命令；API 响应本身不包含 inputs。执行与归档为同一代理，`archive_auditor_independent_from_executor=false`。

## 两平台实际步骤范围

| 检查范围 | Linux 实际步骤 | Windows 实际步骤 | API 结论 |
|---|---|---|---|
| Grok 插件脚本及 Python 文件事务夹具 | 6–7 | 7–8 | success |
| 固定三款 CLI 安装准备 | 8–10 | 9–11 | success |
| 无鉴权原生协议、初始化、权限观察及 Codex 插件边界 | 11–15 | 12–16 | success |
| Bash 与终端通知传输 | 16–17 | 17–21 | success |
| cargo check | 18 | 22 | success |
| Windows SSH worker 构建与参数夹具 | 不适用 | 23–24 | success |
| 生命周期、共享 CLI、harness、双语 TUI、进程所有权 | 19–23 | 25–29 | success |
| 监督器构建、宿主崩溃清理及无鉴权恢复边界 | 24–26 | 30–32 | success |
| 原生边界证据上传 | 27 | 33 | success |
| rust-genai | 28 | 34 | success |
| GUI 集成测试编译 | 29 | 35 | skipped |
| 完整桌面工作区测试 | 30 | 36 | skipped |

表中状态来自真实 jobs/steps API，不将步骤组 success 展开为每个测试用例已通过。Windows 17–21 包含 Git Bash、Codex 原生 hooks 和 ConPTY 的检查与证据上传步骤；不据此推断在线模型运行。完整逐步骤状态与 API 步骤数量保存在 `third-window-jobs.json` 与审计文件。

## 日志获取与限制

终态后对两个固定 job 各实际执行一次 `gh api .../actions/jobs/<id>/logs`，超时预算 45 秒、文件预算 32 MiB；私有文件以排他创建、禁止跟随符号链接、当前 UID、单硬链接及 `0600` 校验。两次命令均 exit 1，日志 stdout 为 0 字节；stderr 为 99 字节且同 SHA256 `8b6407b2fdbca54bf3d53fb93e79a3560dbe59567fa5044cfee4bed1b5b6e467`，安全类别未知，未公开原文或猜测 HTTP 状态。0 字节原日志保留，不覆盖；私有文件不提交。两 job 均成功，失败 job 专用 `--log-failed` 后备不适用，未追加日志请求。

因此本档案没有真实平台逐测试数量、失败断言或原生业务正文，相关字段为 `null` 或空数组，不能用本地 Rust/Python 数量回填。API 可证明两平台相应检查步骤执行成功；无法据未获取的日志补充更细验收证据。

固定 CLI 安装及无鉴权初始化、清理、恢复边界不等于三方在线模型完整交互链。SSH worker 构建和 PowerShell 参数夹具不等于真实 SSH/tmux；双语 TUI 测试不等于真实 GUI 英文与简体中文布局检查。本次 full=false 检查点不替代最终 full=true 门禁。旧 CI11/CI12 的失败记录不回填；尚待验收能力不计完成，Goal 保持未完成。

本次仅新增验证归档，无用户可见功能或文案修改，**无需本地化变更**。
