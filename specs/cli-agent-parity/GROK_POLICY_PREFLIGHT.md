# Grok 固定策略：无模型真实接口调查准备

日期：2026-09-18。状态：**source24 已完成准备预编译、唯一 ignored 入口列表核对及所列本地门禁，运行器准备常量已实际解除为 true。Rust 受影响模块 1378 项、i18n 11 项、Python 15 组共 410 项全部通过，其中包含 22 个 policy 纯协议回归。本夹具原生接口调查尚未运行；结果绑定 111 路径的 dirty 候选快照，不能计作实际新提交的同提交跨平台或原生通过。** 本文件不改变已有成功或失败证据，不新增生产权限策略，不开放 Grok 应用子任务派发。

这一步只调查固定创建和只读诊断的真实响应。即使将来接口调查完整通过，`profile_loading`、`effective_mode`、`configuration_sources`、`builtin_catalog`、`wrapper_closure` 仍投影为 `unknown`，`parent_permission_ceiling_verified=false`、`filesystem_sandbox_verified=false`、`ready_for_policy_implementation=false`。正常退出、opaque 登录缓存、请求中写了 `false` 或正常清理都不能填补这些字段。

## 固定身份与主源边界

- 准备的受测身份：`grok 1.0.30 (04b7ffed98c6)`，macOS arm64 文件 141,869,568 字节，完整 SHA-256 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。当前准备任务没有重新运行版本或帮助。
- 候选官方源码固定在 [xai-org/grok-build/482711333c7195dc16a272777f86086d615e2afb](https://github.com/xai-org/grok-build/tree/482711333c7195dc16a272777f86086d615e2afb)，[SOURCE_REV](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/SOURCE_REV) 为 `be7ce6e8cffe46d20bef9834b211616082ee866b`，与二进制构建 ID 不同。源码中实现方法只支持候选调查白名单，**不证明这些方法在 1.0.30 中存在或行为相同**。
- 固定帮助已暴露 `agent --agent-profile <PATH>`、`--no-leader`、`--always-approve`。本准备任务不重复帮助。后续包装器仅接受固定 `--version` 与 `agent --no-leader --agent-profile <私有固定路径> stdio`，不传 `--always-approve`，不共享 leader，不借用根层 tools/deny 参数当作 agent 已生效的规则。

## 有限且实际存在于候选源码的接口

每个进程最多六个出站 JSON-RPC 请求；顺序固定。第一个进程创建空会话，关闭 stdin 后等待真实正常退出与生产清理；第二个进程以同一 profile/config 恢复该空会话。两次合计最多 12 个请求、两个进程、`native_inputs=0`。不发送 `session/prompt`、认证 RPC、工具调用、模式切换、动态配置、`session/close` 或任何调试执行命令。

| 方法 | 候选实现与可观察范围 | 不能据此证明 |
| --- | --- | --- |
| `initialize` | 现有生产 Grok 协议初始化，版本与能力响应 | 有效权限、工具目录、配置加载 |
| `session/new` / `session/load` | 候选 [session_setup.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/session_setup.rs) 的创建/加载路径；只操作本夹具空会话 | 发送 yolo/auto=false 不等于有效模式 ACK；模型/configOptions/toolOverrides 不等于完整工具目录 |
| `x.ai/session/info` | 候选 [acp_types.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_types.rs#L515) 的信息类型：agent 名称、模型、上下文等 | 固定 profile 已加载、完整来源、全部内建工具或有效模式 |
| `x.ai/session/state` | 候选 [extensions/session_state.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/extensions/session_state.rs)：计划、信号、用量、目标、摘要等会话元数据 | 权限全集或有效工具集合 |
| `x.ai/mcp/list` | 候选 [extensions/mcp.rs:597](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/extensions/mcp.rs#L597)：session MCP 实际条目及状态；最多投影 `mcp_only` | 内建工具全集、后台/gateway 初始化闭包或 wrapper 可达目标 |
| `x.ai/debug/agent` | 候选 [extensions/debug.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/extensions/debug.rs)：registry 计数等只读诊断 | 工具 ID/schema 全集、有效模式、来源封闭 |

没有加入猜测的 `x.ai/tools/list` 或 `x.ai/permissions/...`。只取只读诊断响应的完整散列、字节数、状态与顶层键散列，不归档其正文、模型名称、工具参数、路径或帐号。`method_not_found`、`auth_required`、超时、RPC 错误一律导致调查边界验收失败并保留安全记录；不能把缺少方法投影成空工具集或禁用模式。

模式目前没有已核验的有效值回读方法。即使响应或配置出现 `ask`/`yolo` 等字符串，运行器也只保留散列与 `unknown`。来源目前也没有已核验的完整 readback；本端 profile/config 全字节相同只证明输入快照一致。未知工具名导致完整工具集回退、额外工具/来源、恢复不同 profile 都拒绝进入后续策略验收。

候选 [AgentDefinition](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-agent/src/config.rs#L697) 提供 tools/disallowedTools/toolConfig/hooks 等创建入口；其中非 Bypass 的 permissionMode 仅作为兼容字段。候选 [builder.rs:1006](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-agent/src/builder.rs#L1006) 在未知工具名时保留完整集合，并自动保留 SearchTool/UseTool。真实策略必须验证 Read/Edit/Bash、原生 Agent/Task、Search/Use 的目标闭包、共享 MCP/gateway/插件等所有执行来源；本夹具 MCP 部分目录不能代替这个证明。

## Rust 夹具与接线：准备快照已预编译，唯一入口已核对

已独立新增 `app/src/ai/cli_agent_runtime/grok_policy_preflight_live_tests.rs` 及其子模块 `grok_policy_preflight_protocol_tests.rs`，并由根代理接线。在 `grok.rs` 的 `mod sdk_origin_live_tests;` 之后新增独立测试模块：

```rust
#[cfg(test)]
#[path = "grok_policy_preflight_live_tests.rs"]
mod policy_preflight_live_tests;
```

唯一入口固定为 `ai::cli_agent_runtime::grok::policy_preflight_live_tests::native_fixed_policy_interfaces`，标为 `#[ignore]`。source24 准备快照已预编译，证据见下文。首次核对因要求 stderr 为空的前置断言未通过；根代理安全复核后将 stderr 处理改为有限长度与散列，实际退出码和唯一精确入口核对均通过，[入口证明](/tmp/infinishell-cli-parity-official-24-entrypoint.json) 已生成。文档已实际核对该记录的 `exit_code=0`、`exact_ignored_entrypoint_listed_once=true` 及精确 ignored 列表参数，并核对运行器常量已为 `RUST_ENTRYPOINT_PREPARED=true`。列表只证明测试入口存在，没有执行 ignored 测试、原生 CLI、认证或模型。没有参数或环境变量旁路；运行器每次仍须通过精确 libtest `--list --exact --ignored` 确认唯一入口，再保留产物或复制 opaque auth，不能靠同名 stdout 字符串直接启动原生 CLI。

Rust 夹具直接复用当前实际 `managed_process::spawn`、`ManagedChild`、`finish_after_stdin_close` 和 `confirmed_exit` 的生产派生、控制连接、stdio、监督与清理路径，使用独立有限 ACP 驱动。当前架构不存在名为 `ManagedProcessSpawner` 的类型，未为测试另造 spawner 或 Command。未运行会自动认证/提交 prompt 的普通任务入口，也未修改生产 Grok 权限判据。正常清理还必须是 `macos_resource_coalition`、真实 stdin EOF、退出 0、生产资源域清理核验和严格匹配回执；StopRequested、旧 process group 或无 exit code 均不能冒充。

准确候选签名已补齐：`session/info` 明确传 `{sessionId}`，拒绝 handler 中缺 ID 时选择首个 resident 的 fallback；`session/state` 传 `{sessionId,cwd}`；`mcp/list` 明确传 `{sessionId,cache:true}`；`debug/agent` 使用 `{}`，它只读独立进程 registry，不接收 session 参数。info/MCP/debug 使用候选 `ExtMethodResult`，所以 ACP `result` 内再包 `result`；state 为直接返回对象。部分错误、猜测的 `data`/`status` 包装、缺失 native S/cwd 或非零 turns/turnIndex 均失败，不能以请求中保存的 S 单独证明恢复。

此次仅读取同固定 commit 且已由 GH tree 核实的官方 raw 文件。子任务公开副本及 URL/完整 SHA 记录位于 `/private/tmp/infinishell-grok-policy-preflight-public-23/`，没有认证、帐号或私有日志：

| 官方主源 | 完整 SHA-256 | 用途 |
| --- | --- | --- |
| [agent/mvp_agent/acp_agent.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs#L1970) | `f9d0400c2d72a8bd2288a56842b1c18bdda571d315b0c6c34f5fb42977e3edda` | info 实际路由 |
| [agent/handlers/session.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/handlers/session.rs#L55) | `bfdfbf0f68958043acb4ce8389f8ad25894f6d7a8d8f2ac50803c9ad21bc3a8c` | info Request 与真实 S/cwd/空历史回读 |
| [session/result.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/result.rs#L29) | `ea717cb2245a7cc8fe6ee687186f4b77e80281dde27f3fb6ff851afecd64bad9` | ExtMethodResult 准确包装与部分错误 |
| [extensions/session_admin.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/extensions/session_admin.rs) | `cdc05b3cb5b740b1f07c7906ab2bc2ca04ff073de6a4acf8353ccba24bd69e31` | 确认 admin 修改接口不在白名单 |
| [extensions/mod.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/extensions/mod.rs) | `f2af3d918552016883c4dfff0ac9c3bab477a75ccf3da50f35c281f1ee85f51f` | 准确扩展解析与 direct response |
| [session/mod.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/mod.rs) | `4b3279ad5d5100756dd8b0bff654e9ce151c375a6b0ca8840e0f25d2e4933815` | result 类型实际导出 |

已读取根代理同固定 commit 的 `debug.rs`（`bbbd81dacdb4838bc24fbafb62ce0ad0de83b739d4f7cee66e2957bc39664aa4`）、`mcp.rs`（`cd764b68964071c6bee5f47d12980f46b2e93eb3394b7cbe7808f91e98692e94`）、`acp_types.rs`（`3576669c4c17e300872efb3fb1d2bed05973d04ff14103bd306825f0d0f83e96`）公开副本；源码候选身份不升级为实际 1.0.30 已支持这些方法。

写入守卫必须在序列化后的真实 stdin 写入前执行，且校验完整请求与有限参数，不只校验方法字符串：

1. 先核验计划、全字节 profile/config 散列与固定 CLI 身份；最多两个按序进程，每个独立 runtime generation。私有 cwd、profile、config 必须固定规范路径，恢复前重新计算完整散列。
2. `initialize` 使用现有版本 1 初始化；文件读写与 terminal client capabilities 均为 false，不广播额外执行能力。
3. 创建使用本夹具私有 cwd、空 `mcpServers`，候选源码和已有公开真实 BYOK new/load 帧确认 `_meta.yoloMode=false`、`_meta.autoMode=false`；不传模型 ID、trust 或 startupHints。加载只允许第一个进程真实返回的空会话 ID，同 cwd、空 MCP、同 profile/config；不得调用外部历史会话、重放输入或重新新建来冒充恢复。
4. 四个诊断采用上文已核对签名；session-scoped 方法精确使用本会话 ID，debug 只绑定独占进程 generation。现有源码候选与本机版本不对应，因此未来 native 拒绝方法或包装时仍保存失败，不尝试多个字段或兼容方法。
5. 每次真实写入前检查预算、完整参数、全局序号和 generation，记录实际序列化字节数及完整散列。响应须精确对应请求 ID/generation/序号；禁止重复、旧代、未关联响应被当作成功。
6. 来自 native 的 reverse fs/terminal/MCP/tool/approval 请求、任何 prompt/turn/工具执行活动都先记录安全失败并关闭执行入口；不执行这些请求，也不发送 Allow。原生活动计数必须真实监测，不能因本端未主动发送工具 RPC 就填写 `tool_exec_count=0`。
7. 第一个空会话创建后正常 EOF、原生退出 0、生产 cleanup receipt 均成立才启动第二代；第二代 load 真实同 S，仍无任何输入，正常 EOF、退出 0 与清理。没有真实原生退出/receipt 的 deadline、kill 或连接中断不能计作正常收尾。该夹具没有取消或审批验收，cleanup 不证明这两项能力。
8. 所有失败均通过封闭 NDJSON 模式保留；退出 101 也必须写安全失败或严格投影已完成记录，不用成功 finish 覆盖失败。证据上限 4 MiB、stdout 上限 4 MiB、两代原生读取合计上限 4 MiB/256 帧、截止时间最多 360 秒。原生帧只在夹具内存解析并立即投影为散列，不写 raw stdout 或 frame 正文文件。

外层资源准备或收尾的 `OSError` 已补齐证据保存：在输出产物可写的前提下，临时工作区退出或隧道关闭抛异常时仍保存已收集的严格投影，并追加固定 `outer_resource_failed` 的长度和散列。仅公开 `runner_error_type`、`outer_resource_error_type`，不保存异常文字、路径或原生正文。`runner_cleanup_confirmed=false`，`execution_boundary_passed=false`、`interface_investigation_completed=false`；先前接口审计成功不能覆盖该失败。各清理分量分别保留实际核对结果，例如隧道失败但工作区已删除，与隧道已停止但工作区删除失败保持区别。

封闭事件为 `probe_started`、`launch_snapshot`、`rpc_sent`、`rpc_response`、`diagnostic_observed`、`mode_observation`、`catalog_observation`、`source_observation`、`resume_checked`、`process_cleanup`、`probe_failed`、`probe_finished`，具体准确字段及类型以运行器 `event_schemas()` 为接线合约。未知事件、字段、枚举值或布尔计数都会拒绝；拒绝记录只公开全记录散列与字节数。

## 运行器边界

新增文件：`script/cli-agent-parity/run_grok_policy_preflight.py` 与 `grok_policy_preflight_runner_tests.py`。复用现有 `prepare_grok_cli.py` 的静态常规文件/完整散列校验、官方运行器的 opaque auth 文件复制与官方域名 CONNECT 隧道、既有固定 CLI 包装器和生产 supervisor 参数；不复制任何密钥值，不解析 auth JSON，不读取用户全局配置/环境认证，不提供登录接口。

未来显式 `--profile`/`--config` 只接收审核过的独立输入文件，复制到私有工作区并设为只读、全字节求散列，实际运行前后重验相同。它们的格式或实际加载来源不在 Python 中伪造 native ACK。`chmod` 不是针对同 UID 工具的真实权限防护，私有包装器隔离也不把产品策略标记为 FS sandbox。

有界网络复用官方双域名隧道，最多 16 个连接、8 MiB TLS；TLS 字节预算为 0 时同时把连接预算置为 0，关闭后必须取得真实 drained/线程收尾结果。没有解密 TLS，因此 `native_inputs=0` 仅表示严格协议写入没有模型输入，**不表示经过测量的模型 HTTP 请求数或费用为 0**。启动、认证刷新、MCP/gateway/摘要/标题等内部流量仍需额外验证；任何已观测的原生 turn/工具活动必须失败。无登录缓存或 zero 网络导致创建失败也不能证明模式/上限。

历史准备常量为 false 时，运行器在读取路径、保留产物、复制 auth、spawn 或连接网络之前退出 2；该前置拒绝由离线回归覆盖。当前常量已为 true，仍必须显式提供固定 test binary、生产 supervisor、固定 Grok、独立 profile/config、可选明确的专用认证目录及新的 `.ndjson` 输出路径。运行器先验证固定文件 SHA/大小、专用目录 owner/0700、输出不存在，再核对精确 ignored 入口，之后才保留产物或复制认证；配置漂移、auth 删除失败、真实 tunnel 收尾失败、临时工作区删除失败均不能通过。解除准备标记仅允许进入调查路径，不授予任何生产权限能力。

未来产物为 `.ndjson`、`.metadata.json`、`.network.json`。stdout 只保留完整散列、字节数、定向 credential-shaped 内容计数与 libtest 总结计数；任何检测到的形状均失败，正文及匹配值不写产物。定向形状扫描不是通用秘密检测，因此仍依赖“正文从不归档”的结构限制。未知异常只公开异常类型，不输出异常文字、路径或原生响应。

## 固定规则等价与后续策略门槛

ClaudeRestrictedFilesV1 的已定义上限本身声明 FS sandbox=false；同 Grok 父子也可以只证明固定规则等价，无需宣称 OS 沙箱。Read 默认允许且不经 ACP 不能单独否定此路线；实际规则、模式、工具全集与完整来源固定并相同才可能建立 ceiling。

后续必须取得可验证的 profile 加载、有效模式、完整内建工具 ID/schema 与执行闭包、所有配置/MCP/插件/原生子代理来源。父子 pin 同 1.0.30、完整私有快照、无 always-approve/bypass；未知工具 fallback 拒绝，Search/Use 与原生 Agent/shared MCP 纳入闭包，动态 AllowOnce 不能扩大 child 创建规则。仅 App 对收到的审批采用同一规则、相同本端 config hash、原生会话 ID 或 `RootInherit` 都不能取代这些证明。

若真实 1.0.30 缺乏可验证的目录或来源接口，记录具体缺项，继续保持 Grok ceiling/派发关闭，寻求对应的可审计构建或上游有效策略回读；不要虚构协议。本文只准备这个判定所需调查，不能计作 P3 父子权限验收。

## 本次验证与未验边界

离线测试只用合成事件、内存散列与 mock：前置拒绝、预算 0、方法不存在、未知模式、未知工具回退与 wrapper、额外来源、恢复不同 profile、重复/旧代/乱序响应、正文泄漏、非零 native input、清理结果 false、退出 101 和虚假策略声明。这些测试没有启动原生 CLI、网络或认证。

历史准备阶段实际执行一次 `python3 -B script/cli-agent-parity/grok_policy_preflight_runner_tests.py -v`，结果 `Ran 22 tests in 0.017s`、`OK`。当时两个新 Python 文件以 `compile(..., "exec")` 做内存静态编译，通过且不生成编译缓存；该历史记录保留原范围。

补齐外层收尾异常后、准备开关尚为 false 时，两文件再次实际执行必要离线回归，结果 `Ran 24 tests in 0.031s`、`OK`（历史 22 项加两个收尾 `OSError` 用例）。两个新用例先取得合成接口审计成功，再分别模拟临时工作区退出或隧道关闭异常，验证三个产物仍保存严格投影、完整异常文字与 stdout canary 不泄露、总清理确认和最终结果保持失败，隧道限额恢复且没有调用 `Popen` 或认证复制。两个文件 `py_compile` 均通过；编译缓存仅写入受控临时目录并已清理，仓库没有生成 pycache。这些是当时两文件的运行器离线回归，不是解除准备标记后最终快照的门禁或原生接口 PASS。

最初 Rust 准备阶段已通过 `rustfmt --edition 2024 --config skip_children=true` 语法/格式处理，并以静态字段审计核对 12 种封闭事件、13 个发出位置及生产 spawn/EOF/confirmed receipt 引用。当时准备开关为 false，尚未 Cargo 编译；这是历史范围，不能代替后续执行证据。准备预编译阶段未执行 22 个 Rust 纯协议用例；随后 source24 所列本地门禁已实际执行并全部通过，详见下文。预编译结果与测试结果分别保留。

source24 准备快照的 [预编译记录](/tmp/infinishell-cli-parity-official-24-precompile.json) 已实际存在：`cargo test -p warp --lib --no-run`，退出码 0，耗时 **229.418 秒**；准备快照清单 SHA-256 为 `3a0b00201651e650bff552184f4df64d67a2862504b010a188e8408845482814`，构建日志 SHA-256 为 `99d21548e1ab22919115b09ebcc358a3f009b7c5440e681e6c667b1ed73dc967`。该记录明确限定“准备快照预编译，仅核对测试入口”，不能计入正式门禁或原生验证。

唯一入口证明绑定同一准备快照，受测测试二进制 SHA-256 为 `eb3fdde242fa992f8c9414d96a208878f059a1c3fdaefd18728c054413b53be8`；stderr 仅保留 258 字节及 SHA-256 `b4acbd6a2dfa33de2cd1bfe0524b52cae8b2743b468c06c38a4dd68703422eeb`，不归档正文。解除准备标记后的 [正式验证输入清单](/tmp/infinishell-cli-parity-official-24-inputs.json) 已存在且完整文件 SHA-256 已核对为 `4705e0753c8126490070a2e47dfe3c23d0d80663a69e9da77dd0ef1ec2404706`。准备预编译清单与正式验证输入清单分别保留，不能将前者作为后者的正式门禁结果。

source24 的 [本地 Rust 门禁记录](/tmp/infinishell-cli-parity-official-24-gates.json) 与 [Python 门禁记录](/tmp/infinishell-cli-parity-official-24-python.json) 已实际完成，二者绑定上述正式验证输入清单，均记录 `source_unchanged_after_gates=true`：`cargo check -p warp` 退出 0，`cargo test -p warp --lib i18n::tests` 退出 0且 11 项通过，受影响模块 nextest 退出 0且 **1378 项全部通过、5498 项跳过**。已定向核对 focused 日志中 **22 个 policy 纯协议用例均为 PASS**，没有将 ignored 原生入口的存在计作执行。Python 15 组共 **410 项全部通过**，其中 policy 运行器 24 项通过，耗时 0.125 秒；这是解除准备标记后候选快照的门禁范围，与此前两文件的 22／24 项历史结果分别保留。

两份最终门禁记录明确 `base_commit=dec067d2d53fb65f11b37b66a72fa9e11e82cf04`、`worktree_dirty=true`；受验对象是 111 路径候选快照。源码在门禁期间未变化只证明该快照的本地一致性，**不证明这些修改已经形成实际新提交，也不证明同一实际提交的 Linux／Windows／SSH／tmux 或原生接口验收通过**。本夹具仍没有真实 interface PASS；有效策略、来源、工具闭包保持 unknown，父权限上限与文件系统沙箱保持 false。

| 验证阶段 | 本次文档核对的状态 |
| --- | --- |
| source24 准备快照 Rust 预编译 | 已完成，退出 0；未执行测试 |
| 唯一 ignored 入口列表核对 | 已完成，退出 0且精确入口仅列一次；证明已存在，准备常量实际为 true |
| source24 所列本地 `cargo check`、i18n 与受影响模块门禁 | 全部完成；check 退出 0、i18n 11 项通过、Rust 1378 项通过，包含 policy 22 项 |
| source24 Python 本地门禁 | 15 组共 410 项全部通过，包含 policy 24 项；绑定 dirty 候选快照 |
| 本夹具原生创建、诊断及恢复 | 尚未运行；没有真实 interface PASS |
| 有效策略与父子权限上限 | 仍为 unknown／false，未开放派发 |

上述准备预编译、入口列表和本地纯回归均不包含本夹具真实 CLI/登录/会话/审批/取消/父子任务/模型验收，也没有本夹具的同提交平台验证或 SSH/tmux。运行器未来原生路径目前只准备 macOS arm64；Linux、Windows 均需固定包身份与对应监督/隔离验证，不能继承 macOS 结果。跨平台验证技能的真实同提交门禁留待实际受测提交确定后执行。SDK8只完成现代发现／列表／native ready注册，没有原生来源／工具业务证据，诊断目录、正常 stdio 或模板字段均不能授予父 ceiling。

新增内容只有开发测试、协议稳定字段与文档，没有产品界面文案或语义变化，**无需本地化变更**；这不能替代整个 Goal 的英文/简体中文及布局门禁。

根代理接线后复核候选序列化：`SessionInfoResponse.data` 使用 `serde(flatten)`，原生 `turns/turnIndex` 在顶层；纯回归拒绝臆造的嵌套data。`McpServerSessionState.tools` 的空数组省略键，但仍保留额外server来源并拒绝null等畸形数组。Windows纯回归使用本机绝对路径语法；这些修改已纳入 source24 本地纯协议回归并通过，不代表已在 Windows 原生进程验证。

## 后续调查的独立输入

随附 [profile](fixtures/grok-1.0.30-policy-preflight-profile.md) 与 [config](fixtures/grok-1.0.30-policy-preflight-config.toml) 用于后续空会话只读调查；当前未原生运行。profile 使用候选 [AgentDefinition](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-agent/src/config.rs#L690) 的 Markdown frontmatter，关闭技能、agentsMd、默认工具注入，显式空 toolConfig/MCP/hooks；这只记录请求，不认定固定二进制实际加载或全部执行来源已封闭。配置复用已经观察的私有初始化形态，保留所有工具需审批的 ask 规则；marketplace 标记用于抑制本次首次初始化改写，不能证明插件安装或来源封闭。CLI 自身的有效模式、全部内建工具、子代理／Search／Use 等闭包仍为unknown；不开放子任务、不授予父权限上限。运行前后仍核对全字节散列，任何配置漂移保留失败。英文与简体中文界面没有文案变化，无需本地化变更；这些是测试输入和技术记录。
