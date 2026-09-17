# 第六次跨平台运行：7e0650855

运行已于 2026-09-16 17:42:25 UTC 结束：**Linux x64 passed，Windows x64 failed，整体跨平台门禁 failed。** 后续修复不改变本次失败。

- 精确提交：`7e06508554ae64cdd9321e0a69274e3d7b2d55ce`。
- Actions：[35126330599](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35126330599)，Linux/Windows 均选择，`full_workspace_tests=false`。
- Linux job `104896166333`，Windows job `104896165834`。
- 原始日志及下载产物位于 `/tmp/infinishell-sixth-platform-7e0650855`；[汇总 JSON](validation/sixth-platform-run-7e0650855.json)记录逐步骤状态、artifact 元数据、文件摘要和脱敏原生摘要。

## Windows：新的原生正向证据

固定 Python、三款 CLI 资产、离线脚本、真实 Windows LockFileEx 回归均通过。第五轮 Grok 对已锁 `leader.lock` 直接 read 的失败，本轮已由只读 mmap 取得真实 Windows 正向证据；没有改锁、ACL 或认证配置。

Windows job 用时 10 分 14 秒，22 个步骤成功、2 个失败、15 个跳过。Grok 离线 17 项于 0.402 秒通过，包含真实 Windows 独占锁回归，未将 skip 当作正向证据。

| 范围 | 本次原生结果 |
| --- | --- |
| Grok 1.0.30 无凭据 ACP | initialize 关联正确 ID 与版本；session/new 明确 `blocked_by_auth`；不存在会话的 cancel 只记录已发送通知，无 ACK；load/resume 均返回 `FS_NOT_FOUND` |
| Grok 退出边界 | stdio EOF 后 2142ms 自然退出 0；leader 等待 5004ms 仍活跃，随后通过自持句柄强制回收，退出 1；两者已结束，私有目录删除 |
| Claude 2.1.273 | initialize 与空闲 EOF 通过，41ms 自行退出 0，无强制回收；没有模型输入或 Rust adapter 验收 |
| Codex 0.147.0 原生注册表 | 8 项真实生命周期检查通过，包括缺失、原生安装/禁用、失败重装保持缓存与配置、显式恢复禁用；没有调用 GUI 产品安装器 |
| Git Bash payload | 13 项通过 |
| Codex 候选 Windows hook | PS5.1 语法、原生 argv、stdin/stdout 原始字节、实际 cmd 8191 字符执行与 8192 提前拒绝均通过；两个含中文/空格及 shell 元字符的路径 case 通过真实 SessionStart、UserPromptSubmit 和原生阻断，计数 provider HTTP 为 0 |

Grok 报告明确 `model_http_traffic_measured=false`：没有发送模型输入，但不把它写成系统网络流量测量通过。它仍不覆盖活动审批、活动取消或成功模型历史恢复。范围分别见 [Grok ACP 边界](GROK_FIXED_ACP_BOUNDARIES.md)、[Windows 候选 hook 记录](WINDOWS_VERIFICATION_FIXTURES.md)。

## Windows 首个失败：来源探针退出未完整确认

步骤 `Verify native Codex persistent plugin source` 已在第一次 `cache_only_restart` 观察到 `background_reverted_to_upstream=true`，随后收尾失败。首异常为 `NativeRecorder.close` 的 `ValueError: 原生输出读取失败或没有结束: []`：错误列表为空，但至少一个读取线程仍活跃。接着临时目录析构删除 `.tmp/plugins-clone-*/.git/objects/pack/tmp_pack_*` 时出现 `WinError 32`。JSON 只保存了这个清理异常，因此必须结合 job 原始日志定位首因。

本次没有进入同 ID 受控来源迁移，不能称迁移实现失败或成功。后台 Git 继承管道句柄是与代码/占用文件相符的解释，但旧探针没有采集相应 PID 与 Job 归属，不能从旧 artifact 反推完整进程树。

后续独立修正仅为该探针增加暂停创建后绑定私有 Windows Job、确认所有自有进程及 reader EOF、分别保存自然退出与强制回收、保留首异常及失败目录。新离线套件本机 11 通过、1 项 Windows 真实 Job 测试跳过，尚未在本次提交执行。详见 [持久来源探针收尾说明](CODEX_PLUGIN_CACHE_REFRESH.md#第六轮-windows-探针收尾失败与修正)。

这个前置失败使本次 Windows cargo check、定向 Rust、worker/监督、Rust 原生恢复等步骤跳过；跳过不是编译失败，也不能计通过。后续工作流解耦不能追溯改变本次状态。

## Windows 独立失败：真实 ConPTY 没收到对应通知

`Verify Codex Windows ConPTY notification transport` 独立执行并失败。HPCON 内 driver 自然退出 0，四个原生 Codex 控制进程的实际 console membership 均确认；临时原生 hook case 与零模型 HTTP 检查通过。真正 HPCON 输出为 802 字节，SHA-256 `cd07b30cb800f3d9ac66bff4cedc646b112b3767dfaf928a44e96a5372a91c5d`，含独立 `CONOUT$` canary。

但是第一个中文/空格路径 case 的 SessionStart 对应原生 session/turn OSC 匹配数为 **0**，不是所需的恰好 1。canary 证明诊断通道有输出，不能充当通知。原始 hook started/completed、HPCON 成员身份以及候选脚本的原生 marker，也不能替代终端中真实字节。

旧 marker 没有记录脚本内的 WARP gate/env 与 `/dev/tty` 打开结果，因此本轮不能直接判定 MSYS 控制终端为唯一根因。独立诊断后续补齐；本报告保留失败。Windows 生产自动安装、普通 Codex TUI 与产品通知 UI 均未因此开放或验收。探针契约见 [ConPTY 验证说明](WINDOWS_CONPTY_NOTIFICATION_PROBE.md)。

## Linux 同提交通过

Linux job 用时 33 分 41 秒，31 个步骤成功、2 个按输入配置跳过。

| 门禁 | 实际结果 |
| --- | --- |
| Python 文件事务/探针 10 套件 | 13 / 10 / 8 / 15 / 4 / 2 / 6 / 10 / 17 / 9，均 OK；其中 Windows 专属锁用例在 Linux 跳过 |
| Bash payload / Unix PTY | 13 / 4，均通过 |
| `cargo check -p warp --lib` | 通过，3 分 52 秒 |
| warp 定向 nextest | 1684 passed，19.435 秒执行时间；4788 未选中 |
| ai 共享契约/技能 | 71 passed |
| warp_cli | 128 passed |
| 双语 TUI task message | 9 passed |
| command managed | 1 passed |
| 同提交 supervisor 构建 / host-crash cleanup | 构建通过，4 passed |
| 原生 Codex missing-session / idle-crash | 每项恰好 1 passed，真实 Rust adapter |
| rust-genai | 81 passed |

GUI integration 编译与全工作区套件按 `full_workspace_tests=false` 跳过，没有计为通过。

原生报告核对为干净 `7e0650855`。Grok 1.0.30 的五个有限 ACP 边界通过；stdio EOF 后 2121ms 自然退出 0，leader 等待 5000ms 未退出后由自持句柄强制回收，退出 -9，两者结束、私有目录删除。Claude 2.1.273 initialize/空闲 EOF 在 64ms 自然退出 0。Codex 原生 registry 八项与持久来源十一项检查通过，包括同 ID 完整来源、禁用和其他 marketplace 保留；没有运行 Claude/Grok Rust 生产安装器 ignored live 入口。

真实 Codex Rust adapter 缺失会话被拒绝且未替换为新会话；空闲 CLI 已 Ready 后以 pidfd 绑定目标终止，出现一次 Disconnected，并取得同代 `linux_subtree` 清理回执。test binary SHA-256 为 `09b391ed59f196d36ebe5d28809e2adf6d3d9ca0481666eb962f32d070863f1c`，supervisor 为 `18cbbe67eceff480b0b0bb2513dff76709269a0632049ef3ccf46a318339aca5`。该证据仍不覆盖正在运行的真实模型工具树或应用重启 UI。

两平台四个 artifact 均已下载；JSON 保存每个原始文件和两个完整 job log 的字节数、摘要。原始日志中的 ANSI/可见转义仅去除为阅读副本，没有执行日志文本。原始账号目录、主机名及完整 hook 定义保留在原始产物，入库摘要不复制这些环境身份。

本轮没有重复 dispatch、SSH、本机 Cargo、真实用户配置或 GUI 操作。旧五轮结果保留；无凭据边界不代替完整认证生命周期、应用重启后恢复与最终同提交所有平台验收。
