# Claude 等待 Edit 审批取消夹具

当前为准备态：新增夹具尚未接入模块、未通过 Cargo、未运行真实 CLI 或模型、未验证 GUI 和跨平台。它用于定位 [source6 GUI 第五代失败](CLAUDE_GUI_RUNTIME_CHAIN.md)，不改写旧 Failed 记录，也不放宽生产取消判据。仅新增测试及文档，无产品文案变化，无需本地化变更。

Rust 夹具位于 `app/src/ai/cli_agent_runtime/claude_approval_cancel_live_tests.rs`，设计为已有 `batch_cancel_live_tests` 的子模块，复用其生产 `start`、`submit`、固定策略检查及祖父模块的 `LiveSession`、`Evidence`。根代理审核后，在 `claude_batch_cancel_live_tests.rs` 的模块声明处加入以下两行；本准备任务未写入该文件：

```rust
#[path = "claude_approval_cancel_live_tests.rs"]
mod approval_cancel_live_tests;
```

精确 libtest 名称为 `ai::cli_agent_runtime::claude::live_tests::batch_cancel_live_tests::approval_cancel_live_tests::real_claude_pending_edit_cancel`。原生入口固定 Claude Code 2.1.273，权限使用生产 `ClaudeRestrictedFilesV1`，本地任务 SDK 关闭，模型必须显式固定。

最多发送两个原生 user 输入：第一条进入唯一隔离 `edit-cancel.txt` 的 Edit 审批，保持未回应后发出一次 interrupt；第二条在取消确认后，于同一连接和原生会话中显式继续，只输出新的随机标记。第一阶段最多允许一次 Read 为 Edit 读取自建目标，绝不允许 Bash、搜索、其它编辑或其它工具；原生工具 ID 账本检查 Read 数量和顺序。Read 的具体输入路径没有独立原生投影证明，不能据此宣称该路径已单独审计；生产固定策略仍约束在新建隔离项目内。若 Read 本身需要审批，夹具拒绝并失败。继续阶段禁止任何工具。

Edit 的工具名、执行 ID、原生工具 ID、唯一绝对目标、完整 old/new 文本、可选 `replace_all=false` 和无额外输入字段均在原生 `ApprovalRequested` 到达时检查。绝不发送 AllowOnce；未知或超范围审批按既有拒绝路径结束该次验收。正常范围的 Edit 保持未回应，不把普通 DenyOnce 当取消。公开产物只保留目标相对名、完整文本摘要、输入字段名及安全 ID，不保存任意协议正文。

成功必须同时满足：

- 本次运行代的唯一原生 interrupt 请求及对应嵌套 success ACK。
- 同一原生会话、同一执行 UUID 的 `command_lifecycle.cancelled`。
- 覆盖精确单输入 UUID 集合的原生 result，形态严格为 `aborted_streaming/error_during_execution/is_error=true`，或 `interrupted|cancelled/success/is_error=false`。
- 生产 `TurnFinished::Cancelled` 和本次待定审批撤销；审批撤销本身不能证明执行取消。
- 目标文件在开始、取消后、继续后和连接关闭后均逐字节等于完整初始内容。
- 同会话第二输入有真实 ACK、Started、唯一 Completed 结果及精确随机标记；旧结果或旧回调不得关闭第二输入。
- 生产监督连接真正关闭，并由 `confirmed_exit` 验证 `stdio_closed`、exit 0、对应运行代及清理收据。

初始化单次等候最多 90 秒，取消与继续阶段共用 180 秒 deadline，关闭等候最多 15 秒；继承隔离运行器的外层超时固定 900 秒。原生 ID 投影最多 512 条，超限失败并保留截断投影，不将其计为完整账本。两条输入预算不等于 HTTP 请求数，API 重试及网络数量未测量。

运行器 `script/cli-agent-parity/run_claude_approval_cancel_live.py` 复用基础 API 运行器的认证隔离，创建新的私有 HOME/CLAUDE_CONFIG_DIR，不读取或复制 CLI 登录资料，不将密钥值写入产物。参数为 `--test-binary`、`--claude`、`--supervisor`、`--api-environment-file`、`--model`、全新 `--output <路径>.ndjson`；可显式声明固定 `--max-native-inputs 2 --timeout-seconds 900`。基础运行器在真正执行时记录源码状态和三份二进制摘要，测试库与监督入口必须来自同一审核源码快照。

即使 libtest 退出 101，Rust 也在返回失败之前导出严格白名单 native IDs。Python 再检查全部公开字段：未知字段、任意正文或非安全 ID 被丢弃并追加明确投影失败记录，绝不保留旧成功旗。stdout 最终仅归档脱敏输出摘要、字节数、测试摘要计数及六类签名计数，不归档正文；任何非零凭据形态计数使验收失败。中间基础运行器 stdout 文件位于本轮私有输出范围，运行器返回前完成压缩；受限流程中断和输出错误不能被当作已通过。

离线回归命令为 `python3 -B script/cli-agent-parity/claude_approval_cancel_runner_tests.py -v`。回归涵盖四证据缺失、ACK-only、审批撤销-only、旧运行代/ID/会话、缺失或混合 UUID、API 失败、原生三终态信号乱序、重复原生证明、未授权工具/审批响应、额外 Edit 字段、文件变化、非正常退出、旧继续结果、额外输入、错误计数、101 失败投影及 stdout 凭据形态计数。离线通过只能证明夹具和审核器边界，真实 native、Cargo、本地任务存储、应用重启、GUI、SSH/tmux、Linux/Windows 和最终同提交平台验收仍独立待验。

本准备轮仅实际运行一次离线回归：25 项通过，0.029 秒，退出码 0。两个新 Python 文件在内存静态编译通过；新 Rust 文件通过 `rustfmt --edition 2024 --config skip_children=true`。Rust 的精确 Edit 参数纯回归已编写但未执行，格式检查不等价于 Cargo 编译或真实接口验收。
