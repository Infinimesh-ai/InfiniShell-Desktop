# Codex 原生 hook 与控制 PTY 通道实证

## 本次结果

固定 Codex `0.147.0` 在隔离的真实控制 PTY 中执行原生插件后，诊断脚本和未修改的随附 SessionStart / UserPromptSubmit 脚本均可通过 `/dev/tty` 输出完整 OSC 777。**不能将 bundle7 GUI 未收到富通知的原因归结为 Codex hook runner 主动剥离控制终端。** 本实验没有经过 InfiniShell GUI 接收链，因此也不能反向宣称该 GUI 缺陷已解决。

最终探针两阶段结果：

| 观测 | 实际结果 |
|---|---|
| `initialize` + `thread/start` 后、输入前等待 1 秒 | 没有完成的 hook；不是 hook 执行成功 |
| 提交固定受阻 `turn/start` 输入 | `model_input_submitted = true`，原生阻断状态为 `stopped` |
| 本机拒绝模型 provider 的 HTTP 请求 | `0`；未提供凭据，没有模型生成 |
| 完整原生 HookRunSummary | 两个 SessionStart、两个 UserPromptSubmit 对照为 `completed`；一个 UserPromptSubmit 阻断为 `stopped` |
| 四个诊断/对照进程的标准 fd | stdin、stdout、stderr 均非 TTY |
| 控制终端 | 四者均可打开 `/dev/tty`，与原生 app-server 同 SID、PGID，终端前台进程组一致 |
| 插件根目录 | `PLUGIN_ROOT` / `CLAUDE_PLUGIN_ROOT` 由 Codex 原生注入，等于实际安装缓存目录 |
| 未改参考脚本 | 两者退出码 `0`，捕获的 stdout/stderr 均为空；完整通知到达 PTY |
| PTY 字节 | 两条唯一诊断 OSC 与两条完整产品通知，共 `870` 字节 |
| 最终原生退出码 | `0` |

最终原始 PTY 字节 SHA-256：`a58b3b1dfb414e9a763b3ed8066a6799853ac07b46666e43881ac93292e11bc6`。它是本次运行的实际字节摘要，包含临时路径和随机标记，每次执行会变化，不用作未来运行的固定期望值。

脱敏记录：[codex-hook-transport-0.147.0-macos.json](fixtures/codex-hook-transport-0.147.0-macos.json)。保留完整 HookRunSummary、诊断环境白名单、原生 RPC、PID/PGID/SID、原始字节计数和摘要；路径已脱敏的字节文本明确命名 `pty_redacted_text`，不冒称原始字节。源树外原始报告包含实际 PTY 字节的 Base64。

## 隔离与真实性

探针在新建的 HOME / CODEX_HOME 中，使用真实 `plugin marketplace add` 与 `plugin add` 安装单独的 `transport-probe@infinishell-transport-probe`。参考插件逐文件匹配现有 `SOURCE_METADATA.json`，复制到测试插件内部后保持不变。诊断 hook 通过原生注入的 `PLUGIN_ROOT` 找到它并调用原来的 `on-session-start.sh` / `on-prompt-submit.sh`；没有从宿主伪造 `PLUGIN_ROOT`。

宿主只经原生 `config/value/write` 授权本测试刚生成且核对过的五个精确定义。没有读取或复制用户认证，没有写入 GUI 现场、用户 HOME、产品插件、信任或安装来源。测试清单增加了固定的 UserPromptSubmit 阻断脚本；本机模型端点对任何请求拒绝服务并计数，出现请求即失败。此路径验证 hook 和传输，不是正常模型回合或模型结果验收。

测试启动器只负责建立自己的控制 PTY，然后 `exec` 固定 Codex；标准流仍是 RPC 管道。独立的 PTY master 读取实际内核字节。CLI 路径和 fd 经 argv 传递，未拼接进 shell 源码。RPC 单次限时 15 秒、hook 限时 10 秒；结束时仅关闭或终止本探针持有的进程句柄，临时目录随后释放。

## 固定原生源码依据

依据官方提交 `be6e8eac029b183056b7e4402879f15d2c85f61b`：

- [`command_runner.rs:60–66`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/engine/command_runner.rs#L60) 捕获三个标准流，没有为 hook 调用 `setsid`、更换进程组或设置其他控制终端；本实验补齐了实际可写证据，不能仅由标准 fd 非 TTY 推导没有控制终端。
- [`discovery.rs:228–234`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/engine/discovery.rs#L228) 构造真实插件环境，runner 的 `command.envs(&handler.env)` 导出它。因此 `bash "$PLUGIN_ROOT/scripts/…"` 使用合法 shell 参数展开，不要求改为花括号占位符。
- [`schema.rs:85–98`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/schema.rs#L85) 的通用输出只有 `continue`、`stopReason`、`suppressOutput`、`systemMessage`，并禁止未知字段。该版本没有 `terminalSequence`；不能因为 Claude 官方支持该字段就移植给 Codex。`systemMessage` 是文本警告，不是原始 OSC 转发协议。
- [`session/turn.rs:233–238`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/session/turn.rs#L233) 先执行待处理 SessionStart，再处理输入 hook；UserPromptSubmit 阻断后提前返回。这解释了为什么仅创建线程没有在本次等待内运行 hook，也支持无模型传输的诊断路径。

`hook/completed` 是事件名称，真实状态可能是成功、失败或阻断。GUI 现场原有 SQLite 日志只记录事件名称；此探针直接读取原生事件的完整 `params.run`，没有用名称替代退出结果。

## 执行方式和剩余范围

依赖：macOS 或 Linux、Python 3.11+、Bash、jq、指定的 shell，以及明确 SHA-256 的 Codex 0.147.0 二进制。下面是本次 macOS 固定输入：

```sh
python3 script/cli-agent-parity/probe_codex_hook_transport.py --codex-executable /opt/homebrew/bin/codex --expected-codex-sha256 19c4f144c5226a9f17c58e6f0fa854843b0f77a6eb420f40e2745a12f10f5d37 --output /tmp/infinishell-codex-hook-transport.json
```

Linux 可指定本平台的固定二进制与已核验摘要；本次尚未执行 Linux。本脚本在 Windows 明确拒绝，不冒充 ConPTY 验收。没有验证 tmux/SSH、真实模型成功 Stop、审批允许/拒绝、应用重启恢复或 GUI 接收。

GUI 缺陷下一步应比较真实 hook 的执行结果及环境，并检查控制终端字节到应用解析器、dispatcher、当前 listener 的接线。现有证据不支持先改插件到新的 IPC 或不存在的 `terminalSequence` 接口。普通 PTY 的 Stop 即使可靠送达，仍须按独立 Stop 审计降级为 Unknown，传输成功不能变成任务成功证明。
