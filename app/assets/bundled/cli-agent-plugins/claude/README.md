# Claude 通知插件兼容修补

这是 `warpdotdev/claude-code-warp` 中 `warp` 插件的五个文件替换件，不是新的 marketplace 或独立可安装插件。支持的固定上游版本、提交和逐文件校验值在 `PATCH_METADATA.json`；替换目标为上游 2.2.0，2.1.0 仅作为升级前完整树预检来源。MIT 许可保存在 `LICENSE`。

修补透传 Claude 原生 `prompt_id`。该字段由官方文档规定从 Claude Code 2.1.196 起提供，当前核对的 CLI 为 2.1.273；它不是 Codex 的 `turn_id`。不生成伪造的事件 ID 或序号。

Stop 只读取当前 hook 的 `last_assistant_message`，不等待或扫描转录末尾；缺少关联 ID、没有最终文本或存在后台任务时降级为普通通知。Grok 兼容调用时通过实际 `GROK_HOOK_EVENT` / `GROK_SESSION_ID` 环境静默退出。其余 hooks 经 `build_payload` 自动携带原生关联 ID。

本地安装器核对受测完整插件树的 SHA-256，备份后原子替换 `scripts/` 与 `hooks/hooks.json` 中对应文件并保留可执行位，核对替换后的摘要。上游文件存在用户修改、版本不匹配或显式禁用时不自动覆盖或启用。`upstream.patch` 供审阅和在固定上游副本执行 `git apply --check`；此修补不改全局配置、不改 marketplace 注册，也不自行修改上游版本号。

仍依赖上游 Bash、jq；兼容 TTY 传输使用随附的 `warp-notify.sh`。父应用必须消费原生 `prompt_id` 并拒绝旧回合的终态；只有替换插件不能独立修复接收端。完整真实 hook 生命周期、Windows/Git Bash 与 GUI 验收仍待完成；SSH/tmux 仅脚本传输已验证，见下方范围。


安装器只支持受测 CLI 精确版本；Claude 最低插件版本为 2.2.0，Codex 为 0.4.0。缺失原生关联 ID 的 stop/stop_failure 会携带 `terminal_unverified=true`，不能算成功。详细失败恢复和缓存策略见 `specs/cli-agent-parity/PLUGIN_COMPATIBILITY.md`。

SSH 或容器需用 `script/cli-agent-parity/apply_notification_patch.py --export <目录>` 导出后整体传输到目标机器，由目标 Python 3.11+ 检查和应用；不会把本机安装状态当成远端状态。Windows 自动运行仍未开放，离线脚本的 `--files-only` 只验证与替换文件，不能证明 Bash/jq 或原生通知已可运行。跨平台实际验证未通过前不得视为能力已完成。

修补第 2 版将 `hooks/hooks.json` 纳入同一固定摘要事务，保留原 matcher 和事件集合。Claude 使用原生 exec form（`command: bash` + 单元素 `args`），Codex 在 shell 命令中引用已导出的 `$PLUGIN_ROOT`，不把目录值拼入命令文本。含空格、中文和 shell 特殊字符的路径已通过 macOS Bash/jq 回放；Claude 2.1.273 的原生 `--init-only` 也通过独立路径探测。Codex Windows 的原生默认 hook shell 可能是 `cmd.exe`，本 POSIX 命令不据此获得 Windows 支持；Windows/Git Bash 与真实通知生命周期仍未验证。

修补第 3 版将 `scripts/warp-notify.sh` 纳入固定摘要事务。Codex 与 Claude 兼容 TTY 路径在 tmux 中使用 DCS 封套并双写内部 ESC；Claude ≥ 2.1.141 的 `terminalSequence` JSON 继续只发送原始 OSC，由 CLI 自己处理终端。需要在目标 tmux 会话中显式允许 `allow-passthrough=on`；安装器不会修改全局 tmux 配置。macOS 隔离回环 SSH 已验证三方脚本直连、开启透传与关闭透传的正负路径；这不代表现代 Claude 原生通知、Windows 或完整 InfiniShell SSH/UI 已通过。详见 `specs/cli-agent-parity/SSH_TMUX_VERIFICATION.md`。
