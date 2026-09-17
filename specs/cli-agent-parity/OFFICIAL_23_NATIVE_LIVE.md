# source23 在线原生验证独立归档

本次三个在线运行均已结束：Claude PNG3 通过其限定范围；Grok SDK8 与 Claude 待 Edit 取消1失败。包含实际修改的提交为 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04`，未将任何失败计为通过，Goal 整体尚未完成。

## 提交和产物证据

已归档的 [原生输入预检](validation/macos-official-23-native-inputs-precheck.json) 关联提交、93 个输入及 lib/main/Claude/Grok 文件身份。本归档独立比较各运行公共 metadata 的三项二进制身份，均与预检对应字段一致；[收尾检查](validation/official-23-native/inputs-postcheck.json) 再次报告同一干净提交、同一 93 输入与四项产物哈希一致。

这些来源由根代理实际执行并记录，本归档只读取公共报告并核对字段，没有运行 Git、读取冻结树、target 或二进制。SDK8 原 metadata 的 `same_commit_verified_by_runner=false` 完整保留；runner 没有自己验证提交，提交关联依据独立预检／收尾报告及公共身份比较，不改写这个字段。收尾的 `strict_signature_exit_code=null` 同样保留：根代理未重复签名命令，只核对原产物哈希。

## Claude PNG3：限定范围通过

范围为 `claude_managed_png_process_resume`，Claude Code 2.1.273，test exit 0、`acceptance_passed=true`。公共记录共 57 条，其中 30 条原生协议投影。

| 阶段 | 独立逐事件核对 |
|---|---|
| 首代建连／图像 | initial ready 未关联原生 ID；图像 typed input 提交，native ACK、Started、Completed 各一次，结果全文 SHA 与期望一致 |
| 多行输入 | 第二输入 native ACK、Started、Completed 各一次，结果全文 SHA 与该轮期望一致 |
| 首代关闭 | `stdio_closed`／exit 0，normal_exit、transport_closed、cleanup_confirmed 均 true |
| 磁盘附件恢复 | 文件哈希引用匹配 true；`image_replayed_to_native=false`，此步骤未将图像重新发送 |
| 历史继续 | 第二代请求同一历史会话，startup ready 仍未确认 ID；第三输入发出后，native ACK／Started／Completed 确认同一原生会话，回忆结果 SHA 与首轮一致 |
| 第二代关闭 | `stdio_closed`／exit 0，normal_exit、transport_closed、cleanup_confirmed 均 true |

三轮都关联原生会话 `45854a15-8267-47d6-8b19-258deac5cfef`。两代共 3 个原生 user 和 3 个 success result；第一输入控制器发送两次相同 message ID，其他两轮各一次，原生输入仍为 3，报告同 ID 去重通过。仅一条原生图像内容投影，哈希与初始 PNG 一致；不能将磁盘引用恢复说成再次原生图像输入。初始继续连接没有 ID 不是提前确认历史会话，真正关联由第三轮原生回执完成。

这证明生产 Rust adapter 的图像、多行、去重、进程正常关闭、磁盘附件引用恢复与启动新进程继续同一原生历史会话。metadata 明确 `app_restart_and_ui_verified=false`、`sqlite_verified=false`、`parent_permission_ceiling_verified=false`、`filesystem_sandbox_verified=false`；未证明完整应用重启、SQLite 恢复、GUI、运行中同轮 steering、全权限验收或 HTTP 模型请求数。两次清理据公共 receipt 投影，本归档未读取私有收据或重新查询内核。

## Grok SDK8：现代注册改善，验收失败

范围为 `grok_native_sdk_origin_probe`，Grok 1.0.30，test exit 101、`probe_passed=false`。公共记录共 10 条。实际事件顺序为 capability enabled → init_progress connected 0 → SDK `server/discover` → modern registration → SDK `tools/list` → connected 1 → mcp_initialized → server_status ready → probe_finished。

两次反向 SDK 请求版本均为 `2026-07-28`，carrier 为 `params._meta`；外层与内层 ID 分别为 0、1。登记报告 `initialization_mode=modern_discover`，最终 tool count 1、servedToolNames 为 inspect，discovery 1、tools/list 1、legacy initialization 0。这说明现代发现、列工具和 ready 注册路径已被真实观察，不能将 legacy initialize 计数 0 解释为完全没有注册。

实际 submitted input 1、SDK request 2、inspect call 0、permission request 0，allow/deny/search/inspect 审批计数均为 0。两次注册请求的六种 carrier 中 promptId/sessionId/toolCallId 全部 absent_or_null，业务 `origin_observations=[]`。注册通知能够关联会话，但没有业务工具调用，未取得业务来源链；不能用注册请求缺少业务 ID 判定所有未来业务请求都不携带来源。`origin_verification=unknown`、native contract 与 ledger relation 均 false，产品 gate 继续关闭。

结束事件记录 cleanup_confirmed true、`transport_closed=false`；没有可据此宣称的原生 exit 0。metadata 记录 tunnel stopped、私有 auth copy removed、project entry count 0。私有 settings 字节未保持一致：CLI 初始化 marketplace 表；metadata 仅报告 permission section 不变与限定 scope 验证，本归档未读私有配置，不能宣称设置完全不变或零副作用。

网络记录 9 次 opaque TLS CONNECT、4229308 字节、2 次非官方 origin 拒绝；未耗尽报告预算。TLS 未解密，HTTP 模型调用不可观察，模型请求数／成本预算未强制验证；不能把请求预算 2 当作实际模型调用次数。整体失败保持不变。

## Claude 待 Edit 取消1：原生确认存在，应用终态失败

范围为 `rust_adapter_pending_edit_cancel`，Claude Code 2.1.273，策略 `ClaudeRestrictedFilesV1`、plan profile，两次 profile 校验。test exit 101、`acceptance_passed=false`、`waiting_edit_cancel_verified=false`。公共记录共 84 条，其中 70 条原生协议投影。

实际业务输入 1 次，native ACK／Started 后出现精确 Edit 审批，`edit_allowed=false` 且待审批。应用提交 interrupt，接收 `edit_approval_cancelled` 和 `interrupt_accepted(native_receipt=true)` 各一次。原生投影独立匹配 interrupt 控制请求与 success response；其后还观察到 control_cancel_request、`result:error_during_execution`／terminal_reason unknown，最后 `command_lifecycle.state=cancelled`。

原生取消状态已出现，但公共应用事件没有任何 `turn_finished`／Cancelled 聚合。审批取消事件自身的 `native_execution_cancelled_verified=false` 保留。应用最终验收失败，不能把 native ACK、审批取消或清理完成替代应用 Cancelled 终态。关闭为 `stdio_closed`／exit 1，normal_exit false、transport_closed 与 cleanup_confirmed true。

记录只有取消前的 file_unchanged 校验；后续文件不变、继续历史会话与第二轮验证没有完成证据。本次没有读取文件或私有失败输出，不能补写取消后文件已验证不变。SQLite／应用重启／GUI／父权限 ceiling／文件系统 sandbox／模型调用计数均未验证。

## 档案与剩余验收

- [PNG3 事件](validation/official-23-native/claude-managed-image-live-3.ndjson)、[原 metadata](validation/official-23-native/claude-managed-image-live-3.metadata.json)
- [SDK8 事件](validation/official-23-native/grok-sdk-origin-probe-8.ndjson)、[原 metadata](validation/official-23-native/grok-sdk-origin-probe-8.metadata.json)、[网络记录](validation/official-23-native/grok-sdk-origin-probe-8.network.json)
- [待 Edit 取消1事件](validation/official-23-native/claude-approval-cancel-live-1.ndjson)、[原 metadata](validation/official-23-native/claude-approval-cancel-live-1.metadata.json)
- [收尾检查](validation/official-23-native/inputs-postcheck.json)、[独立审计及 SHA](validation/official-23-native/archive-audit.json)

八份公共原始文件先作固定六类凭据形状扫描再按原字节复制，复制后再次核对源字节；匹配均为 0，不输出匹配正文。没有复制 `.test-output.txt`；只引用 metadata 自带 stdout/private evidence SHA，不读取其正文。新报告／审计也扫描 0，固定形状扫描不能保证识别全部凭据格式。

本归档未运行 Git、Cargo、CLI、模型、网络或 GUI；未读取 auth、私有模型／error／raw stdout、target 或冻结树；未修改旧档和根三份主文档。仅新增验证文档，无需产品本地化变更，真实双语 GUI 布局仍待验。Codex 本轮原生整链、Grok 业务来源与子任务、Claude 取消终态、完整应用重启／持久化、同提交全平台／全工作区与 SSH/tmux 验收均不能计为完成。
