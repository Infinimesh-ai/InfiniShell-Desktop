# Claude 固定文件策略的父子协调器验收夹具

2026-09-17：第 1 轮真实 API 验收失败；第 2 轮生产夹具和两份清理回执均通过，但原运行器误拒绝初始化就绪事件。修正审计后，对第 2 轮未改动的私有全树和公开投影复核均通过。离线运行器测试 56 项通过。此证据只覆盖下述父子生产协调器流程，不能将 GUI、完整重启恢复或 Goal 标为完成。

本夹具复用 `warpui::App::test` 的真实前台异步执行器、正式 SQLite 初始化与写入线程、`LocalCLITaskCoordinator::start_with_input`、原生 Claude stream-json 适配器和生产进程监督入口。测试安装快照仅固定子任务路由到明确指定的原生文件，原生文件摘要、版本、固定策略与 MCP 初始化仍由正式连接路径核验。App 的窗口和平台服务为测试实现，因此不计为 GUI 验收。

## 运行链路及通过条件

1. 父任务以 `ClaudeRestrictedFilesV1` 新建，启用明确的本地派发和消息权限。真实模型调用 `run_agents`，生产工具执行器派发唯一 Claude 子任务。子任务必须保存创建时父代 1 的完整权限上限，继承完整固定策略，拥有独立原生会话 ID。
2. 子代 1 读取临时项目中的 `child-approved.txt`，请求将唯一内容 `CHILD_BEFORE` 改为本次随机标记。主机暂留这次 `Edit` 审批。
3. 父模型通过原生 MCP 调用 `send_message_to_agent`。主机仅在子任务确实等待上述 `Edit` 时允许该调用。消息必须在子审批允许前以 `Acknowledged`、`native_protocol` 落库。原生 CLI 可在后续工具轮之间将输入折入当前执行，也可在其结束后开启独立执行；必须按真实事件区别记录。
4. 子任务处理追加输入，通过原生 MCP 向仍活跃的父代 1 发送进度指令，发送结果必须对应同一个已持久化的消息 ID，接收确认必须为 `native_protocol`。父任务同样按原生事件记录合并或独立执行，不能仅从 ACK 推断消息已执行。
5. 父模型调用一次 `inspect_local_tasks`。主机持有其审批直到子任务的追加输入真实完成。持久化的原生工具结果必须包含实际子任务 ID、实际代数、`completed` 和随机最终结果标记；父代 1 必须输出回收标记。若进度加入父代 1，同一最终输出还必须包含进度标记。
6. 每个真实完成的子代产生唯一 `local_task_result` 自动通知：子任务同轮合并时一条，分成两代时两条。主机不追加人为消息替代它们。每条结果消息须拥有由实际子任务 ID 和代数生成的确定性消息 ID，其 JSON 正文须逐字对应该代真实结果和原生终态证据，接收必须以 `Acknowledged`、`native_protocol` 落库。父任务收到通知后输出随机 `PARENT_AUTOMATIC` 标记；通知合并到正在执行的父回合时保留既有 inspect 和进度要求，不触发额外工具。
7. 两条初始输入、追加指令、进度消息和一至两条自动结果通知合计 5–6 条实际输入；独立原生执行总数为 2–6。独立执行必须有 `TurnStarted` 和真实终态证据；合并输入必须有 `InputJoined {message_id, turn_id}`，SQLite 配置 `claude_joined_inputs` 保存相同输入 ID、活跃执行 ID、提交代数及真实 `Completed` outcome。每条输入的完成必须逐一对应，合并输入的完成先于其活跃执行完成。原生 stdout 的真实 `result` 必须完整列出该执行所消费的全部 `user_message_uuids`，仅最后一个 UUID 不能证明整批完成。四个 MCP 调用必须有成功的持久化结构化结果，审批只能覆盖明确参数的五次调用；文件内容仍须逐字一致。

   首次 `SessionReady` 可出现在原生会话 ID 尚未关联时，仅限代数 1、此前没有该任务的已关联事件、固定完整策略与摘要匹配、模式为 `plan`、`fixedProfileVerified=true` 且 `sessionAssociationConfirmed=false`。这一初始化事件不证明输入执行或完成；其 runtime generation 必须匹配。后续 ACK、加入批次、started、finished 和所有已关联事件仍必须携带实际原生会话 ID。
8. 所有实际输入及执行确认完成后，正式 `Shutdown` 关闭两个仍存活的连接，校验生产退出回执的实际 runtime generation、原生进程容器及 `cleanup_confirmed`，再关闭 SQLite 写入线程。未确认清理、原生工具失败、额外子任务、旧代回调或缺失终态均不能通过。

父模型的实际 `inspect_local_tasks` 回收与自动结果消息是两条独立证据链，必须分别满足审核。不能用成功的 inspect 回收替代自动消息的原生 ACK 或实际执行结果；若单一待处理输入限制令进度与自动结果并发交付未确认，本轮必须失败并记录，不能跳过或重投掩盖它。

## 隔离和预算

运行器仅接受显式的 Anthropic API 环境 JSON，沿用认证键白名单，Unix 下要求该文件仅当前用户可读写。每次新建私有 HOME、Claude 配置、临时项目、SQLite 库和监督数据域，不读取或复制用户原生登录凭据，不写用户全局 CLI 配置；原生 CLI 仅可写此次私有会话历史。

六条应用输入和六个独立原生执行是上限，实际通过数量按 SQLite 结果消息与原生输入图计算；同轮合并时五条输入不会被误当成缺少第六条。四个 MCP 调用和五次允许审批仍为固定行为预算。测试期限为 450 秒，运行器在 600 秒超时终止测试进程并记失败。输入或执行次数不是 HTTP 请求数或费用上限，模型工具循环可能产生更多实际模型请求；报告保持 `http_request_count_verified=false`。异常退出的清理只有真实生产回执才能确认，不能从删除凭据或测试进程退出推断。协调器断开可能早于监督回执落盘，因此夹具在期限内等待真实回执，清理失败另行记录。

临时项目同时携带明确的 `Read` 拒绝规则，以校验策略来源和继承。该夹具没有执行被拒绝的 Read，不能计为此拒绝规则的实际拦截证据。固定文件工具策略也不是操作系统文件或网络沙箱，相关边界沿用 [固定文件策略说明](CLAUDE_FIXED_FILE_POLICY.md)。

## 执行

先把此夹具和两个测试钩子纳入同一冻结输入，按仓库门禁重新编译 libtest 与主程序监督入口。由根代理完成 Cargo 与真实 API 执行；子代理仅完成离线运行器检查。

```sh
python3 -B /Users/zhishi/Tools/github/InfiniShell-Desktop/script/cli-agent-parity/claude_coordinator_runner_tests.py

python3 -B /Users/zhishi/Tools/github/InfiniShell-Desktop/script/cli-agent-parity/run_claude_coordinator_live.py \
  --test-binary /absolute/path/to/warp-libtest \
  --claude /absolute/path/to/fixed-native-claude-2.1.273 \
  --supervisor /absolute/path/to/same-input-infinishell \
  --api-environment-file /absolute/path/to/private-api-environment.json \
  --model claude-sonnet-4-6 \
  --output /private/tmp/infinishell-claude-coordinator-1.ndjson
```

真实测试名称：

```text
ai::cli_agent_runtime::coordinator::claude_live_tests::real_claude_fixed_profile_parent_child
```

公开证据由 `.ndjson`、`.metadata.json` 和 `.test-output.txt` 组成。完整 `coordinator.raw.ndjson` 与 `coordinator.raw.test-output.txt` 只留在新建私有工作目录，文件权限为 0600。运行器先审核私有全树，再审核公开投影，两者同时通过才可显示成功。公开记录保留完整固定策略、工具权限、父代关系、输入合并关联与 outcome，去除巨大的 `permissionObservation.settings`；保留原始记录、配置、权限观察和文件 SHA-256 供私有审核关联。运行中重复的 task 仅公开身份和策略摘要，嵌套的 inspect JSON 正文也遵循同一投影。公开路径与 API 环境值均先替换；禁止归档私有原始证据、原生凭据、原始配置或密钥。真实交付须追加测试二进制、监督入口与原生 CLI 摘要、冻结输入清单、完整通过或失败结果，再对包含实际修改的提交完成相关平台验证。

## 第 1 轮失败事实

真实记录 `/tmp/infinishell-claude-coordinator-live-1.ndjson` 对应基准提交 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 加工作区改动，测试退出码 101，耗时 28.10 秒。父模型成功通过原生 MCP 派发唯一子任务，随后发送追加消息；子任务在 `Edit` 等待审批时收到真实 `native_protocol` ACK。允许 `Edit` 后原生工具返回成功，但原生在首执行仍活跃时直接为追加命令发布 `started`，此前没有旧执行的 `completed`、`cancelled` 或任何 `result`。生产协调器按原先“只能独立下一轮”的假设拒绝这次重叠事件，报 `cli-agent-claude-queue-uncertain`。

这一轮证明派发、运行中原生接收和批准文件工具可用，不能证明进度、最终结果、继续或恢复。两任务仍在代数 1 断开，真实 `result` 数为 0，不计为成功。私有生产退出回执随后确认两个原生资源容器均已清理；原始 NDJSON 未包含这些回执，不把后续诊断变成原轮通过证据。

失败 NDJSON 原文件 SHA-256 为 `b3aa12a74ba9467bf51d9aff5a0f6b39ec944b810bad30e940f3a5b10a77137f`。另生成只含投影的 `/tmp/infinishell-claude-coordinator-live-1.projected.ndjson`，27 条记录从 365,053 字节缩为 34,140 字节，SHA-256 为 `52ea025e5ea7958143b57a03703b149bc71f347a2c8fa15f24ed62141f4b97b5`；明确保留 `acceptance_failed`。第 1 轮结束时，上述新合并语义尚未获得真实重跑证明。

## 第 2 轮真实执行与审计修正

`/tmp/infinishell-claude-coordinator-live-2.ndjson` 保存 63 条公开记录。生产 libtest 退出码为 0，实际完成 5 条输入、3 个原生执行、2 次 `InputJoined`、4 次 MCP 调用和 5 次允许审批。子任务在代数 1 合并追加输入，父代 1 合并进度输入，父代 2 单独处理自动最终结果。三份真实 `success` result 的完整 UUID 列表分别覆盖上述三批输入，追加、进度和自动结果均有唯一持久化消息 ID 及 `native_protocol` ACK。实际 inspect 回收、文件效果与两份正式监督清理回执均通过。

原运行器仍返回 1，原 metadata 中 `acceptance_passed=false` 保持原字节不改。原因是审计对所有 runtime 事件要求已经拥有最终原生会话 ID，而父、子各有一条合法的初始化 `SessionReady`，其 ID 为 null，策略已经验证、会话关联明确为 false。修正仅识别上文限定的初始化阶段；原生 ACK、合并输入、实际完成、代数、旧进程 token 和整批 result 门槛均保留。

修正后的审计对原私有全树和原公开投影分别返回通过；此次复核没有启动模型、CLI、Cargo 或 Git。原始文件的摘要均与原 metadata 所记值一致：

| 产物 | SHA-256 |
| --- | --- |
| 第 2 轮公开 NDJSON | `d86b05e3890635b71a541f9bc41fef7355fb8c8596eb5ccf7615f60db33c4d4c` |
| 未改动的原 metadata | `5dc8cfdc7ac1fae0ddb323509c1aaa9ed447aff8643da162a159199adcea53fd` |
| 私有原始 NDJSON（不归档内容） | `acb03953a83fe3718d28a4f175b42f520bc7fd2d33c0eff845d87453de020eb1` |
| 私有原始 stdout（不归档内容） | `fd822312344e23553ed5f81ac142655944995f0b4383a1477bdb3a45a70f6fd9` |
| 修正后的运行器／审计代码 | `4f9d9940250da4489258cee039ff2394d1ea84036df92594b91b62ff3bab58fd` |
| 实际测试二进制 | `7838e9dfea308a3bcafcc2198cf43f740e8461296f46d55a1530c592cab9188c` |
| 实际监督入口 | `091fe9364cec79601d539fe436d24d4a375b7928548cfde8d9e343e31fb61d4a` |

这不是新的真实重跑，也不改写第 1 轮失败或第 2 轮原运行器结果。应将本次修正后的独立审计结果与原执行记录同时保留；包含实际改动的干净提交及跨平台门禁仍由根代理完成。

### 第 2 轮归档与独立审计

仓库保留三份原公开文件的原字节副本：[NDJSON](validation/claude-coordinator-live-2.ndjson)、[metadata](validation/claude-coordinator-live-2.metadata.json) 与 [stdout](validation/claude-coordinator-live-2.test-output.txt)。metadata 的 `acceptance_passed=false` 和 `automatic_result_delivery_ack_verified=false` 未改写；它们是旧审计的结果。stdout 的真实 libtest 成功、生产记录的 `acceptance_passed`、两份监督清理回执与新增审计的通过分别保留，不能把它们混为原运行器通过。

[独立审计 JSON](validation/claude-coordinator-live-2.audit.json) 记录原 libtest 退出码 0、根代理观察到的原运行器退出码 1、两份未改动证据的修正审计通过、全部实际输入关联、三份原生 result UUID 列表、五条持久化原生 ACK、两份正式清理回执、二进制摘要和冻结输入引用。审计自身 SHA-256 为 `bb92f428779bca35532f29bc98f6bcd343aa1e3525ae2089618a33506e9e448a`。原 stdout 副本 SHA-256 为 `fd822312344e23553ed5f81ac142655944995f0b4383a1477bdb3a45a70f6fd9`，与私有 stdout 一致；私有原始 NDJSON 和 SQLite 仅记录摘要，未归档内容或完整配置。

[精简投影](validation/claude-coordinator-live-2.compact.ndjson) 是明确标注的变换副本，不能称为原字节证据，也未用它替代原始验收谓词输入。它保留 63 条原公开事件及逐行 SHA-256，另加一条注册表记录，将 5 处 `config_json` 引用集中为 3 份公开配置摘要，将 12 处固定策略引用集中为 1 份完整公开策略；输入合并、代数、真实 outcome、父代权限上限、消息状态和原生终态仍可追溯。精简投影 SHA-256 为 `1f6b32b64d241917865b80c0421ad972176bd10a39e2af9d114e70dc5a423b54`。变换只读取已脱敏公开投影，不从私有 SQLite 复制配置。

归档前在内存中扫描三份原公开文件和新增投影，分别核对提供方密钥、Bearer token、凭据赋值、私有 API 端点和 API 环境文件路径签名，五类均零命中；只保留方法与命中数，不输出匹配全文。零命中仅覆盖这些定向签名，不能作为通用秘密检测声明。此次操作没有读取认证环境文件，也没有启动模型、CLI、Cargo、Git 或 GUI；第 2 轮原执行仍是基准提交加工作区改动，未转为新版运行器重跑、同一干净提交跨平台通过或完整应用重启验收。

## 当前证据边界

| 项目 | 当前结果 |
| --- | --- |
| 离线验收谓词、合并语义、自动回收、初始化及隔离边界 | 56 项通过；未启动 CLI、未发起模型请求 |
| Rust 格式／语法解析 | 按仓库 Rust 2024 edition 执行 rustfmt |
| Cargo 检查与模块测试 | 交由根代理在完整冻结输入运行；本文不计为通过 |
| 本夹具真实 API 父子流程 | 第 1 轮失败；第 2 轮生产执行通过，原运行器误拒绝，原记录的修正审计通过 |
| GUI 点击、完整应用重启与恢复 | 本夹具不覆盖 |
| 活跃父任务自动结果消息 ACK 和实际执行 | 第 2 轮原生接收确认及父代 2 实际执行通过；不宣称通知在父代 1 内执行 |
| 运行中追加输入的执行语义 | 审核原生同轮合并或独立下一轮；不把接收 ACK 算成执行完成 |
| 被拒绝 Read 的实际拦截 | 本夹具不覆盖 |
| Linux／Windows／SSH／tmux | 本夹具尚无真实平台记录 |

新增文件只包含验收逻辑和测试文档，未改产品文案，无需本地化变更。
