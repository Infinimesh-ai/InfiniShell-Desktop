# Grok 官方在线授权与验收

2026-09-17 用户重新完成官方设备授权。固定 Grok Build 1.0.30 在独立 HOME 中登录成功；认证文件不进入仓库，未复用 Claude API 密钥。

## 已确认

[单轮证据](validation/grok-official-smoke-1.json)记录实际官方 `grok-4.6-build` 请求：禁用工具与子任务，只要求返回 `GROK_OFFICIAL_READY`。原生返回该标记、`stopReason=end_turn`、一次模型调用、进程退出 0，耗时 15.81 秒，原生报告费用为 0.00919632 美元。没有发生此前记录的 402 额度错误。

这说明本次登录与模型可用性通过。此前额度不足的历史记录仍保留，但不再代表本次账号状态。固定二进制摘要保存在证据中；认证数据和模型思考文本不收录。

## 生产托管第2轮：部分步骤有证据，完整流程失败

[第2轮事件](validation/grok-official-adapter-2.ndjson)记录生产监督进程与 Rust 适配器完成两轮官方模型正文，分别返回 `PARITY_ONE`、`PARITY_TWO`，两轮结果均为 `Completed`。原始[验收元数据](validation/grok-official-adapter-2.metadata.json)仍为 `acceptance_passed=false`；其 `official_grok_model_tested=false` 是本运行器完整流程未通过的标志，不否定这两轮已保留的正文证据。

第3轮收到 `ApprovalRequested`，应用派发 `AllowOnce` 并记录 `approval_resolved`。这两个审批派发/处理事件的 `native_receipt` 均为 `false`，因此不能声称原生审批 ACK 已确认。[诊断投影](validation/grok-official-adapter-2.audit.json)确认 `approval-allow.txt` 实际出现，内容精确为 `PARITY_APPROVAL`；[原生内部事件](validation/grok-official-adapter-2.native-events.json)记录 `write` 工具执行成功。该回合最终结果尚未回收，随后应用报生产 I/O 中断：`CLI process I/O failed: 托管监督进程退出失败`。

[生产退出回执](validation/grok-official-adapter-2.exit-receipt.json)为 `stdio_closed`、退出码 0、`cleanup_confirmed=true`，[macOS 清理回执](validation/grok-official-adapter-2.macos-cleanup.json)确认 job 与 resource coalition 已清理。这些回执证明清理，不能转为任务成功。检查的原生日志没有 ERROR/panic，末尾记录客户端断开及仍有活跃工作；这不足以证明没有崩溃，也没有保留可解释原始 transport 错误的 stdout 帧。主代理已修复原始 transport 错误被 `child.finish` 清理错误覆盖的问题，第3轮复验将保留原始诊断。

原运行器报告 `private_settings_unchanged=false`，该字节差异与失败结论保留。事后把当前 `config.toml` 与固定运行器初始文本的重建值比较，差异仅是原生首次初始化增加的固定官方 `marketplace` 元数据，`permission` 仍为 `ask any`。重建前值不是当时留存的配置快照，审计范围也只有 `config.toml`。新判据仅接受精确的官方市场初始化，持久授权或其余配置变化仍会失败；不会回溯改变本轮验收结果。

[网络记录](validation/grok-official-adapter-2.network.json)保留官方 CONNECT 连接和非官方 origin 拒绝。旧拒绝事件未记录域名，不能把全部拒绝解释为遥测；原生日志另有 `api.x.ai` 的 tokenize-text 辅助请求失败。后续运行器增加拒绝 origin 的固定类别与 authority 散列，网络白名单仍只允许 `auth.x.ai`、`cli-chat-proxy.grok.com`，不解密 TLS。认证内容、私有日志原文和原生模型思考均未归档。

## 生产托管第 5 轮：固定 8 输入流程通过

[第 5 轮事件](validation/grok-official-adapter-5.ndjson)、[原始 metadata](validation/grok-official-adapter-5.metadata.json) 和 [原始 network](validation/grok-official-adapter-5.network.json) 经安全扫描后按原字节归档。固定 `grok 1.0.30 (04b7ffed98c6)` 使用官方 `grok-4.6-build`，生产监督链 libtest 退出 0，`acceptance_passed=true`。实际执行 8 个原生输入，全部有 `native_receipt=true` 的接收确认、独立运行事件及真实终态，结果为 6 次 `Completed`、2 次 `Cancelled`。

两轮正文准确为 `PARITY_ONE`、`PARITY_TWO`。精确 Write 审批允许后，原生完成并返回最后响应 `APPROVED`，夹具确认文件内容；再次 Write 审批拒绝后，原生取消，夹具确认拒绝目标文件没有产生。审批 `resolved` 事件仍为 `native_receipt=false`，不声称单独的原生审批 ACK 已确认。正在输出的回合中追加输入，适配器排入下一轮，两个回合均获得独立原生接收确认和完成结果；不声明同轮 steer 支持。正文开始后真实发送取消并取得 `Cancelled`。第一运行代关闭并确认清理后，第二生产进程继续相同原生 session `01a0af68-fcbe-70f3-a31a-ce6c477ceb03`，最后响应精确返回此前排队标记。这是新进程继续历史会话，没有验证重新关联活跃任务或桌面应用重启。

[最终回执投影](validation/grok-official-adapter-5.native-receipts.json) 保留 8 组已验证的原生 session、turn、完成 watermark、完整正文长度与 SHA-256、最后响应长度与 SHA-256。允许回合累计完整正文为 71 字节，最后响应为 8 字节；固定结果仅比较生产已验证 history 的最后正文响应，产品完整累计正文仍保留，不以 `endswith` 判断成功。独立归档从事件复算了全部 8 组正文与最后响应散列，没有读取或归档原生 chat history、思考或认证正文。

两个不同运行代及独占 leader socket 均有[退出回执](validation/grok-official-adapter-5.exit-receipts.json)，退出原因 `stdio_closed`、退出码 0、`cleanup_confirmed=true`。[macOS 清理](validation/grok-official-adapter-5.macos-cleanups.json)确认两次作业和资源 CID 清理，原生等待状态均为 0。配置仍为 `bytes_unchanged=false`，仅固定原生 marketplace 初始化被接受，权限部分不变。实际网络有 15 次官方 TLS CONNECT，拒绝 190 次 `unknown`、7 次 `xai_api`；不解密 HTTP，不能验证模型请求数或成本，也不能把未知拒绝统一解释为遥测。

本轮冻结输入为 [macos-official-8-inputs.json](validation/macos-official-8-inputs.json)，libtest SHA-256 为 `8decb11d70665c86e60f4e3039702b80f218718400c19ff4c5c5258575984b40`，生产监督器为 `6e1801d768bdcb3e4ae9e2de2dba2137386631294baa79c2e262efc29f8abb78`。基线提交仍为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，`worktree_dirty=true`，是开发中间快照，不能计入包含实际修改的正式提交跨平台验收。[audit](validation/grok-official-adapter-5.audit.json)明确 `public_product_gate_open=false`、`app_restart_and_ui_verified=false`、父权限上限未验证。第 2–4 轮的原始失败均保留，不追认为通过。

## 生产托管第 6 轮：历史继续缺少最终回放，完整流程失败

第 6 轮的[原始事件](validation/grok-official-adapter-6.ndjson)、[原始 metadata](validation/grok-official-adapter-6.metadata.json)、[原始 network](validation/grok-official-adapter-6.network.json) 和[原始输入清单](validation/grok-official-adapter-6.inputs.json)均经七类凭据格式扫描后按原字节归档，命中数为 0。独立 [audit](validation/grok-official-adapter-6.audit.json)保留各文件的字节数、SHA-256、来源与投影方式。原 metadata 的 `acceptance_passed=false`、libtest 退出码 101 保持不变；没有将只读审计或资源清理改计为验收通过。第 5 轮成功及第 2–4 轮失败的原始证据和结论均保留。

实际有 8 条输入、8 条 `native_receipt=true` 的原生接收确认及 8 次独立运行事件；只有前 7 条取得生产验证的完整历史终态，结果为 5 次 `Completed`、2 次 `Cancelled`。审核逐项核对了回合与会话 ID、完成 watermark、完整正文及最后响应的字节数和摘要字段。公开报告省略了非固定模型文本：仅 3 组完整正文及 4 组最后响应可以直接从公开原文重算摘要，其余摘要保留生产验证字段，不声称独立读取私有模型正文完成复算。审批派发与 `resolved` 仍为 `native_receipt=false`，不改称原生审批 ACK；运行中追加是后续回合排队，没有验证同轮 steer。

第一连接正常 `shutdown` 并记录清理确认。第二个生产进程继续相同原生 session `01a0af94-ed56-7303-a668-6c4c472161d3`，第 8 条输入已获原生接收确认和运行事件。[安全原生投影](validation/grok-official-adapter-6.native-events.json)末尾记录该进程的 prompt RPC `id=4`、`stopReason=end_turn`，随后是 final-history RPC `id=5` 的响应。但是夹具没有取得该回合的生产验证最终回放，也没有在公开事件中记录第 8 个最终结果；不能据此计为成功或历史继续验收通过。

私有夹具的固定失败为“回合终态缺少生产验证的最终回放”，来源为 `app/src/ai/cli_agent_runtime/grok_live_tests.rs:511`，UTF-8 长度 45 字节，SHA-256 为 `66c06b934d3a30a5d88781582b969c507630cdd757cbecdbbd19605dad1cadab`。这说明夹具缺少已验证历史，不说明具体历史校验失败原因。本轮安全原生投影没有保存 final-history 响应的错误形状或内部校验原因，未猜测截断、顺序或接口错误。日志中有 1,154 条从行首开始的 incoming 投影，另有 1 条合法初始化投影与 libtest 首行进度串接；归档完整保留这 1,155 条安全记录及源行号，不去重，也不保存其余私有 stdout/stderr。

两个不同的生产 leader socket 与运行代均有[实际退出回执](validation/grok-official-adapter-6.exit-receipts.json)：`stdio_closed`、退出码 0、`cleanup_confirmed=true`。[macOS 清理回执](validation/grok-official-adapter-6.macos-cleanups.json)确认两次 job 移除、resource CID 销毁、原生等待状态 0，未记录 execution failure；第一代为 `3c4ff12f-45ff-4989-844d-6ea822074f37`，历史继续代为 `e5080770-0d88-4862-a010-41247b6b1f63`。隔离认证副本已删除，归档代理仅检查其不存在，没有读取认证内容；运行器记录网络隧道已停止。退出回执按安全字段完整投影并保留原始摘要，不称组合 JSON 为原字节副本，也未独立重新核验 PID、manifest、claim 或 native 资源正文。清理成功不代表第 8 条输入完成。

网络记录有 14 次官方 TLS CONNECT、6,633,013 字节，拒绝 176 次 `unknown` 与 7 次 `xai_api` origin；没有解密 TLS，不能证明模型 HTTP 请求数或费用，也没有模型调用数或费用硬预算。配置字节差异仍保留为 `bytes_unchanged=false`；公开审计只接受固定原生 marketplace 初始化，权限部分不变，未复制私有配置或环境。

本轮组合是 source10 libtest 与 source8 未变的生产监督组件：libtest SHA-256 为 `05ec980d8e0906d7dee608253385c7c11b48bdf68e1d19ab87f3b61557257fcd`，监督器为 `6e1801d768bdcb3e4ae9e2de2dba2137386631294baa79c2e262efc29f8abb78`，source10 输入清单摘要为 `e544e075ca84e5546fd8d70f25c4f69430b27c9d102b79173f37f767ea11c5ef`。基线 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 的工作树仍有改动，source10 整体快照门禁也未通过，没有 bundle10。`production_runtime_commands=true`、`test_only_internal_command_switch=false`，不以内部测试绕过证明流程。没有验证协调器、SQLite、父子回收、SDK 来源、权限上限、GUI、桌面重启、活跃重关联、同提交跨平台或 SSH/tmux。第 5 轮及下文关于产品门禁关闭的表述属于其原始快照；本轮未复核之后的产品入口状态，也不改变 Goal 未完成的结论。本次只归档证据与中文说明，无需本地化变更。

## 生产托管第 7 轮：同 source11 的固定 8 输入流程通过

第7轮的[原事件](validation/grok-official-adapter-7.ndjson)、[元数据](validation/grok-official-adapter-7.metadata.json)、[网络](validation/grok-official-adapter-7.network.json)及[输入](validation/grok-official-adapter-7.inputs.json)经凭据模式扫描后逐字节归档。固定 `grok 1.0.30 (04b7ffed98c6)` 使用真实官方授权，libtest退出码0，`acceptance_passed=true`。本轮通过生产 `run_process`、`run_transport` 和 RuntimeCommand 路径，`production_runtime_commands=true`、`test_only_internal_command_switch=false`，没有使用测试内部命令绕过。

实际8个输入分别有原生接收确认及 Started事件，8个唯一回合均取得生产验证的最终history，结果为6次 `Completed`、2次 `Cancelled`。两轮正文准确返回 `PARITY_ONE`、`PARITY_TWO`；精确Write一次允许后，原生完成并返回最后响应 `APPROVED`，实际文件仅含15字节 `PARITY_APPROVAL`，SHA-256为 `f798dce6c3641f8ef768c8f34eafc743ea5cf37c581fee64d69f79e689cdb2c9`。Write拒绝回合取消、目标文件未产生。审批resolved仍为 `native_receipt=false`，不宣称独立原生审批ACK。

运行中追加输入排入后续回合，取得独立原生接收确认及结果，不声明同轮steer；实际正文开始后发送取消并取得 `Cancelled`。第一生产连接关闭并确认清理后，第二生产进程继续同一原生session，返回此前追加输入的完整排队标记。这是新进程继续历史会话，不是重新关联活跃任务或桌面应用重启。[独立审计](validation/grok-official-adapter-7.audit.json)保存8组原生UUID、完成watermark、完整正文与最后响应的长度及SHA-256。归档仅在内存复算全部8组长度和散列，不复制私有模型正文；允许回合完整正文62字节、最后响应8字节，完整累计正文没有因固定验收字面值比较被裁掉。

[安全历史快照诊断](validation/grok-official-adapter-7.final-snapshot-diagnostics.json)保留16条固定verdict、reason、计数与散列：8组 `pending→verified`，totalCount依次为 `3→4`、`7→8`、`16→17`、`23→24`、`27→28`、`31→32`、`34→36`、`40→41`，无rejected或unverified。各verified快照的原生回合散列、完成水位散列和正文长度与对应最终history回执全部匹配。[1,142帧原生协议投影](validation/grok-official-adapter-7.native-events.json)仅保存身份和稳定方法，未知标记仅散列，不导出思考、工具输入或原始stdout。第6轮失败及未查明原因继续保留；本轮通过不能证明已修复第6轮具体history问题。

[两次生产退出](validation/grok-official-adapter-7.exit-receipt.json)均为 `stdio_closed`、退出码0、清理确认true；[macOS清理](validation/grok-official-adapter-7.macos-cleanup.json)分别确认实际作业移除、资源CID销毁及等待状态0。认证副本路径已不存在，项目只含允许写入文件；配置字节变化仍保留，marketplace-only判断沿用原runner元数据，归档未读取配置原文。网络建立15条官方TLS连接、191次非官方origin请求被拒绝，无连接预算耗尽，不将TLS数当作HTTP模型调用数或成本上限。

本轮 libtest SHA-256为 `8f0e7063c37e7f110d3c022ccb932ca9cc73514b1675592ed8570b59b84ef484`，main为 `8026e131aad657207690144d31e3e5c01b654039c4ed60eb1d5502dc9cb14fbb`，同67文件source11 manifest为 `31619a72300d6a38e82ade3d40e2329039cee864e5c0ca38c29ceb38037e504a`。[本地门禁与构建](OFFICIAL_11_LOCAL_GATES.md)已通过，仍为基线 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 的脏中间快照，不计最终同提交跨平台门禁。`public_product_gate_open=false`、`public_coordinator_entry_verified=false`、`app_restart_and_ui_verified=false`，父级权限上限尚未验证。

## 尚未计为通过

第5与第7轮通过的是 macOS 官方生产监督链的固定8输入夹具。桌面应用重启恢复、重新关联活跃任务、GUI、SQLite任务记录、父子派发及权限上限、SDK工具来源、跨平台和SSH/tmux场景仍须取得完整证据；独立SDK来源探针尚未通过。本轮结果回收是原生回合的真实完整正文与最后响应，不代表协调器父子结果回收已通过。

通过自定义 Claude 后端取得的既有 Grok 协议证据与本次官方模型证据分开记录；不得互相替代。产品 Grok 托管执行门禁保持关闭，完整 Goal 未验收完成。本次仅归档验证证据及更新说明，无需本地化变更。
