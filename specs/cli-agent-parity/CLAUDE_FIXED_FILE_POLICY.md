# Claude 创建时固定的文件审批策略

新增显式 `ClaudeRestrictedFilesV1` 策略，为本地任务提供可保存、可核验并由子任务继承的工具权限上限。普通 `Inherit` 模式继续使用原生配置；不能只凭其权限观察快照开放子任务派发。

## 实施边界

创建前核对固定 Claude Code 2.1.273 发行文件 SHA256，再以无用户输入、无工具能力的独立原生进程查询配置、权限规则和 hooks。仅支持能被明确保留的个人配置子集；管理员策略、未知权限模式或规则、附加目录、hooks 等未证明配置拒绝启动。预检不改写用户全局配置。

任务进程使用 `--bare`、`--restricted`、`plan` 和固定 Read/Edit 工具集。Edit 每次请求审批，shell、技能、自动 hooks 和未允许的 MCP 工具禁用。该模式要求 API 认证；界面提供中英文说明。它是固定 CLI 工具策略，不构成操作系统沙箱声明。

完整策略保存原始目录及创建时规范目录、可执行文件摘要、来源规则、有效 deny 和本地工具权限。每次输入、审批允许前重新核对原生状态。路径审批使用保存的规范目录；目录别名改指向时拒绝，不能随当前符号链接移动原有权限边界。继续历史会话时重建并比较同一策略，目录、规则或工具权限变化均拒绝。

协调器在发布 Ready、释放首条输入前将策略写入 SQLite；恢复重新读取已保存策略。子任务获得相同的完整策略和父代权限上限，普通 Inherit 派发限制保留。重复 Ready 不得扩大策略，过时回合审批或已消费的一次允许不会重放。

## 当前验证

`b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 加 [36 文件快照](validation/macos-profile-2-inputs.json)通过 [本地门禁](validation/macos-profile-2-gates.json)：`cargo check -p warp`、`cargo test -p warp --lib i18n::tests` 11 项和相关 nextest 1,072 项。第一轮遗漏测试构造字段导致编译失败，记录保存在 `macos-profile-1-*`，未计通过；修复后完整重跑。

这些是中间快照门禁，不是最终干净提交的跨平台验收。真实父子 MCP 派发与消息回收、应用重启以及新策略双语 GUI 完整交互仍须分别执行并补证据。

## 第一轮真实 API 结果：未通过

同一 36 文件快照已构建主程序并运行生产监督链，见 [原始事件](validation/claude-profile-live-1.ndjson)与[运行元数据](validation/claude-profile-live-1.metadata.json)。初始化、原生策略复核和两轮文本通过；第三轮实际 Read 成功，但 Edit 的原生工具结果报 `tool approval does not match this running command`，没有发布可操作审批，模型返回 `EDIT_DENIED`。

这不算“允许编辑”通过，整轮验收为失败。项目权限文件与两个编辑夹具未改动。[原生工具投影与退出回执](validation/claude-profile-live-1-native-tool-results.json)保存 Read/Edit 调用结果及生产资源域清理确认。该失败记录保留，第二轮修复与结果如下。

## 第二轮真实 API 结果：限定流程通过

固定原生版本只为每轮首条主 assistant 消息添加回合标记；后续工具调用省略该标记。[来源证据](validation/claude-2.1.273-assistant-provenance.json)区分固定二进制代码、synthetic 原生观察与第一轮真实 API 日志。适配器沿用同一有序输出流中已确认的归属，并核对 API 消息 ID、会话、活跃回合、工具 ID 和输入摘要；生命周期结束、结果、新回合和取消请求清除归属。没有确认来源时不使用当前回合兜底。

[44 文件快照](validation/macos-official-2-inputs.json)通过 [本地门禁](validation/macos-official-2-gates.json)：check、11 项 i18n 与 1,081 项定向测试，包括省略标记、旧 API 消息换 UUID 重放、取消期间回调和审批输入替换负例。[对应主程序构建](validation/macos-official-2-bundle.json)与测试二进制用于 [第二轮真实 API 流程](validation/claude-profile-live-2.ndjson)，[元数据](validation/claude-profile-live-2.metadata.json)确认验收退出 0。

两轮文本、Edit 允许与拒绝及实际文件效果、序列化完整策略后新进程继续同一原生会话、目录／权限规则／本地工具扩权的三项恢复拒绝全部通过。两次连接均有生产清理回执。此项不覆盖完整 GUI 应用重启、父子 MCP 链、操作系统文件沙箱或原生运行中切换权限模式，不将这些验收计为完成。

## 双语策略说明布局

38 文件中间构建的隔离应用已分别检查[简体中文](validation/gui-claude-fixed-help-1/zh-CN.jpg)与[英文](validation/gui-claude-fixed-help-1/en.jpg)：选中固定文件审批策略后，工具限制、拒绝规则、API 认证和不支持配置的完整说明均可见，按钮和下方控件没有截断或重叠。[原始截图摘要](validation/gui-claude-fixed-help-1/manifest.json)关联该构建与重新签名的私有副本，不关联已被后续构建覆盖的共享二进制。

本项仅验证静态双语布局，未提交模型输入，不计固定策略的完整 GUI 权限执行或应用重启恢复通过。
