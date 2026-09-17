# Grok 原生 SDK 来源探针

生产 SDK 本地工具和子任务门禁保持关闭。官方生产监督链第 5 轮的固定 8 输入流程已通过，但不覆盖 SDK 注册、请求来源或创建时权限上限；见 [官方验收](GROK_OFFICIAL_ONLINE.md)。本文件单独记录固定 `grok 1.0.30 (04b7ffed98c6)` 的 SDK 来源探针真实结果。

## 第 1 轮：网络预算耗尽后探针超时，来源仍未知

[原始公开事件](validation/grok-sdk-origin-probe-1.ndjson)、[原始 metadata](validation/grok-sdk-origin-probe-1.metadata.json) 和 [原始 network](validation/grok-sdk-origin-probe-1.network.json) 经安全扫描后按原字节归档。独立 runner 返回 1，内部 libtest 退出 101，`probe_passed=false`、`origin_verification=unknown`、`native_origin_verified=false`、`public_product_gate_open=false`。它使用与官方第 5 轮相同的 source8 开发冻结二进制：libtest SHA-256 为 `8decb11d70665c86e60f4e3039702b80f218718400c19ff4c5c5258575984b40`，生产监督器为 `6e1801d768bdcb3e4ae9e2de2dba2137386631294baa79c2e262efc29f8abb78`。这不是包含实际修改的正式提交跨平台验收。

唯一输入得到真实原生接收确认，原生 session 为 `01a0af68-fdc6-7b92-9918-4d98e45b1ee0`，prompt 为 `9d1a087e-2fd5-430f-a453-ecc8fce96c42`。180 秒夹具总时限耗尽，SDK 初始化、工具列表、反向 SDK 请求、inspect 调用、审批、意外工具全部为零。没有取得 SDK 工具回执或模型最终结果；来源状态保持未知，不能由零请求判断 SDK 缺少 origin、必须审批或不被支持。

本轮 8 次官方 TLS CONNECT 已耗尽预算，其后实际有 12 次 `official_connect_budget_rejected`，另拒绝 38 次未知来源和 5 次 xAI API 来源。TLS 未解密，8 次连接不等于模型请求数，也不构成 HTTP 或费用上限。网络预算对真实请求形成干扰，不能把超时单独归因于协议注册或权限。

[安全协议投影](validation/grok-sdk-origin-probe-1.native-events.json)从固定日志前缀提取 27 帧，仅保存方法、协议 ID 与计数。原生出现 10 次 `retry_state`，以及 `mcp/init_progress`、`mcp_initialized`、`mcp/server_status` 各一次。现有安全 trace 没有这些通知的完整注册状态，不能据方法名断言目标 SDK 服务已注册成功。没有导出日志、模型正文、思考、认证或配置原文。

失败终态仍保留 `no_project_files=true`、`no_side_effects=false`。空项目和零审批不把未完成探针变为通过。[退出回执](validation/grok-sdk-origin-probe-1.exit-receipt.json)确认 `stdio_closed`、退出码 0、`cleanup_confirmed=true`；[macOS 清理](validation/grok-sdk-origin-probe-1.macos-cleanup.json)确认作业及资源 CID 清理，原生等待状态为 0。生产连接、隧道关闭和私有认证副本删除均确认；清理成功不等于 SDK 工具或来源验证成功。[audit](validation/grok-sdk-origin-probe-1.audit.json)保留这些失败边界。

## 载荷比对的证据边界

当前测试在 ACP initialize 的 `clientCapabilities._meta["x.ai/mcp/sdk"]` 写入 `true`，在 `session/new._meta["x.ai/mcp/servers"]` 写入仅一个 `{name, serverId}` 的数组；server ID 由本次运行 generation 派生。后者与 [已归档候选契约](fixtures/grok-1.0.30-local-tools-source-contract.json)的注册键与结构同形，SDK 能力键名也一致。

候选源码提交为 `482711333c7195dc16a272777f86086d615e2afb`，`SOURCE_REV=be7ce6e8cffe46d20bef9834b211616082ee866b`，不对应受测二进制的 `04b7ffed98c6`。此次独立从官方公开 raw URL 读取完整候选文件并在内存核对：`acp_agent.rs` 第528–568行在 initialize **响应**顶层 `_meta` 宣告 SDK 能力；完整文件没有 `client_capabilities` 引用。客户端能力中的同名键是当前探针的额外字段，候选源码不能证明它必需、有害或有效。

[既有固定原生初始化证据](validation/macos-grok-fixed-acp-1.json)也实际观察到 `initialize.result._meta["x.ai/mcp/sdk"]=true`；独立核对的原始产物 SHA-256 为 `814c97c02c9a0d5fad4d83bc7e8abc7039b42e0fd0e0143dc9381e630f0457a6`。这证明固定二进制宣告该能力，仍不证明本轮目标 SDK 服务成功注册。

候选 `wire.rs` 第15行使用不带下划线的 SDK 方法常量，`acp_mcp.rs` 第81行将其交给 `ExtRequest`。候选 `Cargo.lock` 精确固定 ACP 0.10.4，原包 SHA 与 lock checksum 相符；该依赖 `src/lib.rs` 第527–534行在反向 ext 方法序列化时添加首个下划线，第285–289行在客户端解码时去掉它。因此候选契约的实际 JSON-RPC wire 方法为 `_x.ai/mcp/sdk_call`。SDK第1轮的全部27帧安全 trace 中，两种 SDK 方法均未出现；观察到的原生 MCP 初始化和状态通知方法确实带 `_x.ai/` 前缀。不能把候选序列化规则当作受测二进制 SDK 请求的实证，也不能追认本轮漏收了 SDK 请求。

以下 SHA 来自官方公开完整文件；源码只在内存读取，未归档全文：

| 官方公开来源 | 精确位置 | SHA-256 |
| --- | --- | --- |
| [acp_agent.rs](https://raw.githubusercontent.com/xai-org/grok-build/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs) | 528–568 | `f9d0400c2d72a8bd2288a56842b1c18bdda571d315b0c6c34f5fb42977e3edda` |
| [acp_mcp.rs](https://raw.githubusercontent.com/xai-org/grok-build/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_mcp.rs) | 23–47、65–84 | `233912e891371a197c90b2b7de698d2c2d728cf84ca23e5a7e2490b378450995` |
| [wire.rs](https://raw.githubusercontent.com/xai-org/grok-build/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-mcp/src/wire.rs) | 15–21 | `45f9b7c46ad685da069043727afb238e456c00f3d39eea2f1bda9b9a72d7d571` |
| [Cargo.lock](https://raw.githubusercontent.com/xai-org/grok-build/482711333c7195dc16a272777f86086d615e2afb/Cargo.lock) | agent-client-protocol 0.10.4 | `73088da69937bcc0bd8239dcc969cf4a703e836593cd5c7c35aa3a9cea393d0c` |
| [ACP 0.10.4 原包](https://static.crates.io/crates/agent-client-protocol/agent-client-protocol-0.10.4.crate) | `src/lib.rs` 285–289、527–534 | 原包 `10eeef5e80864f9c3c148a3f395c3e35a66d37ec7561c7845b2bffae8e841759`；lib.rs `c0fed4a1d911b10b715cde480ceb1b37ea4621f02690e7e20cc65ca85c6dfd5b` |

原生成功注册、实际反向初始化版本及请求来源均未通过。不根据候选源码猜改 session 注册字段，不把当前活跃回合补成 SDK 来源。

## 下一轮仍待真实验证

下一轮按已授权范围将 TLS CONNECT 上限提高为与官方适配器相同的 32，仍只允许一个原生输入、唯一无副作用 inspect mock，保留 32 MiB 字节预算、450 秒外层期限、180 秒夹具期限和 `auth.x.ai`、`cli-chat-proxy.grok.com` 两个窄白名单。HTTP 模型请求及成本仍不可观测；连接预算不能写作模型请求数。审批仍默认拒绝，SDK 及子任务生产门禁保持关闭。runner 现严格要求外层预检和生产检测共2次版本调用、一次私有 leader；离线回归覆盖少一次、多一次和第33次 TLS 连接拒绝，并保存初始化能力与两种精确 SDK 方法的安全投影。本次39项离线回归通过，不声称下一轮已通过。

后续应先排除网络预算干扰，再保存实际 outgoing initialize 和 session/new 的安全字段投影，以及 incoming initialize 的 SDK 能力、目标服务的原生 MCP 注册状态、进度、错误类别和协议 ID。注册状态应明确关联本运行代的唯一 server ID，不能只观察通知方法名。若在没有模型正文之前仍未收到 MCP 初始化，可另做无模型注册阶段验证；若收到请求，则按实际 MCP 协议版本和双层请求 ID 回复，记录字段存在性、类型、原生工具账本及最终结果，不用当前活跃回合补来源。

只有实际观察到精确 SDK mock 审批时，才评估测试专用一次允许，并绑定运行代、真实 session/prompt、原生请求 ID、toolCallId 与已核实的空参数结构。普通 `exec_shell` 常量输出不能替代 SDK 回执。本轮没有审批请求，不能据此修改权限策略。若实际二进制仍没有可证明的注册接口或可信请求关联，保留明确降级，继续拒绝本地副作用与 `run_agents`。

## 第 2 轮：非探针工具更新触发严格拒绝，来源仍未知

第2轮固定 `grok 1.0.30 (04b7ffed98c6)` 使用真实官方授权，仅提交并确认接收一个原生输入。实际初始化响应宣告 `result._meta["x.ai/mcp/sdk"]=true`，但 SDK 注册、反向请求、初始化、工具列表及 inspect 调用均为零，审批亦为零。整轮失败，来源状态仍为 `unknown`，SDK 与本地子任务生产门禁保持关闭。[原始公开事件](validation/grok-sdk-origin-probe-2.ndjson)、[元数据](validation/grok-sdk-origin-probe-2.metadata.json)、[网络记录](validation/grok-sdk-origin-probe-2.network.json)和[运行输入](validation/grok-sdk-origin-probe-2.inputs.json)经凭据模式扫描后逐字节归档，首轮失败记录保留。

私有失败原因只在内存与固定夹具错误比较，精确命中“原生模型调用了探针之外的工具”，SHA-256 为 `2f044e7029c4d0e74d93aac9027590adbf537d100b28134f2c4fc2526938a75b`。夹具仅在原生 `update._meta["x.ai/tool"].name` 为字符串且未精确匹配四种固定 qualified inspect 名称时设置该错误。`unexpected_tool_count=2` 属于同一 `toolCallId` 的 `tool_call` 和 `tool_call_update` 两次更新，并非两个独立工具；两个更新均无原生 `status` 字段。现有安全 trace 没有保存名称、title、工具参数或完整 `_meta`，无法进一步确认实际调用的是 ToolSearch、普通 shell 或其他工具，亦不能追认 SDK 懒加载、注册失败或 SDK 不受支持。[98帧协议身份投影](validation/grok-sdk-origin-probe-2.native-events.json)仅归档稳定方法、身份、计数和未知标记散列，不导出任何模型或思考正文。

本轮32次 TLS CONNECT 上限中实际建立8条官方连接，未出现 `network_budget_exhausted`；不能将 TLS 连接数当作 HTTP 模型调用数或成本上限。零 SDK 请求没有证明来源缺失或需要审批。测试专用诊断已增加原生名称的类型、散列与固定封闭分类，继续拒绝非 probe 工具；名称只取原生 `_meta["x.ai/tool"].name`，不由 title 或当前回合补身份。是否需要先执行 ToolSearch 仍是假设，未经受测二进制实证不能放宽工具或审批。新增三项 Python 回归后，47项离线安全回归通过；新增三项 Rust 纯夹具只完成定向格式与静态核对，尚待根代理 Cargo 门禁，不计原生验收。

[生产退出回执](validation/grok-sdk-origin-probe-2.exit-receipt.json)确认 `stop_requested`、退出码0及清理完成；[macOS 清理投影](validation/grok-sdk-origin-probe-2.macos-cleanup.json)确认实际作业移除、资源 CID 销毁及原生等待状态0。协议连接与隧道关闭均有探针或 runner 记录，独立只读核对认证副本路径已不存在、项目为空。`no_project_files=true` 和 `no_side_effects=false` 保持真实失败值，清理成功不把探针改为通过。配置字节确实变化：这里只记录原 runner 的 marketplace-only 语义审计，不读取配置原文或独立重建前值。[独立审计](validation/grok-sdk-origin-probe-2.audit.json)保留这些边界。

本轮运行 source10 的 libtest 与 source8 未变的监督组件；二者 SHA 及 source manifest SHA 已记录于运行输入。这是 dirty 中间快照的独立探针，不计 source10 全部门禁、最终同提交验证、应用重启或双语 UI 验收；`same_commit_runtime_verified=false`、`whole_snapshot_passed=false`、`public_product_gate_open=false`、`app_restart_and_ui_verified=false`。SDK 能力宣告与安全投影回归均不能替代真实 SDK 来源、父级权限上限或整个 Goal 验收。

## 第 3 轮：通用工具名称散列匹配，SDK 来源仍未验证

第3轮使用固定 `grok 1.0.30 (04b7ffed98c6)`、source11 的 libtest 与 main。两者散列分别为 `8f0e7063c37e7f110d3c022ccb932ca9cc73514b1675592ed8570b59b84ef484` 和 `8026e131aad657207690144d31e3e5c01b654039c4ed60eb1d5502dc9cb14fbb`，同67文件 manifest SHA-256 为 `31619a72300d6a38e82ade3d40e2329039cee864e5c0ca38c29ceb38037e504a`。[本地门禁及构建](OFFICIAL_11_LOCAL_GATES.md)通过，仍为基线 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 的中间脏快照，不是最终干净提交或整个 Goal 验收。SDK 和本地子任务生产入口仍关闭。

本轮仅一个原生输入及接收确认，初始化响应实际宣告 SDK 能力为 boolean true。SDK 注册、反向请求、初始化、工具列表、inspect 和 MCP状态观察全部为0，出现一次原生审批并由探针拒绝；这不能证明 SDK inspect 本身需要审批。两个工具更新共享同一 `toolCallId`，名称类型均为 string、诊断分类仍为 `other`，均无原生 status 字段。失败原因仍精确命中“原生模型调用了探针之外的工具”，静态错误散列为 `2f044e7029c4d0e74d93aac9027590adbf537d100b28134f2c4fc2526938a75b`。整轮失败，来源仍为 `unknown`。[原事件](validation/grok-sdk-origin-probe-3.ndjson)、[元数据](validation/grok-sdk-origin-probe-3.metadata.json)、[网络](validation/grok-sdk-origin-probe-3.network.json)及[输入](validation/grok-sdk-origin-probe-3.inputs.json)经凭据模式扫描后逐字节归档，前两轮证据保留。

名称 JSON 值散列 `da33f17cc734196e200d8f091338c908fb0b3e1f1f2556d4a823ac95f1877ae0` 与既有[固定原生空会话夹具](fixtures/grok-1.0.30-empty-session-recovery.ndjson)中 `use_tool` 精确相等：算法是 `SHA256(UTF8(JSON字符串值))`，即与 Rust 对名称 Value 调用 `to_string()` 后散列的字节一致。夹具整体 SHA-256 为 `b6152ef9cc1eac7b05918e7b5d6fa8beaabe8002926cc0bcf81533ffb9180398`；第12、13、26、30、50、55行的 `message.params.update._meta.tools` 名称数组提供27个唯一静态名称，唯一命中 `use_tool`。这只确认静态名称关联，没有读取私有名称、title或输入原文；名称数组不提供工具定义、参数或权限 schema，也不证明目标服务注册、参数为空或请求来源可信。

原生通用路由名称不能按名称 alone 允许，更不能由 title 推断原生工具身份。此前夹具只接受四种 qualified inspect 名称是待验证的探针假设，与本轮已匹配的通用 `use_tool` 路由名称不对应；实际载荷仍需独立核对。后续一次允许若有必要，必须精确绑定已验证的 SDK服务、inspect、空参数、真实 session/prompt、原生请求ID、toolCallId及原生工具账本；缺少任一可信关联继续拒绝。零 SDK 请求和一次审批不证明 SDK不受支持、缺少origin或ToolSearch懒加载步骤。

网络实际建立8条官方 TLS连接，32次上限未耗尽；32次非官方 origin 请求遭拒绝，未放宽白名单。TLS未解密，HTTP模型调用数及费用仍不可观测。[94帧原生安全投影](validation/grok-sdk-origin-probe-3.native-events.json)仅保存协议身份、方法、计数、散列及固定名称字典匹配，不复制模型或思考正文。

[生产退出回执](validation/grok-sdk-origin-probe-3.exit-receipt.json)实际为 `stdio_closed`、退出码0、清理确认true；[macOS清理](validation/grok-sdk-origin-probe-3.macos-cleanup.json)确认作业移除、资源CID销毁及原生等待状态0，认证副本路径不存在，项目为空。原始 `transport_closed=false` 保持不变：该字段只在 `run_process` 任务 join 为 `Ok(Ok(Ok(())))` 时置true，故本轮不能计transport正常返回，具体join错误未纳入证据。这与资源清理及原生退出是不同证据，不能据此推断CLI崩溃或资源仍活跃。`no_project_files=true`、`no_side_effects=false` 保留真实失败值。配置审计沿用原runner的marketplace-only结果，不读取配置原文。[独立审计](validation/grok-sdk-origin-probe-3.audit.json)明确不计最终同提交、应用重启、UI、跨平台、可信SDK来源或Goal完成。

公开 JSON/NDJSON 完成 API 密钥、JWT、Bearer、私钥、凭据赋值及邮箱形态扫描，候选计数均为零。本次仅新增证据和文档，没有 GUI/TUI 文案变化，无需本地化变更。

## 后续测试校准：完整原生契约下最多一次 AllowOnce，尚待实跑

第4轮仍是失败记录，前四轮不会由新守卫追认通过。根代理在 source13 的固定 `grok 1.0.30 (04b7ffed98c6)` 私有、0600、有界工具帧中核对了三个实际载荷：初始 `tool_call` 的原生工具名为 `use_tool`，`rawInput` 精确只有 `tool_name=infinishell-sdk-origin-probe__inspect` 与 `tool_input={}`；随后 `tool_call_update` 和审批的 `rawInput` 精确增加 `variant=UseTool`，工具种类为 `other`。三帧的 session 与 toolCallId 一致；两个更新携带相同 promptId，且等于真实 Started 的原生回合，审批帧没有 promptId。实际三选项包含 `optionId=always-allow`、`kind=allow_always`，唯一的一次许可 `optionId=allow-once`、`kind=allow_once`，以及 `optionId=reject-once`、`kind=reject_once`。已校准的永久许可仅可存在于列表中，响应只能选择 `allow-once`。此处引用根代理的有界核对，不是本子任务读取原帧后的独立在线归档；显示标签不参与授权。source13 是中间开发快照，不计最终同提交、UI或跨平台验收。

新增守卫仅存在于测试构建的原生 SDK 探针。它要求同一探针实例的真实会话确认、唯一 MessageAccepted 及相同 TurnStarted，再核对唯一 toolCallId 的初始和最终完整输入账本；审批会话、工具ID、`other` 种类、完整最终输入和唯一精确许可选项必须一致。没有审批 promptId 时，仅凭上述已确认账本核对，绝不从当前活跃回合补 SDK 请求的来源。原生名称 alone、title、历史回放、额外 rawInput 字段、非空 tool_input、其他工具或 SDK 服务均不构成许可依据。

同一仍活跃审批窗口内，同请求ID和整个请求载荷散列完全一致的重投可返回原响应，累计允许次数仍为1。另一请求ID、同ID不同载荷、过时窗口、跨 session/prompt、冲突工具账本、缺少 ACK/Started、未知永久许可的ID/kind、其他未经校准的选项、重复选项ID或多个一次许可均拒绝，并使整轮失败。终态或任一失败退出探针事件循环后立即关闭审批窗口。SDK MCP 版本、初始化与工具调用约束保持原有严格范围；不打开生产 SDK、子任务或权限策略入口。

公开事件增加固定 `probe_approval_observed`，仅记录安全协议ID、封闭决策、固定选中选项、完整请求与工具输入SHA-256、账本匹配布尔和重复标志。最终报告分别保存审批请求、唯一允许、拒绝、重复次数及 `native_contract_verified`；发生一次允许时 `approval_all_denied=false`。Python门禁要求至多一次精确允许、零拒绝和意外工具、计数与事件一致，并关联同原生会话/回合/工具账本及完整输入散列；重复事件还必须保持整条请求散列一致。一次审批成功不等于 SDK注册成功，更不等于可信 SDK 来源。`native_origin_verified` 仍同时要求真实探针通过、完整原生来源字段和最终原生工具账本关联；产品门禁始终为false。

本次62项 Python 离线回归全部通过；新增12项 Rust纯回归已完成 `rustfmt --edition 2024 --config skip_children=true --check`，尚未执行 Cargo，也尚未运行新守卫的真实 CLI。Rust覆盖完整前后输入、许可重投与冲突、过时窗口、缺少确认、跨会话/回合、额外字段、未知工具和选项、历史回放及不能补 SDK来源；Python覆盖计数、事件和账本关联、请求散列冲突、安全投影及独立来源门禁。只有根代理将这四个文件纳入后续冻结快照、通过实际编译并完成新一轮原生校准后，才能判断接口是否可用。

已校准三选项的离线回归把 `always-allow` 排在首位，仍要求响应只选 `allow-once`；未知永久选项ID、错误kind、重复永久选项或一次许可、额外字段均拒绝。同ID重投若仅改变永久选项的显示标签，整个请求SHA已变化，亦拒绝。Python同时校验三选项请求的完整散列、明确拒绝永久许可的选中项和决策，并把这些未经允许的值安全散列；存在永久许可选项不授予永久权限。

## 第 5 轮之后的只读搜索校准：尚待新快照实跑

第5轮在 source15 的整条真实 SDK 探针失败，未观察到 SDK 注册、请求或 inspect 调用。根代理在固定 `grok 1.0.30 (04b7ffed98c6)` 的三个有界私有帧中核对：实际首先请求 `search_tool`，初始 `tool_call.rawInput` 精确为 `{limit:5,query:"infinishell-sdk-origin-probe inspect"}`，且没有 kind；最终工具更新和审批精确增加 `variant:"SearchTool"`，kind 为 `other`。三帧 session 和 toolCallId 相同；工具更新的 promptId 与真实 ACK、Started 的原生回合相符。选项仍为上述三个已校准的唯一固定选项。source15 的 inspect 专用守卫正确拒绝该搜索，不能因此推断 SDK 不存在或请求没有来源。这里引用根代理提供的实际契约核对，本子任务没有读取私有原始帧；前五轮失败不追认为通过。

新的测试专用守卫将只读搜索与 inspect 分为两种账本，每种最多一个原生 toolCallId 和一次 `allow-once`，总共最多两次唯一允许。搜索必须完全匹配已核对的查询、整数5、初始及最终键集合、SearchTool variant、真实同会话及同 ACK/Started 回合，不能接受任意搜索、额外参数、浮点数、布尔值或字符串数值。inspect 仍按原有精确 UseTool 契约允许一次。两个工具使用不同账本和审批请求ID；同一活跃窗口中同请求ID且整条载荷SHA相同的重投只返回既有响应，不增加允许次数。第二个搜索或 inspect、跨会话/回合、关闭窗口、缺少真实确认、同ID改变载荷均拒绝并使整轮失败；永久许可选项始终不能被选中。

公开投影增加 `search_allow_count`、`inspect_allow_count`、封闭的 `calibrated_tool_kind`、`approval_contract_verified`、`discovery_native_contract_verified` 和 `discovery_tool`。搜索许可的 `native_contract_verified=false`，只有精确 inspect 契约才能将该字段置true；原生工具名、查询、参数、title和选项标签均不原样公开。Python门禁分别关联每种许可的真实工具输入SHA、审批整条载荷SHA、会话、回合及工具ID，要求每种至多一次唯一许可、零拒绝/意外工具和相符的重复记录。

搜索的来源和完成不能替代 inspect 的 SDK 来源。只把精确 UseTool inspect 的完整原生账本参与 SDK 来源核对，不能用搜索、当前活跃回合或候选唯一映射补缺失字段。整轮通过现进一步要求完整原生 session/prompt/toolCallId 字段和已完成的 inspect 账本关联；`missing_native_origin`、`candidate_mapping_only` 或 `unknown` 一律失败，注册完成或固定回执本身不能放宽此标准。单输入、严格 MCP 版本、32次 TLS CONNECT 和32 MiB预算、两条官方 host 白名单及既有时限均保持；TLS数不代表HTTP模型请求数或成本。生产 SDK、父子任务派发和权限策略入口继续关闭。

本次72项 Python 离线回归通过，新增8项 Rust纯回归完成定向 Rust 2024 格式和静态核对，尚未执行 Cargo或真实 CLI。Rust覆盖实际 initial 无kind与final other结构、查询/限制/variant/额外键、跨会话/回合、两工具独立许可和重投、第二搜索拒绝、部分 inspect 账本不能审批，以及完整SDK来源指向已完成搜索时仍失败。Python覆盖独立计数及账本、搜索原始输入散列差异、重复整条载荷冲突、封闭安全投影和来源独立门禁。该变更等待根代理纳入后续冻结快照再编译和实跑，不计 source15、既有 source14 图片验收、GUI、最终同提交或跨平台通过；没有 GUI/TUI 文案变化，无需本地化变更。

## SDK7 后：补齐真实标准 discovery，原生来源仍待验

SDK7 的独立 stdio 路径实际收到一次 `_x.ai/mcp/sdk_call`，内层为 `server/discover`、ID 为0，正文只有 `_meta`。公开三项 key 散列准确匹配官方现代 MCP 的 `protocolVersion/clientInfo/clientCapabilities` 名称，但没有公开其值。探针因不支持 discovery 提前失败，实际原生输入确认、initialize、工具列表与 inspect 均为0；这不能说明缺少 SDK 能力或工具来源。SDK6/SDK7 的整轮失败记录保持原值，不由新适配重算。

根据 [MCP 2026-07-28 schema](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/f56f204f6290f6531b14d5734eb3e0a10f0eb201/schema/2026-07-28/schema.ts) 和 [rmcp 3.2.0 固定源码](https://github.com/modelcontextprotocol/rust-sdk/tree/51ccb42993d6eb5075399672ce7a0c21a0e55eea/crates/rmcp)，现代 discovery 请求从 `params._meta["io.modelcontextprotocol/protocolVersion"]` 读取真实日期；逐请求要求对象型 `clientCapabilities`，可选 `clientInfo` 出现时核对 name/version 类型。支持标准 discovery 响应及现代工具列表的 `resultType/ttlMs/cacheScope`、工具结果的 `resultType`，服务身份放在 `_meta`。未知版本返回标准 `-32022`，缺失或错误类型为 `-32602`；不会通过 fallback 模拟成功。[源码夹具](fixtures/grok-1.0.30-modern-discovery-source-contract.json)记录实际 URL/SHA、必填字段及 SDK7 版本值未知的边界。

探针现代模式记录 `discovery_count=1/initialization_count=0`、`negotiatedProtocolVersion`、版本 carrier 与元数据核对布尔；旧版仍限 `2025-11-25 initialize`。`negotiatedProtocolVersion` 取自实际请求日期与本端支持交集，收到 discovery 时仅说明本端接受该值；后续列表和调用均使用相同实际元数据日期，才证明对端继续使用此版本。该字段不说明工具来源或完整验收通过。`servedToolNames` 是实际提供固定工具列表的安全诊断，不能添加为 discovery wire 字段。现代探针只提供一次列表与一次精确空参数 inspect，逐请求元数据缺失、额外正文、未知版本、重复身份冲突均拒绝；同一 ID 完全一致重投只回缓存。runner 仅原样保留固定日期/模式/carrier/工具名，其他值安全散列，未知元数据正文不公开。

初始化可以缺少 S/P/T；真实 inspect 的原生来源、完整原生工具账本和最终结果仍按原门禁核对。注册成功、现代协商成功或固定常量不能把 `missing_native_origin` 变成验收通过。生产 SDK、子任务派发、权限策略与父权限上限均保持关闭。此补齐已通过 source21／22 编译与定向回归（SDK Python85项）；仍是待真实实跑的局部候选，不计 source20/SDK7 或整个 Goal 完成；无需本地化变更。
