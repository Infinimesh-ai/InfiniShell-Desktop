# Grok SDK 业务请求来源映射只读复核

缺少 SDK payload 的 session/prompt/tool 三字段，不能直接解释为 App 请求主体未知或本地任务永远不可实现。独占 native 进程、其受控资源域和创建前预注册的唯一 serverId/generation，可以证明请求属于哪个 Infini App 任务能力域；它与“原生具体哪个回合、哪个工具调用”是两层事实。当前真实业务调用映射仍未观察到：SDK6 有实际完整 UseTool inspect/审批账本，但 SDK 请求为 0；SDK10 只有 discovery/list，outer/inner ID 分别为 0/0、1/1，没有 inspect/审批。两轮不能拼成一次实际关联通过。

当前 Grok 生产入口仍在 grok.rs:64–71 拒绝 local_tools。SDK 专用独占 direct argv 位于 120–130；grok_local_tools.rs:85–121 核对当前 serverId 与 RPC 指纹/重复身份，229–244 要求调用方提供已验证回合，同时仍拒绝没有父创建权限上限的 run_agents。253–257 的 grok-mcp call_id 是 App 对 serverId:innerRPCid 的 SHA 构造，不是原生 toolCallId，也不能反向证明其来源。当前生产 NativeTool 账本在 grok.rs:1502–1520 只保存 native turn/finished，没有名称和参数指纹；要落实本方案仍需以后授权扩充账本，不能宣称现成代码已安全开放。

原生工具来源本身有可靠已有路径：grok.rs:1383–1388 按实际事件 promptId 对已确认原生回合核对；1844–1874 核对 session、拒绝 replay、对 eventID/指纹去重；1502–1520 登记 toolCallId 所属回合，1021–1036 的审批关联同 session、同已登记未结束 toolCallId。SDK probe 的 1189–1205、1403–1499 核对实际 native session、ACK/Started、initial/完整输入及同 callID。其 final_input_seen 表示完整参数更新已出现，不能当作工具最终完成；工具 completed 是独立终态。已验证固定 inspect 的 initial rawInput 为精确 qualified tool_name/tool_input，完整更新增加 UseTool variant，因此直接比较整条 rawInput SHA 会因 variant 不同而失配。应依据已经核验的固定输入契约提取完整名称和 tool_input，与真正 SDK tools/call 的 name/arguments 独立计算指纹，绝不把模型入参中的身份声明当来源。

最小可检验路径是为实际 ToolCallStart/完整输入建立一个 call lease：它持有原生事件中已有的 S/P/T、完整 qualifiedname 和参数指纹，并已通过 session、原生 ACK/Started 与非 replay/事件身份核对。server/discover、initialize、tools/list、catalog 都是注册前序，不建立或消费业务 lease。真实 tools/call 到达时先核对独占传输、当前 capability/generation，再在未关闭的实际工具账本中按精确注册服务名称、工具名称和完整参数指纹找唯一匹配；0 个或多个均拒绝/明确未知，不能选“当前活动 prompt”。成功才将该 lease 一次绑定 serverId/generation/outerID/innerID 和请求指纹，verified_turn_id 来自这个实际事件账本。origins 六 carrier 的字段继续如实缺失，可另记“进程能力主体已验证、native 工具账本映射已验证”这类事实，不能填成 native payload 原有字段。

外层/内层 RPC ID 是请求回包与重复处理身份，绑定之后才能复用现有缓存和副作用去重。现有两个初始化 ID 恰好相等不能证明业务 ID 等于 native callID；本次已有公开缓存中没有 SDK transport ID 生成实现，之前路由文档所载 acp_mcp.rs:62–80 的 serverId/message 契约只能作为已归档来源说明，不能冒充本轮重新读取完整原始 SDK transport。native 固定 binary 与公开候选 SOURCE_REV 的完整映射仍未知。本轮不联网，也不新增这种未经证明的 ID 拼接规则。

单连接、单 prompt 本身仍存在反例：旧未绑定工具调用取消/超时后，同一连接新回合再次调用同名称同参数；旧 SDK 首次回调迟到时，可能错误匹配新回合的唯一 pending lease。不能靠永久禁止重复参数解决。取消、连接中断或未绑定 lease 到期须关闭旧 owned 进程，并使用同 S 的显式历史继续创建新 generation/serverId，旧 callback 不可能绑定新 capability。正常已绑定调用须在真实 native UseTool Completed、已 await SDK reply 之后关闭 lease；同 innerID 精确缓存重投不再执行副作用。若完成后的 native 后台 SDK closure/内部 agent 事件可见性尚未证明，可将每次 SDK epoch 结束均旋转旧进程加同 S 显式 continue，作为有限模式排除跨新回合首次迟到。

CLI 内部 subagent 可以代表同 App 进程任务主体，不必自动虚构为另一个 Infini 子任务；但共享 MCP client/server capability 是否有不同 native session/prompt、是否完整发出所有工具事件，本轮缓存范围未证明。该事实不影响已受控进程的 App principal，可能影响 native 回合/工具 lease 的唯一性；有冲突或无事件关联时不能伪称父原生 S/P/T。工具没有原生审批时，真实完整 ToolCallStart 账本仍可作为请求映射基础，不能要求上游发一个不存在的审批；App 工具权限与父创建上限仍分别执行，缺少审批也不是 allow-once 证明。run_agents 的父 ceiling 未证明，继续拒绝，不因只读来源映射、目录注册或模型权限声明而开放。

SDK11 的必要安全观察为：真实 server/generation 匹配、ACK/Started 哈希，完整 native 工具 S/P/T/event 哈希及 replay/type；从真实工具事件与真实 SDK tools/call 独立提取的 qualifiedname/arguments 指纹和相等布尔，实际 eligible lease 数量与一次绑定；outer/inner 事务指纹和时序，实际审批关联（若存在），回包后同原生工具终态及其他工具/session 身份哈希或计数。当前 probe 在 280–335、755–777 仍要求 SDK 某 carrier 原始完整 S/P/T 才把关系置 true；candidate_mapping_only 并未实现上述完整 lease/epoch 规则，不能改名算通过。若 SDK11 依旧止于维护响应的 invalid_response_id，只能验证维护形状，业务映射保持未观察，随后需新探针继续验证，旧失败不追认。

本次只读主树和显式公开缓存/安全 SDK6、SDK10 档案，没有读取 private body/auth/target，未运行网络、CLI、Cargo、模型、GUI 或测试，未修改 source31 七源、生产或测试。只有本说明和 [安全证据](validation/grok-sdk-business-origin-mapping-review.json) 新增；无需本地化变更。没有能力或 Goal PASS。

STOP；finished_reads_done=true。
