# Linux / Windows 无模型负向验收覆盖审计

审计日期：2026-09-16。读取工作区时的 HEAD 为 `6921a9925955a1955503e259cd935eaea4ac2ac0`；本记录审查的是当时工作区文件，不能作为该提交已经包含全部改动或已通过验证的证据。本轮只读产品代码、工作流和既有测试，没有执行 Cargo、CLI、模型请求或远端任务，没有修改 Rust、脚本及工作流。

依据：[PLAN.md](PLAN.md)、[CAPABILITY_MATRIX.md](CAPABILITY_MATRIX.md)、[VALIDATION_REPORT.md](VALIDATION_REPORT.md)、[cross-platform-preflight.yml](../../.github/workflows/cross-platform-preflight.yml) 及仓库跨平台验证技能。

结论：现有预检已覆盖大部分相关确定性测试，但不足以证明 Linux / Windows 真实 CLI 的最低负向验收。缺口主要是原生 CLI 与应用边界的组合验证，并非漏选整个协议、插件或消息模块。以下最多五项按执行优先级排列；“高 / 中”是补验优先级，不是 PLAN 的阶段编号。

## 后续落实状态

以下原始审计正文保留当时发现，不能再用其中的“当前没有”描述最新工作区。后续进展如下；全部仍为 dirty 工作树中间证据，Linux / Windows 尚未派发同 SHA 验证。

| 项目 | 最新状态 |
| --- | --- |
| 1. 原生缺失会话恢复 | 已修私有目录、随机代次与精确原生错误断言，build15 + bundle5 在 macOS 真实监督路径通过；无凭据 runner 与双平台 workflow 已接入，见 `validation/macos-codex-missing-session-supervised-1.ndjson`。 |
| 2. 原生插件事务 | macOS 原生注册表缺失、禁用、损坏来源失败及恢复已有独立证据；双平台固定 CLI 获取和原生 probe 已接入。GUI 另发现原生启动自动更新会覆盖安装缓存修补，已复现；受控持久来源已实现，build16 两项配置事务失败已修，build17 492 项回归通过；新 GUI 重启验收仍未完成。 |
| 3. 空闲原生 CLI 崩溃 | 已编译并在 macOS 通过仅真实 SessionReady 之后精准终止原生 Codex 的无凭据用例，断线和恢复拒绝有实证；macOS 完整清理仍未通过。双平台 workflow 已接入，尚未执行；详见 `CODEX_IDLE_CRASH_VERIFICATION.md`。 |
| 4. 旧 listener 排队回调 | 已新增真实双 dispatcher、实例替换、旧排队/迟到事件及有效终态重投组合测试，build15 `cli_agent` 470 项通过包含该测试。 |
| 5. Sent 后进程重启 | build16 已通过真实跨进程父测试（0.556s）：生产 SQLite 提交 Sent 后强杀、同库重开、原消息不重投、旧 ACK 拒绝/新 ACK 接受及结果领取唯一性。属于应用持久化故障注入，合成 ACK 不替代三方原生模型生命周期；其他平台仍待执行。 |

## 当前预检实际覆盖（原始审计快照）

双平台 `warp` focused 筛选已包含 `test(cli_agent)`、`test(local_cli_tasks)`、`test(local_cli_mailbox)`、技能、工具栏及编排模块。因此三方协议适配器、事件游标、插件管理器、协调器与相关 SQLite 测试均在筛选范围内。默认 nextest 仍跳过 `#[ignore]`；额外 ignored 步骤仅选择 `test(managed_process::live_tests::supervised_)`，使用同源码构建的 TUI worker。

双平台还安排 `command::managed`、共享 AI 动作 / 结果 / 技能、`warp_cli` 和 TUI 状态标签测试。脚本步骤覆盖 Grok Node hooks、Python 固定摘要文件事务、协议 payload；Windows 明确预检原生 Python、Git Bash / jq，缺依赖不会静默算通过。Linux 另有真实 Unix PTY 的 tmux 通知字节测试。

工作流当前没有安装三款受测 CLI，也没有原生 CLI 无凭据握手、原生恢复失败或原生插件事务步骤。脚本、Rust 夹具和通用进程监督通过，均不能替代这些边界。工作流已安排也不等于同一实际修改提交已经执行通过；最终 SHA 和 Actions 结果仍须另行收口。

## 1. 高：真实适配器恢复不存在的会话，先修正现成 ignored 测试的隔离和断言

对应 P0 / P4 / P5。现成测试名为：

`ai::cli_agent_runtime::codex::tests::live_codex_missing_session_is_not_replaced`

该测试在 [codex_tests.rs](../../app/src/ai/cli_agent_runtime/codex_tests.rs) 中已标记 ignored，当前双平台额外 ignored 筛选不会执行它。它不需要提交模型回合；受测 Codex `0.147.0` 的不存在会话恢复可以在干净配置目录验证。

执行前有实际夹具问题必须修正：它仅把 `cwd` 改成临时目录，仍继承 `options()` 中固定 `generation = Uuid::from_u128(1)` 和系统临时目录 `state_dir`，也没有自行隔离 `CODEX_HOME`。接入持久监督后，重复运行可能碰到同一代次的旧监督记录。当前断言只接受任意 `RuntimeError::Protocol`，尚不能排除另一种协议错误导致的假通过。

最小补验：私有 `state_dir`、唯一 generation、进程级干净 CLI 配置环境，显式提供受测 Codex 和同 SHA supervisor 路径；记录确已到达原生 `thread/resume` 的错误，断言未收到 Ready / Completed、没有新建替代会话，并检查监督终止结果。修正后可由主代理安排精确选择器：

```text
cargo nextest run --no-fail-fast -p warp --lib --run-ignored only -E 'test(ai::cli_agent_runtime::codex::tests::live_codex_missing_session_is_not_replaced)'
```

此命令尚未执行，不能在缺少上述环境或夹具修正时直接作为门禁。相关确定性测试已经安排：Codex `missing_resume_never_creates_a_replacement_thread`、Claude `captured_missing_resume_fails_before_ready_and_never_creates_new_session`、Grok `recovery_error_or_changed_identity_cannot_fall_back_to_a_new_session`。Claude 同类无登录原生失败证据存在，但没有对应 Linux / Windows 真实 Rust 适配器验收；Grok 空会话恢复探测需要已有 cached token，虽不消耗模型额度，也不能描述成无凭据 CI。

## 2. 高：原生插件缺失、禁用、版本不兼容与更新失败恢复，没有双平台事务证据

对应 P0 / P2 / P5。现有测试精确覆盖固定树和状态判定，例如：

- `terminal::cli_agent_sessions::plugin_manager::grok::tests::{missing_plugin_file_does_not_count_as_installed,native_disabled_state_wins_even_if_enabled_list_contains_plugin,stale_registry_version_does_not_hide_ineffective_update,recovery_snapshot_survives_removal_of_original_installation,only_tested_grok_and_supported_node_versions_pass_runtime_probe}`。
- `terminal::cli_agent_sessions::plugin_manager::notification_patch::tests::{runtime_version_requires_exact_probed_identity,minimum_claude_version_does_not_accept_old_stop_failure_gap,hook_manifest_and_notify_failures_restore_files_in_spaced_chinese_path,rollback_preserves_a_concurrent_user_edit}`。
- `terminal::cli_agent_sessions::plugin_manager::codex::tests::needs_update_via_trait_when_version_current_but_patch_missing`。

这些 Grok 测试由 `install_fixture()` 写入模拟 registry，不能证明原生 CLI 在 Windows / Linux 的安装目录、复制行为、卸载与更新回滚语义。Python 事务测试证明受控文件替换，也不等于原生 CLI 接受替换后的安装状态。

最小补验：仅在临时 CLI 配置和私有本地 marketplace 中，通过受测 CLI 执行缺失 → 安装 → 显式禁用 → 更新失败 → 原版本恢复；比对原生 registry、实际安装文件版本和摘要，验证禁用未被启用、无关配置未变。版本不兼容须验证拒绝发生在插件写入或模型启动之前；可使用固定未知版本输出的执行夹具，明确标成应用预检测试，不冒充另一个真实 CLI 版本。

这组原生插件命令不要求 Claude 登录或 Grok 模型额度。已有 [probe_codex_hook_trust.py](../../script/cli-agent-parity/probe_codex_hook_trust.py) 可无凭据验证真实安装后的五个 hooks 仍是 untrusted、配置没有被自动授信，但尚未被当前预检调用。Windows 原生 hooks / 启动器候选验证正在独立进行，不应重复派发；即便该项通过，也不能替代完整更新失败恢复事务。macOS 既有原生证据不能记作 Linux / Windows 通过。

## 3. 高：真实 CLI 空闲崩溃与 supervisor 接线，尚由通用进程替身代替

对应 P0 / P2 / P4 / P5。预检显式运行的四个真实监督测试位于 `ai::cli_agent_runtime::managed_process::live_tests`：

- `supervised_native_nonzero_exit_code_is_preserved`
- `supervised_finish_stops_root_and_descendants_and_missing_receipt_blocks_resume`
- `supervised_host_sigkill_stops_root_and_descendants_before_receipt`
- `supervised_detachment_obeys_platform_containment_boundary`

它们确实运行生产 worker、检查退出和后代，但原生进程是 libtest 夹具。它们尚未覆盖真实 CLI 可执行文件 / 启动包装器、握手和标准流关闭之间的组合，尤其 Windows 原生可执行文件与脚本启动器差异。

最小补验：用精确受测版本与私有配置，只初始化真实 CLI，分别关闭宿主控制流、终止 CLI 根进程；核对适配器退出事件与诊断回执，没有 Completed，未确认退出时不能恢复。对照已有 [probe_process_eof.py](../../script/cli-agent-parity/probe_process_eof.py)：该脚本只验证 Codex / Claude 的直接原生空闲 EOF，没有生产 supervisor、父进程强杀或运行中工具，当前也未进入预检。不能原样运行后扩大其结论。

Codex / Claude 无模型初始化不需要登录；Grok 若在认证前拒绝继续，应准确停留在握手 / 拒绝边界，不上传本机 cached token 补 CI。此项只补空闲原生连接与异常退出路径，不能代替运行中工具树清理验收。macOS 已知进程树限制沿用既有报告，本审计不重复研究或把它计作新缺口。

## 4. 中：旧 listener 排队回调与真实通知入口的组合缺少直接回归

对应 P2 / P5。以下确定性测试已被 `test(cli_agent)` 覆盖：

- `terminal::cli_agent_sessions::event_cursor::tests::{native_turns_reject_old_completion_and_replayed_prompt,local_input_closes_previous_turn_before_native_prompt_arrives,stale_turn_does_not_consume_current_event_sequence}`。
- `terminal::cli_agent_sessions::tests::stale_timer_callback_after_disarming_event_does_not_overwrite_newer_status`。
- `terminal::cli_agent_sessions::listener::tests::codex_try_parse_ignores_osc9_when_plugin_already_active`。

当前 [listener/mod.rs](../../app/src/terminal/cli_agent_sessions/listener/mod.rs) 在事件订阅回调中比较 listener 实例 ID，防止被替换的实例更新新会话。现有 listener 测试主要直接调用 parser / handler；事件游标测试直接构造事件。未找到实例替换后真正投递旧订阅回调、再由新 listener 接收当前回合事件的组合测试。旧取消定时器与旧 listener 不是同一路径。

最小补验：应用测试框架创建真实 dispatcher 和 listener，排队旧终态 → 替换会话 / listener → 发送当前 PromptSubmit → 释放旧回调，断言新会话仍运行、序号未被消耗；再投递当前回合终态，验证一次有效完成。使用既有协议 payload 即可，不需要任何模型账号。尚无此组合测试名，应新建后纳入现有 `cli_agent` 筛选，不能把建议名称当作已存在。SSH / tmux 字节透传与 Windows hooks 候选验证不能单独证明应用订阅替换正确。

## 5. 中：已发未确认消息在进程重启后的故障注入，尚未形成独立组合验收

对应 P3 / P4 / P5。现有测试已覆盖消息与存储不变量：

- `ai::local_cli_mailbox::tests::{local_mailbox_keeps_unconfirmed_dispatch_without_automatic_retry,local_mailbox_ignores_non_mailbox_commands_and_rejects_wrong_run_receipts}`。
- `persistence::local_cli_tasks::tests::{local_cli_task_recovery_marks_disconnected_without_resending_messages,local_cli_messages_deduplicate_without_reverting_acknowledgement,local_cli_new_generation_does_not_receive_old_queued_messages,local_cli_result_claim_is_atomic_and_rejects_ordinary_or_modified_messages,local_cli_failed_checkpoint_rolls_back_and_does_not_acknowledge_success}`。
- `ai::cli_agent_runtime::coordinator::tests::resume_claims_unstarted_generation_but_rejects_incomplete_process_records`。

例如 `local_mailbox_keeps_unconfirmed_dispatch_without_automatic_retry` 在同一 writer 生命周期中让派发闭包返回连接错误，再调用第二次派发检查不重投。它不覆盖另一个应用进程重新打开 SQLite 后的行为。既有 GUI 重启成功会话证据也不能替代“持久化 Sent 后、原生 ACK 前”这个失败窗口。

最小补验：独立进程在真实 SQLite 提交 Sent 后、ACK 前被终止；新进程打开同一私有状态目录，检查原 message ID 保持未确认、没有自动再次派发；随后送入旧 generation 的 ACK 与当前 generation 的合法 ACK，核对各自接受边界。结果领取也应跨重启保持唯一；故意损坏恢复回执时应拒绝继续且保留旧结果。全部可以使用确定性协议端点，不需要 Claude 登录或 Grok 额度；必须标为应用持久化故障注入，不能替代三方真实模型生命周期。尚未找到现成跨进程组合测试，当前工作流也未安排独立此类步骤。

## 本轮范围与验收记账

以上建议不要求向 GitHub Actions 新增模型密钥，不请求额外 Claude 登录，不重复 Grok 402。原生 CLI 获取失败、Git Bash / jq 缺失、版本不匹配或产品平台限制都须记录为具体阻塞，不能通过跳过来改为成功。最终同 SHA 验证应分别保留确定性测试、原生无模型边界与真实模型 / 产品验收三类证据。

本轮只新增本验证记录，无产品可见文本变化，无需本地化变更；未执行任何新增验收，也未把上述缺口标记为完成。
