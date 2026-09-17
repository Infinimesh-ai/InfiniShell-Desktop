# Grok 本地任务工具的候选 SDK 契约与测试专用准备

受测官方 Grok Build `1.0.30 (04b7ffed98c6)` 的 ACP 握手已展示 `x.ai/mcp/sdk: true`。本文研究的公开源码快照提交为 `482711333c7195dc16a272777f86086d615e2afb`，其 `SOURCE_REV` 为 `be7ce6e8cffe46d20bef9834b211616082ee866b`，与受测二进制 revision 不对应。因此这里只记录公开候选源码契约，不能称为精确 1.0.30 原生源码。现有真实 CLI 夹具没有注册非空 SDK 服务，也没有收到 `x.ai/mcp/sdk_call`；请求 origin、权限提示和创建上限的实际二进制行为仍未验证，不能计作真实本地任务验收通过。

## 已核对的候选源码接口

核对公开候选源码提交为 `482711333c7195dc16a272777f86086d615e2afb`。键名称来自 [wire.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/wire.rs)，服务注册和反向载荷来自 [acp_mcp.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_mcp.rs)。

候选实现通过 `session/new` 或 `session/load` 的下面 `_meta` 注册服务；受测二进制的实际注册回路尚未验证。`serverId` 由应用生成，并且绑定本次监督进程的 `runtime_generation`；恢复时创建的新进程必须使用新 ID，不接受旧连接回调。

```json
{
  "_meta": {
    "x.ai/mcp/servers": [
      {"name": "infinishell-local-tasks", "serverId": "infinishell-<runtime_generation>"}
    ]
  }
}
```

候选实现向宿主发送反向 ACP 请求，`params.message` 是完整 MCP JSON-RPC 请求。ACP 外层 ID 和 MCP 内层 ID 不共享身份空间；回复的 ACP `result` 直接是完整 MCP JSON-RPC 回复，不再加一层 `message`。

```json
{
  "jsonrpc": "2.0", "id": "reverse-request-1", "method": "x.ai/mcp/sdk_call",
  "params": {
    "serverId": "infinishell-<runtime_generation>",
    "message": {
      "jsonrpc": "2.0", "id": 18, "method": "tools/call",
      "params": {"name": "inspect_local_tasks", "arguments": {}}
    }
  }
}
```

```json
{
  "jsonrpc": "2.0", "id": "reverse-request-1",
  "result": {
    "jsonrpc": "2.0", "id": 18,
    "result": {"isError": true, "content": [{"type": "text", "text": "本地工具已取消"}]}
  }
}
```

[acp_transport.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/acp_transport.rs) 将该实现定义为半双工请求/响应通道，丢弃没有 ID 的 MCP 通知。本转换器只声明 `tools`，不声称已经支持通知转发、sampling、roots 或 elicitation。[servers.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/servers.rs) 使用服务名和工具名构造原生工具命名空间；宿主仍按收到的 MCP `params.name` 匹配现有工具名称，不从命名空间猜测执行身份。

候选源码的 [Cargo.lock](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/Cargo.lock) 使用 `rmcp 3.2.0`，其 [默认协议版本](https://docs.rs/rmcp/3.2.0/src/rmcp/model.rs.html) 是 `2025-11-25`。转换器当前只接受这个版本；这是源码依赖证据，尚不足以证明已发布二进制实际发出的 SDK 初始化版本。若真实回路出现不同版本，应保留失败证据并补验证，不静默放宽。

## 纯转换器与接线要求

测试专用协议准备代码位于 `app/src/ai/cli_agent_runtime/grok_local_tools.rs`，仅通过 `local_tools.rs` 的 `#[cfg(test)]` 私有 `grok` 模块编译，没有生产重导出。`LocalToolReplyTarget::Grok` 及对应回复分支也限定为 `#[cfg(test)]`。28 项纯转换测试直接使用私有模块，共享回复回归也只在测试编译中存在；生产构建不包含这些 SDK 占位入口。转换器不派生进程、不改用户配置，也不自行操作 SQLite。下面的接口和接线要求描述后续原生验证的前提，不表示生产已经接入。

| 接口 | 调用方必须提供的可信信息 | 返回语义 |
| --- | --- | --- |
| `new(runtime_generation, permissions)` | 当前监督进程代数、创建任务时确定的消息工具许可 | 生成本连接唯一服务 ID；不授予 Grok 子任务派发许可 |
| `registration()` | 当前原生初始化已确认 SDK 能力 | 写入会话请求 `_meta["x.ai/mcp/servers"]` 的数组 |
| `receive(message, verified_turn_id)` | 调用方已通过原生证据证明该 SDK 请求所属、仍未关闭的回合 ID；不能使用当前活跃回合补缺失来源 | `Immediate` 是完整 ACP 回复；`Tool` 交给既有可信上下文绑定；`Duplicate` 不再次执行 |
| `reply(request, result, verified_turn_id)` | 原始工具请求快照、已证明所属且仍未关闭的同一原生回合 | 为等待同一 MCP 请求的所有 ACP 外层 ID 返回回复；不代表原生接收确认 |
| `cancel_turn(turn_id)` | 真实取消、回合终结或连接关闭的原生回合 ID | 保留取消身份，返回待取消的协调器 `call_id` |

接线时，应先确认固定 CLI 版本与 `x.ai/mcp/sdk`，再注册服务。适配器处理 `x.ai/mcp/sdk_call` 时，必须先通过真实协议证据证明 SDK 请求所属的原生回合。`serverId` 只证明监督进程注册身份；同一会话跨回合仍共享它，不能阻止旧工具请求在下一轮晚到。不得把当前活跃回合当作缺失来源标记的回退值。出现会话断开、关闭的回合、缺少可靠来源或不同进程代数时，拒绝执行并返回错误，不能从工具输入补回身份。`Tool` 继续使用 `bind_local_tool_call`、`TrustedLocalToolContext` 和现有 `LocalToolRequested` 路径；这里没有新增工具参数格式，也没有绕过父子地址范围检查。

完成工具后，适配器通过 `reply` 生成关联回复；写入 stdin 只证明应用派发，不能生成 `native_receipt: true`。取消、回合终结和断线都必须调用 `cancel_turn`，并为返回的 ID 发出 `LocalToolCancelled`。不能在 SDK 通道建立失败后回退到普通终端输入并声称工具完成。

同一 ACP ID 换内容、同一 MCP ID 换工具或参数都会被拒绝。相同 MCP 请求换 ACP 外层 ID 不再次执行，只关联等待回复；已完成的重复请求复用缓存回复。旧回合、取消后的请求不重新绑定新回合。账本最多保存 256 个外层身份和 256 个内层身份，单个输入与回复限制为 1 MiB，回复缓存合计限制为 4 MiB；达到限制后拒绝新执行，不删除身份后允许重投。新进程创建新服务身份，旧进程账本不继续授权执行。

## 权限、追加指令和结果回收仍需补齐

Grok 当前的审批配置观察不能证明子任务权限上限。`ask`、原生设置快照、一次工具允许以及父任务请求的 `allow_spawn` 都不能代替创建时固定并可验证的策略。因此转换器目前只提供 `inspect_local_tasks` 和获准的 `send_message_to_agent`；即使 `allow_spawn: true`，`run_agents` 也不出现在工具列表，伪造调用会收到关联 MCP 工具错误。后续只有新增并验证 Grok 创建策略、持久化上限和原生有效权限之后，才能解除这个限制。

独立审计时，`grok.rs` 尚未注册 SDK 服务或处理 SDK 工具请求，`coordinator.rs` 的启动入口尚未支持 Grok；`permissions.rs` 的固定原生策略与父上限验证也只支持 Claude/Codex。`run_agents` 的共享 harness 枚举、子任务解析和 CLI 派发解析同样需要增加 Grok。纯转换器交付不代表上述接线已经实现。尤其是 SDK 请求与原生工具调用、回合之间的可靠关联尚未验证：只有确定的 `toolCallId`、`promptId`，或经真实夹具证明唯一且先行的原生工具事件，才可能建立来源链；仅匹配名称和参数不能证明晚到请求属于当前回合。在没有这条来源链之前，生产适配器应降级返回关联错误，不能开放可能产生副作用的本地工具入口。

Grok 没有已验证的同一回合 `Steer` 接口。父子消息及运行中追加指令应明确进入下一轮队列，等待当前原生回合结束后用 `Submit` 接纳新回合；不得把排队、stdin 写入或 SDK 回复当作同一回合追加成功。UI 和持久化记录应区分排队、应用派发和真实原生接纳。

现有协调器在真实 `TurnFinished` 后持久化结果，按子任务代数生成稳定结果消息 ID，并固定创建时的父任务代数。主代理的新自动结果路径允许活跃 Claude 使用 `Submit`：存在 pending 槽时，结果保持 `Queued` 并延期；原生 `started` / `joined` 清槽后，仅重试尚未领取的结果。同 ID 重复不会新增槽，也不会重写原生输入。已经领取但未确认的消息仍不能自动重投，已完成父任务也不能仅凭保存的会话记录视为有原生端点。此路径当前处于 `official-5` 门禁阶段，真实接收 ACK 和最终结果仍待复验，不能计为通过。Grok 的下一轮消息策略和父任务稍后空闲时的结果回收仍需要明确接线与真实验证。不能用“父会话存在”代替结果已经由原生 CLI 接收，也不能通过重启历史会话重复执行来伪造恢复成功。

## 候选源码证据与未通过验收

以下是固定快照提交 `482711333c7195dc16a272777f86086d615e2afb` 的公开候选源码观察，其 `SOURCE_REV=be7ce6e8cffe46d20bef9834b211616082ee866b`，不对应受测官方 `1.0.30 (04b7ffed98c6)`。SHA-256 对应完整候选原始文件字节，行号来自同一快照；它们不是已发布 CLI 二进制的哈希，也不证明实际 1.0.30 原生会话已执行对应路径。契约夹具的 `evidence_limits` 分别记录源码观察、推断边界及 `native_verified: false`，`source_hashes` 保存完整路径、行范围和文件哈希。

1. **SDK 请求来源未通过。** 在候选源码中， `acp_mcp.rs:65–68` 的反向参数只有 `serverId`、`message`，`79–84` 创建并发送扩展请求。`servers.rs:1779–1780` 构造 MCP 工具参数时只填名称和 `arguments`，`2049–2053` 请求选项只指定超时；`gateway.rs:158–160` 表明标准扩展请求没有 `meta` 字段。这条候选源码调用链未展示已验证的 `toolCallId`、`promptId` 来源。实际 1.0.30 是否已有可靠 origin 字段尚未验证，不能由候选结构缺字段断言受测二进制没有 origin。同会话 `serverId` 跨回合复用，不能用当前活跃回合补缺失 origin，也不能把纯转换器的人工 `verified_turn_id` 当作原生证明。
2. **先行工具参数关联未通过。** 候选源码中， `tool_calls.rs:1397–1407` 先发带原生工具 ID、名称和早期 `rawInput` 的 Pending 事件；`1533–1536` 允许 PreToolUse 改写 `tool_input`、`raw_arguments`、`raw_input`；`2516–2531` 再发含更新输入的工具事件。因此早期指纹不保证等于最终 SDK 参数。真实 SDK 与 hooks 改写、并行调用、相同参数及跨回合晚到请求的唯一关联均未验证。仅看到当前唯一 Pending 请求不能授权本地副作用。
3. **SDK 取消传播未通过。** 候选源码中， `servers.rs:2038–2058` 的取消路径使用 rmcp 取消通知，`acp_transport.rs:127–139` 的半双工实现丢弃无 ID 通知。候选源码的这条路径不能保证宿主收到 `notifications/cancelled`；实际 1.0.30 是否采用同样转发行为，以及 SDK 取消和回合终结时序，均尚未验证。应用必须先建立可靠工具来源，再由真实回合终止或断线撤销关联调用，不能将取消按钮或回复写入视为清理成功。
4. **创建权限上限未通过。** 候选源码中， `acp_types.rs:625–629`、`resolution.rs:299–326` 的 `startupHints.permissionMode` 只接受 `alwaysAllow`，受显式 `defaultMode` 和 bypass pin 约束；候选实现忽略 `ask`、`default` 提示，且已驻留会话重新关联不重新应用这个提示；不能据此断言实际 1.0.30 的 ask/default 行为。`config_option.rs:18–46` 仅支持 `model`、`reasoning_effort`，没有权限或工具集选项。`session_setup.rs:343–353` 的 `yoloMode`、`autoMode` 可请求关闭，但不构成有效权限全集或 sandbox 证明。静态 `inspect` 来源列表、当前 ask 观察、未知 sandbox 不能授予父任务派发权限；尚无经过真实会话查询、重连和恢复核对的固定创建策略。
5. **兼容配置与继承边界未通过。** 候选源码中， `types/compat.rs:217–245` 提供 `[compat.claude]` 的 `mcps = false`、`hooks = false` 开关，`inspect/compat.rs:287–305` 有关闭 hooks 后配置来源的源码测试。另一方面，`resolution.rs:511–515` 独立依据 Claude import marker 决定权限配置 fallback；`agent_ops.rs:4372–4384` 的创建路径还独立加载 Claude env。候选实现中，关闭兼容 hooks/MCP 不足以排除 Claude permission `defaultMode` 或 env；实际 1.0.30 是否采用这些独立继承路径未验证，不能以候选源码代替兼容开关、全局与项目来源、权限和环境变量的原生联合验收。

生产结论保持为：**不接入 SDK 本地工具执行，`run_agents` 继续拒绝**。可靠原生关联和创建权限上限都需要后续验证。后续 `official-6` 冻结拟纳入这些测试专用候选协议准备与纯回归产物，但生产没有 SDK 模块、回复目标或接线入口，不能计为 P3 或副作用安全验收完成；这里没有新增 CLI 或模型请求。

本地没有 `SOURCE_REV` 全文文件。子任务核对公开源码树索引 `infinishell-grok-source-tree.json`：顶层 SHA 为快照提交，`SOURCE_REV` 的 Git blob SHA-1 为 `0c594fbe842905b9db14f523b7acaa5ebc9a853a`、大小为 41 字节；已知 revision 文本加 LF 的 blob SHA-1 与索引一致，根代理另通过 [官方原始文件](https://raw.githubusercontent.com/xai-org/grok-build/482711333c7195dc16a272777f86086d615e2afb/SOURCE_REV) 确认文本。子任务对本地受测报告仅投影 `cli_version`，得到 `grok 1.0.30 (04b7ffed98c6)`，没有读取身份文件或私有日志。

下表保存完整公开候选源码路径与哈希。文件名的简写仅用于上面的说明，证据定位以完整路径为准。

| 候选源码文件 | 核对行范围 | SHA-256 |
| --- | --- | --- |
| [crates/codegen/xai-grok-mcp/src/wire.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/wire.rs) | 依赖或协议键核对 | `45f9b7c46ad685da069043727afb238e456c00f3d39eea2f1bda9b9a72d7d571` |
| [crates/codegen/xai-grok-shell/src/session/acp_mcp.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_mcp.rs#L65) | 65–68、79–84 | `233912e891371a197c90b2b7de698d2c2d728cf84ca23e5a7e2490b378450995` |
| [crates/codegen/xai-grok-mcp/src/acp_transport.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/acp_transport.rs#L127) | 127–139 | `76faec6afccc78d6ff6dbafd66ba6a5ef3b58ae15b07df53d35b7e9e2f646b1d` |
| [crates/codegen/xai-grok-mcp/src/servers.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/servers.rs#L1779) | 1779–1780、2038–2058 | `38d5eda9917781ae188f3c73d9536e2fd103d381626a081b61516f2dd358027b` |
| [Cargo.lock](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/Cargo.lock) | 依赖或协议键核对 | `73088da69937bcc0bd8239dcc969cf4a703e836593cd5c7c35aa3a9cea393d0c` |
| [crates/codegen/xai-acp-lib/src/gateway.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-acp-lib/src/gateway.rs#L158) | 158–160、361–381 | `13d154a54d2285d65bd168c3616ec78cbef369e128083c85428ffafba6034153` |
| [crates/codegen/xai-grok-shell/src/session/acp_session_impl/tool_calls.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_session_impl/tool_calls.rs#L1397) | 1397–1407、1533–1536、2516–2531 | `5d470ccabfc29bed9f256c694e3865dcae349767e660cd9c2bd8a967dc1f071d` |
| [crates/codegen/xai-grok-shell/src/session/acp_types.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_types.rs#L625) | 625–629 | `3576669c4c17e300872efb3fb1d2bed05973d04ff14103bd306825f0d0f83e96` |
| [crates/codegen/xai-grok-workspace/src/permission/resolution.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-workspace/src/permission/resolution.rs#L299) | 299–326、511–515 | `28faac4ec1c8021c5eb7bad6009f3256bc9bfd5eb12e42019120c8f756b256a6` |
| [crates/codegen/xai-grok-shell/src/agent/handlers/config_option.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/handlers/config_option.rs#L18) | 18–46 | `073b73b6ef08c5012a36b23edd729c2e082388ba4f7b5b9ff2f5b5772dc7cde0` |
| [crates/codegen/xai-grok-tools/src/types/compat.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-tools/src/types/compat.rs#L217) | 217–245 | `72b6e89d06b61c253531e66d440d58887100704e03a51dd0cdd8715a1008ae02` |
| [crates/codegen/xai-grok-shell/src/inspect/compat.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/inspect/compat.rs#L287) | 287–305 | `dd36eb7a7bdf13c4ae42baba957b7be4db8a06c71f24d2978eb5ff664f180e20` |
| [crates/codegen/xai-grok-shell/src/agent/mvp_agent/agent_ops.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/agent_ops.rs#L4372) | 4372–4384 | `5c1d3182780e92724445fc08f6c78f6d008d115d4a59ab35cbf155c159fc26cd` |
| [crates/codegen/xai-grok-shell/src/agent/mvp_agent/session_setup.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/session_setup.rs#L343) | 343–353 | `de2d373a19bfac413d08f1195cf8a43313e0aa0731493dd5ef3ba0c32f8bdde1` |
| [SOURCE_REV](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/SOURCE_REV#L1) | 1（已知文本与本地 blob 索引核对） | `f42d40d90d620527204dd41d9a0bb4a585d69dc8525ad5e81633f33a83c7d6e4` |

`rmcp 3.2.0` [原始公开 crate 归档](https://crates.io/api/v1/crates/rmcp/3.2.0/download) 的 SHA-256 为 `42b6914fac0be956fe704a38239c3f44a9f841d1b06a5713d2f638065593f5b5`。归档中的 `rmcp-3.2.0/src/model.rs:175` 将默认最新协议定义为 `2025-11-25`，初始化构造在 `1011` 使用默认协议；此次在内存中解包核对，没有运行该依赖或 CLI。原包及固定 `Cargo.lock` 的依赖证据不等同于已发布 Grok 二进制实际 SDK 初始化帧。

## 验证记录

`fixtures/grok-1.0.30-local-tools-source-contract.json` 保存候选源码文件哈希和导出的双层请求结构，标记 `source_derived: true`、`native_cli_executed: false`、模型请求为零。它没有使用账户凭据、原生私有日志或模型正文。

新增 28 项纯 Grok Rust 回归覆盖注册代数隔离、双层 ID、未获准工具、可信回合绑定、父子任务范围、ID 和输入去重、取消后重投、过期回复、关联错误、大小及身份账本限制。测试中的回合来源由测试显式构造，不能视为真实 SDK 回调已具备副作用安全性。此子任务只做夹具 JSON 审计和 rustfmt 静态解析，未运行 Cargo；主代理仍需运行所属 Rust 测试、i18n 门禁及 `cargo check -p warp`，并对真实 SDK 注册及三款 CLI 父子任务闭环留存证据。

本转换器没有新增 GUI/TUI 控件或界面文案，工具定义复用既有协议描述，**无需本地化变更**。接线若改变工具入口、运行中追加提示或恢复界面，仍需同步英文与简体中文并完成双语布局检查。
