# CI11：固定提交的跨平台失败归档

本报告记录 [CI11（35248824124）](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35248824124) 的最终结果。GitHub API 独立核对的提交为 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04`，分支 `codex/cli-agent-parity`，运行与两个作业均为 `completed / failure`。CI11 未通过，不能计为 P5 或 Goal 完成。

本轮调度为 `full=false`，来源是根代理的实际 dispatch 记录；本次 run/jobs API 不返回 inputs，归档未伪造该字段。GUI 集成编译与完整 workspace 测试在两个作业中实际均为 `skipped`。

## 最终失败与原始计数

| 平台与步骤 | 实际失败 | 原日志统计与范围 |
|---|---|---|
| [Linux x64](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35248824124/job/105295774175)，步骤 7 | `SourceRunnerTests.test_timeout_retains_partial_output_after_process_group_stop`，`codex_source_runner_tests.py:144` 的 `self.assertTrue(timed_out)` 得到 `AssertionError: False is not true` | 该 Python 文件 `Ran 10 tests in 0.007s`，`FAILED (failures=1)`。它是该步骤的唯一主失败，不表示前面 Python 文件也失败。 |
| Windows x64，步骤 8 | `IsolationTests.test_ambient_provider_keys_are_not_forwarded`，CI 源码 `claude_coordinator_runner_tests.py:537` 对 `/fresh/home` 的硬编码字符串断言失败，实际为 Windows 反斜杠路径 | `Ran 56 tests in 0.107s`，`FAILED (failures=1, skipped=1)`。属于夹具路径表示差异；本次失败记录没有证明产品转发了认证信息。 |
| [Windows x64](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35248824124/job/105295774516)，步骤 25 | `ai::cli_agent_runtime::grok::sdk_origin_live_tests::private_native_tool_and_approval_frames_stay_out_of_public_probe_reports`，CI 源码 `grok_sdk_origin_live_tests.rs:2109:9` 的 `private_text.contains(canary)` 失败 | 同一测试连续 3 次失败；该步骤最终 `1989 tests run: 1988 passed, 1 failed, 4701 skipped`，耗时 `372.247s`。私有 JSON 中 Windows 路径会转义，原文子串断言不成立；不得由此认定产品发生正文泄漏或真实 SDK 来源验收已通过。 |

Linux 步骤 27、Windows 步骤 33 的原生边界证据上传也失败，日志明确报告所需文件不存在。前置固定原生准备、初始化、权限、插件与恢复步骤已经跳过，因此这是证据缺失的继发失败。Windows 步骤 19、21 各自成功上传 hooks、ConPTY 证据，范围独立，不能覆盖步骤 33 的失败。

## 已执行、通过及跳过的范围

| 能力或门禁 | Linux x64 | Windows x64 |
|---|---|---|
| Grok 通知插件脚本 | 步骤 6 PASS | 步骤 7 PASS；只代表脚本测试 |
| Python 通知事务与运行器组 | 步骤 7 FAILED | 步骤 8 FAILED；后续固定原生准备未执行 |
| 固定 Codex/Claude/Grok 原生准备、无鉴权 ACP、Claude 初始化/权限、Codex 插件边界 | 步骤 8–15 SKIPPED | 步骤 9–16 SKIPPED |
| Bash 通知与终端传输 | 步骤 16–17 SKIPPED，Unix PTY 未验 | 步骤 17 Git Bash PASS（`Ran 13 tests`、`OK`）；步骤 18–19 原生 hooks 无模型请求边界与其上传 PASS；步骤 20–21 ConPTY 通知传输与其上传 PASS（该步骤 Python `Ran 16 tests`、`OK (skipped=1)`） |
| 应用 `cargo check` 对应步骤 | 步骤 18 SKIPPED | 步骤 22 PASS，仅针对本报告固定提交 |
| Agent 生命周期、终端取消、Responses 受影响测试 | 步骤 19 SKIPPED | 步骤 25 FAILED，保留上述原始计数 |
| 共享 CLI 动作/技能、harness 参数 | 步骤 20–21 SKIPPED | 步骤 26 PASS（71 passed / 224 skipped）；步骤 27 PASS（128 passed / 0 skipped） |
| 双语 TUI 任务消息状态 | 步骤 22 SKIPPED | 步骤 28 PASS（9 passed / 925 skipped）；没有真实 GUI 双语布局证据 |
| 受监管进程所有权、监督器构建 | 步骤 23–24 SKIPPED | 步骤 29 PASS（2 passed / 7 skipped）、步骤 30 构建 PASS |
| host crash 清理、原生继续/idle-crash 恢复 | 步骤 25–26 SKIPPED | 步骤 31–32 SKIPPED；构建成功不能替代运行验收 |
| SSH/tmux | 未验证 | 步骤 23 SSH worker 构建与步骤 24 PowerShell 参数测试 PASS；真实 SSH 连接、远端任务链与 tmux 均未验证 |
| 原生边界证据上传 | 步骤 27 FAILED | 步骤 33 FAILED，见继发失败说明 |
| rust-genai | 步骤 28 SKIPPED | 步骤 34 PASS（81 passed / 0 failed） |
| GUI 集成编译、完整 workspace | 步骤 29–30 SKIPPED | 步骤 35–36 SKIPPED |

上述 PASS 仅指实际步骤范围。CI11 没有证明三款 CLI 的「新建→两轮交互→审批允许/拒绝→追加→取消→继续→应用重启→恢复→结果回收」完整链路，也没有证明同一提交的三平台完整门禁、真实 SSH/tmux 或跨平台重启恢复。

## 修复状态与重新验证边界

下列修复状态由根代理提供；本归档任务没有重读或修改生产源码，没有运行 Cargo、CLI、模型或平台测试。

- Linux 超时夹具已改用 `/bin/sh sleep` 与进程组 EOF，新增立即退出码 7 不得误报 timeout 的回归；生产 `run_codex_source_live.py` 未修改。根代理报告本地 source24 的该 Python 文件 11 项 PASS。这是后续本地纯 Python 证据，CI11 的原始 10 项计数和失败结果保持不变，修复后 Linux 平台复验未执行。
- Windows Python 两项路径期望已改为 `str(Path(...))`；Rust 私有帧断言检查 JSON 编码后的内容，公开投影同时拒绝原文与 JSON 编码内容。根代理正在执行 source25 门禁；本报告未获得修复后平台复验结果，不能记录 PASS。
- 原生证据上传失败仍保留。需要在前置步骤实际执行并产生证据后重新验收，不能降低缺文件失败要求以代替验证。

## 归档与安全边界

| 文件 | 来源与保留方式 |
|---|---|
| [run.json](validation/eleventh-dec067/run.json) | 最终 run API 的白名单字段；固定提交核对、`full=false` 来源和未完成状态 |
| [jobs.json](validation/eleventh-dec067/jobs.json) | 最终 jobs/steps API 的白名单字段、精确步骤号与有限范围评估 |
| [windows-safe-excerpt.json](validation/eleventh-dec067/windows-safe-excerpt.json) | 先前已生成的安全失败摘录，26 条，整文件原字节复制；SHA `3509785901882f3d83545e07ba10e8a67a0b8152e2382e06ac1f77e977ff2043` |
| [linux-safe-excerpt.json](validation/eleventh-dec067/linux-safe-excerpt.json) | 本次 derived 摘录，6 条，只保存 timeout 测试名、公开测试路径、断言及 Ran/FAILED；`original_line_utf8` 保留逐行原 UTF-8 字节与换行，可按逐行 SHA 核对 |
| [windows-passed-step-excerpt.json](validation/eleventh-dec067/windows-passed-step-excerpt.json) | 本次 derived 摘录，9 条，只保存已通过步骤的实际统计；保留逐行原 UTF-8 字节，不补算未运行测试 |
| [audit.json](validation/eleventh-dec067/audit.json) | 新文件大小/SHA、原始来源前后核对、原字节复制和六类凭据形状扫描；扫描只记录计数 |

原始完整作业日志保留在本机 `/private/tmp/`，未复制到仓库：Linux 日志 `75252B`、SHA `922e5dbea3697d118624fb0230865b9003f8910fcd3ceb1a99bcf5b8a981e9a0`；Windows 日志 `940721B`、SHA `3574dafb3280f4920b08383e20a87feee4bcc3c34aa6fbabc34b74a87c402af3`。原文件字节与权限保持不变（Linux 原为 `0644`，Windows 原为 `0600`）。

本归档未读取认证文件、私有模型正文、target 或冻结目录；没有派发/取消 CI、登录、浏览器操作、Git 修改或源码修改。新增内容仅为验证文档与安全 JSON，无产品文案变化，**无需本地化变更**。六类固定形状零匹配不保证识别所有凭据格式，不能替代完整安全审计。
