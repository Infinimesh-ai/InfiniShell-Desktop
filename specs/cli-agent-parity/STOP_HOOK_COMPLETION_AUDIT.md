# 普通 PTY 的 Stop hook 与真实完成边界

本次审计针对普通终端的通知状态。Codex、Claude、Grok 的 Stop 通知都不能单独证明任务成功：它们可能在其他 Stop hook 决定继续之前执行。原生 turn / prompt 标识能证明回调属于哪一轮，不能证明所有 hook 已允许结束。应用因此保留当前响应，将三者的普通 PTY Stop 显示为 `Unknown`，继续接收同回合的工具、审批和明确失败事件。

托管运行时保持独立：Codex `turn/completed` 的明确成功状态、Claude 结构化 `result` 的成功判定继续由各自 adapter 和 coordinator 处理。本次没有修改它们，也没有开放 Grok 托管入口。

## Codex 0.147.0：固定源码已证实同回合继续

依据官方 `rust-v0.147.0` 对应提交 `be6e8eac029b183056b7e4402879f15d2c85f61b`：

- [`dispatcher.rs:90–117`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/engine/dispatcher.rs#L90) 并行执行匹配 handlers；单个通知脚本可以先完成。
- [`events/stop.rs:177–199`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/events/stop.rs#L177) 在全部 handlers 返回后才聚合 `should_block` / `should_stop`。
- [`session/turn.rs:464–515`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/session/turn.rs#L464) 收到阻止结束的决定后，在同一 `turn_context` 追加 hook 提示，设置 `stop_hook_active = true` 并继续当前循环；这里没有重新调用 UserPromptSubmit。
- 官方 [`stop_hook_can_block_multiple_times_in_same_turn`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/tests/suite/hooks.rs#L1217) 使用本地模拟 SSE，断言三次 Stop 的 `turn_id` 相同，`stop_hook_active` 依次为 `false, true, true`。这是上游已存在的回归代码，本次没有运行上游 Cargo 测试。

故原路径可确定为：第一次 Stop 带有助手文本 → InfiniShell 标记 Success → 另一个 hook 要求继续 → 无新 PromptSubmit → 后续 Stop 因 `stop_hook_active` 被通知脚本跳过 → 应用的成功终态保护又拒绝本轮后续审批。若继续期间失败，成功显示也不能作为可信结果保留。

更晚的原生证据是 [`tasks/mod.rs:770–797`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/tasks/mod.rs#L770)：任务循环返回后发出终结事件，包含回合错误或中断信息。托管 adapter 使用该类原生生命周期；普通 PTY 当前没有接收它。旧 `notify` 对应的 AfterAgent 虽在 Stop 聚合之后，但仍先于最终生命周期，且其路径还处理 FailedAbort，不能未经独立验证就把现有 Stop 替换为同等成功断言。[固定 AfterAgent 实现](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/hook_runtime.rs#L465)

## Claude 2.1.273：官方契约不允许从 Stop 推导成功

Claude 主程序未提供可在这里核对的对应版本开源执行循环，因此区分受测 CLI 版本和当前官方文档，不能把文档当成本次真实成功模型回合记录。

官方说明匹配 handlers 并行执行；Stop 的 `decision: block`、退出码 2 或追加上下文可以让模型继续。`stop_hook_active` 表示此前已经因 Stop hook 继续，不能预测本次其他 handler 的决定。已有 `last_assistant_message` 只是本次候选响应，不是 hook 聚合结果。[执行规则](https://code.claude.com/docs/en/hooks#hook-handler-fields)、[Stop 输入与决定](https://code.claude.com/docs/en/hooks#stop-input)

因此当前插件在 `stop_hook_active = false` 时发送 Stop，也不能保证成功。后台任务列表为空同样不能消除这个问题。`Notification.agent_completed` 只覆盖 agent view 中的后台会话，成功与失败均可能触发，不能充当普通前台回合成功证明。[Notification](https://code.claude.com/docs/en/hooks#notification)

当前可用的严格结果来自应用托管的结构化 `result`，必须检查成功/错误分类并维持连接、会话及回合关联。普通 PTY 没有等价的已验证终结通道；`idle_prompt` 仅说明等待输入，不能升级为成功。正常退出整个 CLI 也不能证明某一历史回合成功。

## Grok 1.0.30：现有成功证据不足，官方契约还明确否定该推断

现有受测 1.0.30 原生模型调用被额度错误阻断，没有正向 Stop 完成证据；`reason = end_turn`、响应非空和 `stopHookActive = false` 的组合只能作为候选通知条件。

额外核对官方仓库当前固定提交 `482711333c7195dc16a272777f86086d615e2afb` 的 [Stop 决定与报告说明](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-pager/docs/user-guide/10-hooks.md#stop-decision-control)：Stop 是可阻止结束的 gate；继续轮没有 UserPromptSubmit；被动通知无法分辨继续触发与最终触发。文档还列出 Stop 执行后发生中断、以及完成后写盘失败的情况。该文档不是受测 1.0.30 的二进制证明，不能据此新增版本能力，但足以否定无条件信任 Stop 的通用假设。

Grok `idle_prompt` 也会在错误或中断后发出，只能表明空闲。ACP 的 `_x.ai/session/prompt_complete` 名称不能当作成功，已记录的 `stopReason: error` 与后续 RPC 402 仍按原实现归并为一次失败。审批、实际运行取消和有成功历史的恢复仍保持门控。

## 修复范围和回归边界

- 三款普通 PTY 的 Stop 保存当前关联响应、清除旧审批摘要，并进入 `Unknown`；重复 Stop 不反复发出状态变化。
- 已通过现有会话/回合游标校验的后续 ToolComplete 可由 `Unknown` 恢复为 InProgress；当前审批和 StopFailure 仍可更新状态。旧会话、旧回合、重投保护保持原有行为。
- Grok 当前通知未透传原生 promptId，本次不补造关联或序号。其测试只证明保守降级，不声称补齐原生成功、乱序或跨回合关联验收。
- 其他 CLI 的 Stop 语义保持原样；明确失败/取消和既有成功终态仍不可被迟到普通回调覆盖。
- 仅修改会话状态模块及其测试，未修改插件内容或摘要、安装来源、托管 adapter、Fluent 文案。复用现有中英文 Unknown 文案，本项无需新增本地化键；状态含义的主文档和双语 GUI 验证由主代理同步。

新增测试 `correlated_stop_hooks_allow_same_turn_continuation_and_failure` 用随附通知的真实字段形状，经 `parse_event` 与 `update_from_event` 验证上述状态序列，不生成原生不存在的序号。`grok_stop_without_verified_completion_remains_unknown_and_can_continue` 覆盖当前 Grok 输出边界；`other_cli_stop_keeps_existing_success_semantics` 保留其他 CLI 对照。真实 dispatcher 的 listener 替换回归继续检查旧实例不能改变新会话，并将当前 Stop 的正向对照调整为 Unknown 和响应保存。

这些是确定性的应用状态回归，不是重新跑过认证模型的端到端证据。本次没有访问模型、用户 HOME 或 GUI 验证现场，也未自行运行 Cargo。通过固定原生执行器配合隔离的本地模拟模型响应，可以在不使用凭据/付费模型的情况下复验 hook 执行顺序；结果必须标为模拟传输的原生执行器验证，不能替代三款真实成功模型与完整恢复验收。仅登录失败、初始化或手工执行通知脚本不会到达真实 Stop gate，不能计为这个缺陷的原生复现。
