# Grok 生产工具租约与 source34b 验证

代码检查点 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607` 已提交并推送，129 项受测输入逐项与提交内容相符。macOS 构建来自父提交上的隔离候选工作树；不能将源码对应核对写成干净提交构建。Goal 仍在进行。

- 本地 `cargo check -p warp`、`cargo test -p warp --lib i18n::tests`（11 项）、受影响 Rust 1495 项全部通过；76 项新增或重命名的非 ignored 测试逐项执行。Python 17 组共478项、Node13项通过。Python/Node运行后仅修正一个Rust测试名称预期，脚本输入未变化。
- 初次 source34 有1项新增测试失败：预期工具名误写为 `send_message`，生产注册实际是 `send_message_to_agent`。修正后的 source34b 全绿，原始失败报告保留。
- main 构建314.503秒、严格签名和完整英文／简体中文资源嵌入通过，测试库前后未变；不等同双语界面布局验收。
- **真实 lease1 通过**：1次原生输入、发现工具与inspect审批各1次；生产SDK登记2次，1次实际业务派发、1次响应写入、1次原生工具完成。原生最终历史、指定结果与进程清理均确认；认证副本已删除，原始认证文件元数据不变。
- lease1 使用真实生产连接及租约账本，工具返回隔离空任务集。它证明传输与调用归属，不证明协调器数据库派发、父子消息或权限继承。原生SDK回调仍缺S/P/T字段；关联由独占进程、注册nonce、真实工具事件与已写入审批建立，没有回填或伪造原生字段。
- **policy7 失败**：0次模型输入，仅initialize、authenticate、session/new三个RPC；兼容模型通知被拒绝，New响应仅在退出排空中观察，不能计New成功。既有官方空会话夹具已显示本次预检遗漏 `reasoning_effort` 字段及 `_x.ai/models/update` 通知，正在修正。原始失败及清理收据保持；父权限上限未验证。
- [CI14](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35363371460)实际head为上述提交，Linux与Windows已启动。此次 `full_workspace_tests=false`，不是最终全工作区门禁。

证据：[输入](validation/macos-official-34b-inputs.json)、[Rust门禁](validation/macos-official-34b-gates.json)、[脚本门禁](validation/macos-official-34-python.json)、[Node](validation/macos-official-34-node.json)、[main](validation/macos-official-34b-bundle.json)、[租约事件](validation/macos-official-34b-lease1-events.ndjson)、[租约边界](validation/macos-official-34b-lease1-events.metadata.json)、[权限调查失败](validation/macos-official-34b-policy7-events.metadata.json)、[提交对应](validation/source34b-code-commit.json)。原始认证、原生私有日志和任意响应正文未归档。

后续未提交候选已接入Grok消息开关及双语说明，并拆分本地派发类型以保留Grok身份，尚待统一编译与UI验收。子任务权限、三方完整流程、最终同提交跨平台、完整工作区与SSH/tmux仍未完成。
