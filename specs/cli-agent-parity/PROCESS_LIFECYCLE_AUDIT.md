# 托管 CLI 进程退出与崩溃恢复审计

本节记录 P4 最初审计、现已加入的监督实现与尚待执行的验收。审计对象为三款 adapter、`crates/command` 和固定 Codex 0.147.0 上游实现；此故障验证不运行模型，也不重复请求 Grok 402。

## 修补前的缺口

- 三个 `run_process` 都使用 `command::async::Command::new`、管道 stdin/stdout 和 `kill_on_drop(true)`。正常传输结束后调用 `child.kill()` 与限时 `child.status()`，但丢弃两者错误，然后返回原传输结果。此处只能发起终止，不能证明终止已确认。
- `async-process::ChildGuard::drop` 内部同样忽略直属进程 kill 错误。析构器不会在应用 SIGKILL、崩溃或断电时运行，也没有证明工具后代进程退出。
- `new_with_process_group` 在 Unix 只创建以子 PID 为 PGID 的进程组。调用直属 `child.kill()` 不会自动发送进程组信号；建组本身也没有父进程死亡清理机制。该方法的 Windows 实现明确留有 TODO。
- Windows async wrapper 默认同时设置 `CREATE_NO_WINDOW` 和 `CREATE_BREAKAWAY_FROM_JOB`，没有 blocking wrapper 的 `kill_on_parent_process_close` 关联逻辑。不能用应用全局 Job Object 证明当前异步 CLI 已随应用退出。
- 现有 `JobObject` builder 允许 breakaway，且通过泄漏句柄使 Job 与应用同寿命。它不是可按任务关闭的 guard；把进程创建后再赋予 Job 也存在赋予前派生子进程的窗口。需要明确受控派生/赋予顺序与失败行为，不能忽略 Job 赋予失败继续运行任务。
- 应用重启把活动任务持久化为 Disconnected 是正确的状态降级，但当前记录没有“旧执行进程已确认退出”的证明。generation 能过滤旧回调，不能阻止旧进程继续产生文件副作用。

## 原生 EOF 证据及限制

Codex 0.147.0 的单 stdio 连接关闭会结束处理循环，等待连接清理、后台任务清理，然后调用 `shutdown_threads`；后者使用 `shutdown_all_threads_bounded(Duration::from_secs(10))`，对超时/失败记录警告。它是尽力关闭路径，不是客户端收到的完整工具树退出回执。[固定版本主循环](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/app-server/src/lib.rs)、[固定版本 thread processor](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/app-server/src/request_processors/thread_processor.rs)

`probe_process_eof.py` 在临时 HOME/CLI_HOME 中只完成原生握手，再关闭 stdin。`PROCESS_EOF_EVIDENCE.json` 记录本机 Codex 0.147.0 和 Claude 2.1.273 均在 5 秒内自行以 0 退出。探测没有活动回合、工具后代或父进程 SIGKILL；它只能验证空闲 EOF，不是完整崩溃恢复验收。Grok 未加入此探测。

Claude 和 Grok 的运行中 EOF/父崩溃/后代退出仍缺直接实测。不得根据空闲 EOF 的成功、PID 不存在或某个 native session 可 load，就允许同一历史会话自动启动第二个执行进程。

## 实施顺序与边界

1. 将三 adapter 的正常停止提取为同一进程生命周期边界：先关闭 stdin，限时等待；超时后终止受控进程组或专属 Job，再等待确认；退出失败需返回明确错误。发送 Shutdown 只能标记本地已发送，不能直接成为 native ACK 或进程退出确认。
2. 持久化区分“已确认退出”和“因应用崩溃而未知”。只有确认旧执行已退出的 generation 才允许 spawn 新进程继续同 native ID；未知状态保持断开并拒绝执行，不能用仅检查 PID 或解锁成功替代证明。这是暂时的安全门禁，不计为自动恢复完成。
3. 要完成自动崩溃恢复，需要独立受控监督者或操作系统级所有权机制：监督者持有本 generation 的生命周期，父管道 EOF 后关闭/终止进程树，确认退出后原子写入 generation 绑定的退出回执。监督者本身意外消失且没有回执时仍按未知处理。Unix 进程组、Windows Job 均需覆盖工具后代及明确的 breakaway 边界；单靠父进程析构不满足此目标。

上述顺序只处理应用托管任务，普通终端中的 CLI 生命周期继续交给 PTY/用户终端管理。

## 无需模型的故障验证设计

- 使用仓库内受控 CLI 夹具模拟握手后派生一个只向临时目录追加 heartbeat 的工具进程；记录直属与后代的创建令牌，不仅记录 PID。
- 分别覆盖正常 Shutdown、stdin EOF、协议错误、wait 超时、kill/Job 关联失败、应用宿主 SIGKILL，以及监督者自身 SIGKILL。退出未确认时恢复必须拒绝，并且没有第二次派生事件。
- 新应用恢复后，只有对应 generation 的退出回执已持久化才可继续；重投旧回执、重复恢复、PID 重用、过期 generation 均不能解锁新一轮执行。
- 停止确认后观察 heartbeat 不再追加，再启动下一代；用实际修改的同一提交分别在 macOS、Linux、Windows 执行。夹具验证不冒充三款 CLI 的真实运行中取消与恢复，后者仍需要独立验收。

本审计没有变动用户界面文案，无需本地化变更；若实现“退出未知”的用户提示，需要主代理同步英文与简体中文。


## 已实现的监督契约

`managed_process::spawn` 在当前 SQLite scope 的状态目录中原子占用 `cli-agent-processes/<generation>`，记录受控 executable、argv、cwd 与随机令牌。它启动当前已打包二进制的隐藏 `cli-agent-supervisor` worker。控制连接使用独立的本机 TCP 通道，核对 generation 与随机令牌；应用退出导致该连接 EOF，即使 CLI 的 stdin 写入阻塞，监督者仍可接收断线。

监督者先派生只等待授权的内部执行 worker，再建立专属进程组或严格 Job，最后才授权它启动真实 CLI。Unix 内部 worker 通过 exec 保留 PID/组；Windows 内部 worker 派生 CLI 时显式继承 Job，不带 breakaway 标志。普通 `Command` 的默认行为不变。

- Linux：独立监督者成为 subreaper，按内核列出的直属子进程清理并回收被重新收养的后代；主动 setsid 的后代也纳入 `linux_subtree` 回执。
- macOS：保留根进程的未回收 PID，终止专属组并核对该组没有活跃成员，再回收根进程；回执是 `unix_process_group`。主动 setsid 脱组的任意外部守护进程不在覆盖内，负向测试必须体现这点，不能宣称整个外部进程树退出。
- Windows：执行授权前将内部 worker 赋予禁止 breakaway 的专属 Job；清理后查询 `ActiveProcesses == 0`，再生成 `windows_job` 回执。Job 赋予失败时不授权真实 CLI。该实现仍需 Windows 同提交真实运行验证。

只有清理成功才以临时文件原子持久化 `exit.json`，其中包含 generation、manifest SHA-256、实际 containment 和退出原因。`finish` 需要监督者实际成功退出且回执匹配，不能用超时、PID 不存在或空闲锁代替。确认结果保留在进程 guard 中，回收后重复确认不再发送 PID/组信号。

若应用已持久化 generation、却在派生前退出，恢复可以调用 `record_not_started` 竞争同一个原子目录。只有成功独占此前未被领取的 generation，才写入 `launch_allowed:false` 与 `not_started` 回执；任何迟到启动都会因目录已存在被拒绝，worker 也拒绝已封存 manifest。已有完整或不完整账本均不覆盖；因此该证明来自原子封存，而不是把“文件缺失”当成已经退出。

监督者自身被强制结束、记录掉盘或不可解析时没有可接受回执，恢复保持阻止。Windows Job handle 关闭会终止受管进程；Unix 监督者本身被 SIGKILL 时不能依赖 Rust Drop 清理，因此缺失回执仍是必须保留的失败边界。

## 验收入口与当前执行状态

以下新增测试已经落盘，编译与实际运行由主代理统一安排；在实际日志返回前不计为通过。

```sh
cargo test -p command managed::tests::managed_tree_reaps_descendants_and_never_signals_after_confirmation
cargo test -p warp --lib managed_process::tests
INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=<同一源码已构建主程序或TUI绝对路径> \
  <warp-libtest绝对路径> 'managed_process::live_tests::supervised_' \
  --ignored --nocapture --test-threads=1
```

最后一条必须实际运行 3 项验收；其余 3 个 ignored fixture 只供隔离子进程调用，不是独立通过证据。libtest 仅作为受控 CLI/宿主替身，监督入口必须是已构建主程序或 TUI，禁止将 libtest 标准输出误当成原生 JSON 协议。

普通文件测试覆盖缺失/部分回执、旧 generation、manifest 摘要变化、作用域隔离、并发封存、拒绝错误 generation/token。进程测试覆盖根自然退出后后代停止、正常 finish、宿主 SIGKILL、回执丢失后不放行，以及各平台主动脱离边界。确认后身份替换测试是确定性的 guard 回归，未强制操作系统复用 PID，不将它描述为真实 PID 分配器压力测试。夹具只写临时 heartbeat，且自行限时退出。

本模块无新增用户界面文案，无需本地化变更；恢复错误的界面提示由主代理在中英文资源中同步。真实 CLI 经过新监督路径的生命周期仍要重跑，先前直接派生路径的成功记录不能替代它。

## macOS 真实 worker 首轮验收发现及修补

首个 GUI bundle 的真实监督验收 3 项均失败，外层只显示等待就绪超时。进一步直接调用隔离宿主夹具，实际在约 0.2 秒内得到 `WouldBlock (35)`；改用诊断脚本提供阻塞控制端后，真实 worker 可以报告 ready，却在空闲原生进程退出后返回 `EPERM (1)`，没有退出回执。两项失败原文和受测二进制 SHA-256 保存在 `fixtures/supervisor-macos-startup-before-fix.json`，不能计为监督验收通过。

- macOS 接受连接继承 listener 的非阻塞属性。监听器为了限时 accept 使用 nonblocking；已接受的控制连接现在显式切回阻塞模式，再应用握手读写超时。新增延迟发送 ready 字节的回归，不通过缩短等待或忽略错误绕过握手。
- macOS 对仅剩僵尸根进程的进程组执行 `killpg(SIGKILL)` 会返回 EPERM。清理现在先检查受管理组无活跃成员且原根进程已经通过 WNOWAIT 确认退出，此时直接回收；如果最后一个活跃成员在快照后消失，EPERM 也必须重新通过同样的完整退出检查。存在任何活跃成员或无法确认时仍失败，不把 EPERM 本身解释为成功。
- 隔离夹具将自身 stderr 写入临时文件，父测试在子宿主提前退出时直接报告具体错误，不再用统一就绪超时掩盖它。验收仍要求根进程和后代实际停止写入，且回执匹配。

修补后的 Rust 测试及重新构建的实际 worker 仍需主代理统一执行。可用 `probe_supervisor_startup.py --test-binary <libtest> --supervisor <worker> --output <artifact>` 重跑相同诊断；该脚本不会请求模型或变更用户 CLI 配置。
