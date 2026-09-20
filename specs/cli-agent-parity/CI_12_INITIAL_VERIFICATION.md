# CI12：十五分钟有界早期观察

本报告是 **initial / 非最终验收**。只读 GitHub API 核对 [CI12（35255292452）](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35255292452) 的实际提交为 `436cc739234061c092cedd106432bbbfa3c2d645`，分支 `codex/cli-agent-parity`。最后一次查询时 run 为 `in_progress`、conclusion 为 `null`；不能标为最终成功或最终失败。

根代理实际 dispatch 参数为 Linux/Windows 均启用、`full_workspace_tests=false`（`full=false`）；本次 run/jobs API 不返回 inputs，未补造该字段。

## 观察时间与交接

- 观察开始：`2026-09-17 17:52:37 UTC`；截止：`2026-09-17 18:07:37 UTC`。
- 实际 API 查询：`17:52:37`、`17:58:14`、`18:03:26 UTC`，间隔均至少五分钟。
- 最后一次 API 查询为 `2026-09-17 18:03:26 UTC`；下一允许查询窗口为 `18:08:26 UTC`，已晚于本任务截止，因此没有为结尾追加越频率查询。
- 停止时真实运行状态未重新查询。交接应从 [initial-run.json](validation/twelfth-436cc7/initial-run.json) 与 [initial-jobs.json](validation/twelfth-436cc7/initial-jobs.json) 的 `handoff_cursor` 继续，不能把四分钟前的快照描述为停止瞬间的状态。

## 最后查询时的实际步骤

| 平台 | Python 与原生准备 | 后续实际结果 |
|---|---|---|
| [Linux x64，job 105317311967](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35255292452/job/105317311967) | job 已 `completed / failure`；步骤 7 Python 事务/运行器组 FAILED；固定 Codex/Claude/Grok 准备步骤 8–10 均 SKIPPED，没有真正执行；原生边界 11–15 均 SKIPPED | 步骤 27 证据上传 FAILED；Bash/Unix PTY 16–17、check 18、受影响 Rust 19、共享 CLI/harness/TUI 20–22、进程监督/恢复 23–26、rust-genai 28、GUI/workspace 29–30 均 SKIPPED |
| [Windows x64，job 105317312218](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35255292452/job/105317312218) | job 为 `in_progress`；步骤 8 Python 组 FAILED；固定原生准备 9–11 与对应边界 12–16 均 SKIPPED，没有真正执行 | 步骤 17 Git Bash PASS；18–19 原生 hooks 无模型请求边界及其上传 PASS；20–21 ConPTY 通知传输及其上传 PASS；22 应用检查 PASS；23 SSH worker 构建、24 PowerShell 参数 PASS；25 受影响 Rust/生命周期/终端取消/Responses 测试 IN_PROGRESS；26–37 仍 PENDING |

上述 PASS 仅限实际步骤范围：hooks 与 ConPTY 没有证明完整在线审批/恢复任务链；SSH worker 构建与参数测试没有证明真实 SSH 连接、远端任务或 tmux。未运行步骤和 PENDING 步骤没有回填测试计数、PASS 或 SKIPPED。

## 失败原因及 CI11 修复的边界

两平台 Python 组失败已由 API 确认，但本任务没有取得具体失败测试名、文件行号、断言或 Ran/FAILED 统计，均保留 **`cause_unknown`**。不能猜测 CI11 的超时、路径或 JSON 断言再次失败，也不能认定其修复已经在 CI12 通过。Windows 步骤 25 尚无最终结果，Rust 隐私夹具修复的跨平台验收仍未知。

Linux 已完成失败 job 的直接日志 API 不可取；按根代理后续授权，在第二查询窗口仅执行一次有界 `gh run view --job --log` 后备。它也返回非零退出码，stdout 为 0 字节。新文件 `/private/tmp/infinishell-cli-parity-ci12-linux-failed-job.log` 以独占 `0600` 创建并保留为 0 字节，SHA 为 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`。该文件不是已取得的真实完整日志，不用它推断失败原因；没有覆写后再重复尝试。

Linux 步骤 27 的具体上传错误也保持未知。API 只证明该步骤 failure 与其前置原生步骤 skipped，没有本轮日志证明具体缺文件错误，不能直接套用 CI11 的继发失败原因。Windows job 未完成，本任务没有读取其未完成作业日志或浏览器日志。

根代理报告同 SHA 的干净 source26 本地 `cargo check`、i18n 11 项、受影响 Rust 1393 项、Python 410 项均 PASS。这些是另一本地验证范围，来源为根代理状态通知；本观察没有读取 target/冻结目录或复跑门禁，不能将这些计数填入 CI12 平台结果。

## 归档与限制

[initial-run.json](validation/twelfth-436cc7/initial-run.json) 保存三次 run 白名单快照、查询时间、日志访问失败摘要及 cursor；[initial-jobs.json](validation/twelfth-436cc7/initial-jobs.json) 保存三次 jobs/steps 白名单快照与有限范围评估；[initial-audit.json](validation/twelfth-436cc7/initial-audit.json) 保存文件 SHA、身份/间隔核对和六类凭据形状扫描计数。

stdout 完整日志正文未回显或提交，stderr 仅保存长度与 SHA。未取得可审查日志，因此没有创建虚构 safe-excerpt。没有读取认证、环境正文、API 密钥、模型/result/tool 正文、target 或冻结目录；没有 Cargo、原生 CLI、模型、Git 修改、CI 派发/取消、浏览器或登录操作。新增内容只有验证专报和安全 JSON，无产品文案变化，**无需本地化变更**。

CI12 的最终结论、两平台具体 Python 原因、Rust 最终结果、原生准备/边界验收、完整 workspace、真实 SSH/tmux 以及三款 CLI 完整在线链路仍未满足。Goal 不计为完成。
