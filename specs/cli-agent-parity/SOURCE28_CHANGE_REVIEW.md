# source28 改动独立只读交付审计

结论：**可以安全提交本次 11 文件的诊断与回归检查点**，未发现提交前必须修复的正文泄露、请求 ID 放宽或失败转成功问题。本结论只覆盖这 5 个 Rust 文件和 6 个 Python 文件的实际 diff，不覆盖其余待交付材料，也不替代新提交上的 macOS 原生、Linux、Windows 或 SSH/tmux 验收；不能据此将整 Goal 标记完成。

独立核验了主树 HEAD 为 `436cc739234061c092cedd106432bbbfa3c2d645`。`macos-official-28-inputs.json` 的 SHA256 为 `7bfe731c3a40207d8d428702deac95a8fa4fd9d6b603312c8ef4cb0310f6acd7`，111 个条目在当前主树、candidate28 与清单中全部一致；其中未改 100 项的哈希还分别与 436 父提交的 Git 对象一致。candidate28 的已跟踪修改集合恰为这 11 项；限定 11 文件的 `git diff --check` 返回 0。逐项 SHA 和 diff 摘要记录在配套安全 JSON。用户原有两个修改文件不在清单中，未读取其正文，也未修改。

| 核验范围 | 具体实现与回归 | 判定与证据边界 |
| --- | --- | --- |
| SDK 响应信封与公开诊断 | grok_sdk_origin_live_tests.rs:380–475；run_grok_sdk_origin_probe.py:417–490；Rust 回归1887–2064，Python新 TransactionEnvelopeDiagnosticTests | 信封只接受无 method 的单个响应，把精确 ID 留在私有文件；result 只保留最多128个键的类型/长度/SHA以及字段值类型，绝不复制 result、工具或模型字段值。Unix目录核UID/0700，独占文件核0600、普通文件、无链接及nlink=1，写前复核空文件，预算64KiB、只捕获首次；Python用目录fd与O_NOFOLLOW再核，拒绝重复字段、多记录、非有限ID、额外正文及预算溢出。公开元数据只保留文件SHA/字节/状态并核账本一致；捕获失败使probe及origin失败关门。非Unix没有权限证明，明确拒绝，未冒充Windows验收。 |
| 实际写出事务的记录 | grok.rs:194–203、446–453、607–625、1566–1577；grok_sdk_origin_live_tests.rs:252–266 | 仅 write_message 成功后记录请求身份；不保存参数、提示、审批入参或MCP结果。任意非白名单method、字符串ID与inner ID只记录形状摘要。新增逻辑在cfg(test)中，事务上下文不改变生产协议行为；初始化的临时变量等价保留原写入顺序。 |
| 请求与重放关联 | grok.rs:1621–1645保持原生产守卫；grok_policy_preflight_live_tests.rs:459–490、717–753、905–923及新增协议回归 | 生产仍只接收已关联的数字响应ID。调查账本只在实际写入/flush成功后建立当前pending；未知、未来、字符串或无归属ID拒绝且不消费pending；完成响应保存完整帧指纹，精确重放不完成新RPC，冲突重放拒绝。drain中的迟到响应只闭合已发事务，不能补成恢复或权限证明；公开runner仍核generation/RPC身份，异常非重复drain不得通过。 |
| 权限调查主次失败 | grok_policy_preflight_live_tests.rs:296–305、554–588、900–973；run_grok_policy_preflight.py:196–207、237–284、559–565及协议/Python新回归 | 主语义失败、cleanup失败和drain失败分别只保留固定阶段/长度/SHA；phase_outcome优先返回主失败，再检查cleanup及drain。StopRequested或清理成功不能升级为正常stdio完成，成功主路径仍必须满足真实退出回执。未知通知只保留形状且原严格处理器仍拒绝；phase_failure存在时伪造成功finish也不得通过。policy结果持续unknown，不把零模型请求、身份清理或摘要视为父权限上限证明。 |
| CI12 Linux/Windows离线目录回归 | grok_coordinator_runner_tests.py:454–472；run_grok_coordinator_live.py:477 | 仅fixture替换mkdtemp到自身TemporaryDirectory，并断言传入仍为/private/tmp；生产macOS原生runner固定目录未改。没有靠创建跨平台/private/tmp目录掩盖问题。 |
| CI12 SQLite关闭与路径哈希回归 | run_grok_coordinator_live.py:83–96、165–176；grok_coordinator_runner_tests.py:356、373–398 | 只读projection采用contextlib.closing，fixture写事务完成后也真实close；保留真实连接引用，成功和JSON异常后均用SQL调用证明已关闭并删除数据库，避免GC掩盖Windows锁。文件名哈希保持生产str(relative_path)规范，只把fixture expected改成str(Path(...))使用当前平台分隔符，canary数量/哈希/正文拒绝断言保留。 |

一个非阻断的诊断可读性边界：生产 `grok.rs:765` 发出的关闭方法为 `session/close`，而新增 Rust `transaction_method`（grok_sdk_origin_live_tests.rs:374）及 Python `TRANSACTION_METHODS`（run_grok_sdk_origin_probe.py:58）列出的是 `x.ai/session/close`。实际 `session/close` 因此走安全形状/SHA fallback，不能在公开账本直接看到固定方法名；仍能看到pending_kind=close_session。这不影响生产关闭、保密或成功判断，不要求在本检查点顺手改动。若后续需增强诊断，可只对齐真实固定方法名并补离线回归。

执行边界：本子代理只读代码diff、公开清单及候选源码字节/Git对象，只创建本审计文档和安全JSON；未读取私有artifact、认证或模型正文，未操作GUI，未运行Cargo、native CLI或模型，也未作Git变更。根代理明确报告source28已通过cargo check、i18n11、Rust1410、Python426；本子代理没有重跑或读取这些原始测试输出，统计按根代理已执行事实标注来源，不能算作本子代理独立执行。无需本地化变更。本提交只增强失败定位与修复离线跨平台fixture，不代表Grok SDK业务来源或权限接口已修复。修后同提交跨平台验收仍待执行；CI12原失败结论不得回填。
