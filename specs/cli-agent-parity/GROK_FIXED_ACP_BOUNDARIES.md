# Grok 1.0.30 无凭据 ACP 边界探针

新增 `script/cli-agent-parity/grok_fixed_acp_probe.py`，用于固定 macOS arm64、Linux x64、Windows x64 原生文件的有限 ACP 验证。它不发送 `authenticate`、`session/prompt`，不加载已有会话或用户配置，也不解除产品中的 Grok 执行门禁。`passed=true` 只表示下表的无凭据边界及两个自持进程的清理符合断言；不代表新建任务、运行中取消、审批或历史恢复成功。

## 旧入口的具体缺口与最小替代

旧 `probe_protocol.py` 的 Grok 分支有三个不能直接用于跨平台通过判定的边界：

1. 从完整父进程环境中删除部分敏感键，未把 HOME、GROK_HOME、代理和其他 CLI 配置位置全部置于私有作用域。
2. `until()` 可能超时返回 `None`，调用方没有逐项验证原生版本、认证状态、响应错误及请求完整性。进程退出 0 不能证明每个请求都有正确响应。
3. 只持有 stdio 子进程。固定官方源码的 [`spawn_leader_subprocess`](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/leader/mod.rs#L1608) 会另外创建带 `--no-exit-on-disconnect` 的 leader，且使用独立进程组；stdio 退出不等于 leader 已退出。

新探针只导入现有准备器的固定文件/环境校验和 `probe_claude_no_credentials.Recorder` 的有界 UTF-8 读写、退出回收能力，不调用 Claude 协议处理，也不修改这些旧文件。由探针分别持有以下两个 `Popen` 句柄：

```text
grok agent leader --relay-on-demand --no-auto-update --leader-socket <私有路径>
grok agent stdio --leader-socket <同一私有路径>
```

本机固定 1.0.30 的 `--help` 已确认这些原生参数。leader 使用按需 relay，并不传永久驻留参数。私有 [`GROK_LEADER_SOCKET`](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/leader/lock.rs#L34) 同时约束客户端、leader 和关联 `.lock` 路径；原生锁中写入的 PID 必须等于本次真实持有的 leader PID，每次请求及等待响应期间再次核对，不能自动关联其他实例。

Unix 使用短路径的 Unix socket；Windows 官方 [`transport.rs`](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/leader/transport.rs#L214) 把这个路径映射为固定 SipHash 派生的命名管道。探针不重新实现该哈希，也不在 Windows 用 socket 文件存在来假定管道已连接，而是核对原生锁 PID 后执行真实协议握手。Windows 实际连接与清理仍待目标机器验证。

上述固定源码快照为 `482711333c7195dc16a272777f86086d615e2afb`，不是对二进制构建提交的额外证明。文件完整摘要、下载来源和 Linux/Windows 架构检查沿用 [GROK_FIXED_PLATFORM_INPUTS.md](GROK_FIXED_PLATFORM_INPUTS.md)。

## 请求和通过边界

探针只允许以下固定顺序。四个请求的字符串 ID 包含本次随机 nonce；`cancel` 遵循通知形式，没有 ID。等待中的响应必须精确匹配当前请求；完整记录再次拒绝重复、过期、未发先到、未知 ID、缺失响应及模型/审批事件。允许的附带通知只有空列表的 `_x.ai/mcp/servers_updated`。

| 操作 | 必须观察的结果 | 证据中的状态 | 不可据此宣称 |
| --- | --- | --- | --- |
| `initialize` | ACP 整数版本 1、`agentVersion=1.0.30`、仅 `grok.com` 认证方式、无默认已认证方式、空 MCP 列表 | `passed` | 模型可用、审批可用 |
| `session/new` | `-32000 Authentication required` | `blocked_by_auth`，`session_created=false` | 已创建可运行会话 |
| `session/cancel` | 只向固定不存在会话发送无 ID 通知 | `notification_sent_without_ack` | 已取消活动回合、取得取消回包 |
| `session/load` | `-32603`，`error.data.code=FS_NOT_FOUND` | `missing_session_rejected` | 历史加载成功 |
| `session/resume` | 同上 | `missing_session_rejected` | 历史继续成功 |

没有凭据时出现非上述明确错误、未知接口或意外成功均会失败并保留实际响应，不自动认证或改用另一接口。固定不存在会话 ID 为 `00000000-0000-4000-8000-000000000000`；整个项目、配置和历史目录为新建空目录。

leader 启动等待上限 10 秒，每个请求等待上限 20 秒；stdio EOF 等待 5 秒，必须自行退出 0。之后单独观察 leader 5 秒：自然退出的非零码会失败；未自然退出时通过自己持有的句柄终止并 `wait`，记录 `forced_termination=true`。这只证明两个自持进程已回收，既不证明 leader 自然退出，也不覆盖任意脱离的工具后代。本探针没有执行模型或工具。

读取使用严格 UTF-8、单行 1 MiB 和 256 条记录上限；超限、非法编码、读取线程未结束、超时或清理失败均不能通过。只有进程回收和私有目录移除完成后才写最终通过报告。取消缺失会话产生的无回包不会被制造成成功响应。

## CI 最小接线

要求 Python 3.11+、Git 可执行文件及准备器已验证的本平台 Grok 1.0.30 文件；无需 Node、Bash、模型账户或额外 Python 依赖。Windows 的官方配套组件是否影响此入口，仍以真实运行结果为准，不提前跳过错误。只读源树，不修改 runner 全局 PATH、用户配置或凭据。

Linux（`GROK_CLI` 为准备器输出的绝对路径）：

```sh
python -B script/cli-agent-parity/grok_fixed_acp_probe.py --executable "$GROK_CLI" --output "$RUNNER_TEMP/grok-fixed-acp.json"
```

Windows PowerShell：

```powershell
python -B script/cli-agent-parity/grok_fixed_acp_probe.py --executable $env:GROK_CLI --output (Join-Path $env:RUNNER_TEMP 'grok-fixed-acp.json')
```

`--output` 必须是源树外的新绝对路径，不能覆盖先前结果。stdout 成功时只打印证据绝对路径；失败返回非零，已进入探测阶段的错误写入 `failure`。完整 JSON 包含：

- `repository_commit`、`worktree_dirty`、固定二进制大小/SHA、真实 `--version` 和文件前后不变断言。
- `cases[]` 的 method/request ID、原生错误和各自状态，以及脱敏后的 `stdio_events` / `leader_events` 完整记录。
- 两个实际 PID、私有 endpoint 类型、锁 PID 确认、stdio EOF、leader 独立等待、两进程清理、端点残留诊断和最终 `private_directory_removed`。
- `credentials_provided=false`、`authenticate_sent=false`、`model_input_submitted=false`；`running_approval_verified`、`running_cancel_verified`、`history_recovery_verified` 全部为 false。

本探针通过固定无模型操作证明没有提交模型输入；没有抓取系统网络流量，所以显式记录 `model_http_traffic_measured=false`，不能把它改写为经过测量的“HTTP 请求数 0”。环境白名单、私有 Grok/Codex/Claude 目录及禁用兼容加载沿用准备器；这不等于 Windows KnownFolder 沙箱证明。

## 本机实测与未验证项

实测原生版本为 `grok 1.0.30 (04b7ffed98c6)`，macOS arm64 文件大小 141869568 字节，SHA-256 为 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。首次独立正式输出位于 `/tmp/infinishell-grok-fixed-acp-macos-first.json`；补完私有目录清理断言后的最终输出位于 `/tmp/infinishell-grok-fixed-acp-macos-final.json`。两份报告中的源码基线是 `328d5ed35227f67884013f8c403e2a692470151d` 且 `worktree_dirty=true`，均不能算最终同提交跨平台证据。

五个边界符合上表。首次正式运行 stdio 在 EOF 后 2122 毫秒自行退出 0；leader 等待 5001 毫秒仍未退出，随后自持句柄终止，原生退出码为 -9。终止前残留 socket/lock 的诊断保留，随后移除整个自有临时目录。更早的 `/tmp` 手工草稿因把 leader 5 秒自然退出当作前提而触发 `TimeoutExpired`，已清理自持进程；该失败没有改写成自然退出成功。最终脚本将“stdio 自行退出”与“leader 强制回收”分开判定和记录。

Linux/Windows 的固定资产此前只在本机验证完整下载字节，没有在目标平台执行这份 ACP 探针。目标平台命名管道、退出码、读写线程与目录清理仍未验证；任何平台差异必须保留失败证据。认证后的新建、运行审批、活动回合取消、有内容历史恢复及产品托管入口均不在本次通过范围。

## 定向回归

```sh
python3 -B script/cli-agent-parity/grok_fixed_acp_probe_tests.py -v
```

本地 10 项通过：消费已有真实 Grok 握手形状并验证版本/认证/缺失历史；拒绝重复/过期/提前响应和模型事件；拒绝替换 leader PID；真实 Python 子进程验证 UTF-8、exit 37、响应前提前退出与 stderr、超时强制清理区别，以及报告脱敏。Python 子进程仅用于 I/O 和收尾夹具，不冒充真实 Grok。

未运行 Cargo，未改 workflow、Rust、插件、能力开关或主计划文档。新增内容是独立验证入口与技术证据，无需本地化变更。后续平台执行由根代理依照跨平台验证技能统一安排。

## 第五轮 Windows 排他锁读取失败与窄修

以上“目标平台未运行”是探针初次冻结时的历史状态。[第四轮 6635f9869](FOURTH_PLATFORM_RUN_6635F9869.md) 已在 Linux 真实通过五个有限边界；[第五轮 94a412eb](FIFTH_PLATFORM_RUN_94A412EB.md) 的 Windows 固定文件和 `--version` 通过，但首次 ACP 探测在自持 leader 启动后、stdio 创建前失败。原证据只保留 `PermissionError`/errno 13，没有操作栈，不能把推断补写成原生已记录的错误位置；没有发送 ACP 请求，也没有模型输入。

固定 [Grok `lock.rs`](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/leader/lock.rs#L149) 用 `try_lock_exclusive` 在整个 leader 生命周期持锁，再写 PID。[同提交 Cargo.lock](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/Cargo.lock) 固定 `fs2=0.4.3`，其发布包 SHA-256 `9564fc758e15025b46aa6643b1b77d047d1a56a1aea6e01002ac0c7026876213` 已下载核对。该实现 `src/windows.rs:93–114` 调用 `LockFileEx`，排他范围低/高 DWORD 均为 `0xffffffff`。[微软契约](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex#remarks) 明确其他句柄不能普通读写排他范围，但只读映射不受该字节锁限制。这解释了旧探针 `Path.read_text` 与 Windows 原生锁冲突；不是缺少认证或取消接口失败。

后续探针窄修仅把 Windows 自有 PID 诊断改为 `mmap.ACCESS_READ`：文件必须为普通私有文件，长度 1–32 字节，ASCII 正整数且不超过原生 DWORD PID；读前后检查自持 `Popen` 存活，PID 必须精确匹配。映射和文件句柄立即关闭，不释放原生锁、不修改文件、ACL 或用户配置，不读取不属于探针的路径，也不增加命名管道连接或 ACP 操作。Unix 仍普通读取。错误不降级放行，报告新增 `phase` 及原始 `errno`/`winerror`、安全操作类别 `leader_lock_pid_read` 和读取方式，供下一 Windows 结果定位。

`app/src/ai/cli_agent_runtime/grok.rs` 只将私有 `--leader-socket` 交给原生 stdio，没有上述 PID 文件读取；本次不改产品适配器或 Grok 执行 gate，也不以它未读取锁推定产品托管生命周期已通过。

新增 `test_windows_exclusive_lock_allows_only_mapped_pid_from_owned_live_child` 在 Windows 启动真实独立 Python 子进程，以原生 `LockFileEx` 持有整个范围：父普通读必须得到 `PermissionError`，只读映射必须取得实际子 PID，文件 mtime 不变；子进程 EOF 自行退出 0 后普通读恢复、旧进程身份必须拒绝。它属于现有 `grok_fixed_acp_probe_tests.py` 套件，下一 workflow 无需新步骤。另有真实文件映射的非法/超限 PID 拒绝、内容及 mtime 保留，错误操作与 errno 保留回归。

启动还有合法空文件窗口：固定 [`agent/app.rs:704–706`](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/app.rs#L704) 在创建并取得锁后单独调用 `write_pid`，后者先 `set_len(0)` 再写入。旧 `wait_leader` 实际捕获了所有 `ValueError`，所以会等待空文件，但也把错误 PID 和非法文件类型拖到超时。现只对“文件尚未创建”或专门的 `EmptyLeaderPid` 在原总 deadline 内重试；每轮继续核对自持进程存活，非空错误身份/非规则文件立即失败，正常协议阶段的空 PID 不可放行。没有重启或重新关联 leader。

确定性真实文件回归覆盖空→同一正确 PID、持续空到原 deadline、空时原进程退出，以及错误 PID/非法非空内容/目录立即拒绝。最终本机 macOS 定向结果为 17 项中 16 通过、上述 Windows 原生用例 1 项明确 skipped（0.267 秒）。这证明可本地运行的转换与隔离断言，不是 Windows 映射或 ACP 复验。新代码尚不属于第五轮 SHA，须后续同提交 Windows 运行；旧 errno 13 失败完整保留。
