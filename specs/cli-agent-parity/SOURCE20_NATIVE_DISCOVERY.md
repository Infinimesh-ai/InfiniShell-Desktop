# Source20：真实正常关闭改善与首次 SDK 发现回调

本报告归档两个已结束的真实 CLI 运行，均为整轮 FAILED：原测试退出码 `101`，根代理提供包装器退出码 `1`。前两轮结果、正常退出或单次 SDK 发现请求各自有具体证据，但均不能计为完整验收通过。旧 Grok SDK6 失败记录保留，本报告不覆盖或改写旧证据。

## 构建身份与独立核对范围

两个公开原报告记录相同 lib SHA `c5819a0d7e73031b3ce19716e27e143b3188e03a5c6e7b0bc5e1a8b2a88c7716` 和 main／监督 worker SHA `8b1ad22b400b71b13e08ca6d9f8fdcc3b78da69c965d3fd9366a8cfa1ed3ddea`，根代理将其关联为 source20 中间脏快照。本归档独立核对原报告字段一致，没有读取 target、frozen 或二进制，不能证明最终包含实际修改的同一提交跨平台验收。

五份公开轨迹／元数据／SDK 网络报告以及明确授权的六份 exit／macOS 清理证明均按原字节复制。各审计记录来源、SHA、字节数和固定六类凭据形态扫描结果。本归档只核对既有回执字段与代次关联，没有重新查询内核，也没有读取认证、配置、原始正文或 test-output。

## Claude PNG2：首次正常关闭通过，恢复身份判据失败

公开轨迹共 42 条，含 23 条原生协议摘要。PNG 识色及中文多行各一次实际输入、接收、开始和 Completed；原生 user／result 各 2 条。两轮完整输出 SHA 均匹配期望，且对应原生成功 result 摘要确实出现。PNG 数组、图片、文本 SHA／字节数和块类型均与准备记录一致。

第一代 `cleanup_checked` 明确 `normal_exit=true`；限定回执为 `stdio_closed`、退出码 0、清理确认。与第一次 PNG1 的 `stop_requested` 失败不同，本轮已有正常关闭来源。附件随后出现 `attachment_restore_verified`，记录文件哈希／大小及 typed 引用匹配，`image_replayed_to_native=false`。这只支持磁盘附件引用恢复检查，没有将图片向原生 CLI 重投，也没有证明完整继续流程；原 metadata 的整轮 `durable_attachment_restore_verified` 仍为 false。

恢复启动后最新 SessionReady 的 `native_session_id=null`、`native_session_association_confirmed=false`，夹具以 `resume_ready_identity_failed` 失败，第三次输入没有发送。这里不能确认恢复到了原生历史会话，也不能认为记忆回收通过。第二代限定退出回执同样为 `stdio_closed`、退出码 0。两代 macOS 清理证明各记录同代次、原始等待状态 0、Job 移除及 CID 销毁；字段关联只证明既有清理记录完整，没有改变 Resume 验收失败或整轮 `acceptance_passed=false`。

文件：[轨迹](validation/claude-managed-image-live-2.ndjson)、[元数据](validation/claude-managed-image-live-2.metadata.json)、[首代退出](validation/claude-managed-image-live-2.initial.exit.json)、[首代清理](validation/claude-managed-image-live-2.initial.macos-cleanup.json)、[恢复代退出](validation/claude-managed-image-live-2.resume.exit.json)、[恢复代清理](validation/claude-managed-image-live-2.resume.macos-cleanup.json)、[独立审计](validation/claude-managed-image-live-2.archive-audit.json)。

## Grok SDK7：首次真实 server/discover 回调，未进入业务工具阶段

公开轨迹 5 条。本轮 direct agent 启动诊断首次收到真实 `_x.ai/mcp/sdk_call` 反向请求，内层方法 `server/discover`，outer／inner ID 均为 0；SDK params 的两个键为 `message`、`serverId`，内层工具参数仅 `_meta`。公开投影的 `requested_protocol_version=null`，mcp_params 的三个元数据键仅保留散列。根代理依据官方接口提供标准发现键关联，本归档未读取键对应实际值；投影版本为 null 不能证明 `_meta` 内没有版本，也不能完成协议版本兼容验收。

六个公开来源 carrier 中 sessionId／promptId／toolCallId 均缺失或 null。这是本次发现阶段的观察，不能推出后续业务工具回调也没有来源。应用已提交 1 次输入，但原生输入确认计数 0；初始化、tools/list、inspect 回调及权限请求均为 0。根代理说明没有原生 ACK／Started／Result，本归档的公开计数与未进入业务工具阶段一致。整体 SDK 探针 `passed=false`，来源 unknown、来源账本关系未确认、产品 gate 关闭，父子任务能力未获验收。

与 [SDK6 旧失败](SOURCE18_NATIVE_FAILURES.md) 相比，收到一次反向请求只支持发现回调路径改善，不能追认 leader 为旧失败根因，不能证明 SDK 工具调用、权限上限、结果回收或完整流程通过。旧 SDK6 档案仍保留。

限定回执记录 `stdio_closed`、`cleanup_confirmed=true`，但 `exit_code=null`；对应 macOS 清理证明的 `native_wait_status=9`，Job 移除、CID 销毁、代次匹配。原始等待状态不是退出码，绝不能记为原生退出 0。公开网络摘要 8 次官方 TLS 隧道、212,869 字节、2 次隧道 I/O 失败，预算未耗尽；认证副本已移除、隧道已停止，均来自既有原运行报告。本归档没有重执行网络或推定这些网络事件的失败因果。

文件：[轨迹](validation/grok-sdk-origin-probe-7.ndjson)、[元数据](validation/grok-sdk-origin-probe-7.metadata.json)、[网络摘要](validation/grok-sdk-origin-probe-7.network.json)、[退出回执](validation/grok-sdk-origin-probe-7.exit.json)、[清理证明](validation/grok-sdk-origin-probe-7.macos-cleanup.json)、[独立审计](validation/grok-sdk-origin-probe-7.archive-audit.json)。

## 归档与验收边界

十一份原始证据和全部新增档案固定六类凭据形态扫描均为零匹配；不能保证识别未知凭据格式。本次只有证据归档，无需产品本地化变更，没有执行双语 GUI 验收。没有运行 CLI、Cargo、模型、Git、网络或 GUI，没有修改夹具／主计划／能力矩阵／主验证报告，也未将任何未通过项算作完成。
