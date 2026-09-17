# Claude 合并输入批次取消验证

2026-09-17，第 1 轮真实固定 Claude Code 2.1.273 验收通过，使用用户授权 API 与固定 claude-sonnet-4-6。作用域为 Rust 生产适配器的合并批次取消；不计完整 GUI 或最终同提交跨平台生命周期。

三个原生输入中，A 与运行时追加的 J 都取得原生 ACK，并出现明确 InputJoined。仅单次允许常量 SDK inspect 工具，不读取项目或派发任务。输出开始后发出 interrupt，严格取得控制请求的嵌套成功 ACK、执行 cancelled 事件以及含完整 [A,J] 输入 UUID 集合的原生 result，才把两条输入分别记为 Cancelled。随后 C 在同一原生会话中完成精确随机标记，生产监督链关闭传输并取得正常退出和清理回执。

[独立原生帧审计](validation/claude-batch-cancel-live-1.audit.json)、[原字节安全事件](validation/claude-batch-cancel-live-1.ndjson)与[运行器元数据](validation/claude-batch-cancel-live-1.metadata.json)保留原始摘要。审计直接检查原生帧中的实际输入、工具 ID、控制请求 ID、完整批次与继续结果，缺少任一信号不能成功。18 项离线负向回归包含缺失确认、旧代、混合批次、重复消息、假摘要和投影漂移。

本轮使用 official-9 的测试库与 official-8 的主程序监督组件；监督组件在两快照之间未改，二进制摘要分别记录。official-9 的三个旧 Grok 门禁断言失败独立保留，不能将 Claude 这项成功写为整个快照通过。本轮未验证任务存储、父子权限上限、应用重启、活动重连、GUI 批次取消或其他操作系统。source6 GUI 等待 Edit 审批取消实际返回 Failed 的失败证据继续保留，不能由本项替代。
