# Codex Windows 原生 hook 独立验证

本验证只覆盖固定 Codex 0.147.0 的 `cmd.exe → Windows PowerShell 5.1 → Git Bash` 命令路径，以及原生 SessionStart、UserPromptSubmit 和输入阻断。脚本只在临时插件生成 `commandWindows`，不修改应用随附资源、用户插件、全局 shell 配置、审批策略或 Windows 自动安装门控。当前实现尚未接入产品，不能记作 Windows 通知插件支持已经完成。

## 固定依据和输入

Codex 固定发布为 [rust-v0.147.0](https://github.com/openai/codex/releases/tag/rust-v0.147.0)，源码提交为 `be6e8eac029b183056b7e4402879f15d2c85f61b`。可执行文件的大小和 SHA-256 固定在 `script/cli-agent-parity/codex_windows_hook_inputs.py`，来自该发布的官方 API `assets[].digest`：

| 平台 | 官方资产 | SHA-256 |
| --- | --- | --- |
| Windows x64 | `codex-x86_64-pc-windows-msvc.exe` | `935a1911ed2556e4ffcec995f4886ac2ac425863ba26fed264df62e30272ad9d` |
| Windows ARM64 | `codex-aarch64-pc-windows-msvc.exe` | `1f0e8c2dd3c6b471e985fac76908366c1cf31155094fde606fb2d3052cf00584` |

下载 URL 固定为 `https://github.com/openai/codex/releases/download/rust-v0.147.0/<资产名>`。脚本不接受 `latest`、版本字符串代替摘要或调用方提供的新摘要。已有缓存也必须重新核验，损坏文件直接失败而不覆盖；下载先写临时文件，大小和摘要都正确才发布到缓存。Windows x64 自托管 CI 使用 x64；ARM64 只是可选固定输入，未运行不能计为平台通过。

原始通知插件为 [codex-warp 固定提交](https://github.com/warpdotdev/codex-warp/tree/31ce59d9011cfb1d78f265649a228dac5de58d76/plugins/warp) 中的 `plugins/warp`，版本 0.4.0。从现有 `PATCH_METADATA.json` 读取这个确切提交的十个文件摘要，逐个从固定 raw URL 下载，或验证调用方给出的原始目录。验证要求全树精确相等，拒绝额外、缺失、已修补、符号链接、硬链接和 Windows 重解析点。只有临时副本应用现有受控修补，然后添加本验证的 Windows 命令与插桩。

## 命令表示的依据

- [hook_config.rs](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/config/src/hook_config.rs#L153) 定义 `commandWindows`；这不是独立的可执行文件和参数数组。
- [discovery.rs](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/engine/discovery.rs#L472) 选择平台命令并计算该命令的信任摘要；该文件也设置 `PLUGIN_ROOT` 环境变量，并对 `${变量}` 进行文本替换。因此命令中不使用 `${PLUGIN_ROOT}` 或 `%PLUGIN_ROOT%` 拼接目录。
- [command_runner.rs](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/engine/command_runner.rs#L191) 将环境传给进程，Windows 默认使用 `COMSPEC` / `cmd.exe /C`。验证保留这个原生路径。

`codex_windows_hook_command.ps1` 是可审阅的固定明文。生成器仅前置五个白名单脚本名之一，将 LF 规范化后的明文编码为 UTF-16LE Base64，随后立即解码逐字比较。命令为 `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand <派生内容>`，不提交或手改不透明编码串。

插件目录从进程环境读取，经路径 API 进入原生 argv；不进入 cmd、PowerShell 或 Bash 源码。Windows PS 5.1 没有 `ProcessStartInfo.ArgumentList`，所以启动器使用固定的 Windows CRT argv 编码函数，由实际原生进程捕获参数验证。stdin 通过原始流复制，stdout/stderr 由子进程继承，避免 PS 文本管道改变 UTF-8 JSON 字节。Git Bash 只接受同目录有 `msys-2.0.dll` 的 `usr/bin/bash.exe`，使用 `--noprofile --norc`；不会把 WSL shim 当作可用环境。

## 零模型请求的依据和断言

固定源码的 [UserPromptSubmit 解析](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/events/user_prompt_submit.rs#L181) 将 `continue:false` 转成停止结果。[回合入口](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/session/turn.rs#L233) 在首次模型采样前运行 hook，阻断未接收输入时提前返回。

仅靠这个先后顺序仍不够，因为 [普通任务](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/tasks/regular.rs) 可能等待启动预热。验证为隔离 provider 明确设置 `supports_websockets=false`，依据 [session_startup_prewarm.rs](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/session_startup_prewarm.rs#L185) 不进入 websocket 模型预热；`requires_openai_auth=false`，没有 env_key，环境中不传模型凭据。provider HTTP 地址只指向本机计数服务，所有请求均记录并拒绝，不返回模型内容。只有原生阻断事件确实到达且进程退出后累计请求仍为零，才记录 `zero_model_requests=true`。

探测使用新 `CODEX_HOME`、HOME、USERPROFILE、APPDATA 和 LOCALAPPDATA。先要求原生 `hooks/list` 的六个临时 hook 均未受信任，核对完整已安装副本及平台命令，再仅在测试 HOME 为这六个已核对 hook 记录授权。随后原生列表必须确认 enabled/trusted。测试没有修改用户信任，也没有关闭审批。runner 的 cmd AutoRun 非空会直接失败，不会清理或绕过该设置。

## Windows CI 一条执行命令

前提为 Windows 原生 Python 3.11+、系统 Windows PowerShell 5.1/cmd.exe、预装 Git `usr/bin/bash.exe` 和 `jq.exe`。`INFINISHELL_GIT_BASH_EXE`、`INFINISHELL_JQ_EXE` 必须是这两个工具的绝对路径，`GITHUB_WORKSPACE` 指向待验的同一提交，`RUNNER_TEMP` 位于源树外。首次下载约 299 MB 的 x64 exe，需要访问上述固定 GitHub release 与 raw URL；不需要用户凭据。执行环境必须为受控、没有额外机器级 Codex provider 覆盖的 runner。

```powershell
python -B "$env:GITHUB_WORKSPACE\script\cli-agent-parity\probe_codex_windows_hooks.py" --repo "$env:GITHUB_WORKSPACE" --architecture x86_64 --download-dir "$env:RUNNER_TEMP\codex-0147-hook-inputs" --bash-executable "$env:INFINISHELL_GIT_BASH_EXE" --jq-executable "$env:INFINISHELL_JQ_EXE" --output "$env:RUNNER_TEMP\codex-windows-native-hooks.json"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
```

也可用 `--codex-executable <绝对路径> --upstream-plugin <绝对路径>` 传入 workflow 预先准备的输入；仍执行同一固定摘要验证，不存在跳过开关。Python `-B` 和脚本的 `dont_write_bytecode` 防止向源树写入 pycache。输出和下载目录位于源树内会失败。

报告为 `RUNNER_TEMP/codex-windows-native-hooks.json`；失败也包含具体阶段和已取得的原生 trace。workflow 应保留失败报告，并核对任务的 `headSha`，不能以其他提交的结果代替当前代码。

## 独立检查与结果边界

1. 编码：五个命令都能从固定明文逐字重建，单命令不超过 8000 字符。
2. 语法：Windows PS 5.1 的原生 parser 分别检查五份完整源码，不用 PS 7 结果替代。
3. argv：同一编码函数在原生 PS 中启动 Python 捕获参数，覆盖空串、中文、空格、单双引号、反斜杠和 shell 特殊字符。
4. 字节及长度：同一固定启动器经真实 cmd/PS/Bash 执行五个临时脚本，比较 stdin/stdout 原始字节；执行恰好 8191 个 UTF-16 单元的命令；8192 在创建进程前拒绝。长度包含 cmd 路径、`/C` 和外层引号。
5. 原生 hook：在普通中文空格目录和含 shell 特殊字符目录各执行一次。原始脚本逐字保留并实际调用，必须收到 SessionStart completed、UserPromptSubmit completed 和独立阻断 stopped；验证 session/turn 关联、中文多行 prompt 与 PLUGIN_ROOT。不能只凭插桩文件出现就通过。
6. 模型：原生停止必须成立，所有原生进程结束后本机 provider 请求累计为零。

缺工具、摘要不符、语法失败、字节变化、未执行、原始脚本失败、超时或模型请求出现均使命令非零退出。非 Windows 执行也失败，不算跳过通过。

本地低成本回归命令为 `python3 -B script/cli-agent-parity/codex_windows_hook_tests.py`。2026-09-16 已通过 12 项纯编码、边界防护和真实文件摘要回归，并实际下载固定上游十个文件核对全树摘要。Windows PS 5.1、原生 cmd/Bash 字节与边界执行、原生 Codex 阻断尚待同提交 CI 实跑；本地 Python 检查不计作这些项目通过。

本验证始终输出 `native_conpty_notifications_verified=false`、`full_lifecycle_verified=false`、`model_generation_verified=false`。管道模式中脚本可能吞掉 `/dev/tty` 写失败，进程退出 0 不能证明 InfiniShell 收到 OSC。仍需单独在真实 Windows ConPTY 内直接启动原生 Codex，捕获 OSC 并验证终端解析；Stop、PermissionRequest、PostToolUse 的模型驱动事件也未覆盖。在这些门槛完成前，Windows 自动安装继续关闭。此次没有用户界面变化，无需本地化变更。
