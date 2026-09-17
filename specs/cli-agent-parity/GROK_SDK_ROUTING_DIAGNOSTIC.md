# Grok SDK 反向请求路由诊断

本候选用于区分独立 stdio 与 leader 路由对 SDK MCP 注册的影响。固定 Grok Build `1.0.30`、二进制 SHA-256 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。第7轮独立 stdio 实际收到一次 SDK `server/discover`，探针拒绝该标准方法后整轮失败。它证明本轮反向请求可达，不能计为 SDK、子任务或权限上限验收通过，也不能追认 SDK6 的 leader 路由假设为已证实根因。

## 已核对的真实参数与公开契约

仓库记录 [固定原生 CLI 的 help 证据](validation/grok-1.0.30-direct-agent-help-1.json)：`grok agent --help` 退出码为 `0`，模型输入为 `0`，`--no-leader` 定义为即使配置启用 leader 也启动独立 agent。证据归档保留原始字节与原 SHA，不由候选重新生成。

公开源码固定到 xAI 官方仓库提交 `482711333c7195dc16a272777f86086d615e2afb`。官方仓库当前没有版本 tag；该公开源码与固定二进制的编译来源尚未建立对应关系。以下源码用于设计可检验的对照，不能代替固定原生 CLI 的实测。

| 核对项 | 官方源码位置与结论 |
| --- | --- |
| SDK 注册 | [acp_mcp.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_mcp.rs#L1) 第 1–4、23–45 行说明 `session/new._meta["x.ai/mcp/servers"]` 接受 `{name, serverId}`；[servers.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/servers.rs#L346) 第 346–354 行通过 serde 固定 `serverId`。当前候选的字段拼写符合此契约。 |
| SDK 反向请求 | [acp_mcp.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_mcp.rs#L62) 第 62–80 行只序列化 `serverId` 与 `message`，没有 `sessionId`、`promptId` 或 `toolCallId`。 |
| SDK 能力声明 | [acp_agent.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs#L566) 第 566–568 行在 agent 的 initialize 响应 `_meta` 宣告 `x.ai/mcp/sdk=true`。客户端同名能力不能证明注册已实际执行。 |
| leader 请求路由 | [leader/server.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/leader/server.rs#L408) 第 408–423 行仅读取外层或扩展嵌套 `sessionId/session_id`；第 2177–2221 行依赖该身份分派 driver 请求；第 2283–2318 行对不可路由的非通知消息直接丢弃，只有通知可使用最后活动客户端。SDK 请求与此路由条件存在不匹配。 |
| 独立 stdio | [AgentArgs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-pager/src/app/cli.rs#L288) 第 288–292 行定义 `--no-leader`；第 1180–1195 行验证参数在 `stdio` 前可解析。[入口](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-pager-bin/src/main.rs#L1558) 第 1558–1562 行的直接 stdio 路径调用 `agent_command::run_stdio`。 |
| 工具名字 | [tool_name.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/tool_name.rs#L84) 允许名字片段中的连字符及单个 `__` 分隔符；`infinishell-sdk-origin-probe__inspect` 满足语法及长度限制。 |

静态扫描固定公共二进制同时找到 `--no-leader`（字节偏移 `115497339`）、不可路由消息丢弃日志（`116160200`）、通知回退日志（`116160256`）及 driver 路由日志（`116159743`）。这些字符串与上述源码结构一致，只能加强路由问题假设，不能证明其执行路径。

## 最小对照候选

普通根任务继续传入 `agent stdio --leader-socket <独立路径>`。只有设置 SDK 本地工具的准备分支，以及 `cfg(test)` 的 SDK 来源探针，使用精确 argv `agent --no-leader stdio`。现有生产 `validate_options` 继续拒绝 Grok 本地工具；本改动不开放产品入口。

SDK 专用包装器对共享包装器的完整 leader 分支做一次精确替换：删除该分支增加的 Unix socket 权限，仅接受 `--version` 或完整的 `agent --no-leader stdio`。其它参数次序、额外参数、旧 leader 参数均拒绝。计数必须是版本检测 `2` 次、直接 agent `1` 次，审计总数必须为 `3`。`native_launch_arguments_unchanged` 表示把经过校验的当前 argv 原样传递给固定二进制，不表示当前 argv 与旧 SDK6 相同。

保持一轮原生输入、固定只读 `inspect`、单次精确 SearchTool/UseTool 审批、项目禁止写入、官方认证复制及清理、固定网络预算、原生回执校验和来源门禁。旧 SDK6 的失败证据不得用新代码重算。现代 MCP 先核对 `server/discover`，旧版才使用 `initialize`；随后确认实际 `tools/list` 和 `tools/call`，再评估来源与权限。初始化成功不能直接宣布任务派发能力通过。

## 标准现代 discovery 的局部支持

[MCP 2026-07-28 Discovery](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/f56f204f6290f6531b14d5734eb3e0a10f0eb201/docs/specification/2026-07-28/server/discover.mdx) 使用逐请求元数据，取消该版本的 `initialize/initialized` 握手。SDK7 的公开三项 metadata key 散列分别精确匹配 `io.modelcontextprotocol/protocolVersion`、`clientInfo`、`clientCapabilities`；公开证据没有版本值，不能由字段名或二进制版本字符串推断值。下一候选从真实请求 `params._meta` 读取日期，只支持已核对的 `2026-07-28`，未知版本返回 `-32022`，缺失或错误类型返回 `-32602`。

官方规范与 [rmcp 3.2.0](https://github.com/modelcontextprotocol/rust-sdk/tree/51ccb42993d6eb5075399672ce7a0c21a0e55eea/crates/rmcp) 均要求逐请求 `protocolVersion` 和对象型 `clientCapabilities`；`clientInfo` 可选，出现时必须有字符串 `name/version`。discovery 返回 `supportedVersions/capabilities/resultType=complete/ttlMs=0/cacheScope=private`，身份放在 `result._meta["io.modelcontextprotocol/serverInfo"]`。现代工具列表也带结果类型和缓存字段，工具结果带 `resultType=complete`。`negotiatedProtocolVersion` 与 `servedToolNames` 只用于应用安全诊断，不是标准 discovery wire 字段。源码 URL、校验和及真实值未知边界记录在[源码夹具](fixtures/grok-1.0.30-modern-discovery-source-contract.json)。

discovery 不需要 session/prompt/tool 来源；真实 `tools/call` 的来源与权限检查继续保持独立。`discovery_count` 不计入 `initialization_count`，现代模式允许 `0 initialize + 1 discovery`，不能将现代模式伪装成旧握手。`negotiatedProtocolVersion` 在 discovery 阶段只记录本端接受的真实请求日期，后续列表与调用使用同一实际日期才确认对端继续使用，不能据此宣称完整握手或来源通过。每次现代请求都重新核对元数据，缺字段不能沿用前次 capability 或补版本。原生来源与父权限门禁继续关闭，旧失败记录保持原值。此局部协议支持已纳入 source21 和 source22 冻结输入并通过编译与定向离线回归，真实 CLI 验证仍待完成；连接内身份缓存与原生重建/fallback 的兼容边界也尚未实测。

## 请求归属与尚未验证的边界

[每会话初始化](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_session_impl/spawn.rs#L1120) 创建各自的 `McpState` 并设置 ACP 注册；[SDK 客户端](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/servers.rs#L577) 从注册中的名字、`serverId` 与 invoker 创建独立客户端。因此，独占进程/连接、预注册的唯一 `serverId`、运行代和 native session 绑定可以成为会话归属的验证基础，不必要求上游在每个 SDK 帧中重复会话字段。

这仍需真实验证取消后的迟到请求、继续时更换运行代、消息重投、同会话多回合及并发工具请求。回合和工具调用归属若无法由精确审批账本唯一确定，应拒绝产生本地副作用，不能把当前活动输入写成原生来源。此候选没有新增这种映射，也没有打开父子任务权限上限门禁。

本改动没有界面入口、用户可见文案或能力判断变化，**无需本地化变更**；提交前仍需按仓库要求运行 i18n 和 Cargo 门禁。离线 Python 回归只证明包装器参数与审计边界；本次真实直接 stdio、三平台、SSH/tmux 及同一提交验收均由后续验证报告记录。
