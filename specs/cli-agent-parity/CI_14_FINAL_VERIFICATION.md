# CI14 最终验证记录

[Actions 35363371460](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35363371460) 已结束，整体 **FAILED**。被测提交为 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`；后续 source36 与 100 秒探针候选均未包含在本次验证中。

| 平台 | 最终结果 | 步骤统计（含设置及收尾） |
| --- | --- | --- |
| Linux x64 | success | 32 成功，2 跳过 |
| Windows x64 | failure | 36 成功，1 失败，3 跳过 |

两平台 `cargo check -p warp --lib`、定向 Rust/i18n、CLI 参数、双语 TUI、进程所有权、监督进程及无凭据恢复边界步骤成功。Windows 原生 hooks、ConPTY 和 SSH worker 参数步骤亦成功。逐步骤名称、编号、时间及结论完整保留在 [逐步骤安全报告](validation/ci14-af1040/ci14-final-verification.safe.json)。

`full_workspace_tests=false`：依据固定 workflow 条件与最终步骤状态确认，并非仅引用默认值。两平台 **GUI integration 编译、全量桌面 workspace 测试均跳过**；没有真实 GUI 操作、macOS 或 SSH/tmux 端到端验收。i18n 在定向 nextest 筛选内，不宣称 CI 另跑独立 cargo test 命令。

Windows 唯一失败为 step 16 `Verify native Codex persistent plugin source`。固定 Codex 0.147.0 的 `cache_only_restart` 未在探针 **20 秒预算**内同时观察到完整缓存还原及固定 `last_revision`；原报告未记录精确等待耗时，不能把预算冒充测量值。同 ID 迁移及后续阶段未到达。主进程自然退出、退出码 0，读取线程 EOF，私有 Job 的 2 个残余后代被回收，最终活跃数 0、清理确认成功；失败现场保留。清理成功不能覆盖原生验证失败，现有证据不足以确认底层迟延原因。

Linux 对应探针通过。其他无凭据探测只证明报告声明的边界，不能计作三款 CLI 真实模型审批、取消、继续、应用重启恢复及结果回收的完整验收。

交付仅含安全字段投影与摘要。原始 test-output、NDJSON、PTY、事件正文、配置/环境、命令输出及 runner 本地路径均未复制。[原产物摘要](validation/ci14-af1040/ci14-source-artifacts.sha256.json) 逐项记录两个 native-boundaries 产物文件的大小和 SHA256；[校验清单](validation/ci14-af1040/SHA256SUMS) 校验本目录交付文件。原 CI14 FAILED 保留，未发起新 CI、原生调用或 runner SSH。
