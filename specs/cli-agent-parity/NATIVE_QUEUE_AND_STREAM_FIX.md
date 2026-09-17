# 原生队列和流边界的失败证据

Grok 官方模型第 3 轮托管验收和 Claude 真实父子协调器第 1 轮验收均未通过。两轮使用 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 基线之上的 48 文件冻结快照；输入清单为 [macos-official-3-inputs.json](validation/macos-official-3-inputs.json)。这是开发中间快照，不能计入同一实际提交的正式跨平台验收。针对以下问题的修复及下一轮真实验收由主任务继续执行。

| 本轮固定产物 | SHA-256 |
| --- | --- |
| Rust libtest | `1484200b4959a7c498f7d02f6d7bace7ea11b9e16ba4f0f2876c80021b90c630` |
| 生产监督器 | `c39eb6998f21ead9eb73155f3bfaa15e81a7715568eda37ca4f6d8586953a723` |
| Grok 1.0.30 | `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb` |
| Claude Code 2.1.273 | `953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb` |

## Grok：思考和正文共用原生 chunk 序列

两轮官方 `grok-4.6-build` 正文分别真实完成为 `PARITY_ONE` 和 `PARITY_TWO`。第三轮输入得到原生接收确认，应用处理 `AllowOnce` 后，原生 `write` 工具报告执行成功，项目文件准确包含 `PARITY_APPROVAL`，没有换行。随后适配器断开，保留的原始错误为：

```text
CLI protocol error: Grok text stream arrived across an unresolved or closed boundary
```

这次清理错误没有覆盖原始协议错误。[native-events.json](validation/grok-official-adapter-3.native-events.json) 逐行提取了 191 条诊断，包括第一行带 libtest 前缀的记录，仅保存事件类型、协议 ID、流标识和计数。失败前的新 `streamStartMs=1789645228367` 中，`agent_thought_chunk` 的真实 `chunkId` 为 1–20，紧接着 `agent_message_chunk` 的 `chunkId` 为 21；没有 `chunkId=0`。两个通道共用原生计数序列。正文不能被解释为从独立编号开始的另一条连续流；只有按实际原生顺序核对后，才能判断正文缺片或响应边界。

[audit.json](validation/grok-official-adapter-3.audit.json) 同时保留以下限制：

- 第三轮没有最终完成事件。应用的审批处理记录带 `native_receipt=false`，文件写入和应用事件不能代替原生审批 ACK。
- 配置的运行前实际初始字节与运行后字节不同，`bytes_unchanged=false` 原样保留。严格 TOML 审计仅接受固定的原生 marketplace 初始化，权限部分保持相同；没有放宽其他配置变更。
- TLS 未解密，没有核对 HTTP 模型请求数或成本。拒绝来源共 70 次 `unknown`、3 次 `xai_api`，不能把未知来源解释为遥测。网络白名单仍为 `auth.x.ai` 和 `cli-chat-proxy.grok.com`。
- 生产监督器确认资源清理，退出原因 `stop_requested`、退出码 0。清理成功不表示模型任务成功。

公开 Grok 托管入口仍关闭。本轮不覆盖审批回合完成、拒绝、追加指令、取消、继续、应用重启恢复及最终结果回收。

## Grok 第 4 轮：工具前后正文累计导致严格夹具失败

第 4 轮使用 [macos-official-4-inputs.json](validation/macos-official-4-inputs.json) 的开发冻结输入，基线提交仍为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，工作区包含实际修改。固定 libtest 为 `7838e9dfea308a3bcafcc2198cf43f740e8461296f46d55a1530c592cab9188c`，生产监督器为 `091fe9364cec79601d539fe436d24d4a375b7928548cfde8d9e343e31fb61d4a`，Grok 1.0.30 二进制不变。这仍不能代替包含实际修改的正式同一提交跨平台验收。

两轮官方正文分别真实完成为 `PARITY_ONE` 和 `PARITY_TWO`。第三轮原生接收确认后，应用处理 `AllowOnce`，原生工具更新为 `completed`，第三轮任务真实结束为 `Completed`。实际文件 `approval-allow.txt` 为 15 字节，准确包含无换行的 `PARITY_APPROVAL`，SHA-256 为 `f798dce6c3641f8ef768c8f34eafc743ea5cf37c581fee64d69f79e689cdb2c9`。应用的审批结束记录仍为 `native_receipt=false`，这些成功事实不能代替单独的原生审批 ACK。

严格夹具要求累计回合正文准确等于 `APPROVED`，实际累计正文为 72 字节：获批工具前的 64 字节前言，以及工具后的 8 字节 `APPROVED`。原生持久化 history 的第 14 行 assistant 记录前言，第 17 行最后 assistant 准确为 `APPROVED`；两条正文的字节拼接与应用累计正文完全一致。最后正文流 `streamStartMs=1789647538995` 属于第三轮原生 `promptId=6c9aa468-0767-49ec-8ea8-19c81282ba73`，11 个思考分片后出现正文 `chunkId=12,13`，最终正文准确为 `APPROVED`。这些只是正文长度、匹配与散列证据；前言及思考正文均未进入归档。

[native-events.json](validation/grok-official-adapter-4.native-events.json) 保存了 191 帧协议 ID 投影，原生共出现 4 次 `response_completed`、3 次 `turn_completed` 和 3 次 prompt 完成通知。第三轮真实 `turn_completed` 的 `eventId` 后缀为 158；持久化 updates 最后还记录 `memory_session_saved`，最大后缀为 159。实际 history 和 watermark 已落盘，并不表示继续、重连或恢复已验证。本轮未执行审批拒绝、排队追加、取消、继续、应用重启、恢复及结果回收。

整条验收原始退出码为 101，`acceptance_passed=false` 原样保留。生产退出回执为 `exit_reason=host_disconnected`、退出码 0、`cleanup_confirmed=true`；macOS 原生等待状态为 0，作业及资源 CID 均确认清理。清理与原生第三轮完成不改变严格夹具失败的事实。配置 `bytes_unchanged=false`，严格审计仅接受原生 marketplace 初始化，权限部分不变。网络实际有 9 次官方 TLS CONNECT，拒绝来源为 82 次 `unknown`、3 次 `xai_api`；未解密 HTTP，不能据此核对模型请求数、成本或把未知来源统称遥测。

本轮 [公开事件](validation/grok-official-adapter-4.ndjson)、[metadata](validation/grok-official-adapter-4.metadata.json)、[network](validation/grok-official-adapter-4.network.json)、[退出回执](validation/grok-official-adapter-4.exit-receipt.json)、[macOS 清理](validation/grok-official-adapter-4.macos-cleanup.json) 及 [audit](validation/grok-official-adapter-4.audit.json) 保留原始公开文件散列、安全状态投影及未执行范围。第 3 轮的流边界失败记录保持不变；第 4 轮只证明上述三轮的实际完成路径和累计正文事实，公开托管入口仍关闭。

## Grok 第 5 轮：以原生最终 history 验证完整回执

[第 5 轮公开事件](validation/grok-official-adapter-5.ndjson)记录固定 8 输入生产监督链通过，libtest 退出 0。输入清单为 [macos-official-8-inputs.json](validation/macos-official-8-inputs.json)，libtest SHA-256 为 `8decb11d70665c86e60f4e3039702b80f218718400c19ff4c5c5258575984b40`，生产监督器为 `6e1801d768bdcb3e4ae9e2de2dba2137386631294baa79c2e262efc29f8abb78`。固定 Grok 1.0.30 与官方模型不变；基线提交和 dirty 工作区状态仍属于开发冻结快照。第 4 轮全部失败产物及上述失败段落保持原字节，不按新夹具追认通过。

8 个输入全部有原生接收确认、独立原生回合 ID、运行及真实终态：两轮文本完成，精确 Write 允许后完成，Write 拒绝后取消，正在输出时追加的下一轮输入与原回合分别完成，正文开始后取消取得真实取消终态，新进程继续历史原生 session 后回收此前排队标记。共 6 次 `Completed`、2 次 `Cancelled`。正在运行时追加输入只验证了下一轮队列；没有验证同轮 steer。

每个终态均取自生产已验证的最终 history，并绑定原生 session、turn 和真实完成 watermark。允许回合完整累计正文仍为 71 字节，最后正文响应为 8 字节且准确等于 `APPROVED`；两份长度与 SHA-256 分别保留，产品完整正文没有被裁掉。夹具以最后正文响应精确比较固定标记，既不以 `endswith` 猜测完成，也不将工具前言当作最后回复。[回执投影](validation/grok-official-adapter-5.native-receipts.json)包含 8 组真实完成水位，后缀依次为 55、91、184、250、892、942、973、1038；独立归档对事件中的完整正文和最后回复长度及散列逐一复核，没有归档正文或读取原生思考日志。

两个生产运行代依次退出并确认清理；第二进程使用不同的独占 leader socket，继续相同原生 session `01a0af68-fcbe-70f3-a31a-ce6c477ceb03`。继续流程取得新原生输入、回合、完成水位及与先前排队结果完全相同的最后回复散列。这证明新进程继续原生历史会话，不能计为重新关联仍活跃的任务或桌面应用重启恢复。[退出回执](validation/grok-official-adapter-5.exit-receipts.json)与 [macOS 清理](validation/grok-official-adapter-5.macos-cleanups.json)确认两次 `stdio_closed`、退出码 0、清理完成、原生等待状态 0。

[audit](validation/grok-official-adapter-5.audit.json)保留审批处理 `native_receipt=false` 的限制，真实文件效果和终态不代替独立审批 ACK。[原始 metadata](validation/grok-official-adapter-5.metadata.json)明确 `public_product_gate_open=false`、`app_restart_and_ui_verified=false`、`worktree_dirty=true`。[网络](validation/grok-official-adapter-5.network.json)有 15 次官方 TLS CONNECT、190 次未知来源及 7 次 xAI API 来源拒绝，未观察 HTTP 模型请求数或成本。独立 SDK 来源探针、父权限上限、SQLite 父子协调器、GUI、应用重启、跨平台及 SSH/tmux 均不属于本轮通过范围，完整 Goal 和产品门禁保持未完成。

## Claude：原生接收确认不等于独立新回合

真实生产协调器派发了一个 Claude 子任务。父子关系、固定文件权限 profile 和子任务权限上限落盘，父子 profile 完全相同。父任务真实调用 `run_agents` 和 `send_message_to_agent`；子任务 Edit 正在等待审批时，父到子追加指令已在 SQLite 中记录为 `acknowledged` / `native_protocol`。该接收确认发生在子 Edit 允许之前。

随后子 Edit 得到 `AllowOnce`，原生会话记录了对应工具结果，文件内容与此次获批的 `new_string` 完全一致。原生追加指令生命周期也出现 `queued` 和 `started`，但原有回合还没有 `result`。协调器把这次 `started` 按独立新回合解释，转入 `queue-uncertain` 并关闭运行时。父任务发送工具最终返回 `is_error=true`；SQLite 中的原生接收确认仍然存在。这两个事实必须同时保留，不能把发送工具整体标为成功或将消息重发。

Claude 工具回合期间的队列输入可由原生处理合入当前工作，适配器需要保留实际 command 和 `user_message_uuids` 关联，区分接收、开始处理、合并和真实结果，不能只凭 `started` 推进独立回合编号。修复后的行为仍需原生结果和 SQLite 状态共同证明；本轮没有验证修复通过。

[紧凑事件记录](validation/claude-coordinator-live-1.ndjson) 去除了重复的 `config_json`、`permissionObservation`、模型输入和消息正文；[audit.json](validation/claude-coordinator-live-1.audit.json) 保留原始公开记录 SHA-256、父子关系、profile、审批、消息接收状态和最终 SQLite 投影。[原生 ID 记录](validation/claude-coordinator-live-1.native-events.json) 保存 152 帧诊断的类型计数及 24 帧相关协议 ID，还保留两份原生会话日志的哈希和工具结果状态。原生诊断中 `result` 为 0，最终 SQLite 中两项任务均为 `disconnected`，`result=null`。

两个生产运行代的 macOS 资源清理均得到确认，但退出码为空、原生等待状态为 9，不能记为进程正常完成或任务成功。本轮没有子到父进度 ACK、四个真实回合结果或最终结果回收，也没有验证真实 GUI、应用重启及自动最终结果投递 ACK。

## 归档与后续门槛

原始私有工作区不进入仓库。公开记录只保存安全投影和原始文件哈希，不包含思考正文、OAuth、API 密钥、监督器 token 或身份信息。公开 JSON 与 NDJSON 完成 API 密钥、JWT、Bearer、私钥、凭据赋值及邮箱形态扫描，候选计数均为 0。监督器 manifest 只保存 SHA-256，不归档含 token 的正文。

主任务需在修复后重新运行真实 Grok 官方流程和 Claude 父子协调器流程，保留失败历史；再将包含实际修改的提交用于正式平台门禁。本次仅增加验证记录和说明，无需本地化变更。
