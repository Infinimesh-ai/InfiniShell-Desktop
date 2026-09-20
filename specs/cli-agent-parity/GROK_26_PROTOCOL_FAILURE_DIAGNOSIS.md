# source26 Grok SDK9 与无输入预检协议失败诊断

本次仅核对指定公共 SDK9 / policy1 证据、仓库静态实现及此前已获取的官方候选源码。没有执行 native、网络、Cargo、GUI 或模型，没有读取认证、私有原始协议帧、target 或冻结目录。父任务报告源码提交为 `436cc739234061c092cedd106432bbbfa3c2d645`；本子任务未独立校验提交归属，SDK runner 自身 `same_commit_verified_by_runner=false`，因此不将该字段扩写为验收结论。

## 可以证实的 SDK9 失败点

SDK9 真实失败，测试退出码 101。固定原生为 `grok 1.0.30 (04b7ffed98c6)`，CLI SHA256 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。公共记录显示：

- `server/discover` 1 次、`tools/list` 1 次、`initialize` 0 次、inspect 0 次；应用提交输入 1 次，native_inputs 1。
- discovery 和 list 请求中的实际 metadata 均为 `2026-07-28` 且 schema 校验通过。两请求的 outer/inner ID 分别为数字 0 与数字 1。
- 原生随后报告 `init_progress` connected=1 / total=1、`mcp_initialized` mcp_tool_count=1、对应 server_status ready / initialized。这支持该 SDK 工具服务在初始化阶段已被原生接受，不代表 inspect 或来源验证成功。
- 最后记录的帧是 JSON-RPC 2.0 响应，无 method，有 object result，无 error，result.stopReason 缺失。响应 ID 为 string，13 字节，SHA256 `731058153ec89c5e512596d24d5ac95c7262a9cb4e5c58fbdcebd36677f9b38e`。
- 生产 `grok.rs:1589–1592` 对无 method 响应调用 `as_u64()`，在该位置返回 `invalid_response_id`，transport_task_status 为 runtime_error。
- 私有错误正文的公共状态是 `not_observed`、0 字节。只检查公共状态，没有读取该私有文件。未观察到 error 字段，不能将失败判为 API、账号或登录问题。

## 响应 ID 的所有权边界

当前应用发起的 ACP 请求在 `grok.rs:633–640` 使用递增数字 ID；响应还必须在 `1603–1609` 精确匹配 pending ID。原生反向请求允许字符串 ID 是另一个方向的契约：`local_tools.rs:334–339`、`grok_local_tools.rs:98–107` 接受有效字符串或整数，MCP 适配按 JSON 类型保留其身份。SDK 专用服务在 `grok_sdk_origin_live_tests.rs:813–818` 分别原样回显实际 inner 和 outer ID，不自行创建或将数字转为字符串。

因此这次故障不是已经证实的“所有字符串 ID 都被 MCP 服务拒绝”。是无 method 的原生响应进入应用自有数字请求账本后失败。目前没有公共出站请求摘要或 pending 快照将 13 字节字符串归属到某个本应用请求；它也不等于本次已观察 SDK 请求的数字 ID 0 / 1。

对限定目录的已知静态字符串做长度和 SHA256 对照，没有取得该响应 ID 的直接字面量匹配。该结果不覆盖动态构造、转义字符串、未读取文件或原生二进制，不能声称该 ID 不来自原生某个合法内部流程。没有通过未知模型或协议正文进行反推。

`grok_tests.rs:268–287` 现有回归明确要求未知字符串响应不能替代数字 pending 请求。这与过时、乱序及未关联事件的防护要求一致。没有足够证据将任意字符串响应绑定当前 pending 或忽略后继续宣布成功；本报告不建议这样放宽。

下一次最小诊断应在出站 ACP 请求、SDK inner/outer 回复处保留 ID 类型、字节数、SHA、数字值、固定方法枚举和 generation，并在失败前保存 pending ID / kind 与 result 顶层键的安全摘要。不能保存正文，不能根据当前回合猜测或回填来源。这些摘要才能区分原生 ID 变换、未关联额外响应及应用适配遗漏。

## policy1 的确切守卫失败及收尾边界

无输入预检退出 101，native_inputs=0、runner_cleanup_confirmed=true，全部 policy 能力仍 unknown，权限上限和文件系统沙箱仍 false。

公共失败序列为 82 字节哈希、36 字节原因哈希、422 字节哈希、24 字节原因哈希。两个固定原因取得确切源码匹配：

| 固定源码原因 | 公共字节数与 SHA256 | 位置 |
| --- | --- | --- |
| `native_unknown_notification_rejected` | 36，`553467e5c8ad28e8e0d6d88da80163ba14785868c0d169461e0d092631e96237` | `grok_policy_preflight_live_tests.rs:505` |
| `native_exit_code_missing` | 24，`315ae2f1192d0c3184a943fc25736613ca76cd8f3b2fc7fc2ecfe5d0d127e1d7` | `grok_policy_preflight_live_tests.rs:721` |

首个固定原因证明无输入预检拒绝了不在当前通知白名单内的方法；按 `462–505` 的顺序，它此前通过了通知信封、无 prompt 字段等前置形状守卫。当前白名单只允许几种 `session/update` 和 `_x.ai/mcp_initialized`。然而公共记录只有 raw 帧长度与 SHA，没有 method 摘要，不能确认未知通知是否为 SDK9 观察到的 init_progress，或任何其他初始化通知。

82 字节哈希是 `75e55ebcd7c129c45a7f554738965ffcc20f2c36b260953f7e22cbd7a1a79406`，422 字节哈希是 `979116740b5f7564d78917dcd90122af492fb796bf1fc8ca0badb472466481e9`。没有可靠静态匹配，保留未知，不编造其原文或错误类别。

静态收尾另有局限：`824–829` 将全部剩余帧按通知验证；提前失败后剩余的 pending RPC 响应如果到达，会被通知守卫拒绝。当前只有 422 字节 raw 哈希，不能确认它就是该响应。`849–856` 的清理及 cleanup_event 验证先于回传首次语义失败；若退出码缺失，返回值可被 `native_exit_code_missing` 替代。公共多条哈希仍保留首次失败，但不能用 Python 外层清理成功替代 Rust 正常 EOF 和原生退出码证明。

必要局部改进是保留首次语义失败与独立收尾诊断，并为未知通知增加 method 的安全类型、长度、SHA、固定白名单判定及关联摘要；拿到真实方法与形状后再支持必要初始化通知。不能删除原有零 prompt、零工具副作用守卫，不能将强制 stop 视为正常 EOF。

## 公开源码边界与当前状态

已缓存的官方候选来自 `482711333c7195dc16a272777f86086d615e2afb`，其 SOURCE_REV 为 `be7ce6e8cffe46d20bef9834b211616082ee866b`，未映射到固定原生短 ID `04b7ffed98c6`。[官方 SOURCE_REV](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/SOURCE_REV)

候选 `acp_agent.rs:566–568` 说明 SDK MCP 通过 ACP 反向通道，但不能用于证明固定 1.0.30 的完整响应 ID 行为。[官方候选实现](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_agent.rs)

目前可以定位真实失败的代码分支和 policy1 的两个固定原因，不能精确归因为 native bug、账号故障或可安全放宽的应用 ID 规则。没有实施生产修复，没有改变 Grok 来源或权限门禁，没有重新运行或修改旧证据，未计任何 native PASS。

配套安全证据：[grok-26-protocol-static-audit.json](validation/grok-26-protocol-static-audit.json)。新增文档无需本地化变更。
