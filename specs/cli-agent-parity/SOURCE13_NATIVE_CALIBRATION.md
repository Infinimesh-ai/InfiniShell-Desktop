# Source13 原生能力校准：两轮整体失败

本记录归档 macOS 的 Grok SDK4 与 Claude 图片1。两轮使用固定原生 CLI、source13 的 lib test/主程序和生产进程监督与清理链，**整体均为 FAILED**。协议子证据和资源清理不能替代夹具成功，不计三方能力对齐、产品能力开放或 Goal 完成。

| 校准 | 固定 CLI | 原生输入 | 整体结果 | 产品门禁 |
|---|---|---:|---|---|
| Grok SDK4 | 1.0.30 (04b7ffed98c6) | 1 | FAILED，test exit 101 | SDK/子任务关闭，origin unknown |
| Claude 图片1 | 2.1.273 | 1 | FAILED，native_cleanup_not_confirmed，test exit 101 | 图片关闭，仅 test-only framing 校准 |

## 来源绑定

source13 为 baseline `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 上的 dirty 中间快照；原 runner 未确认最终同一提交，不计最终验收。本次首轮独立只读核对 [73文件来源清单](validation/macos-official-13-inputs.json)，冻结73文件的 SHA256 全部一致，没有重新读取大二进制。

- 来源清单 SHA256：`419e6a6fac30237c4577061cb753078360fdf42e5833de29266363e3cdd5dd82`
- lib test SHA256：`a857fbb82541ff572591bb9ab526a187e1a96325063636a7f72f928d6864e471`
- main / supervisor SHA256：`0c9e05fe2c75aaf909e31c2dfb4508e5f7ea3ee9da6c7a8e167713ae8205d77f`

两轮元数据与已归档的 [source13 本部门禁及构建](OFFICIAL_13_LOCAL_GATES.md) 使用同组 lib/main SHA。二进制散列来自原 runner 和构建报告，本次没有重新读取二进制计算。

归档准备期间冻结树已前进 source14；后一次只读发现来源清单内4项改变，根代理确认 source13 两轮执行完成后才前进，source14 有75文件、manifest SHA `587c446d737259bc48a913256008558c87b450d5f29304015a10841ed31eec93`。两组 inputs.json 明确记录 snapshot_advanced，不把后续树或新 lib 替换本轮身份，也不把这一前进当作 source13 校准失败原因。本次没有修改冻结树或 target。

## Grok SDK4

[安全事件](validation/grok-sdk-origin-probe-4.ndjson)、[原 runner 元数据](validation/grok-sdk-origin-probe-4.metadata.json)、[网络记录](validation/grok-sdk-origin-probe-4.network.json) 均原样归档；[独立审计](validation/grok-sdk-origin-probe-4.audit.json) 区分公开事件的独立核对与根代理的私有帧安全投影。

公开事件中两个 native tool update 的 session、prompt、toolCall 投影散列完全相同，event ID 不同，序列为2和3；工具名 JSON 散列精确对应固定字面量 `use_tool`。ACP initialize 宣告 SDK capability=true，但本轮 SDK 请求、SDK initialize、tools/list 与 inspect 实际调用均为0。原夹具把这两个包装工具帧计为 unexpected=2，出现1次原生权限请求并全部拒绝，失败原因为“原生模型调用了探针之外的工具”。origin 保留 unknown，不能据此宣称 SDK 不支持、来源不存在，或 inspect 必需审批。

根代理提供的安全投影另外确认：包装目标为 `infinishell-sdk-origin-probe__inspect`，`tool_input={}`；initial rawInput 只有 `tool_name`、`tool_input`，update 和 permission 多出 `variant=UseTool`，属于同一原生 session/toolCall/prompt。本归档子代理**没有打开、读取或复制私有原始工具帧**；3帧、2395字节、0600权限来自根代理报告，不是本次独立原帧核验。

[生产退出收据安全投影](validation/grok-sdk-origin-probe-4.exit-receipt.json) 确认为 `stop_requested`、exit code=0、cleanup=true；[macOS 清理回执](validation/grok-sdk-origin-probe-4.macos-cleanup.json) 确认 job_removed 和 resource_cid_destroyed。原 probe 的 transport_closed=false、no_side_effects=false 仍原样保留，资源清理不等于传输任务正常返回。代理停止、授权副本移除仅按原 runner 元数据报告，本次没有读取认证内容。

网络层观察8次 TLS连接、4,328,380字节，32次非官方来源拒绝，预算未耗尽；TLS未解密，HTTP模型请求和实际费用不可观测，不把一次原生 prompt 当作一次HTTP请求或费用证明。私有配置原始 bytes_unchanged=false 保留；严格TOML审计只确认原生 marketplace 初始化，权限段未变，此项来自原 runner 元数据，本次未读取配置原文。

## Claude 图片1

[安全事件](validation/claude-image-probe-1.ndjson)、[原 runner 元数据](validation/claude-image-probe-1.metadata.json)、[独立审计](validation/claude-image-probe-1.audit.json) 记录了真实协议子证据。64×64、自生成四象限PNG为12,420字节；用户输入为 `[text,image]` 数组，image source 为 base64、MIME为 image/png，原生 replay 数组投影与输入完全一致。

- PNG SHA256：`427acf9b875f380ccff526a3841fcd96117edf890e551438b05cf84e4da169e5`
- 完整输入/replay数组 SHA256：`e636f86fc89c86f457f5ff83f9ba4e6f4ac65f2267bd4124eeaf614dcbafa08e`
- 提示文本 SHA256：`356737d1060e0761b76a4b7cb20ffb253b4b3d0209229def05c63e0d150f0c06`
- 21字节预期四色答案、完整 assistant 与完整 result 文本 SHA256：`eb2bb99544bcf25a042403af71b64fac362c1d4abe748e81bfd79e82f94cc69a`

输入UUID `9acbdb87-4e59-4bfb-9575-49b9752926f7` 与原生 queued ACK、started、user replay、assistant/result 的单元素 UUID数组逐项对应；后续帧 native session 均为 `b5576cf1-431c-42e3-a23b-3abfc3800330`。result 为 success、is_error=false、terminal_reason=completed；工具与 MCP为0，没有额外工具或权限请求。这些只证明本轮图片framing协议交互的子路径，整体 native_image_input_verified 仍保留 false。

[生产退出收据安全投影](validation/claude-image-probe-1.exit-receipt.json) 独立确认 `stdio_closed`、exit code=0、cleanup=true；[macOS 清理回执](validation/claude-image-probe-1.macos-cleanup.json) 的 job_removed/resource_cid_destroyed 均为true。但 child.finish 返回错误（根代理报告），公开 cleanup_checked 的 transport_closed=false、cleanup_confirmed=false，因此严格门槛给出 native_cleanup_not_confirmed，整体失败。不能把独立收据的正常退出替代传输任务成功，不能计为产品图片支持。

原始 test-output 为651字节，仅凭据形态扫描与SHA256记录，未复制或打印正文；SHA256为 `2e50b7263600ddce40a33eed9d6d64c59a29b75733b29160d86c754c18f80d53`。图片数据、base64、assistant正文、认证或全局配置均未进入新增归档。本轮未验证生产字符串adapter图片路径、持久化、应用重启、GUI或跨平台；生产图片能力门禁维持关闭。

## 归档边界

原样归档的5份安全公开产物、Claude原始测试输出，以及两组退出/清理回执均完成凭据形态扫描，只报告计数，全部为0。新增JSON均可解析，安全原产物复制逐字节一致。SDK原始私有工具帧不读取；退出收据只保留固定类型字段与原文件SHA，manifest/claim/native文件不读取。没有新增用户可见产品行为，无需本地化变更。

## Source14 图片2对照：原生校准 PASSED

[图片2安全事件](validation/claude-image-probe-2.ndjson)、[原 runner 元数据](validation/claude-image-probe-2.metadata.json)、[独立审计](validation/claude-image-probe-2.audit.json) 确认固定 Claude Code 2.1.273 的一次 bare PNG 数组framing校准为 **PASSED**，test exit=0。native_image_input_verified=true 仅属于该校准范围；production_image_gate_open=false 保留，SDK、typed生产connect、图片GUI、持久图像、其它图像格式与平台、应用重启及最终同提交均未计通过。

| 观察 | Source13 图片1 | Source14 图片2 |
|---|---|---|
| PNG数组输入/replay、原生ACK/Started、单UUID答案/result | 本轮协议子证据成立 | 本轮严格校准成立 |
| 完整标准输出末帧 | 没有归档末帧证明 | 201字节、1帧、同session/input的completed |
| transport_closed / 夹具cleanup_confirmed | false / false | true / true |
| 独立生产exit收据 | stdio_closed/0/cleanup=true | stdio_closed/0/cleanup=true |
| 整体 | FAILED，保留原判定 | PASSED，限bare原生PNG校准 |
| 产品图片门禁 | false | false |

本轮来源是 [source14的75文件清单](validation/macos-official-14-inputs.json)，SHA256 `587c446d737259bc48a913256008558c87b450d5f29304015a10841ed31eec93`；lib test SHA `ec8e9c0f1c5281b7dd35050dd1c6383b0ff854bfe7bf2b29609f69a109024880`，main/supervisor SHA `0993d97e13317064d25895a0791c884cb00363684a9e8f6f5675640cd3ccbd2b`。归档首轮和完成前独立核对冻结75文件，两次均全部匹配；public清单与原始清单逐字节相同。二进制SHA按实际runner元数据/根代理报告绑定，不读取后续新lib替换身份。baseline仍为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 上的dirty快照，不计最终同提交。

本次离线调用 source14 runner 的纯审计函数 `audit_events`，仅以AST载入常量与纯函数、标准库支持，不导入adapter、不读取环境/凭据、不执行CLI或模型。返回proof与原metadata完全一致；除公开安全事件及固定路径退出/清理收据之外，未读取私有协议原帧。

本轮64×64四象限PNG为12,420字节，PNG SHA `223a558566823dd0cc308641902bed47eee314225f6504ed3c3288d23499a782`；完整输入/replay数组 SHA `16a46d3f9cdaada34815814d54c31bdd2dd8e1f66b87a3c71f7a3e2a683ad0e4`，MIME image/png、source base64、`[text,image]`块类型与投影完全对应。

输入UUID `7fd113e2-e047-4713-aded-5e38efd8046f`、native session `37559744-50aa-401c-99f4-21304bf21e79` 与原生queued ACK、Started、replay、assistant和result逐项对应；result UUID为 `c0fb0060-a0fb-44ff-97a8-2d2d17b77845`，user_message_uuids严格只有该输入。21字节四色答案、完整assistant/result文本 SHA均为 `5ef33ded0c6b498e18f8ee013d6f59b00785bb1382ca39497ed97054ed8dd364`，result success/is_error=false/terminal_reason=completed，工具/MCP为0，无额外审批或工具调用。

shutdown_output_drained记录201字节、1个同session/input的completed帧，SHA `4870f76703b935d202d02fdb8a2cefad5aff511acb5c33d36de04f41b0f40a3f`。原始尾流字节没有复制或复读；本次独立核对公开投影、严格顺序与对应关系。公开transport_closed=true表明正常finish，另外只读[生产exit收据](validation/claude-image-probe-2.exit-receipt.json)确认为stdio_closed/0/cleanup=true；[macOS清理收据](validation/claude-image-probe-2.macos-cleanup.json)的job_removed/resource_cid_destroyed均为true、native_wait_status=0。

source14收尾实现调整后，本轮观察到完整末帧与成功收尾。source13没有保存具体old.finish.error，**不根据本轮成功追认旧轮通过，也不据此证明旧轮失败的精确I/O因果**。source13 SDK4全部拒绝和整体FAILED保持原记录，本次没有重跑或开放SDK能力。

图片2两份公开产物原样逐字节归档，新增JSON均可解析，全部凭据形态计数为0；原始test-output为483字节，仅扫描散列及安全测试摘要计数，SHA `d7ca7a03748b081fe29de36a348488e6ee0f1f74e1d9e4b4c84b830dfebbc3fc`，没有复制/打印正文。没有新增产品用户可见行为，无需本地化变更。
