# Codex 通知插件兼容修补

这里随附 `warpdotdev/codex-warp` 固定提交的完整 marketplace，以及 `warp` 0.4.0 的五个通知替换件。插件 ID 仍为 `warp@codex-warp`；同来源的 orchestration 文件及索引保持原样。`source/` 包含全部 36 个跟踪文件，`SOURCE_METADATA.json` 记录原始与部署摘要、模式位和固定提交。通知差异另见 `PATCH_METADATA.json` 与 `upstream.patch`，上游 MIT 许可保存在 `LICENSE` 及 `source/LICENSE`。

Codex `rust-v0.147.0` 官方 hook schema 在 UserPromptSubmit、Stop、PermissionRequest 和 PostToolUse 中明确声明原生 `turn_id`，此修补只透传该字段，不制造 Claude `prompt_id`、事件 ID 或序号。缺少关联字段的终态降级为普通通知；`stop_hook_active=true` 不报告完成，缺少最终文本也不声称成功。

本地安装器把完整来源发布到 CLI HOME 下的 `plugins/infinishell-sources/codex-warp-0.4.0-rev4/source`，在私有暂存 HOME 用真实 Codex 完成来源注册及安装，再验证缓存并迁入单一 marketplace 与目标插件启用字段。没有只修缓存的默认路径：Codex 的后台 Git marketplace 刷新会覆盖这种修补。原用户信任字段、无关配置、其他 marketplace、orchestration 的禁用及缓存均须保留。未知来源、完整源码或缓存的自定义修改、未经验证的版本、显式禁用会拒绝操作。

配置提交使用限定字段重读比较和原子文件替换，并非跨进程原子 CAS。发现并发目标变化时停止；失败恢复仅覆盖仍匹配本操作写入值的状态。无法安全恢复的旧缓存及阶段记录保留在 `plugins/infinishell-transactions/`，错误日志给出路径。原始 Git snapshot 不会被删除；新版本应随应用交付新的完整受控来源，不能直接执行原生 Git upgrade 覆盖当前来源。详细实证、恢复边界及原生 `expectedVersion` 的限制见 `specs/cli-agent-parity/CODEX_PLUGIN_CACHE_REFRESH.md`。

仍依赖上游 Bash、jq 和 `should-use-structured.sh`；TTY 传输使用随附的 `warp-notify.sh`。父应用必须保留并核对原生 `turn_id`，拒绝旧回合终态；本地安装器已接入受控修补；普通原生安装命令本身不会应用这些替换件。真实 hook 端到端、Windows/Git Bash 与 GUI 验收仍待完成；SSH/tmux 仅脚本传输已验证，见下方范围。


安装器只支持受测 CLI 精确版本；Claude 最低插件版本为 2.2.0，Codex 为 0.4.0。缺失原生关联 ID 的 stop/stop_failure 会携带 `terminal_unverified=true`，不能算成功。详细失败恢复和缓存策略见 `specs/cli-agent-parity/PLUGIN_COMPATIBILITY.md`。

SSH 或容器需用 `script/cli-agent-parity/apply_notification_patch.py --export <目录>` 导出后整体传输到目标机器。导出包包含完整来源与 `codex_persistent_source.py`；进入导出目录运行 `python3 apply_notification_patch.py --agent codex`，由目标 Python 3.11+、受测 CLI、Bash/jq 检查并安装或更新。只检查用 `--check`。不会把本机状态当成远端状态，失败不会降级为只修改缓存。Windows 自动运行仍未开放；显式 `--files-only` 的文件事务成功不能证明原生通知已运行。跨平台实际验证未通过前不得视为能力已完成。

修补第 2 版将 `hooks/hooks.json` 纳入同一固定摘要事务，保留原 matcher 和事件集合。Claude 使用原生 exec form（`command: bash` + 单元素 `args`），Codex 在 shell 命令中引用已导出的 `$PLUGIN_ROOT`，不把目录值拼入命令文本。含空格、中文和 shell 特殊字符的路径已通过 macOS Bash/jq 回放；Claude 2.1.273 的原生 `--init-only` 也通过独立路径探测。Codex Windows 的原生默认 hook shell 可能是 `cmd.exe`，本 POSIX 命令不据此获得 Windows 支持；Windows/Git Bash 与真实通知生命周期仍未验证。

修补第 3 版将 `scripts/warp-notify.sh` 纳入固定摘要事务。Codex 与 Claude 兼容 TTY 路径在 tmux 中使用 DCS 封套并双写内部 ESC；Claude ≥ 2.1.141 的 `terminalSequence` JSON 继续只发送原始 OSC，由 CLI 自己处理终端。需要在目标 tmux 会话中显式允许 `allow-passthrough=on`；安装器不会修改全局 tmux 配置。macOS 隔离回环 SSH 已验证三方脚本直连、开启透传与关闭透传的正负路径；这不代表现代 Claude 原生通知、Windows 或完整 InfiniShell SSH/UI 已通过。详见 `specs/cli-agent-parity/SSH_TMUX_VERIFICATION.md`。

修补第 4 版为五个既有事件保留 POSIX `command`，并加入从仓库 `script/cli-agent-parity/codex_windows_hook_command.ps1` 确定生成的 `commandWindows`。`warp-notify.sh` 在 Git Bash 中使用从 `codex_windows_notify.ps1` 编码的 `CONOUT$` / `WriteConsoleW` 通道。新增 `on-prompt-submit.sh` 替换件，仅 Windows 分支为 jq 开启 `--binary`，逐字保留 LF/CRLF；Unix 继续使用既有 `jq -r`，不要求实现 Windows 专用选项。未改动其他 CLI 的资源或平台门控。

rev3 迁移按 `revisions/rev3/SOURCE_METADATA.json` 的全部 36 个文件、固定路径、元数据原文和模式位识别旧不可变来源，并按完整 10 文件树识别通知缓存。单独改版本号、伪造清单、混合不同修补版本或用户自定义脚本均不构成受控迁移来源。升级发布新 rev4 目录，旧 rev3 来源不变；升级后的旧缓存及 `state.json` 也保留在事务目录。回滚只恢复仍匹配此次事务的缓存和受影响配置，保留用户信任、显式禁用和无关配置。

rev4 的 macOS 无凭据原生 `hooks/list` 已采集为 `NATIVE_HOOK_TRUST.json`：仅五个正式 hook，无测试阻断 hook、无插桩 wrapper、无输入回合及模型调用。这只证明注册与原生摘要。Windows 生产自动安装仍关闭，授权状态保持 Unknown；没有捏造 Windows 原生可信摘要。第九轮临时插桩候选即使通过，也不能替代正式五 hook 的原生注册表与 ConPTY 验证。

固定 macOS Codex 0.147.0 还通过 Python 安装器的无凭据来源迁移：起点是精确 rev3 受控夹具，暂存调用真实原生插件命令，结果保留旧树、信任和编排禁用状态；见 `specs/cli-agent-parity/fixtures/codex-plugin-rev3-rev4-macos.json`。此记录明确不证明 Rust 安装器、GUI 或 Windows 已通过。候选运输脚本生成的末尾空行在正式资源中收敛为一个 LF；所有正式文件使用新摘要，后续实际通知验收必须以正式字节为准。

Windows 原生探针 `probe_codex_windows_hooks.py` 与 `probe_codex_windows_conpty.py` 默认 `--mode formal`，直接使用本版完整资源，不再二次应用候选变换。显式 `--mode candidate` 从 `revisions/rev3/` 的固定原文重建旧插桩候选；两种模式须使用不同输出文件，旧候选通过不能替代正式直验。正式采集分为两个独立私有安装：先从未改动的五项资源取得原生注册表并核对未信任、授权后确认与配置回滚，不创建会话；再仅给临时 `hooks.json` 增加独立第六项阻断 hook，脚本字节不变且不注入 Bash observer。正式五项注册必须逐项一致，原生阻断完成之前发生任何模型请求都会失败。

输出使用 `schema_version: 2`，关键证据如下：

- `mode` 区分正式与候选；`cases[].formal_registration` 保留未插桩资源的完整文件摘要、`source_metadata_sha256`、五项 `untrusted_hooks` / `trusted_hooks`、原生 trace 及精确配置回滚。只有这里的 `currentHash` 才能作为正式 Windows 摘要采集来源；未执行前不填充任何假摘要。
- `cases[].trigger_validation` 将五项 `formal_hooks` 与唯一 `test_only_blocker` 分开。原生事件完成只证明 `sessionStart` / `userPromptSubmit` 及测试阻断；`stop`、`permissionRequest`、`postToolUse` 明确列为未验证。它不是普通用户的允许/拒绝审批测试。
- `cases[].transport_expectations` 来自 RPC 请求的原始工作目录、LF/CRLF 文本与响应的原生会话/回合 ID，不使用 wrapper marker 或归一化。ConPTY 的外层报告另存两项实际 OSC、原始 PTY 字节与摘要；hook 完成和合成 console 标记都不能替代通知到达。
- `process_closes` 要求 app-server 正常退出和双输出 EOF，`config_rollback` 要求恢复授权前的精确字节。成功才清理私有 CLI case；异常配置、权限和文件占用保留错误及剩余现场，不自动解除只读。ConPTY 还需确认 console 关闭和输出 EOF；承载已加载 DLL 的宿主根目录保留给调用方收尾。这些回执不声称 Job 子孙进程收尾已验证。

现有唯一跨平台 workflow 无需改参数即可执行正式模式；ConPTY driver 总等待上限为 480 秒，单次 hook 完成等待 45 秒，超时仍判失败。合入 Windows 支持前仍需原生采集通过，保留真实允许/拒绝证据，实际验证剩余通知事件、进程与 Job 清理，以及 rev3→rev4 安装、重复更新、失败恢复和后台刷新。随后才能分别调整 Codex 平台门控与双语安装说明；Claude/Grok 必须独立验收。所有相关平台门禁须基于同一包含正式改动的提交，完整生命周期仍单独验收。
