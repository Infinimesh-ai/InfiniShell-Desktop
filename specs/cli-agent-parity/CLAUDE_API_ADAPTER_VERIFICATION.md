# Claude API 与生产适配器验收

2026-09-17 使用用户明确指定的 API，在隔离配置和项目中验证固定 Claude Code 2.1.273。密钥只从仓库外私有文件读入环境，未进入源码、命令参数或交付记录；没有改变用户全局 Claude 设置。

## 来源与范围

- 基础提交：`49456958279cc193392b2d09118e680fc0b14ea4`。
- 四文件冻结输入新增 ignored 验收和运行器，直接调用生产 `claude::connect`，不使用模拟协议。
- 冻结输入通过 `cargo check -p warp`、11 项 i18n 测试、988 项相关测试；运行器 9 项离线测试通过。
- 同一输入构建的监督程序 SHA-256：`b2c463740a9cffa34fa99c8a67b9fcc685270d4a3c075a184d4f10748386064c`。
- [来源与门禁记录](validation/macos-claude-adapter-494-source.json)明确标记 dirty 中间快照，不能代替最终同提交验收。

## 修复前真实结果

[第四次尝试的事件](validation/macos-claude-adapter-494-frozen-4.ndjson)及[运行元数据](validation/macos-claude-adapter-494-frozen-4.metadata.json)保留以下边界：

| 项目 | 实际结果 |
| --- | --- |
| 初始化与真实两轮 | 通过；原生确认与回合结果均关联同一会话 |
| 同消息 ID 重投 | 未产生重复回合；每个消息均有原生接收确认 |
| 审批允许 | 精确匹配临时文件 Write，一次允许后文件内容与预期一致 |
| 审批拒绝 | 一次拒绝后原生结束，目标文件不存在 |
| 运行中追加 | 下一条输入排队，原生确认并完成独立回合，返回随机追加标记；不计为同回合 steer |
| 刚开始时取消 | 未通过；interrupt 得到确认，但终态为执行错误，未转成 Cancelled |
| 继续与历史恢复 | 本次未执行，取消断言阻止后续流程 |
| GUI、应用重启、父子任务与结果回收 | 本次不覆盖 |

取消错误为 `[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null`。适配器保留 Failed，没有把 interrupt 的确认冒充取消完成，也没有把失败升级为成功。后续须核对运行中与刚启动时的真实原生终态，并用夹具和模型验证修复。

前三次尝试在模型请求前失败：launchd 已启动 wrapper，但采样显示它停在 dyld 打开外置磁盘上的监督程序，控制握手超时。将同一二进制逐字复制到本机临时目录后，第四次完成初始化并进入上述模型流程。这里只确认实际采样和复制后的结果，未证明磁盘、权限或系统内部的最终原因。

该快照验收未通过；项目级审批设置保持不变。此测试不证明 Claude 父任务的完整权限上限，也不证明文件系统沙箱。

## 取消事件修复

[真实流输出后中断](validation/macos-claude-native-cancel-partial-1.json)与[刚开始时中断](validation/macos-claude-native-cancel-started-1.json)都确认相同顺序：原生 ACK 成功，随后 `result` 的 `terminal_reason=aborted_streaming`，最后同一 `command_uuid` 的 `command_lifecycle.cancelled`。原生结果形态是 `error_during_execution`，因此旧适配器过早固定 Failed 并忽略后续取消。

修复聚合同回合的已发出 interrupt、原生成功确认、明确流中止结果及原生取消生命周期，支持事件乱序。认证和 API 错误仍优先 Failed；缺少任一必要证据时最多等待 30 秒，失败而不冒称已取消。新增 11 个回归保留原认证失败负例。

## 修复后生产适配器复验

19 文件冻结输入通过 check、i18n 11 项和相关模块 1001 项；[来源清单](validation/macos-claude-api-cancel-1-source.json)绑定输入、测试与监督二进制摘要。同字节监督程序复制到内部磁盘，仍经生产进程监督入口启动。该输入是 `494569582` 基础上的 dirty 快照，最终提交仍须验证。

[真实事件](validation/macos-claude-api-cancel-1.ndjson)与[运行元数据](validation/macos-claude-api-cancel-1.metadata.json)证明：

- 新建、两轮中英文输入及消息 ID 重投去重通过。
- 单次 Write 允许与拒绝通过；分别核对真实文件内容与不存在的目标文件。
- 运行中追加输入得到原生确认，排队为独立回合并返回随机标记；不声明同回合 steer。
- 刚开始时 interrupt 在完整原生证据聚合后得到 Cancelled，不再错误显示 Failed。
- 关闭连接并启动新进程，以同一原生会话 `52a4086a-90c2-4347-be86-af3886571211` 恢复，未重新提供随机标记仍精确回收其内容。
- 测试正常退出 0，项目审批配置摘要未改变。

本轮 Rust 适配器进程重启链路通过；GUI、应用数据库重启恢复、父子消息与结果回收、父权限上限及文件系统沙箱仍不在通过范围内。
