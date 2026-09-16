# Codex 原生就绪后空闲崩溃验收

状态：新的 macOS 专属资源域实现已通过真实 Codex 0.147.0 空闲崩溃用例：原生 Ready/Disconnected 各一次，身份与生产清理证明均核对，回执 `macos_resource_coalition`、cleanup=true、信号退出码 None，未请求模型。[新证据](validation/macos-codex-coalition-boundaries-1.json)绑定 328d5ed352 加冻结源码，尚非最终同提交验收。Linux 已在干净 328d5ed352 通过同一原生用例；Windows 因前置夹具失败跳过。空闲根进程清理不等于运行中工具清理。

旧进程组中间证据（保持为恢复拒绝的负向边界）：[macos-codex-idle-crash-supervised-1.ndjson](validation/macos-codex-idle-crash-supervised-1.ndjson) 与对应 `.metadata.json` / `.test-output.txt`。实际事件恰好为同 generation、同 native ID 的 SessionReady 和 Disconnected，模型命令数为 0，原生根进程经绑定 PID version 的 audit token 精准终止，异常退出回执被恢复 gate 拒绝。libtest SHA256 为 `9507e5d5b31f01e4f18d2451b5f2ea4f4e93edb1481909de1827fe5349d3e5f0`，worker 为 bundle6 的 `eeb2486cc28e50f499ddf279fcabade4c3efea5783af54f98b090dd8b313557b`；均属于 dirty 工作树中间构建，不能替代最终同提交验收。

## 实际信息增益

现有 4 个 `managed_process::live_tests::supervised_*` 使用通用原生夹具，不能证明真实 Codex 握手后事件身份/终态正确；`live_codex_missing_session_is_not_replaced` 则验证 Ready 前原生拒绝。本次只新增一个 ignored 测试：

`ai::cli_agent_runtime::codex::tests::idle_crash::live_codex_idle_crash_after_ready_disconnects_once`

测试通过生产 Codex `connect` 和真实 InfiniShell worker 派生固定 0.147.0 原生 CLI，在空白私有 `CODEX_HOME` 中等待真实 `thread/start` 的 `SessionReady` 与 native ID，再强制终止真实 Codex。整个过程中 controller 保持存活，发送模型命令数为 0，不主动发送 Shutdown、取消或审批。

## 精准终止范围

1. 测试进程 PID → 直接子监督 worker，其绝对 executable 与参数 `cli-agent-supervisor <本次唯一 generation 的 manifest>` 完全匹配。
2. Linux 的 execute worker 已 exec 为真实 Codex，native 是监督 worker 的直接子进程。Windows 必须经过同 executable、同 manifest、附 `--execute` 的中间 worker，再找到直接子原生 `codex.exe app-server --stdio`。macOS 则通过本代 manifest→coalition claim→native claim 摘要链定位 launchd wrapper；核对同 boot、独占 CID、wrapper 与真实 Codex 父子关系、真实镜像及 argv。不能把 wrapper 的 PPID=1 误当成失去归属，也不能沿 PPID 猜测它回到应用。
3. 每层要求唯一匹配，只刷新候选自身子进程的路径/参数，不输出其他进程命令行。读取 PID/start-time/父 PID 只用于初筛；取得稳定身份后再次核对整条链，才允许故障注入。
4. Windows 持有 `OpenProcess` 的进程对象句柄，核对该句柄的创建时间、镜像路径与存活状态，`TerminateProcess(handle, 73)`，以同句柄等待退出并核对 73；不调用 taskkill。Linux 用 `pidfd_open` 和 `pidfd_send_signal`，等待该 fd 的退出事件，内核不支持就失败，绝不降级到裸 PID。macOS 使用本机已实证的 `PROC_PIDUNIQIDENTIFIERINFO` + `proc_signal_with_audittoken` PID version 绑定，先注册 NOTE_EXIT、复核路径/身份，再发 SIGKILL 并等待对应进程事件；符号缺失则失败。
5. 该 helper 不是全树管理器。它只对预先锁定的真实根进程注入崩溃，清理证明仍由生产监督模块判断。不会扫描并杀死任意后代或按名称批量杀进程。

## 验收及故障证据

- 收完整 task，并释放已完成 future，随后排空事件通道至关闭：必须恰好一次 Ready + 一次 Disconnected，generation 和原生 ID 完全一致；任何 turn/审批/成功事件或重复断线均失败。
- Linux/Windows 的错误必须为真实 stdout EOF 协议错误，回执分别为 `linux_subtree`/`windows_job`，cleanup=true，`confirmed_exit` 返回同一回执。Windows 还必须保留 73；Unix 必须保留信号退出的 None。
- macOS 新资源域分支必须为真实 stdout EOF 协议错误，原始回执 `macos_resource_coalition`、cleanup=true，`confirmed_exit` 验证完整生产证明，`macos_coalition_ownership_verified` 与 `macos_cleanup_proof_verified` 均为 true。PID unique ID 与 CID 必须保持，exec 引起的 PID version 变化单独核对。旧 `unix_process_group` 分支仍要求 cleanup=false、拒绝恢复及 `idle_process_cleanup_confirmed=false`，运行器不能把旧负向证据升级。
- 运行器保存 Ready、句柄绑定、根进程退出、适配器结果和原始回执各阶段的 NDJSON；只有完整断言后追加 `idle_crash_probe_finished/passed=true`。运行器还必须看到实际 1 测试通过、exit=0、未超时，不能采用阶段记录或 0 tests 作为成功。
- Ready 限 40 秒、注入后的根退出限 5 秒、适配器/监督收尾限 30 秒。断言失败先释放适配器及 controller 的控制管道，并等待已经绑定的监督 worker 最多 30 秒。运行器总计限 120 秒；终止外层 libtest 只使用自己持有的 Popen 子进程句柄，监督者仍按生产控制 EOF 清理。超时或无法取得身份都不计通过。

## 调用

执行前必须统一构建同提交的 libtest 和主程序/TUI worker。可复用当前 `prepare_codex_cli.py` 的固定摘要下载；预检沿用固定下载器，并与缺失会话用例共用同一原生可执行文件。

Linux shell（路径变量由该次构建步骤提供，必须是绝对路径）：

```sh
CODEX_EXE="$(python3 script/cli-agent-parity/prepare_codex_cli.py --download-dir "$RUNNER_TEMP/codex-0.147.0-idle-crash")"
python3 script/cli-agent-parity/run_codex_adapter_live.py --test-case idle-crash --test-binary "$LIBTEST_EXE" --codex "$CODEX_EXE" --supervisor "$SUPERVISOR_EXE" --output "$RUNNER_TEMP/codex-idle-crash.ndjson"
```

Windows PowerShell（直接使用原生 EXE；不把 `.cmd` 包装入口计入此用例）：

```powershell
$codexExe = (& $pythonExe script/cli-agent-parity/prepare_codex_cli.py --download-dir (Join-Path $env:RUNNER_TEMP 'codex-0.147.0-idle-crash')).Trim()
if ($LASTEXITCODE -ne 0) { throw '固定 Codex 准备失败' }
& $pythonExe script/cli-agent-parity/run_codex_adapter_live.py --test-case idle-crash --test-binary $libtestExe --codex $codexExe --supervisor $supervisorExe --output (Join-Path $env:RUNNER_TEMP 'codex-idle-crash.ndjson')
if ($LASTEXITCODE -ne 0) { throw '原生空闲崩溃验收未通过' }
```

macOS 可以直接把已受测的原生 0.147.0 绝对路径传给相同 runner；不要传 `--credential-source`。runner 为此用例拒绝认证来源，隔离 HOME、XDG、APPDATA/TEMP/CODEX_HOME，固定 `cli_auth_credentials_store="file"`，不读取或复制登录信息。环境通过子进程传入，测试不改全局 Rust 环境。

## 未覆盖

- 真实 CLI 在 thread/start 之前/执行中强杀（没有新增生产测试 barrier）。missing-session 是原生拒绝，不能冒充这个崩溃分支。
- Claude/Grok 的原生 idle crash；本次无需登录的确定性增益首先收敛在 Codex。
- 模型生成中或工具运行中清理、应用 UI 重启恢复、SSH/tmux、ConPTY、NPM/.cmd 启动入口。
- macOS 任意脱离组工具的完整清理仍受已记录的限制；本用例只有空闲 native 根进程，不能填补该限制。
- Windows 原生分支尚待修复前置夹具后实跑；Linux 同提交已通过，但本次更新后的最终同提交平台验证仍须执行。

测试/协议文件与运行器帮助无产品文案变化；无需本地化变更。
