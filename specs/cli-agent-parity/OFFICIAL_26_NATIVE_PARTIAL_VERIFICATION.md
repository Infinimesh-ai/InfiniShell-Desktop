# source26 原生阶段独立验证：Claude待Edit取消2与Grok只读预检1

本报告仅审计两组已结束的公共严格证据。实际受测提交为 `436cc739234061c092cedd106432bbbfa3c2d645`；输入清单SHA `3ff03385c6f9635c1e60304d3220c49cdb281f314e73735a7f90a3f3298875c9`，关联[入口先验](validation/macos-official-26-native-inputs-precheck.json)SHA `5bce6e8a4412d0dc7107ab7c9fb755af177b6ab7d0fc7d780cd670c2e4c7f163`。提交、干净111路径、固定CLI及前后产物身份由根实际报告提供；本次没有读取源码验证树、target或二进制。

| 在线阶段 | 实际退出／墙钟 | 本次独立结论 |
| --- | --- | --- |
| Claude等待Edit取消2 | runner 0、原生测试0；18.756秒 | 有限范围PASS：待审批取消→同S继续→正常stdio退出与清理 |
| Grok只读policy预检1 | runner 1、原生测试101；11.514秒 | FAILED：initialize完成，session/new没有关联响应；具体原因未知 |

## Claude：严格原生身份与取消证据通过

[公共事件](validation/official-26-native/claude-edit-cancel2-events.ndjson)经现有`project_events`和`audit_events`独立重算，通过封闭字段/类型、事件顺序及76条原生协议投影的连续序号与generation验证。没有启动运行器主流程，仅在独立内存环境提取并执行其纯审计函数与常量，未导入会派生CLI的适配运行器。

实际先发第一个输入，原生ACK与Started关联完整UUID，然后仅出现一个精确Edit审批。Edit没有获准，原生审批请求没有对应Allow响应。待审批时发送interrupt，native成功ACK、`aborted_tools`／`error_during_execution`／`is_error=true`结果及匹配该输入的cancelled生命周期全部成立，完整输入UUID、会话ID与generation一致，应用聚合为Cancelled。审批撤回事件本身的`native_execution_cancelled_verified=false`保持原值；真实取消结论来自独立关联的ACK、结果和生命周期组合。

取消后第二个输入在同一个原生S继续，重新取得ACK、Started及成功result，并聚合Completed。续轮完整输出摘要与夹具限定标记的摘要匹配；本报告不打印标记、Completed.output或模型正文。两输入、两结果UUID互不相同，前一轮全部取消证据完成后才发续轮，不能把过时结果作为新输入完成。续轮`cancel_terminal_observed`中的审批/interrupt布尔值是观测状态，不声称续轮产生了一次新interrupt或审批。

指定验收文件在before、after_cancel、after_continue和after_shutdown四次全文验证均为23字节、同SHA `bc1f758aa7017501e9e013d489b314cf9d43bbeac72db3859a0c7815e1ed6705`；仅证明该目标文件没有被Edit改写，不扩大为整个文件系统无写入。项目设置报告为未变化。正常关闭stdio后，公共生产收据明确`exit_reason=stdio_closed`、`exit_code=0`、generation匹配、`macos_resource_coalition`及`cleanup_confirmed=true`；未读取私有收据或重新查询内核。

[Claude原始metadata](validation/official-26-native/claude-edit-cancel2-metadata.json)和[根调用记录](validation/official-26-native/claude-edit-cancel2-invocation.json)均原字节保留。此有限PASS不包含App重启、SQLite恢复、GUI、同回合steer、父子权限上限、文件系统沙箱、模型HTTP计数或完整三方整链；原false字段没有回填。旧Edit-cancel1失败仍保留，不能追认为成功。

## Grok：只完成初始化，保留未知与失败

[公共事件](validation/official-26-native/grok-policy1-events.ndjson)共9条，现有封闭`project_events`验证0拒绝。固定版本1.0.30、固定profile/config摘要、没有always-approve／auto-mode请求。实际发送initialize（sequence/rpcId 1）并收到匹配generation/ID的ok响应；然后发送session/new（sequence/rpcId 2），写入前守卫均为true，但没有第二个关联rpc_response。

随后只有4条`probe_failed`，分别公开原因的长度和SHA，没有原因正文。本次不知道具体原生原因，不由散列推断认证、配额、方法不支持、profile加载或协议不兼容。没有完成会话创建、四个诊断接口、load、恢复或原生正常退出证明；也没有`probe_finished`或`process_cleanup`公共事件。未读取stdout正文，未用虚构stdout重新运行成功判据；仅严格验证公开投影及实际失败链。

[metadata](validation/official-26-native/grok-policy1-metadata.json)的`native_inputs=0`、`execution_boundary_passed=false`、`interface_investigation_completed=false`及`same_commit_verified_by_runner=false`原样保留。profile加载、有效模式、配置来源、内建目录和wrapper闭包全部unknown；父权限上限、沙箱及ready_for_policy_implementation保持false。失败不证明这些接口已经支持，也不证明它们不支持，不开放生产派发。

[网络记录](validation/official-26-native/grok-policy1-network.json)记录6次连接尝试、126452 TLS字节，最大16连接／8MiB；TLS未解密、模型HTTP数量未测量。协议native_inputs为0不能证明模型请求数或费用为0。runner报告opaque认证副本删除、隧道停止、私有工作区删除与总清理确认均true；这些只证明运行器资源收尾，不能冒充原生stdio退出0或正常生产收据。profile/config字节未变同样不能证明原生加载或策略有效。

## 来源、归档及验收边界

[根后置身份核对](validation/official-26-native/native-inputs-postcheck.json)SHA `a46b0b3526c9d63fc4c06ebf7af1ca7a1006e460cc5de122e097fd76f6854371`记录同436干净111路径及lib／main／固定CLI身份前后不变。独立归档核对前置引用、两次调用记录摘要和metadata产物摘要一致；身份重算由根完成，未重复读树或二进制。该后置报告还包含SDK9调用的汇总元数据，本报告没有读取或审计SDK9事件、私有诊断或结果，不据此扩张本次scope，也不改Grok运行器的same_commit=false。

全部8份原始公共JSON/NDJSON按原字节复制并收尾复检；[独立归档审计](validation/official-26-native/archive-audit.json)记录每文件字节数/SHA、原属性保留、纯审计函数身份及六类形状计数。序列化字节和各解码字符串均0命中；固定形状不能识别所有秘密。任何test-output.txt、私有runnerlog、nativeerror、raw stdout、认证和模型正文均未读取或复制；没有执行Cargo、Git、网络、原生CLI、模型或GUI，旧档及主三文档未改。

此次仅新增开发验证档案，**无需本地化变更**。英文／简体中文完整GUI布局、同提交全部平台、SSH／tmux、全工作区与三款CLI完整规定验收仍未完成；Goal保持未完成。

## 后续SDK9独立公共失败补档

本节补充前阶段尚未读取的SDK9公共记录，前文当时的有限范围与旧前缀原样保留。根[调用记录](validation/official-26-native/grok-sdk9-invocation.json)为同436提交、同输入／入口先验，runner退出1、耗时 **8.792秒**；[metadata](validation/official-26-native/grok-sdk9-metadata.json)记录原生测试101，整体仍为**FAILED**。调用记录摘要与前节根后置身份报告中的SDK9摘要一致；身份仍来自根原报告，没有重读源码或二进制，`same_commit_verified_by_runner=false`原值未改。

[10条公共事件](validation/official-26-native/grok-sdk9-events.ndjson)与metadata的public_evidence SHA完全匹配，[网络原记录](validation/official-26-native/grok-sdk9-network.json)同样匹配public_network SHA。独立核对生产失败为`production_runtime`／`runtime_error`／`protocol`／`invalid_response_id`。最后响应诊断为JSON-RPC2.0、无method、ID为13字节字符串、result为对象、没有error对象或stopReason；只公开类型／长度／SHA，没有读取ID实际值或响应正文。原生error侧文件的公开摘要为0字节、0条、空SHA，capture为`not_observed`，不是账户、认证、额度或API错误的证据。协议解析拒绝字符串ID的观察不能单独证明其更深来源或责任组件。

实际计数为现代discover1、tools/list1、SDK请求2、旧式initialize0、inspect0、权限请求0；应用提交与输入计数均1。现代发现／列表请求存在不能记为完整注册或业务调用成功：`discovery_native_contract_verified=false`、`native_contract_verified=false`，没有inspect业务来源与结果，origin仍unknown、产品gate和父权限上限保持false。旧式initialize计数0也不能单独否定现代发现路线。

原字段`transport_closed=false`、`cleanup_confirmed=true`及`cleanup_receipt_read=true`保留；资源清理确认不能代替连接正常关闭或原生退出0。`private_settings_unchanged=false`也保留：公共settings审计仅记录私有工作区marketplace初始化改变、permissions部分不变，不把该私有初始化说成用户全局配置改写或有效权限证明。没有读取任何私有settings、原生error、认证、stdout或模型正文。

本次用固定SHA的运行器纯`public_events`再次核对事件枚举与安全转换，没有执行主流程。该函数不是幂等验证器：两条已公开记录的未知散列键／键集散列成员在二次投影时再次散列；独立核对差异只限这两种重散列，其余固定字段相等，没有未知event。原档案不做二次转换，不将其冒充exact固定点PASS。[独立SDK9审计](validation/official-26-native/grok-sdk9-archive-audit.json)明确保留此边界和四份原字节证据的SHA／字节数；六类凭据形状的原字节及逐解码字符串计数均0。未改前阶段archive-audit，也未复制test-output或任何私有材料。

SDK9不计SDK注册、业务工具结果、子任务派发、父子权限、GUI、跨平台或完整Goal通过。**无需本地化变更**，原有三方整链／双语布局／SSH与tmux／全工作区验收仍待完成。
