# 最新版启动接口核验（2026-09-19）

本次是运行真实二进制取得的版本、帮助或初始化证据，不属于完整任务验收。没有复制登录凭据或提交模型输入。

| CLI | 实际执行 | 确认结果 | 限制 |
| --- | --- | --- | --- |
| Codex 0.155.1 | 版本、app-server 帮助、默认及 experimental schema 导出，共 5 次正常退出 | 完整运行包 42 文件摘要吻合；现有方法及主要字段保留，15 项出站局部形状检查通过；dynamicTools 仍需 experimentalApi | 未连接 app-server 或认证；新增而未支持的服务端请求仍应拒绝 |
| Claude 2.1.278 | 版本和帮助，均正常退出 | stream-json、permission、resume、session-id、partial/replay、settings、plugin-dir 参数存在 | 未运行双向协议、模型或取消链 |
| Grok 1.0.34 | 5 个版本／帮助命令，唯一无认证 ACP initialize | protocolVersion=1、agentVersion=1.0.34、image=false、audio=false、embeddedContext=true；load/list/resume/close 公布；2.632 秒正常退出且无进程残留 | 无认证态能力不代表登录后任务验收；不能由随包文档类别写法改动线上 `_x.ai` 通知前缀 |

Grok 私有合成 HOME 出现文档／空会话索引，并增加 `marketplace.default_skills_installs_purged`；只记录了整组前后状态，无法归因于某一条 help 或 initialize。既有合成配置值保持；未访问真实用户 HOME。该观察需纳入后续探测副作用验证。

新版适配候选将精确绑定本次版本探测与握手版本，保留旧版本历史测试；原生完整任务链、审批、插件及恢复仍待验收，不直接把新版本纳为交付已通过。

安全证据：

- [Codex 协议结构](validation/codex-latest-protocol-interface-20260919.safe.json)
- [Claude 参数](validation/claude-21278-version-help-20260919.safe.json)
- [Grok 初始化](validation/grok-latest-protocol-interface-20260919.safe.json)


## Grok 1.0.34 独立原生 P0 首例

候选探针通过 27 项离线回归后，绑定脚本、共享导入和官方 34 二进制进入原始 ACP。首例发送 3 个协议请求，尚未发送模型输入，就因配置校验退出：原生只新增 `marketplace.default_skills_installs_purged = true`，而复用的旧审计仅识别完整市场初始化结构。权限和其余配置没有变化。

此次有 6 个官方 TLS 连接、24,534 字节，未解密；认证副本已删除，进程／代理清理确认，正常 EOF 次数为 0。它不构成审批、取消或恢复通过。[失败收据](validation/grok-1034-native-p0-native1-failed.safe.json)保留不覆写；下一候选将只为实际观察到的单字段初始化状态添加精确审计，不扩大权限，也不把它并入产品能力通过。


第二例只放行已观察到的精确市场清理标记，通过 33 项离线回归后重新建独立夹具。第一条输入写出后收到尚未识别的原生通知，探针在回合／工具／审批身份绑定前停止：实际 1 条输入、4 个 RPC、8 个 TLS 连接／27,544 字节；没有通过的阶段，正常 EOF 0 次，进程／代理清理和认证副本删除确认。该事件待核对官方接口；[第二例失败收据](validation/grok-1034-native-p0-native2-failed.safe.json)不替代产品生命周期通过。

后续三例分别定位 `_x.ai/settings/update`、`_x.ai/announcements/update` 和 `session_summary_generated`。每例均为 1 条输入、4 个 RPC、0 个通过阶段；工具和审批尚未观察，清理及认证副本删除确认。独立失败收据：[第三例](validation/grok-1034-native-p0-native3-failed.safe.json)、[第四例](validation/grok-1034-native-p0-native4-failed.safe.json)、[第五例](validation/grok-1034-native-p0-native5-failed.safe.json)。原生二进制字符串与保存的官方源码对应说明：前两项为远端界面设置／公告广播，后一项为会话标题通知。源码快照不宣称与 1.0.34 提交完全相同；实际通知形状和摘要来自该固定原生二进制。

独立探针的后续候选只忽略上述非任务载荷；设置／公告正文仅保留摘要，不改变权限、会话、当前回合或完成判据。标题仍须属于已绑定会话，不能进入回合输出。未知方法、带 ID 的反向请求及真实终态仍分别核对。该候选通过 40 项离线回归，不属于冻结 source45 的通过计数，也不代表生产 Grok 能力已开放。

## Grok 1.0.34 原始 P0 后续与当前边界

第六至第八例继续定位会话信息通知、原生内部 skills reload 回执和非终态扩展；均保留原失败，未把通知误作任务完成。当前独立探针仅对已核对的非终态扩展保留计数／摘要，真正终态、反向审批请求、会话／回合身份和未知响应 ID 仍严格核对。内部 reload 只接受精确封闭结构及无符号计数，不消耗业务 pending。

[第九例](validation/grok-1034-native-p0-native9-partial-failed.safe.json)实际通过前三阶段：合成文件读取允许并返回隐藏标记、拒绝读取且取消类别吻合、输出中取消；各阶段取得真实审批／工具／终态和历史记录。实际 3 条输入、15 个 RPC、2 个原生进程，第一个进程正常 EOF。第二进程加载旧会话时，原生通过 `_x.ai/session/update` 回放历史，探针当时尚不支持该别名，整体失败且未发送第四条输入。

后续候选只允许同会话、无活动新输入且明确 `isReplay=true` 的该历史别名，回放不会推进业务事件序列或完成新任务。44 项离线回归通过，但不并入 source45 的 321 项计数。[第十例](validation/grok-1034-native-p0-native10-failed.safe.json)使用此候选另建夹具，首阶段已观察并允许一次真实读取审批，但 120 秒内没有取得回合响应，报 `native_response_timeout`；1 条输入、4 个 RPC、0 个通过阶段，不能确定超时原因。两例的进程／代理清理和认证副本删除均确认，失败不覆盖。以上均为原始 ACP 探针，不等于生产适配器、父子任务或应用恢复验收，1.0.34 的未验证托管能力继续关闭。


第十一例在真实读取和历史查询后报 `allowed_read_result`，旧断言同时检查工具完成和精确文本，原收据不足以区分哪一项失败；不归因为已知格式问题。[原失败](validation/grok-1034-native-p0-native11-failed.safe.json)保留。下一候选将两项分别记录，允许返回内容周围的排版文字，但仍要求唯一完整随机标记、2048 字节上限、真实工具完成；多个／错误／更长标记及失败工具仍拒绝。冷恢复改为与第一轮已验证输出的完整摘要相等。47 项离线回归通过。

[第十二例](validation/grok-1034-native-p0-native12.safe.json)原始 ACP 四阶段全部通过：读取允许、读取拒绝、真实输出后取消、旧进程正常退出后同原生会话加载并回忆第一轮内容。实际 4 条输入、19 个 RPC、2 个原生进程与 2 次正常 EOF；允许和回忆阶段实际均为精确 64 字节标记。14 个官方 TLS 连接、5,326,061 字节，未解密；进程／代理清理和认证副本删除确认。只发生允许的原生市场初始化，权限部分与其他配置语义保持。它证明固定 1.0.34 原始接口四阶段可用，**生产运行器、固定策略、SDK／父子／技能、GUI 和跨平台仍未通过，代码能力门禁尚未据此开放**。独立源码摘要 `37aee0beeaee9ec8ff1d27b5449a41c73478be20a449cd3c14c6a50ac39da5eb`，不属于 source45 的冻结代码。
