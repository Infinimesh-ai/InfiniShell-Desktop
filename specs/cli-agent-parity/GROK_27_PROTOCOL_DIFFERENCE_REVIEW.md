# source27 Grok 协议差异只读核对

本次只读取 source27 的安全公共证据、已缓存的 xAI 官方候选源码和固定 Grok 二进制。没有读取任何私有响应信封、日志、认证或原生／模型错误正文，也没有执行 native、网络、Cargo、UI 或 Git 修改。source27 仍是父提交 `436cc739234061c092cedd106432bbbfa3c2d645` 上的未提交候选；不计修复或最终验收通过。

## 已确认的固定协议字面量

SDK9／SDK10 的公开响应 ID 摘要为 string、13 字节、SHA-256 `731058153ec89c5e512596d24d5ac95c7262a9cb4e5c58fbdcebd36677f9b38e`。在实际固定文件 SHA `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb` 中，offset `116151162` 的 13 个原样 ASCII 字节 `skills-reload` 与该摘要精确一致，定向扫描匹配数为 1。仅公开这个固定协议名称，没有从私有信封回收任意 ID。

source27 实际到达该响应时仍等待数值 prompt 请求 4；公开发出事务只有 initialize 1、authenticate 2、session/new 3、session/prompt 4，以及 SDK discovery/list 的 outer/inner 0、1 回复，没有应用发出的 `skills-reload` 所属请求。响应无 method/error、result 为 object、stop reason absent。因此可确认的差异是：固定 CLI 在此阶段产生了与应用 pending 数值请求不同的固定 string ID 响应，当前 adapter 将它按未关联响应拒绝，SDK 工具调用尚未开始。

名称相同不能证明它是无害元数据或当前 prompt 完成，也不能证明其触发路径、结果字段、跨回合归属或安全接纳规则。现有生产数值 ID 和审批来源守卫不应因此泛化接纳未知 string ID。后续须取得固定版本的触发与结果契约，再考虑独立、明确的事务关联；本报告不修改规则，也不建议把该响应转成完成、审批或能力成功。

## 尚未匹配的通知

policy2 的未知 method 摘要为 string、25 字节、SHA-256 `5ad0b9eadd8fadb2225bf5c00b21c1cf42ab869332b86333e722aa248a106580`。其 params 为 object，session ID absent，无 id/result/error。本次在固定二进制的原样 ASCII 协议字符窗口、quoted ASCII 候选及 9 份缓存 Rust 文件的原样字符串字面量中均未匹配，名称及具体语义继续为 unknown。

最终正确宽度扫描测试了 4,439,592 个原样 13／25 字节协议字符窗口和 472 个 quoted ASCII 候选；长字符 span 已纳入，没有沿用首轮跳过长 span 的结论。未匹配范围不覆盖非 ASCII、转义编码、动态拼接或所有上游源码，因此不能证明该通知在上游不存在，也不能推断登录或 API 失败。

## 摘要口径与源码边界

当前 `app/src/ai/cli_agent_runtime/grok.rs:299–306` 明确规定：string 摘要使用 `text.as_bytes()`，即 UTF-8 原字节；非 string 才使用 JSON 序列化。两个目标的 13／25 字节不是包含 JSON 引号的长度。本次初轮曾按 quoted JSON 宽度检索，已在口径自查后纠正，初轮未匹配不参与最终结论。此前 source27 诊断报告中 method 的“序列化 25 字节”措辞应理解并修正为“UTF-8 原字节 25”；本任务遵守写入边界，未改动原报告或档案。

缓存源码来自官方不可变快照 `482711333c7195dc16a272777f86086d615e2afb`，共 9 份局部 Rust 文件：ACP types、MCP/debug 扩展、ACP agent、session handler/admin/mod/result 等。源码路径、逐文件 SHA 和官方 URL 已保存在配套 JSON；在这些文件中没有两个完整目标字面量的匹配行，不能提供虚构的上游赋值行号。[官方 ACP agent 源文件](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs)

该快照声明的 SOURCE_REV 为 `be7ce6e8cffe46d20bef9834b211616082ee866b`，尚未证实与 native 短构建 ID `04b7ffed98c6` 的精确映射；也没有读取完整公开树。字面量匹配依赖实际固定二进制，而不是将候选源码冒充固定版本完整来源。

公开信封证据只有 captured、1 条、259 字节、文件 SHA `ed82aa0c7f5a228ef65737cef12ddb6099a764d6b7bd486d94c488d80fab8523`、范围验证和 ledger 对账，以及已公开的 response 形状；没有公开 result_fields 的键／类型投影。本次未通过读取私有信封补齐这些字段。

安全摘要：[grok-27-protocol-difference-review.json](validation/grok-27-protocol-difference-review.json)。两项实际 FAILED、policy unknown、父权限 ceiling false、SDK 产品 gate 关闭及 source26 历史失败均保持原结论。没有用户功能或界面文案变化；无需本地化变更。

`finished_reads_done=true`，STOP。
