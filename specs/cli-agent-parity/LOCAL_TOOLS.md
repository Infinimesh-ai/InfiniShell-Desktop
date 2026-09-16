# 本地任务工具协议接入记录

记录日期：2026-09-16。此记录区分原生注册验证、协议转换测试与实际模型调用；注册成功不等于任务派发验收通过。

## 已取得的原生证据

| CLI | 无凭据验证 | 证据 | 尚未验证 |
| --- | --- | --- | --- |
| Codex CLI 0.147.0 | `initialize.capabilities.experimentalApi=true` 后，`thread/start.dynamicTools` 接受 `type=function` 定义并创建临时会话 | `fixtures/codex-0.147.0-local-tools-registration.ndjson` | 模型触发 `item/tool/call`、真实工具返回、调用中取消/重连 |
| Claude Code 2.1.273 | 独立 `--mcp-config` SDK server 经 `mcp_message` 完成 initialize、notifications/initialized、tools/list；`mcp_status` 返回 connected 与工具名 | `fixtures/claude-2.1.273-local-tools-registration.ndjson` | 模型触发 tools/call、实际权限允许/拒绝、调用中取消 |
| Grok Build 1.0.30 | 本轮未探测任务工具桥 | 继续保持现有能力门控 | SDK MCP 或 ACP MCP 的任务工具调用与审批 |

探测脚本为 `script/cli-agent-parity/probe_local_tools.py`，仅继承启动所需环境，使用临时 CLI 配置根，未读取或复制用户凭据，未提交模型回合。输出省略无关命令说明并复用现有脱敏器。

Codex 的本机 `app-server generate-json-schema --experimental` 明确包含 `ThreadStartParams.dynamicTools`；不带 experimental 时该属性被移除。原生服务端请求为 `item/tool/call`，参数包含 threadId、turnId、callId、tool 与 arguments，返回 `{contentItems, success}`。

Claude SDK 官方实现通过 `--mcp-config` 传入 `{type: sdk, name: ...}`；收到 `mcp_message` 时，宿主把 JSON-RPC 响应放在 `control_response.response.response.mcp_response`。参见 [官方传输实现](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/transport/subprocess_cli.py) 与 [官方控制协议实现](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/query.py)。本机注册夹具已验证这些字段形状。

## 最窄接入契约

纯转换与边界校验在 `app/src/ai/cli_agent_runtime/local_tools.rs`：

- `NativeLocalToolRequest` 保留原生 call/turn 标识、参数与内部回包目标。
- `TrustedLocalToolContext` 由协调器从当前连接、已提交 task/generation、权限快照及数据库父子关系生成；工具参数无法指定发送者身份。
- `bind_local_tool_call` 返回 `BoundLocalToolCall`，复用既有 RunAgents/SendMessage 参数转换，并产生按 task、generation、turn、call 稳定派生的消息 UUID。
- `LocalToolOperation::Inspect` 只读本任务或已记录父子任务的状态、回执和结果；执行层仍须重新检查数据库关系和当前 generation。
- `RespondLocalTool` 必须通过 adapter 中保存的 pending call 回包。宿主只能指定 call/turn，不能向原生进程传任意 JSON-RPC 响应目标。

原生 adapter 应缓存 call ID 对应的参数指纹及最终回包。同 ID 同参数重放仅返回缓存，不再次派发；同 ID 异参数拒绝；旧回合和过时代数回包拒绝。所有派发先持久化提交确认，再进入实际执行，消息只有原生接收确认后才标 Acknowledged。完成事件、连接关闭和任务换代不能把 Sent 倒推为失败。

子任务记录保存创建时的 `parent_generation`，这一字段不会随父任务继续而改变。旧子任务结果固定保存到原父代；父任务已经完成、断开、未知或继续到新一代时，仅保留记录供查询，不自动启动或投递。当前新指令仍允许父子任务按当前连接代数主动通信，显式查询可读取历史亲属记录。缺少父代的旧记录保持可读，不能推断补齐或自动派发。结果消息有 UTF-8 有界摘录、截断标志和 task/generation 定位，完整原文保留在任务历史。

新一代任务及第一条用户输入通过 `checkpoint_task_with_message` 在一个 SQLite 事务中提交。历史消息 ID 冲突、内容超限或任务检查点失败时，代数、上一代结果和已有消息全部回滚；只有事务提交后才返回确认，调用方随后才可进入原生传输。结果消息另用原子领取避免重复发送，传输回复丢失仍保留 Sent 未确认。

父 Oz 会话结果接入使用真实 `append_byop_preflight_messages_to_task` 源历史入口，随后等待既有会话检查点的 SQLite 提交确认，才标记 `receipt_kind=application_history`。这表示已保存到父会话，运行中会在下一次正常提供商请求中可见，不代表即时追加或模型已处理。CLI 真实回执标记 `native_protocol`。旧事件服务的 drain 方法没有实际调用者，因此桥不再使用该队列。

父 Oz 关联额外保存根任务 ID 和明确用户输入 exchange ID；action-result、结果消息和自动摘要等内部续流不会被当成新用户轮。领取前与同步写入前均校验这些标识。父 UI 进入终态或新的用户轮时，旧关联先降为 Disconnected；下一次明确派发才登记新代。缺少标识的旧关联只供读取，不推断当前轮。此降级不伪造外部 CLI 的原生完成证据；真实 GUI 消费与自动唤醒仍须后续验收。

不新增本地网络监听。仓库现有 `crates/ipc` 支持 Unix socket 与 Windows named pipe，但其通用 framing、反序列化与鉴权并非为此类不可信工具入口设计；复用 CLI 当前托管 stdio 回调可避免扩大攻击面和增加启动配置。

## 未完成项

- 本文纯转换与原生注册证据不代表 actor 派发、CLI 父子双向消息和结果回收已完成；以最终调用与持久化验收报告为准。
- BYOP 本地子任务保留 SkillReference 及来源，并只允许已启用本地目录中的技能；启动后台会有界重读与解析。托管 Codex 传原生 skill 输入。普通 PTY 与无法映射的远端/内置技能仍明确拒绝。
- 原生 Windows、Linux、SSH/tmux 与中英文布局尚不能由这些 macOS 无模型探测替代。

## Claude 技能补充证据与恢复

`probe_claude_skills.py` 在临时插件中创建完整 SKILL.md 与 references 资源，使用 `--plugin-dir` 注册。`claude-2.1.273-local-skill-registration.ndjson` 证明初始化命令清单包含命名空间技能。

`claude-2.1.273-local-skill-command.ndjson` 与 `claude-2.1.273-local-skill-command-args.ndjson` 进一步验证原生命令解析：`/namespace:name` 或 `/namespace:name 参数` 被回放为 command-message、command-name 和可选 command-args；中英文、多行、反引号、美元符号、尖括号和双引号完整保留。随后出现 authentication_failed，模型 token 与费用均为零，不能计为技能执行成功。

准备模块在 `runtime/local_skills.rs`。`SelectedLocalSkill` 只保存源路径与名称；`prepare_claude_skill_plugin` 每次连接重新复制完整技能资源目录至独立临时插件，返回值持有目录生命周期，退出时清理。它限制数量、总字节和嵌套深度，拒绝无法安全映射的符号链接及非普通文件。任务恢复使用保存的源引用重新准备，不复用旧临时路径。Claude adapter 已接入临时插件并核对初始化命令清单；每轮只转换一个已选技能及文本参数，回放严格匹配已验证的命令包装形状。多个技能明确拒绝，不丢弃原始请求。完整认证回合仍以最终验证报告为准。
