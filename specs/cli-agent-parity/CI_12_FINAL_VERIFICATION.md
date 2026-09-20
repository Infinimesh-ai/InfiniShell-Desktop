# CI12：固定提交最终失败与真实日志诊断

[CI12（35255292452）](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35255292452) 已由 `2026-09-17 18:27:30 UTC` 的 GitHub API 确认 **completed / failure**，实际提交 `436cc739234061c092cedd106432bbbfa3c2d645`，分支 `codex/cli-agent-parity`；Linux/Windows 两作业均 completed / failure。run 的 API updated_at 为 `2026-09-17T18:23:09Z`。本报告 `final=true` 仅表示取得 API 终态，**CI12 未通过，Goal 未完成**。

根代理实际 dispatch 参数启用 Linux/Windows、`full_workspace_tests=false`（`full=false`）；本次 API 不返回 inputs，没有补造。此前 [initial 专报](CI_12_INITIAL_VERIFICATION.md) 与四个 initial 文件原字节不变。

## 真实 Python 主失败

| 平台 | 真实失败与 CI 源码位置 | 实际统计与诊断边界 |
|---|---|---|
| [Linux x64，105317311967](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35255292452/job/105317311967)，步骤 7 | `GrokCoordinatorRunnerTests.test_failed_offline_process_still_removes_auth_and_keeps_safe_sources`；`grok_coordinator_runner_tests.py:454` → `run_grok_coordinator_live.py:476`，FileNotFoundError / Errno2，隔离根位于不存在的 `/private/tmp` 父目录 | `Ran 27 tests in 0.191s`，`FAILED (errors=1)`。根代理已核源码：生产 macOS 原生 probe 固定 `tempfile.mkdtemp(dir='/private/tmp')`，纯 fixture 没有替换。最小修复在 fixture 自己的 TemporaryDirectory 内替换 mkdtemp，不改生产 macOS runner 配置。 |
| [Windows x64，105317312218](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35255292452/job/105317312218)，步骤 8 | 同一 offline 测试 `:454` → runner `:476`，FileNotFoundError / WinError3；`/private/tmp` 父目录不存在 | 同一 Grok coordinator 文件 `Ran 27 tests in 0.160s`，`FAILED (failures=1, errors=2)`；本行与后两行共同构成该文件三项失败，没有回填本地结果。 |
| Windows，步骤 8 | `test_actual_sqlite_projection_uses_safe_json_and_body_hashes`，`grok_coordinator_runner_tests.py:370`，PermissionError / WinError32：TemporaryDirectory 清理 `coordinator.sqlite` 时文件仍被占用 | 日志证明文件句柄未释放，未独立定位是 fixture writer、只读 projection reader 或两者。应核两处连接关闭；SQLite 的事务上下文并不代替显式 close。 |
| Windows，步骤 8 | `test_project_file_projection_preserves_only_fixed_names_and_hashes_canary`，`grok_coordinator_runner_tests.py:354`；`unexpected_project_file_name_sha256` 预期/实际列表不同 | 主机路径分隔符差异是待核推断，日志本身未证明具体 hash 输入；先核生产规范，再将 fixture expected 按真实定义构造，不能改 hash 或放宽 canary 审核。 |

这些不是 CI11 的同一失败记录。Linux 超时测试所在文件实际执行到 `Ran 11 tests`，两平台 Claude coordinator 文件实际执行到 `Ran 56 tests`；失败发生在更后的 Grok coordinator 文件。没有将文件级统计扩大为逐测试日志未提取的独立证明。

Linux 步骤 27、Windows 步骤 33 真实失败日志均报告 **No files were found**，固定原生准备及对应边界已经 SKIPPED，上传所需原生产物没有生成。这是证据缺失的继发失败，必须保持失败门禁；Windows hooks/ConPTY 各自上传成功不替代步骤 33。

## 最终步骤范围

| 能力 | Linux x64 | Windows x64 |
|---|---|---|
| Grok 通知插件脚本 | 步骤 6 PASS | 步骤 7 PASS |
| Python 事务/运行器组 | 步骤 7 FAILED，见上述 Grok 27 项 | 步骤 8 FAILED，见上述 Grok 27 项 |
| 固定三 CLI 准备及 ACP/Claude初始化/权限/Codex插件边界 | 8–15 SKIPPED | 9–16 SKIPPED，没有真正执行 |
| Bash 与通知传输 | 16–17 SKIPPED，Unix PTY未验 | 17 Git Bash、18–19 无模型请求原生hooks/上传、20–21 ConPTY/上传 PASS，限通知边界 |
| 应用check、受影响Rust | 18–19 SKIPPED | 22 check、25 生命周期/终端取消/Responses 步骤组 PASS，25实际完成 `18:13:45Z`；通过步骤日志与统计未获取，不单独宣称某个CI11测试或原生SDK验收 |
| 共享CLI/skills、harness、双语TUI | 20–22 SKIPPED | 26–28 PASS；真实GUI双语布局未验 |
| SSH/tmux | 未验 | 23 SSH worker构建、24 PowerShell参数 PASS；真实连接/远端任务/tmux未验 |
| 进程监督与host crash | 23–25 SKIPPED | 29所有权、30监督器构建、31 host crash cleanup步骤 PASS；未读取逐测试日志或cleanup receipt，不扩大为三CLI全链证明 |
| 原生恢复/idle-crash | 26 SKIPPED | 32 SKIPPED |
| 证据上传 | 27 FAILED | 33 FAILED；hooks/ConPTY独立上传成功不覆盖此失败 |
| rust-genai | 28 SKIPPED | 34 PASS |
| GUI集成编译、完整workspace | 29–30 SKIPPED | 35–36 SKIPPED |

## 日志获取与安全归档

接续状态查询为 `18:11:16`、`18:16:30`、`18:22:16`、`18:27:30 UTC`，相邻间隔均不少于300秒。第二个十分钟窗口由根代理另行授权；最后查询已取得终态，随后诊断没有追加状态查询。

终态后两平台 joblog API 各一次均 exit1、stdout0，stderr各99B、SHA `8b6407b2fdbca54bf3d53fb93e79a3560dbe59567fa5044cfee4bed1b5b6e467`。前次 stderr 当时只保留长度/SHA，其原文内存已释放，无法事后真实分类；没有重新请求API冒充前次分类。两个 API final-job.log 新排他 `0600` 文件均0B，不是实际日志；initial阶段旧0B文件也未覆盖。

根代理随后明确授权各一次终态后 `gh run view --repo Infinimesh-ai/InfiniShell-Desktop --job <固定ID> --log-failed`。两次后备均成功，真实来源是 **gh失败步骤日志后备**，不是 joblog API。新排他私有文件保持0600，完整正文未工具回显或提交：

- Linux `/private/tmp/infinishell-cli-parity-ci12-linux-terminal-fallback-job.log`：85277B，SHA `b88e02c127d289cdde612d0e812682c70c416f401fe1762e3c05cff4196eeee2`。
- Windows `/private/tmp/infinishell-cli-parity-ci12-windows-terminal-fallback-job.log`：100909B，SHA `3e115c7fa990aab471d8bff7157cecfbb908c1644cda11160b2f8abe6a8a1850`。

[Linux安全摘录](validation/twelfth-436cc7/linux-terminal-safe-excerpt.log) 与 [Windows安全摘录](validation/twelfth-436cc7/windows-terminal-safe-excerpt.log) 仅为定向 derived 诊断，保留日志行号与公开测试/仓库文件名、断言、错误类型、真实统计及缺产物原因。Windows临时用户名路径在存档前剔除，SQLite错误只保留文件名与WinError32；URL查询、认证/bearer和六类凭据形状均不进入归档，不声明为原日志字节完整复制。

[final-run.json](validation/twelfth-436cc7/final-run.json)、[final-jobs.json](validation/twelfth-436cc7/final-jobs.json)、[final-audit.json](validation/twelfth-436cc7/final-audit.json) 保存API终态、实际诊断/获取方式、SHA、初始文件保留与扫描计数。后续修复前本次失败与计数固定；source27门禁或后续修复通过不得回填为CI12成功。

本归档未读源码、冻结目录、target、认证文件或模型/result/tool正文，未运行Cargo/原生CLI/模型、Git修改、CI派发/取消、CU/浏览器/登录；新增内容仅为验证文档与安全证据，**无需本地化变更**。六类固定形状零匹配不保证识别所有凭据格式。部分步骤PASS不满足三CLI完整在线、重启恢复、同提交三平台完整门禁和真实SSH/tmux验收，Goal不计为完成。
