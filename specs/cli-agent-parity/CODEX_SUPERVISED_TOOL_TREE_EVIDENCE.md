# Codex 真实工具与监督清理验证

2026-09-16，macOS arm64，Codex CLI 0.147.0。使用实际 InfiniShell bundle3 worker，SHA-256 为 `17a2dd3d147072681a52acc7cadc00c1a4880552ff81ede6e8c6c0d5832a41b2`，两次运行中二进制摘要未变化。宿主是独立 Python 进程，使用生产监督 worker 的控制握手、stdin/stdout 与退出回执；本 probe 不直接调用 Rust runtime adapter，不是 GUI 验收。

只读复制已有 Codex auth 到临时 CODEX_HOME，复制文件权限 0600，HOME 和项目均隔离，测试结束后删除。模型仅获准一次完整匹配的 `python heartbeat.py` 命令，approvalPolicy 为 untrusted、sandbox 为 workspace-write；其他命令拒绝。固定脚本写自己的 PID/PPID/PGID/SID、每 100ms 追加并 fsync 一次心跳，并运行 `/bin/sleep 60` 子进程。脚本本身没有 setsid/setpgid，也不主动脱离父进程。

## 宿主强制退出：通过本次路径

证据：`fixtures/codex-0.147-supervised-tool-tree-macos.json`。

宿主 PID 96612，worker 96613，真实 Codex PID/PGID 96614；工具 PID/PGID/SID 96714，sleep 96716 与工具同组。工具是 Codex 原生 exec_command 创建，确实在独立组/会话。观察到三次心跳后 SIGKILL 宿主；约 49ms 后收到关联 generation 与 manifest SHA 的有效回执，原因 stdio_closed，Codex exit code 0。CLI、工具与 sleep 均已不存在，回执时及 1.2 秒后心跳均为三行。

本结果说明该次宿主断线引发 Codex 正常清理；它不能证明监督者能够在 Codex 崩溃时捕获独立工具组。

## Codex 自身强制退出：失败并复现缺口

证据：`fixtures/codex-0.147-supervised-cli-crash-macos.json`，probe 正确返回非零。

| 对象 | PID | 崩溃前 PPID / PGID / SID | 收到回执后 |
| --- | --- | --- | --- |
| 宿主 | 96981 | 96979 / 96979 / 96979 | 仍运行 |
| 监督 worker | 96982 | 96981 / 96979 / 96979 | 随后退出 |
| 真实 Codex | 96983 | 96982 / 96983 / 96979 | 已被本测试 SIGKILL |
| 心跳工具 | 97003 | 96983 / 97003 / 97003 | PPID 变为 1，继续运行 |
| sleep | 97005 | 97003 / 97003 / 97003 | 继续运行 |

强杀前核对本次 worker 唯一 Codex 子进程的完整身份。约 30ms 后监督者写出 `cleanup_confirmed=true`、`containment=unix_process_group`、`exit_code=null` 的回执；generation 与 manifest SHA 均匹配。但心跳从回执时四行增加到 1.2 秒后的十六行。直到观测完成后，测试才写固定 `fixture-stop` 标记；夹具正常停止自身与 sleep，最终两个 PID 都不存在。未按全局进程名或未知 PGID 清理任何其他进程。

该证据不能被解释为任意外部恶意守护进程主动脱组：独立组是受测 Codex 自己为普通工具创建的。当前组级退出回执不足以作为此崩溃场景的安全恢复许可；修复与复测完成前，该项验收失败。

## bundle4 重验：阻止恢复已落实，完整清理仍失败

使用新 worker，SHA-256 为 `39b79941a19dabdfb8d08e8282b7c671521f1303991d667b0e5638fd9dfd363e`。真实重跑相同固定工具、一次匹配审批和 Codex 根 SIGKILL；没有修改原有两份证据。结果保存在 `fixtures/codex-0.147-supervised-cli-crash-macos-recovery-gated.json`。

Codex PID/PGID 99981，工具 PID/PGID/SID 114，sleep 116。约 27ms 后收到同 generation 与 manifest SHA 的回执，现为 `cleanup_confirmed=false`、`exit_code=null`。宿主仍活着；心跳从回执时四行增加到 1.2 秒后的十六行，说明清理没有完成。观察之后仅写本次专用停止标记，工具与 sleep 都已退出。

| 判断 | 实际结果 | 含义 |
| --- | --- | --- |
| `unsafe_recovery_prevented` | true | 关联回执明确拒绝恢复 |
| `cleanup_failed` | true | 独立工具组仍有残留，未获得完整清理证明 |
| `known_tool_processes_stopped` | false | 观测结束时原生工具与 sleep 尚未停止 |
| `passed` | false | 不将保守阻断计为完整验收通过 |

该 probe 的恢复验证范围是回执字段；它没有调用 GUI 恢复入口或 Rust `confirmed_exit`，后者的旧回执拒绝回归由主代理统一测试。二进制在本次运行前后摘要一致；此结果也不能替代最终同提交的跨平台门禁。

## 固定源码依据

取得 `rust-v0.147.0` 原文并记录 SHA-256 于 `fixtures/codex-0.147-process-source.json`。pipe spawn 的 pre_exec 调 `detach_from_tty()`，Unix 实现调用 setsid，失败为 EPERM 时退回 setpgid；PTY spawn 也直接 setsid。Linux 的 parent-death signal 有专用实现，其他平台无对应效果。[固定 pipe 实现](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/utils/pty/src/pipe.rs)、[进程组实现](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/utils/pty/src/process_group.rs)、[PTY 实现](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/utils/pty/src/pty.rs)。

统一工具进程对象与底层 ProcessHandle 在正常 Drop 中调用 terminate；SIGKILL 不运行 Rust Drop。这与两项实测的差异一致，但不作为替代实测的退出证明。[统一执行对象](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/core/src/unified_exec/process.rs)、[进程句柄](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/utils/pty/src/process.rs)。

## 复跑命令

```sh
python3 script/cli-agent-parity/probe_codex_supervisor_tool_tree.py \
  --codex /absolute/path/to/codex \
  --supervisor /absolute/path/to/InfiniShell.app/Contents/MacOS/infinishell \
  --credential-source /absolute/path/to/existing/auth.json \
  --crash-target host \
  --output /absolute/path/to/host-crash.json
```

将 `--crash-target` 改为 `codex` 单独复跑原生 CLI 崩溃。两项都会请求一次真实模型工具调用；只在已授权本机环境执行，不能用于 CI 默认测试。若模型、审批或进程关系不符合固定夹具则失败，不生成伪成功。此证据尚不覆盖 Linux、Windows、应用 UI 重启或最终同提交验证。
