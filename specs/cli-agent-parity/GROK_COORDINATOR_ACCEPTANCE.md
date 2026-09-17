# Grok 根任务生产协调器验收夹具

本夹具使用 `App::test` 装配真正的 `LocalCLITaskCoordinator`、独立 SQLite 写入线程、正常 `grok::connect` 和生产监督链。source13 首轮真实官方模型验收通过，独立只读 SQLite 审计再次确认八条输入及完整历史结果关联。它不是 GUI 自动化，不证明桌面应用重启或重新关联活跃任务，也不计为最终同提交跨平台验收。

固定 Grok Build `1.0.30 (04b7ffed98c6)`，使用专用官方登录缓存和官方 `grok-4.6-build`。只允许根任务、`PermissionPolicy::Inherit`、`permission_ceiling=None`、`claude_profile=None`、`model=None`、空技能。当前 Grok 生产接口拒绝任何 `Some(local_tools)`，即使内部字段均为 `false`，因此传 `local_tools=None` 表达全关闭；不修改适配器门禁，不开放 SDK、父子任务或权限上限能力。任务使用现有 `LocalCLIManagedTasks` 开关；原生版本由隔离包装器真实检测，不靠安装模型的假版本替代。

固定最多 8 条原生输入，成功条件实际要求恰好 8 条，每条有唯一消息 ID、原生回合 ID、接收确认、运行事件、生产验证历史及 SQLite 终态。流程如下：

| 输入 | 流程 | 原提交代 | 执行代 | 要求 |
| --- | --- | --- | --- | --- |
| A | 新建第一轮 | 1 | 1 | 完整结果精确为 `GROK_COORD_ONE` |
| B | 第二轮 | 2 | 2 | 完整结果精确为 `GROK_COORD_TWO` |
| C | 唯一 Write 允许 | 3 | 3 | 精确路径和内容的 `AllowOnce`，原生完成、实际文件内容吻合 |
| D | 唯一 Write 拒绝 | 4 | 4 | `DenyOnce` 后原生取消，拒绝目标未产生 |
| E | 运行中追加的原执行 | 5 | 5 | 收到真实正文后派发 F，E 完成 |
| F | 后续回合排队指令 | 5 | 6 | 保留原提交代、原生关联及 ACK；完整结果精确为随机排队标记 |
| G | 真实取消 | 7 | 7 | 先有原生 ACK、运行和正文，再真实派发 Interrupt；完整历史终态为 `Cancelled` |
| H | 关闭后新进程继续历史 | 8 | 8 | 旧进程退出后加载同原生会话，未重投旧消息；显式输入才执行，完整结果为此前排队标记 |

F 保持第 5 代的消息记录；不能因为第 6 代执行而改写 `sender_generation`、`recipient_generation` 或 `submission_generation`。本流程的“取消后继续”是 G 清理后新进程显式继续历史，没有另测同连接的额外继续输入。追加为后续回合排队，不宣称同轮 steer 或 Claude `InputJoined`。审批派发、控制运输确认与本地 `ApprovalResolved` 的 `native_receipt=false`，不冒称单独原生审批 ACK。

每次生产运行事件提交后都经真实数据库读取审核。安全快照只含任务、消息、亲缘、代数、状态、回执、原生关联、正文长度及 SHA-256；不导出完整 `config_json`、权限观察树、模型正文、消息体或环境。当前和待发输入只接受 `message_id`、`submission_generation`、`runtime_generation`、`native_turn_id` 四键契约，身份必须为规范 UUID，提交代必须为正整数，未知键或类型直接拒绝投影。终态读取对应实际历史代，允许下一排队回合已经换代，避免以当前代暂时变化误判旧代未完成。每条 `input_submitted` 在派发前记录实际 `RuntimeAction::Submit` 序列化后的字节数和 SHA-256；运行器逐条核对实际 SQLite 消息体，避免误存正文仅凭数据库和最终快照相互一致而漏过审核。运行器在 libtest 退出后再通过 `sqlite3` 只读读取本次隔离数据库，复算结果和消息体摘要，要求与 Rust 最终快照逐字段一致。

运行器严格需要下一快照新增的 `GROK_NATIVE_FINAL_HISTORY_VERIFIED` 安全事件，字段为：`runtime_generation`、`session_id`、`turn_id`、`completion_watermark`、`outcome`、`full_output_bytes`、`full_output_sha256`。该事件只能在生产 `verified_final_snapshot` 成功、正文前缀一致后发出，不含正文。每个真实终态必须与该事件及 SQLite 结果摘要一一对应；仅出现 `end_turn` RPC、夹具 `acceptance_passed` 或退出码 0 都不足以通过。source11 没有此事件，不能作为该夹具已具备证明能力的快照。

恢复阶段先读取实际 SQLite 记录，再以 `SessionTarget::Resume` 启动新进程。`resume_ready_no_replay` 快照要求旧 7 条消息保持原字节摘要、原始代数及原生 ACK，任务为第 8 代 `Queued`，没有当前或待发输入、没有结果。随后单独提交 H。两次运行代均须经过生产 `confirmed_exit` 回执验证，分别记录清理确认、实际退出码 0 与 `stdio_closed`；断开、面板关闭或清理本身不作为任务成功。

官方沙箱、opaque `auth.json` 副本、精确 leader socket 规则及 TLS 白名单复用 [官方运行器](../../script/cli-agent-parity/run_grok_official_adapter_live.py)。认证来源必须是当前用户独占的普通私有文件，原生才解析隔离副本。网络只允许 `cli-chat-proxy.grok.com` 与 `auth.x.ai` 的 TLS CONNECT，不解密 HTTP；设定期限、TLS 连接数与字节预算，模型 HTTP 调用数与费用没有硬预算。无论通过与否都删除隔离认证副本、停止隧道。公开报告按已知事件、运行事件类型和嵌套对象的显式字段契约投影；未知事件和无效字段只用固定省略标记，未知字段直接删除，不能因正文形似 UUID 或 SHA 而放行。原生审批身份只直接保留规范 UUID、固定 Grok 数字或 UUID 请求身份，其他身份仅留 SHA-256。项目文件只公开 `approval-allow.txt` 和 `approval-deny.txt` 两个固定名称，其他相对名称只留数量和名称摘要；通过仍须实际文件集合精确为唯一允许目标，额外文件不能因过滤而漏检。原始诊断留在私有目录。

根代理注册新 Rust 模块时，需要在 `coordinator.rs` 添加：

```rust
#[cfg(test)]
#[path = "grok_coordinator_live_tests.rs"]
mod grok_live_tests;
```

精确测试名为 `ai::cli_agent_runtime::coordinator::grok_live_tests::real_grok_root_coordinator`。本次写入域没有修改模块注册、协议文件、工作流或 Cargo 门禁；这些集成操作由根代理完成。离线回归可运行：

```sh
python3 -B script/cli-agent-parity/grok_coordinator_runner_tests.py -v
```

真实运行命令需要包含该夹具与安全事件的冻结 libtest、同提交生产监督入口、固定原生二进制和用户授权的专用官方登录缓存：

```sh
python3 -B script/cli-agent-parity/run_grok_coordinator_live.py \
  --test-binary /absolute/path/to/frozen-libtest \
  --supervisor /absolute/path/to/same-commit-infinishell \
  --grok /absolute/path/to/fixed-grok-1.0.30 \
  --official-grok-home /absolute/path/to/private-official-grok-home \
  --max-acp-inputs 8 --timeout 900 \
  --output /absolute/path/to/new-private-evidence.ndjson
```

新增 27 项离线回归已通过，包括逐项缺失原生历史、假 `end_turn` 成功、重复与错代回执、应用历史冒充原生 ACK、排队消息改代、SQLite 结果或终态缺失、恢复重投、控制派发缺失、审批回执虚标、真实退出缺失以及公开字段契约。另有真实临时 SQLite 文件的只读投影回归，拒绝非纯文本输入、终态正文不符、当前任务与历史分离及权限边界变化；补充原提交摘要变化、额外关联字段、未知嵌套正文形似 64 位摘要、未知 UUID 字段和任意文件名 canary 的负向回归。模拟进程失败仍核对认证副本删除、隧道停止、来源摘要保留及任意文件名不导出。它们验证审核器拒绝路径，不证明模型、协调器生产运行或 GUI 通过。新增两项 Rust 纯投影回归随后纳入根代理 source13 冻结输入；[source13 局部门禁](OFFICIAL_13_LOCAL_GATES.md)记录实际编译、国际化 11 项、定向 1,199 项及 Python 231 项通过，本归档未重跑门禁。

没有新增或修改产品界面文案，无需本地化变更。最终同提交跨平台、SSH/tmux、GUI 双语布局、完整桌面重启、活跃重关联、故障恢复、父子结果回收及三方能力对齐仍为独立门槛，不能由本夹具通过代替。

## source13 首轮真实验收与独立归档

原运行器及 libtest 均正常退出，libtest 退出码 0、原 `acceptance_passed=true` 保留。真实流程经生产 runtime commands 执行，没有启用内部测试命令旁路。固定官方 Grok 模型的八条输入分别收到八条原生 `MessageAccepted`、八条 `TurnStarted`、八条生产完整历史终态，结果为 6 `Completed`、2 `Cancelled`；两次取消分别是 Write 拒绝和正文出现后实际 Interrupt。两条审批的运输派发与 `ApprovalResolved` 仍为 `native_receipt=false`，不冒称独立原生审批 ACK。

独立归档使用只读 SQLite URI、`PRAGMA query_only=ON` 和一致事务，按唯一任务 `f92b78ab-626f-4cc0-be67-f4dc4b15a39b` 与接收任务 ID 精确过滤，只读取该任务的当前记录、八代历史和八条消息。没有读取其他任务或复制、散列整个数据库。逐条重新计算原始 Submit 消息体 UTF-8 字节数与 SHA-256，核对派发前 `input_submitted` 记录及公开最终 SQLite 投影；实际结果与原生终态正文仅在内存中比较，公开产物只保留长度、摘要和关联 ID。八条生产 `verified_native_history` 的完整正文摘要、运行代、原生会话和回合均与真实 SQLite 对应，不能以 RPC `end_turn` 单独成功替代。

排队指令保持原提交第 5 代、实际执行第 6 代，消息的发送与接收代数均未改写。第 5 代历史仍保留当时已接收的后续输入关联，与第 6 代 current 关联完全一致；它是历史快照，不代表恢复时重投。旧进程正常清理后，以新运行代加载同一原生会话 `01a0afcf-18cf-73e0-9e35-7650b63db148`；ready 快照仍只有原七条消息，没有当前或待发输入、没有结果。随后显式第 8 条中文输入不包含完整答案标记，返回结果摘要精确匹配第 6 代的随机记忆标记。本轮继续属于新进程历史恢复，没有证明活跃任务重新关联，也没有额外验证同连接取消后继续。

两个精确运行代 `018a035a-471a-4216-8e09-605bbe58952f`、`5a445012-b8cc-4fae-8565-d972a2de4eb4` 的实际 `0600` 退出收据与公开记录逐字段一致，均为 `macos_resource_coalition`、退出码 0、`stdio_closed`、`cleanup_confirmed=true`。对应真实清理证明记录 job removed、资源 CID destroyed、native wait status 0、execution failed false。生产夹具通过 `confirmed_exit` 后才发布；本归档重新读取精确收据与清理证明并保留源 SHA，没有重新执行系统 PID/出生身份、job 或 CID 查询。原运行器确认代理线程停止，归档独立确认私有认证副本已不存在，未读取认证内容。唯一允许目标实际文件内容吻合固定夹具，拒绝目标不存在；私有原生配置只有固定 marketplace 初始化变化，权限段保持原始语义。

官方 TLS 共 15 条连接、9,721,731 字节，低于 32 条与 32 MiB 上限；开放 `auth.x.ai` 一条及 `cli-chat-proxy.grok.com` 十四条。190 次其他 origin 请求被拒绝，仅保留类别与 authority SHA，其中 6 次属于 `xai_api`、184 次 `unknown`，不猜测未知地址。未解密 HTTP，模型调用数及费用没有硬预算。OS 沙箱 canary 结果保留原运行器来源，本归档没有重跑 canary、CLI、模型或授权流程。

执行绑定 [source13 输入身份](validation/grok-coordinator-live-1.inputs.json)：73 文件中间脏快照 manifest SHA `419e6a6fac30237c4577061cb753078360fdf42e5833de29266363e3cdd5dd82`，libtest SHA `a857fbb82541ff572591bb9ab526a187e1a96325063636a7f72f928d6864e471`，生产监督主程序 SHA `0c9e05fe2c75aaf909e31c2dfb4508e5f7ea3ee9da6c7a8e167713ae8205d77f`。归档独立对照已有 source13 门禁/构建报告，没有重新读取大二进制计算散列。基线 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 不包含本轮未提交修改，不计为最终同一实际修改提交。

[原公开事件](validation/grok-coordinator-live-1.ndjson)、[原运行器报告](validation/grok-coordinator-live-1.metadata.json)与[原安全网络记录](validation/grok-coordinator-live-1.network.json)按原字节复制，分别为 134,348、3,289、37,862 字节。三份原件及安全派生产物通过七类定向凭据模式扫描，命中数均为 0；[独立审计](validation/grok-coordinator-live-1.audit.json)保留所有源映射、SHA、计数、逐消息摘要与未验证边界。另附 [SQLite 安全投影](validation/grok-coordinator-live-1.sqlite.json)、[退出收据安全投影](validation/grok-coordinator-live-1.exit-receipts.json)、[macOS 清理安全投影](validation/grok-coordinator-live-1.macos-cleanups.json)。派生 JSON 明确标记转换，不能称为私有原件。未归档或读取私有模型正文、完整原协议、API 环境或凭据；私有 stdout 与原始夹具文件 SHA 仅保留原报告字段来源，不冒称本归档重新核验。

本轮证明 macOS `App::test` 的真实生产根协调器、SQLite 消息和结果持久化、后续回合排队、审批、取消与同 ID 新进程继续历史。应用本地工具关闭、父子关系为空、权限策略为 Inherit；SDK、子任务、父权限上限、GUI 与桌面应用重启、SSH/tmux、Linux/Windows 和最终同提交三方验收均未由本轮证明，Goal 保持实施中。
