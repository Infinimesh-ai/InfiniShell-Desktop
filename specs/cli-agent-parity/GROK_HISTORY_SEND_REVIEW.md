# Grok 历史恢复后的发送只读复核

本次未发现足以证明生产代码缺陷的因果链，前两次点击未生效的原因为 **unknown**。根代理随后报告：编辑器聚焦、输入一个空格再撤销、第三次点击 Send 后，第 7 条消息获得原生 ACK 并开始第 7 代回合，草稿清空，未重复投递。这证明该现场恢复连接最终能够发送新输入；不能证明前两次鼠标操作已经派发 Send action，也不能把鼠标命中或焦点问题认定为根因。第 7 回合最终结果未纳入本次证据。

现场依据仅为根代理的两条明确状态消息：应用为 `436cc739234061c092cedd106432bbbfa3c2d645` / source26；历史会话断开后 ContinueHistory 使用同一原生会话，恢复第 7 代处于 queued + ready，原消息总数 6；前两次点击未新增消息或模型输入；第三次操作后原生 ACK 与回合启动发生。本子代理没有操作 GUI，也没有读取私有日志、数据库、认证信息或模型正文。源码为当前工作区的只读快照，逐文件哈希见配套 JSON；未独立确认这些工作区字节与运行中 source26 二进制完全一致。

| 调用环节 | 精确源码位置 | 结论 |
| --- | --- | --- |
| Send action 派发 | task_manager_view.rs:339–345、1837 | 按钮回调派发现有 Send action，随后调用 send_input。视觉蓝色本身不能证明鼠标 action 已到达此处理器。 |
| 按钮启用 | task_manager_view.rs:1178–1219 | 要求 connected + ready，且没有 pending、Claude 队列占用、空输入或 preparing；queued 任务状态本身不禁止发送。 |
| 获取发送目标 | task_manager_input.rs:486–501 | 捕获当前 task ID、任务代数、harness 与 active turn；没有要求恢复后的任务必须已 Running。 |
| 准备输入 | task_manager_input.rs:504–565 | 准备中的输入、图片处理、已有 pending 会明确报错；后台完成后仅在 token、input_generation 或 draft_key 失效时静默丢弃旧回调。 |
| 动作后的输入刷新 | task_manager_view.rs:1944；task_manager_input.rs:136–197 | refresh_managed_input 更新图片能力与技能列表，不改变 input_generation、draft_key 或 preparing。Editor 的图片选项 setter（editor/view/mod.rs:3313–3320）只赋值并 notify；Dropdown::set_items（view_components/dropdown.rs:486–490）只更新列表并 notify。不能据此构造每次 Send 都让自己的回调过时的因果。 |
| 明确使输入代数失效的入口 | task_manager_view.rs:248–257、514–553、1730–1743、1761–1773 | 目录编辑或替换、选择任务、空任务的权限与能力切换、选择 CLI 会更新代数并取消旧准备。是否在前两次点击之间发生这些动作没有证据。 |
| 发送前二次检查 | task_manager_view.rs:847–884 | 再确认连接、pending、非空输入、task ID、任务代数及 harness 一致，才把请求送入 coordinator；此处失败会保留草稿并显示错误。 |
| 历史恢复不重投 | task_manager_input.rs:480–481；task_manager_view.rs:812–813、830–841；coordinator.rs:943–954 | 恢复不携带 initial_input、不新增 WaitingReady pending；协调器更新 runtime_generation 并删除旧 Grok pending/current 关联，历史输入不会自动重投。 |
| queued + ready 接收输入 | coordinator.rs:685–725、1195–1235、1398–1474 | 普通用户请求不要求 task 已 Running；有效恢复后 Submit 会保存关联与消息，然后交付原生控制器。邮箱消息的 active 判断属于另一路径，不能用它推导普通 Send 被拒绝。 |
| Grok 就绪与提交 | grok.rs:1723–1764、776–840 | session/load 回复核对同一会话 ID 后发布 SessionReady；文本 Submit 可立即启动或排入后续队列，存在控制请求、关闭状态等才拒绝。 |
| ACK 后清草稿 | task_manager_view.rs:974–987；task_manager_input.rs:675–704 | 只有匹配 message ID 的 ACK 才触发清理；重新编辑的 revision 或新附件不会被旧 ACK 清除。 |

未提出生产逻辑修复。最小进一步取证应是公开的有限状态：Send action 是否进入、准备回调是成功/失败/过时、发送目标代数是否匹配，以及是否存在 pending；不能记录输入正文、任意错误正文或原生结果正文。若再次稳定复现，先补能证明 action 确实到达的证据，再决定修改点，保留现有过时回调与原生 ACK 守卫。

有意义的回归设计：用现有 manager_view_with_records 与 composer_snapshot 测试基础，加入同任务历史恢复的 queued + ready 快照，验证非空纯文本只能生成一次 Submit；模拟在准备完成前切换任务或改变输入代数，验证旧回调不能发送；模拟重复点击与 pending ACK，验证消息不重投且草稿只在匹配 ACK 后清空。已有 task_manager_view_tests.rs:38–57、114–121、322–364 分别覆盖恢复身份/代数、Grok Submit 动作及 ACK 草稿 revision，但未直接证明完整恢复 Send 链或鼠标命中。此处仅提出设计，未新增或运行测试。

执行边界：只读源码与 GUI 指南；仅创建本专报和安全 JSON。未修改生产源码、冻结树、目标目录、主三文档；未运行 Cargo、CLI、模型、网络、GUI、Git。无需本地化变更。本次结论不是三方全链、父权限上限或跨平台验收通过。
