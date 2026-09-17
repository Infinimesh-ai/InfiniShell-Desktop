# source15 SDK 第五轮失败诊断

Grok Build 1.0.30 SDK 第五轮真实验收结果为 **FAILED**：libtest 退出码 101，运行器退出码 1（根代理实际执行报告）；`probe_passed`、`native_origin_verified`、完整 CLI parity 均为 false，产品 SDK 门禁保持关闭。初始化声明 SDK 能力 true 不能代替真实 SDK 初始化、调用和原生来源关联。

本轮属于 source15 未提交快照：75 文件 manifest SHA `6830223673d39301f2b9884d02661f22a4a19e5df27c85dbccdda4542fccdba2`，lib SHA `71e42409bc656cea8d050b30a1e4408576c9e46e6e45c775bf1bfde3860b79bf`，监督 main SHA `d9a5590cc68c303aca53f5912df861b008063959d3c5ba002999776781d4e3db`。身份与 [source15 门禁](OFFICIAL_15_LOCAL_GATES.md)一致，原生版本为 `grok 1.0.30 (04b7ffed98c6)`。[输入绑定](validation/grok-sdk-origin-probe-5.inputs.json)是安全投影，不称原 manifest 字节复制；75 文件前后核对沿用独立 official15 审计，本次仅读取 SHA 精确匹配 source15 的两份小 SDK 源文件校准摘要规则，没有重读完整 frozen 或大二进制。本轮不含并行生产 Claude PNG 与新增文案，失败不能计为后续 SDK 守卫成功。

| 实际公开证据 | 结果 |
| --- | --- |
| 原生输入 / 提交输入 | 1 / 1 |
| 工具更新 / 审批请求 | 2 / 1 |
| 审批允许 / 拒绝 / 重复 | 0 / 1 / 0，全部拒绝 |
| SDK 请求 / 初始化 / 工具列表 / inspect | 全部 0 |
| 完整原生来源字段 / 原生调用账本关系 | 未验证 |
| `transport_closed` / `no_side_effects` | false / false |
| 监督收据 `cleanup_confirmed` | true，不能替代以上门槛 |

[原始公开 NDJSON](validation/grok-sdk-origin-probe-5.ndjson)六条记录、[原始元数据](validation/grok-sdk-origin-probe-5.metadata.json)与[原始网络报告](validation/grok-sdk-origin-probe-5.network.json)按原字节复制，保留 FAILED。审批显示 `native_contract_verified=false`、`tool_call_id_in_native_ledger=false`；不猜具体失败字段，不把自然语言工具意图视作 SDK 执行。

[原生契约安全投影修订版](validation/grok-sdk-origin-probe-5.native-contract-projection.json)由根代理读取实际三帧后生成，修订 2 SHA 为 `a497b6b17b60ff88471266013c58cf5c8bf4de71a2ec53d8ae545605a54ab51c`，本归档没有读取原始帧。根投影确认两条 `session/update` 与一条 `session/request_permission` 的原生 `search_tool`、固定 `server + inspect` 查询、limit 5、`finalSearchTool` 等契约字段匹配；SDK 来源仍为 false，不凭局部工具字段通过改写本轮失败。

[投影第一版](validation/grok-sdk-origin-probe-5.native-contract-projection-v1.json)原字节保留，SHA `19b4c16f094c9a57dfbe37570cc27a77e5b61a82ea88be1c9d39caeb1ae7b805`。第一版 session 摘要误用含引号 JSON 字符串散列，出现 `c0eda669…`；修订 2 改为 UUID 原始字符串 UTF8 字节散列，得到 `5e01773a…`，三帧均与公开 trace 匹配，并另字段保留旧 JSON 摘要。归档代理独立核对 source15 Rust `projected_id`（第 300 行）保留 UUID 字符串、非 UUID ID 先做 JSON 字符串 SHA；Python `safe_id`（第 140 行）对原始字符串做 UTF8 SHA、保留 typed 摘要。这解释工具调用摘要第一版已匹配而 session 摘要需纠错，**不是运行时发生跨 session，也不是补填 SDK origin**。

修订 2 的三条 `raw_input_sha256`、调用 ID 摘要和修正的 session 摘要均逐帧与公开事件匹配。两版私有原帧来源 SHA 相同：`0165adfa2bba777485db4c123b11637115709d94157f37d2647774117ae8e615`，2,438 字节；除摘要修订没有改实际三帧，`runtime_or_sdk_origin_filled_in=false`。原始工具帧、私有 stdout、auth/config/env 不归档、不读取。

[退出收据](validation/grok-sdk-origin-probe-5.exit-receipt.json)绑定运行代 `c7ed2d50-107b-40c3-88ce-25c685e79383`：`stop_requested`、退出码 0、macOS resource coalition 清理确认。[macOS 清理收据](validation/grok-sdk-origin-probe-5.macos-cleanup.json)记录 job 已移除、资源 CID 已销毁、native wait status 0、`execution_failed=false`。这是停止请求后的监督清理，不改写成正常 stdio EOF，也不覆盖探针 `transport_closed=false`。归档代理未复查操作系统 job/CID；auth 副本已删除、隧道已停止沿用原 runner 元数据，未读认证路径。

隔离私有 settings 字节发生变化，但原 runner 语义审计确认仅 marketplace 初始化、TOML 可解析、权限段未变与 settings scope 通过；不把 `bytes_unchanged=false` 解释为改写用户全局配置。本归档未读私有 settings，项目条目 0 也不能补足完整 `no_side_effects` 门槛。

网络报告为 8 次官方 TLS 连接尝试、4,340,052 字节，在 32 次连接与 32 MiB 字节上限内，预算未耗尽；另有 32 条非官方 origin 拒绝。TLS 不解密，无法观测 HTTP 模型请求或强制两次请求和费用预算；请求预算 2 仅为申请值。

[独立归档审计](validation/grok-sdk-origin-probe-5.audit.json)保存七份原字节证据的源映射、SHA、计数、投影纠错和清理边界。全部档案八类定向凭据扫描零命中，只记录方法与命中数，不归档扫描正文。没有运行 Cargo、模型、原生 CLI、授权、GUI，没有修改产品源、旧证据或主计划/矩阵/验证报告。

本轮不能计 SDK、子任务、父权限上限、GUI、应用重启、SSH/tmux、Linux/Windows、最终干净提交平台验收或完整 Goal 完成。本次仅新增工程验证档案，无需本地化变更。
