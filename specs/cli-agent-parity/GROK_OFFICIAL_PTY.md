# Grok 官方普通 PTY 输入准备

本次仅准备固定 Grok Build `1.0.30 (04b7ffed98c6)` 的普通 TUI 输入探针，区别于托管 ACP、`App::test` 协调器及应用 GUI 验收。**实际 live 启动仍被运行器拒绝，不能计为普通终端输入、文本响应或取消通过。** 未复制官方认证、未开网络隧道、未启动带认证 CLI、未发模型请求。

2026-09-17 在临时 HOME、禁止网络和宿主配置写入的 macOS 沙箱中，只读执行固定二进制 `--help`，退出码 0，帮助正文 7,512 字节，SHA-256 为 `cda6873e2f90a7d77de94c2e3026794671fac04b2e40ac74e4d91f429f829403`。原生二进制摘要为 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。帮助确认 `--minimal`、`--no-alt-screen`、`--leader-socket`、`--session-id`、`--no-subagents`、`--disable-web-search` 可用于普通 TUI；这些参数尚未一起实际启动验证。不使用 `agent stdio`、headless、`--always-approve` 或新的网页授权流程。

现有官方包装器仅接受版本检测和 `agent stdio --leader-socket`，本次未扩展它的参数白名单。现有 macOS 监督执行入口将原生 stdio 接入 UnixStream，而非控制终端；尚未验证在该资源域内提供 PTY 的桥接方式。普通 Grok 的私有 leader 可能脱离 PTY 进程组，清理 Python 父进程或进程组不足以证明全树退出。因此 `--live` 在认证复制、配置准备和网络启动之前固定拒绝，原因稳定值为 `pty_containment_not_verified`，没有绕过开关。Linux、Windows 同样拒绝启动原生 CLI；离线测试可使用模拟通道。

输入计划固定最多三条：

| 阶段 | 准备的交互 | 未来可证明的边界 |
| --- | --- | --- |
| 英文多行 | bracketed paste 两行英文，先观察输入渲染，再写 Enter | 只证明特定文本响应出现，不证明原生 ACK 或任务完成 |
| 中文多行 | bracketed paste 三行简体中文，观察中文输入渲染，再写 Enter | 同上，完整答复标记不出现在提示中，避免 prompt echo 冒充答复 |
| 取消键探针 | 第三条纯文本要求逐行计数，观察连续三行数字后写 Ctrl-C 字节 | 只证明键写入和此前正文出现，不证明中断被接收、Cancelled 或全树清理 |

两条答复标记通过分段拼接描述，完整预期字符串从未进入提示；渲染器还拒绝 Enter 前已出现的旧标记。完整输入、原始 TUI 输出和控制字节仅写私有帧文件，使用独占创建与 macOS `0600` 权限，输入及输出合计最多 8 MiB，编码后帧文件最多 16 MiB。公开事件只含固定阶段、字节数、SHA-256 与明确为 false 的未验证项，不包含提示、答复正文、终端标题、任意文件名或凭据。探针使用有界 VT 视口处理光标、清屏、颜色、UTF-8 与中文宽度，未知控制序列直接使观察判据失效，非法 UTF-8 只返回固定错误；它不是 InfiniShell 实际 GUI 的渲染证明。单阶段读取有期限，EOF、短写及超时不能计为成功。

准备工作复用官方模型私有配置生成，保留精确 `ask any` 审批，不放宽权限；生成后移除只适用于 ACP 的私有可执行包装器。固定 TUI 参数和输入计划只存私有文件，不留下可无监督启动的 TUI 包装器。私有 HOME 的 SSH 配置仅使用自己的空 known_hosts，强制严格检查并关闭身份代理；宿主 `.ssh`、Claude/Codex 兼容配置和 Grok 认证配置在预备沙箱规则中不可读。Claude/Codex hooks 与 MCP 的关闭环境沿用官方运行器设计，尚未由本轮实际 TUI 验证，不声称兼容加载行为通过。

下一步需要先验证控制终端桥接与完整 macOS 资源域：绑定唯一运行代、封闭 leader socket 与私有 HOME，核对 launchd wrapper、原生进程的 PID/出生身份和资源 CID，核验实际 PTY 尺寸为 120 列、40 行，不能只依据环境中的 COLUMNS/LINES。参数显式固定 `--cwd` 到隔离空项目。清理必须具有实际退出、job removed、resource CID destroyed 及可信回执。不能以 socket 消失、Ctrl-C、普通 EOF、父组退出或广泛 `pkill` 替代。完成该门槛后，才能接入现有 opaque 官方认证私有副本、真实 OS 沙箱 canary 与精确 TLS 白名单；认证只能由原生解析，退出无论成功失败都删除副本并关闭隧道。继续使用 `cli-chat-proxy.grok.com`、`auth.x.ai`，最多 32 条 TLS 连接和 32 MiB，不解密 HTTP；模型 HTTP 调用数和费用不具有硬预算。本轮 `network_canary_verified=false`、认证副本状态不适用，不能把准备阶段零网络当成真实网络隔离已验证。

2026-09-17 的独立只读架构评估提出以下最小路径，**仅用于专用监督 PTY 验收夹具，尚未实现或验证**。它不表示普通 GUI TerminalServer 已接入，也不把夹具额外的身份与资源证明计作已实现产品能力；普通 GUI 的终端渲染、输入、resize 和工具栏交互必须另行真实验证。

在监督 manifest 中新增显式 `Pipe/Pty` 传输类型与 PTY 尺寸，保留既有调用的 Pipe 默认值；未实现的平台明确拒绝 Pty。由 launchd 资源 CID 内的 wrapper 复用现有终端启动逻辑，为 gated child 配置 slave 标准流、复位信号、`setsid` 与 `TIOCSCTTY`。通过已认证 Unix 控制通道将 master 以恰好一个 FD 的 `SCM_RIGHTS` 交给域外监督者，拒绝缺失、多余、截断或错误类型的 FD。现有 TerminalServer 已有 FD 传递机制，但消息绑定 `SpawnShellResponse`，不能直接用于监督者当前的控制帧协议；桥接必须使用有界、与本代身份关联的专用消息。

允许 exec 前，子 worker 还需核验三路 `isatty`、相同终端设备、session、foreground PGID 及内核返回的 120×40 尺寸；将证明关联 generation、manifest 摘要、原生 PID/出生身份及源快照和可执行文件身份，保存后才放行真实 CLI。私有 HOME 和精确 leader socket 继续封闭；leader 的实际 PID/出生身份须由内核 socket peer identity 核对，且必须属于本次 CID，不能信任自报 PID 或仅凭 socket 路径。继续复用现有资源域清理与退出回执，不以普通 PTY 进程组清理替代 job 移除和 CID 销毁。

监督者应持有 master 至资源域清理及限时输出 drain 完成；另 session 的 daemon 可能保留 slave，不能等待 PTY EOF 后才开始清理，也不能因 wrapper 被 bootout 提前丢失 master。PTY 没有 socket 式写半关闭，输入 EOF 必须走监督控制通道，不能照搬 `shutdown(Write)` 或关闭读端。Ctrl-C 仍只是 TUI 输入字节，最终关闭须独立取得准确原生 wait、job removed、CID destroyed 与可信回执；文本响应和 Ctrl-C 观察不升级为任务 Completed/Cancelled。

建议的最小写域如下，均为未来修改范围，本轮没有修改这些源文件或运行夹具：

| 路径 | 拟议改动 |
| --- | --- |
| [managed_process.rs](../../app/src/ai/cli_agent_runtime/managed_process.rs) | manifest 传输类型、尺寸、PTY 启动入口及子 worker 的启动前 TTY 证明握手 |
| [managed_process_macos.rs](../../app/src/ai/cli_agent_runtime/managed_process_macos.rs) | wrapper 内 PTY 启动、已认证单 FD 桥接、master 输入输出、身份关联、清理与 EOF 时序 |
| [local_tty/unix.rs](../../app/src/terminal/local_tty/unix.rs)、[local_tty/spawner.rs](../../app/src/terminal/local_tty/spawner.rs) | 仅开放复用现有 controlling TTY 启动函数及必要返回类型的内部可见性，避免复制启动逻辑 |
| [managed_process_tests.rs](../../app/src/ai/cli_agent_runtime/managed_process_tests.rs)、[managed_process_macos_tests.rs](../../app/src/ai/cli_agent_runtime/managed_process_macos_tests.rs)、[managed_process_macos_live_tests.rs](../../app/src/ai/cli_agent_runtime/managed_process_macos_live_tests.rs) | 传输与握手回归，以及无认证的真实 TTY、另 session 后代、断连与资源清理验证 |
| [run_grok_official_pty_live.py](../../script/cli-agent-parity/run_grok_official_pty_live.py)、[grok_official_pty_runner_tests.py](../../script/cli-agent-parity/grok_official_pty_runner_tests.py) | 单独生成精确 TUI 参数包装器，绑定监督桥接与清理证据；无认证资源验证通过后才考虑解除在线守卫，不扩大 ACP 包装器白名单 |

离线回归命令：

```sh
python3 -B script/cli-agent-parity/grok_official_pty_runner_tests.py -v
```

20 项离线回归已通过，覆盖中英文分块 UTF-8、paste/Enter 顺序、prompt echo、旧响应、隐藏 OSC 标记、未知 VT 控制、固定参数与精确沙箱、私有原始帧权限与大小、短写/EOF/超时、仅发送取消键的证据边界、API 环境不继承、三个平台在认证和网络前拒绝 live，以及模拟文件准备不导出提示。它们未运行真实 CLI 或模型，不证明 GUI 或平台实际效果。

macOS 仅准备命令，不读取官方认证、不执行原生 TUI：

```sh
python3 -B script/cli-agent-parity/run_grok_official_pty_live.py \
  --prepare-only --grok /absolute/path/to/fixed-grok-1.0.30 \
  --timeout 120 --output /absolute/path/to/new-preparation.ndjson
```

该命令退出 0 仅表示文件准备成功，报告中 `real_input_verified`、`pty_containment_verified`、`normal_cleanup_verified` 均为 false。记录的帮助摘要来自上述历史只读校准，不冒称本次重新执行版本或帮助检测。`--live` 固定退出 2；本轮未提供真实 live 验收命令。

无需本地化变更：新增内容均为工程验收脚本、稳定证据字段及文档，没有产品 UI 文案。后续 InfiniShell GUI 工具栏、普通终端实际输入与附件/技能组合、Claude hooks 兼容、SSH/tmux、其他平台以及最终同提交门禁仍未验证。
