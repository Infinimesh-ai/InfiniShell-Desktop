# Grok 恢复接口与官方网络契约只读复核

保持现有两个官方 host 白名单。新的公共记录已经证明第 8 代 Completed、同原生 session/当前 native turn，72 字节结果 SHA `bfdc32dfa6ef1c4e44b7f96298edd0f608073f5db9513f0596c2d1a3f79521f5`。根代理的现场收据是恢复同端口 network4 后，没有重启 CLI/App 或重投输入即取得结果，仍只允许 `auth.x.ai` 和 `cli-chat-proxy.grok.com`。额外被拒 origin 不是这个回合完成所必需。第 7 代在同一公共记录中仍为 Disconnected，不能强认其单一阻塞原因。[第 8 代公共证据](validation/gui-grok-436cc739/59-grok-restart-new-input-result.json)

官方文档列出核心认证/推理的两个 host；`api.x.ai` 用于直接 API-key 路径，远程同步、资产和安装域名属于额外功能。代理支持标准环境变量，文档建议流式代理空闲超时至少 10 分钟；这与根代理给出的 network4 900 秒预算相容，不证明现场所有流都已完成。[官方 enterprise 文档](https://docs.x.ai/build/enterprise)

独立统计 scope2：32 次官方连接尝试和 32 次转发、6056728 字节、72 次官方连接预算拒绝、214 次非白名单拒绝。runner 的 `forwarded` 是整轮累计连接数，达到 32 后后续 CONNECT 会拒绝；关闭或 deadline 也可触发同一事件，因此不能逐条反推唯一触发条件。现有字节数低于 32 MiB，不表示累计连接预算没有耗尽。auth/chat 的实际转发分别为 1/31。[scope2 原公共记录](validation/gui-grok-436cc739/61-grok-network-scope2-final.json)、[runner 的预算判据](../../script/cli-agent-parity/run_grok_official_adapter_live.py#L148)

仅将文档明确的七个固定 host 加 `:443`，按 UTF-8 SHA-256 比较三个公开 authority；不猜测未知域名，不读取二进制 strings。

| 公开类别 | authority SHA-256 | 次数 | 固定候选匹配 |
| --- | --- | --- | --- |
| xai_api | `ad168a42d22791de501ff25ab3ccdac9ed74c96cb70daf753118c036af83a3c3` | 11 | `api.x.ai:443` |
| unknown | `bf68bba4f5a0724001e6b8a3e6dee3271fc34d5b71aced2e68e38f61d8fd2782` | 99 | 无匹配，保持未知 |
| unknown | `9b733062390bcae6f4e51fe39bc56e510a3bdd7aa504bf3d97191dc4f8164896` | 104 | 无匹配，保持未知 |

固定官方候选 `482711333c7195dc16a272777f86086d615e2afb` 的 `session/load` 进入 attach。恢复先按当前默认生成 summary 配置，随后依据持久化 `summary.current_model_id` 重新解析采样 URL，再恢复模型及 effort；当前 catalog 可将旧模型映射、选同族 fallback，或阻断不可用模型。这证明恢复有自己的模型恢复路径，不能仅凭新会话默认模型推断历史模型端点。[候选 session_setup.rs:804](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/session_setup.rs#L804)、[949](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/session_setup.rs#L949)、[1530](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/session_setup.rs#L1530)

候选采样 helper 依据 model 与当前认证策略解析凭据；API-key 的路由可与全局推理代理配置不同，模型解析失败则保留全局采样配置。模型切换也重新生成采样配置。另一方面，候选 image 工具即使 OAuth 也可直连 xAI API；被拒 `api.x.ai` 本身不足以证明恢复转 API、登录失效或核心推理失败。本审计未读取现场历史模型、配置、令牌、错误或私有正文。[agent_ops.rs:2069](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/agent_ops.rs#L2069)、[2159](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/agent_ops.rs#L2159)、[2213](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/agent_ops.rs#L2213)

仓库工作区的 Grok adapter 仅在 load 请求增加已验证的 `sessionId`，不传模型覆盖；返回模型与配置元数据仍按实际原生报告记录。官方 runner 使用隔离环境和合成配置固定默认及辅助模型，不继承用户的自定义 base URL；这份静态审查不能证明现场历史内容或固定二进制的内部决策。官方设置文档也区分新会话默认与每模型 `base_url`。[设置参考](https://docs.x.ai/build/settings/reference)、[应用 open_session](../../app/src/ai/cli_agent_runtime/grok.rs#L1536)、[官方 runner 配置](../../script/cli-agent-parity/run_grok_official_adapter_live.py#L239)

下一步最小验证建议：继续限定这两个 host，按同 session/native prompt/generation 保留结果链；维护累计 CONNECT 与 deadline 统计，明确预算失效时的应用状态。若需验证模型路由差异，仅新增固定 auth-mode/endpoint 类别与模型 ID 摘要的安全投影，在 new/load 前后关联，不导出凭据、配置正文或历史输出。本任务只提出建议，没有执行请求、重投、取消、重新授权或网络白名单修改。

本轮实际获取固定候选 raw 6 文件，合计 493295 字节，每个小于 256 KiB；递归 tree 1 次，1225421 字节，HTTP 200、未截断。复用五个已授权的官方公开缓存文件，逐文件与 manifest 的 bytes/SHA 核对一致。所有 URL、SHA、范围及来源区分见 [安全 JSON](validation/grok-resume-network-contract-review.json)。候选源与已发布固定 native 二进制的完整构建映射未证明，不能将候选行为直接当作现场内部事实。没有读取私有认证/配置/数据库/envelope/模型或错误正文、target/冻结树；没有执行 CLI、Cargo、UI、Git 修改或模型请求。未改产品文案，无需本地化变更；不将本审计计为 Goal 完成。
