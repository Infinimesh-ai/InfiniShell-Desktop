# Codex 通知插件兼容修补

这里随附 `warpdotdev/codex-warp` 固定提交的完整 marketplace，以及 `warp` 0.4.0 的四个通知替换件。插件 ID 仍为 `warp@codex-warp`；同来源的 orchestration 文件及索引保持原样。`source/` 包含全部 36 个跟踪文件，`SOURCE_METADATA.json` 记录原始与部署摘要、模式位和固定提交。通知差异另见 `PATCH_METADATA.json` 与 `upstream.patch`，上游 MIT 许可保存在 `LICENSE` 及 `source/LICENSE`。

Codex `rust-v0.147.0` 官方 hook schema 在 UserPromptSubmit、Stop、PermissionRequest 和 PostToolUse 中明确声明原生 `turn_id`，此修补只透传该字段，不制造 Claude `prompt_id`、事件 ID 或序号。缺少关联字段的终态降级为普通通知；`stop_hook_active=true` 不报告完成，缺少最终文本也不声称成功。

本地安装器把完整来源发布到 CLI HOME 下的 `plugins/infinishell-sources/codex-warp-0.4.0-rev3/source`，在私有暂存 HOME 用真实 Codex 完成来源注册及安装，再验证缓存并迁入单一 marketplace 与目标插件启用字段。没有只修缓存的默认路径：Codex 的后台 Git marketplace 刷新会覆盖这种修补。原用户信任字段、无关配置、其他 marketplace、orchestration 的禁用及缓存均须保留。未知来源、完整源码或缓存的自定义修改、未经验证的版本、显式禁用会拒绝操作。

配置提交使用限定字段重读比较和原子文件替换，并非跨进程原子 CAS。发现并发目标变化时停止；失败恢复仅覆盖仍匹配本操作写入值的状态。无法安全恢复的旧缓存及阶段记录保留在 `plugins/infinishell-transactions/`，错误日志给出路径。原始 Git snapshot 不会被删除；新版本应随应用交付新的完整受控来源，不能直接执行原生 Git upgrade 覆盖当前来源。详细实证、恢复边界及原生 `expectedVersion` 的限制见 `specs/cli-agent-parity/CODEX_PLUGIN_CACHE_REFRESH.md`。

仍依赖上游 Bash、jq 和 `should-use-structured.sh`；TTY 传输使用随附的 `warp-notify.sh`。父应用必须保留并核对原生 `turn_id`，拒绝旧回合终态；本地安装器已接入受控修补；普通原生安装命令本身不会应用这些替换件。真实 hook 端到端、Windows/Git Bash 与 GUI 验收仍待完成；SSH/tmux 仅脚本传输已验证，见下方范围。


安装器只支持受测 CLI 精确版本；Claude 最低插件版本为 2.2.0，Codex 为 0.4.0。缺失原生关联 ID 的 stop/stop_failure 会携带 `terminal_unverified=true`，不能算成功。详细失败恢复和缓存策略见 `specs/cli-agent-parity/PLUGIN_COMPATIBILITY.md`。

SSH 或容器需用 `script/cli-agent-parity/apply_notification_patch.py --export <目录>` 导出后整体传输到目标机器。导出包包含完整来源与 `codex_persistent_source.py`；进入导出目录运行 `python3 apply_notification_patch.py --agent codex`，由目标 Python 3.11+、受测 CLI、Bash/jq 检查并安装或更新。只检查用 `--check`。不会把本机状态当成远端状态，失败不会降级为只修改缓存。Windows 自动运行仍未开放；显式 `--files-only` 的文件事务成功不能证明原生通知已运行。跨平台实际验证未通过前不得视为能力已完成。

修补第 2 版将 `hooks/hooks.json` 纳入同一固定摘要事务，保留原 matcher 和事件集合。Claude 使用原生 exec form（`command: bash` + 单元素 `args`），Codex 在 shell 命令中引用已导出的 `$PLUGIN_ROOT`，不把目录值拼入命令文本。含空格、中文和 shell 特殊字符的路径已通过 macOS Bash/jq 回放；Claude 2.1.273 的原生 `--init-only` 也通过独立路径探测。Codex Windows 的原生默认 hook shell 可能是 `cmd.exe`，本 POSIX 命令不据此获得 Windows 支持；Windows/Git Bash 与真实通知生命周期仍未验证。

修补第 3 版将 `scripts/warp-notify.sh` 纳入固定摘要事务。Codex 与 Claude 兼容 TTY 路径在 tmux 中使用 DCS 封套并双写内部 ESC；Claude ≥ 2.1.141 的 `terminalSequence` JSON 继续只发送原始 OSC，由 CLI 自己处理终端。需要在目标 tmux 会话中显式允许 `allow-passthrough=on`；安装器不会修改全局 tmux 配置。macOS 隔离回环 SSH 已验证三方脚本直连、开启透传与关闭透传的正负路径；这不代表现代 Claude 原生通知、Windows 或完整 InfiniShell SSH/UI 已通过。详见 `specs/cli-agent-parity/SSH_TMUX_VERIFICATION.md`。
