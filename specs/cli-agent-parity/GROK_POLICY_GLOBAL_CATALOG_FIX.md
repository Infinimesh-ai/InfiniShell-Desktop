# Grok 无输入预检：全局空 MCP 目录修复

本候选修复 source27 已真实观察到的 `_x.ai/mcp/servers_updated` 通知形态缺口。预检现在只接纳完整 JSON-RPC 2.0 通知 `{jsonrpc, method, params}`，无 id/result/error，且参数严格为 `{ "mcpServers": [] }`。全局通知在原生会话绑定前单独处理，不要求或伪造 session ID，不消费 pending 数值 RPC。原 profile 及 New/Load 请求确实注册空 MCP 列表；此次没有新增非空名单政策。

协议依据是此前已完成的 [定向契约核对](GROK_SKILLS_RELOAD_CONTRACT_REVIEW.md)：真实通知名称的原 UTF-8 长度 25B、SHA-256 `5ad0b9eadd8fadb2225bf5c00b21c1cf42ab869332b86333e722aa248a106580` 与固定构造精确匹配，候选公开源码定义 `{mcpServers:[...]}` 的全局 MCP 目录推送。候选公开来源与已发布二进制的完整构建映射仍未证明；目录实际值只能由下一次受控原生复验确定。旧 policy2/SDK10 失败档案保持原结果。

Rust 解析继续拒绝非空目录、未知服务、非数组、缺键/多键、sessionId、generation、_meta、权限/工具/回合字段与外层响应或请求身份注入。服务数组上限沿既有目录边界为 64，通知数量上限为 256，WireReader 原有连接字节及帧预算保留。合法重复空目录只增加观察计数，不建立会话、输入或审批身份；跨 RPC 到达顺序不会改变事务归属。全局通知的来源仅为当前阶段独占的进程传输域，不声称原生发送了 generation 字段。

Rust 诊断对精确方法额外输出布尔值 `global_catalog_closed_empty`，与实际目录解析复用同一校验；失败参数、名称、env 值均不公开。Python 投影仅对此方法要求该额外布尔字段；审计要求它为 true、方法摘要精确为原 UTF-8 的 type/25B/SHA，以及 session 摘要严格为 `{type:"absent"}`。实际实现使用 `.get("sessionId")`，所以缺字段生成 absent；显式 null、字符串会话、错误摘要与未知额外元数据均不能冒充合法全局目录。原有其他通知仍要求字符串会话摘要，未知方法只保留安全摘要或投影拒绝。

本次基线为实际读取的 `e6873e2cbb12a12ffce0b8283ee2ac96b940da39`，四个源码文件是新的未提交候选。实际离线验证为 Python 41 tests PASS（运行一次，unittest 报告 0.051 秒）、两个 Python 文件 py_compile exit 0、两个 Rust 文件定向 rustfmt（edition 2024、skip_children=true）exit 0，以及四文件 git diff --check exit 0。新增 10 个 Rust 纯回归后该模块共有 40 个纯测试；本次没有执行 Rust 测试或 Cargo，没有启动 CLI、访问认证、读取私有原生正文、请求模型或操作 GUI。Python 合成审计正例不是原生接口通过、同提交跨平台通过或产品验收通过。

新增回归覆盖空目录、真实 outbound New/Load 的空注册列表、非空来源、畸形/额外元数据、伪会话/代次、请求/响应身份注入、重复及乱序通知、预算、方法变体和公开诊断正文隔离；Python 另覆盖缺失 pending 响应不被通知补齐、过时代次/阶段、证明字段类型及固定方法摘要的 hash/size/type。完整测试名、四源码真实 bytes/SHA 与验证范围见 [安全 JSON](validation/grok-policy-global-catalog-fix.json)。

修复没有改变未拥有的 `skills-reload` 响应处理；未知 string ID 仍不能替代数值 pending。primary/cleanup/drain 失败账本、零原生输入守卫与生产能力门禁保持现有规则。policy、根权限 ceiling 和业务/工具来源仍为 unknown/false。后续由根代理审查后完成 source29 统一 Rust/Cargo 门禁及 policy-preflight-3 原生复验，失败不予回填。

无需本地化变更：此次只修改受控验收夹具与安全诊断投影，没有用户可见产品行为变化。没有验证新候选的跨平台或 SSH/tmux 原生能力。

STOP；finished_reads_done=true。
