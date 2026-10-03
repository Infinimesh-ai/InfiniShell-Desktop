# InfiniShell Grok 状态插件

原生插件 `0.1.6` 仅转换已知 hook 为 InfiniShell v1 通知，不决定权限或执行工具。管理器的 CLI 精确支持集合为桌面平台上的 `1.0.30` 与 `1.0.41`，不把兼容性推广到其他版本。各平台安装、原生派发和传输证据按对应固定构建分别验收。

## 安装、升级与禁用

需要 Node.js 18 或更新版本，以及 InfiniShell 引导提供的绝对路径 `WARP_CLI_AGENT_NOTIFY_EXECUTABLE`。仅有 Node 和已安装插件不证明通知链路可用。缺少 worker、协议不兼容、终端不可写或写入超时均安静降级，普通原生终端继续工作。

```text
grok plugin validate <插件绝对目录>
grok plugin install --trust <插件绝对目录>
grok plugin disable infinishell-grok
grok plugin enable infinishell-grok
grok plugin uninstall infinishell-grok
```

真实 1.0.30 的本地插件更新不会可靠替换缓存，管理器继续使用已识别插件的备份、卸载、安装及失败恢复流程。0.1.0/0.1.1 的历史脚本与九项 hooks、0.1.2–0.1.5 的全部四个文件按已保存配方核对；用户改动、混合版本和未知旧版本不取得应用所有权。重复同名注册及已禁用插件不自动升级。

升级后在 Grok `/plugins` 的 **Plugins** 页按 `r` 重新加载，再检查 Hooks 页的当前版本与十项 hooks。仅有 `plugin list --json` 的 installed 状态不足以证明启用或发送成功。

## 身份与状态

- `WARP_CLI_AGENT_PROTOCOL_VERSION=1`、`GROK_HOOK_EVENT`、`GROK_SESSION_ID` 必须与载荷一致。兼容 camelCase/snake_case，冲突拒绝；任何 `subagentType` 或 `subagent_type` 字段出现均过滤，包括空值。
- 原生 session/prompt ID 必须为 1–256 字节的 ASCII 字母数字及 `-_.:`，不截断、不补造。实际存在的 `prompt_id` 保留到应用，由 EventCursor 处理重复、过时回调和新输入等待；时间戳不用于推进回合。
- `StopCancelled` 仅在有有效原生 prompt 且 reason 为 `user_interrupt`、`permission_rejected` 或 `permission_cancelled` 时表示取消。其余类别、缺失归属、SessionEnd 和精确 idle_prompt 只发 `notification + terminal_unverified`。
- Stop 仅提供候选响应，不能证明任务成功；StopFailure 是原生失败候选，仍由应用核对回合归属。Notification 仅在结构化 notificationType 为 `permission_prompt` 时表示等待审批；不按标题或 message 猜测。PermissionDenied 不表示等待审批，单个工具失败不表示整个任务失败。未知 reasonDetails/cancelTrigger 不进入通知。

## 原生通知 worker

Node 不创建持久状态或锁，也不直接打开、写入 TTY。一次调用 `<绝对路径> cli-agent-notify --require-protocol 1`，stdin 为单个紧凑 JSON。worker 在读取输入或打开终端前拒绝不兼容协议，成功写完后返回协议 1、最大帧 4096 字节的精确 JSON。使用 `execFileSync`，无 shell 拼接、旧写法回退或自动重投；发送期限和帧预算保持不变。

Node 按包含 tmux 包装的完整 OSC 帧预估 4096 字节上限；C1 和分号转义计入预算，超限时省略可选文本字段而不截断身份。最终大小、终端选择、并发写入及取消收尾由 worker 再严格核验。worker 成功且回复精确协议 JSON 仅表示写完终端，不是应用接收确认；hook stdout 永远不输出审批决定或模型正文。旧插件的独立协议查询及空 stdout 发送合同继续受支持。

The plugin sends each notification with one `cli-agent-notify --require-protocol 1` process. The worker rejects incompatible protocols before reading input or opening a terminal, then returns the exact protocol reply only after a successful write. Existing deadlines and frame limits remain unchanged. This confirms a terminal write, not application receipt. Legacy protocol queries and sends remain supported; the plugin does not retry automatically.

## 验证边界

安装或 inspect 成功不等于原生 hook 已派发，也不等于应用已接收。无凭据安装检查不宣称 SessionStart 或审批 UI 验收。Windows 的编码启动命令沿用相同通知 mapper 与 worker；真实 shell 参数与 CONOUT 传输须由对应平台验收。

原生 `permissionMode` / `permission_mode` 仅作为同一活动会话的权限观察值透传；缺失、冲突或变化会撤销旧证据，不改变审批规则，也不构成独立身份认证。

Native `permissionMode` / `permission_mode` is forwarded only as an observation for the same active session. Missing, conflicting, or changed values invalidate prior evidence; they neither change approval rules nor authenticate the session on their own.
