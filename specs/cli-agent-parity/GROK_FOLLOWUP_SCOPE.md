# Grok 连接门禁后的可实施范围

本记录只复核已有 Grok Build 1.0.30 原始证据；没有重新请求模型，也没有重试 402。产品中的托管任务入口继续关闭。以下项目是 adapter 内部的实现候选，不代表验收完成。

| 候选 | 已有真实证据 | 实施边界 |
| --- | --- | --- |
| 空历史 load / resume / close | `grok-1.0.30-empty-session-recovery.ndjson` 三次独立进程连接；new 后 load、resume 保持原 ID，close 返回 `_meta["x.ai/closeOutcome"] = "closed"` | load / resume 返回没有顶层 sessionId，应使用原请求 ID；只能标记空历史恢复契约已验，不能开放有运行中操作的产品恢复 |
| 原生队列的已收取、开始处理进度 | `grok-1.0.30-authenticated-quota-error.ndjson` 中 6 条 `_x.ai/queue/changed`；每轮依次 queued entry、runningPromptId、空队列 | queue entry `version: 0` 是该条目版本，不是全局事件序号；先限单个未完成 prompt，无法唯一关联时不确认消息，不靠文本哈希关联重复提示词 |
| 原生失败回收 | 两条 `_x.ai/session/prompt_complete` 的 stopReason 都是 `error`，promptId 与 runningPromptId 一致；随后原 RPC 返回 -32603 且 data.http_status = 402 | 失败通知和 RPC 错误属于同一轮，去重后只产生一次失败；空队列、working 状态消失、方法名 prompt_complete 都不能推导成功。未出现过的 stopReason 不映射 Completed |
| 能力与展示元数据更新 | 真实 available_commands_update、models/configOptions、`_x.ai/models/update` | 只更新实际收到的命令/模型信息，不从方法名称推断审批、取消或恢复能力；initialize 的声明与本版本实际通过项仍分开保存 |
| 扩展通知去重 | 部分 `_x.ai/session_notification` 含 `_meta.eventId` 与 agentTimestampMs | 仅在原事件确实携带 eventId 时按当前 native session 去重；时间戳不是全局顺序，不能为其他扩展合成序号 |

建议先用这两份 raw fixture 驱动确定性回归：重复队列事件、迟到的旧 prompt_complete、先通知后 RPC 错误、相同文本连续两轮、未知 stopReason、load 缺失 sessionId。实现内部解析并不改变 `RuntimeCapabilities.verified` 或应用托管入口，避免尚未受测的正常完成路径被开放。

以下项目仍没有成功的真实证据：模型正常两轮输出、审批允许/拒绝、运行中取消确认、追加指令确实改变执行、有真实历史的继续、崩溃后防重执行、最终结果成功回收。空闲 `session/cancel` 是无响应通知，只能证明请求被接受到传输层，不能证明运行中操作已经中止。`session/close` 关闭驻留会话，也不是任务成功或取消的替代确认。
