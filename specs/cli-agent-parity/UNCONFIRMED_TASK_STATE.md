# PTY 结果未确认与本地持久状态

## 原因与范围

原生 CLI 的 `Stop` hook 可能先于后续 hook 决策执行，不能仅凭 `Stop` 锁定成功。会话模型将该阶段显示为 `CLIAgentSessionStatus::Unknown`，但旧 `local_tasks::apply_update` 会把它及缺少完成证据的 `Success` 写为 `Disconnected`。此时交互 PTY 与本地任务绑定仍可活跃，后续消息也仍属于同一运行，所以持久状态的“连接中断”并不准确。

实际产品路径是 `launch_local_harness_child` 在 `LocalCLIManagedTasks` 未启用时，通过 `bind_local_task` 绑定兼容 PTY 子 pane。启用托管模式的 Claude/Codex 通过协调器写状态；普通手动终端不会自动调用该绑定。本问题不是所有终端或全部托管任务的共同结果。当前恢复入口仍要求有效启动配置和退出证明，不能由错误状态推断已经发生重复执行。

`LocalCliTaskState::Unknown` 带 `#[serde(other)]`，专用于未来版本/不兼容记录。持久化和消息校验明确拒绝它；本次不改变这一安全边界。

## 明确的生产状态

新增可序列化值 `Unconfirmed`（`"unconfirmed"`），含义为“结果待确认，当前运行仍占用原生会话”。它不是成功、失败、取消或连接已中断，也不证明 CLI 正在输出。

- PTY `Unknown`、无可靠完成证据的 `Success` 写入 `Unconfirmed`；保存已有原生 session ID 与响应供核对，不把 `Stop` 附带事件信息写成真实完成证据。
- `is_active()` 包含此状态，`is_terminal()` 不包含。原生会话唯一索引继续保留占用；同任务的新 generation 必须被拒绝。
- 后续真实 Prompt/审批/失败/取消事件可以在同一 generation 更新。明确移除会话时记录 `Disconnected`；应用启动恢复的 `load_tasks(true)` 才将尚占用的旧运行降为 `Disconnected`，并保持不启动进程、不重放输入。
- `load_tasks(false)` 保留原值；未确认任务不能生成最终结果。仍活跃的未确认父运行可以领取已经确认的子任务结果，但领取只是 `Sent`，不能伪造原生 ACK。
- 已有消息的 ID、generation、Queued/Sent 和回执来源保持不变。实际派发还需要真实 endpoint 的 ready/connected 状态；此状态不会创建接收者、替代 ACK 或自动重投 Sent。
- 任务管理器和会话桥单独显示结果待确认。启动进程式“继续”拒绝此状态，提示检查现有终端；重新查看现有 pane 与启动新原生进程是不同操作。

## 数据迁移

保留 `2026-09-16-000000_add_local_cli_tasks`，新增 `2026-09-16-000001_include_unconfirmed_local_cli_sessions`。迁移只重建 `local_cli_tasks_active_session` 索引，在已有 queued/running/waiting_for_user 谓词中加入 unconfirmed，不改表结构、原始任务 JSON、消息或历史记录。

向下迁移先检查没有 `unconfirmed` 活动记录。若存在则在迁移事务中失败回滚，保留原索引和运行占用；不把未知结果转换成结束、断开或运行成功来迁就降级。清理条件满足后才恢复旧索引。

## 验证记录

新增/更新的 Rust 回归使用实际 SQLite 与应用 writer，覆盖：

1. `local_cli_session_cannot_persist_unverified_success`：无证据成功保存为 Unconfirmed。
2. `unconfirmed_pty_stop_preserves_the_current_run_until_a_real_followup_event`：Running → Stop/Unknown 保存响应但无终态证据，后续审批、运行和失败保持同代。
3. `explicit_pty_disconnect_is_distinct_from_an_unconfirmed_result`：明确断线才记录 Disconnected。
4. `local_cli_unconfirmed_run_keeps_native_session_and_generation_ownership`：新状态序列化、活动占用、同会话重复关联和新代拒绝、无最终结果。
5. `local_cli_unconfirmed_restart_changes_only_connection_state_without_replaying_messages`：关闭真实 SQLite 连接后重新打开，普通读取保留 Unconfirmed；启动恢复才降级，原 Sent/Queued 消息不变、重投不重置、错误 generation ACK 拒绝。
6. `local_cli_unconfirmed_parent_can_claim_a_verified_child_result_only_once`：父运行未结束时领取真实子结果一次，状态仅变 Sent，无伪 ACK。
7. `local_cli_future_unknown_still_rejects_messages_and_receipts`：保留未来未知状态的写入/消息/回执屏障。
8. `local_cli_unconfirmed_index_migration_upgrades_existing_rows_without_rewriting_them`：通过 Diesel 的真实迁移事务升级已有任务/消息，校验占用及失败降级回滚。

本子任务没有自行运行 Cargo，以上 Rust 回归等待主代理统一门禁；不能将测试代码已落地视为通过。任务管理器、会话桥及中英文说明由主代理在独立写域同步验证。

已对 bundle7 的运行中隔离 GUI 测试库使用 SQLite `mode=ro` + backup 取得一致副本，仅在私有副本执行新迁移 SQL。该旧库已应用 `20260916000000`，其中本地任务/历史/消息三表均为 0 行，因此不能把本次 GUI 库检查宣称为有用户任务的升级验收。副本上另建明确的临时索引夹具，确认 Unconfirmed 排斥同原生 ID 的新任务；有未确认行时降级失败、事务回滚后索引保留；无未确认行时降级成功，重新升级后 `PRAGMA integrity_check=ok`。这是本机 SQLite SQL 证据，不是 Diesel/新 GUI 启动或跨平台通过证据。

脱敏结果见 `fixtures/unconfirmed-gui-sqlite-index-migration.json`。原 GUI 库及 WAL 未修改，也未复制账号配置。

## 原生文件名负向测试的精确边界

此前 `non_utf8_filename_cannot_enter_the_controlled_tree` 在 macOS 的 `fs::rename` 阶段返回 `EILSEQ (92)`，尚未进入产品校验。该原生路径回归现仅在 Linux 执行；不将 macOS 文件系统拒绝或未执行计为产品已经验证非 UTF-8 文件名。Unix 字面反斜杠负向回归继续覆盖 macOS/Linux，并保留文件名原始组件语义。
