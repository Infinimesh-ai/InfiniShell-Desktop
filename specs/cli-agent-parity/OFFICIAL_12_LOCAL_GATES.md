# official-12 本地门禁

source12 的本地编译检查、i18n 门禁、受影响模块测试及离线运行器回归通过。独立归档核对了冻结目录中的73个源文件、四份日志的 SHA-256，以及日志中的实际测试计数；没有重新执行 Cargo、原生 CLI、模型请求或 GUI。

基线为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，这是有既存改动的中间脏快照，不是包含本次修改的干净提交。source12 增加了测试专用 Grok 已验证完整历史安全事件、生产根任务协调器与 SQLite 真实验收夹具，以及 Claude 原生图片单输入校准夹具；新增真实夹具尚未执行，主程序尚未构建。SDK、本地子任务与父权限上限门禁仍关闭，夹具编译通过不能计为产品能力验收通过。

| 门禁 | 原始报告 wall 耗时 | 独立核对的日志结果 |
| --- | --- | --- |
| `cargo check -p warp` | 60.109秒 | 退出码0，完成 dev profile |
| `cargo test -p warp --lib i18n::tests` | 177.701秒 | 11项通过，0失败、0忽略；6,718项 filtered out，实际测试4.69秒 |
| 受影响模块 `cargo nextest` | 23.269秒 | 1,197项运行且全部通过，5,532项 skipped；实际测试15.469秒 |
| 10组 Python 离线运行器回归 | 合计4.091秒 | 223项通过，10组均显示 OK |

Python各组计数为：Claude适配器11、Claude权限profile 7、Claude协调器56、Claude批量取消18、Claude图片校准22、Codex来源10、Grok适配器13、Grok协调器20、Grok官方适配器19、Grok SDK来源探针47。独立核对分别确认了1,197条 Rust PASS、11条 i18n ok 和223条 Python ok；未选中及忽略的测试不计为已验证。退出码和 wall 耗时来自原始执行报告，归档任务没有重跑进程。

[冻结输入](validation/macos-official-12-inputs.json)、[Rust门禁](validation/macos-official-12-gates.json)、[Python门禁](validation/macos-official-12-python.json)经凭据模式扫描后逐字节复制。输入 manifest SHA-256 为 `1356d9799b926439329cdb96a4bf4a818344faa00a10724b43360f2fabb92bb3`；73条路径唯一，在 `.worktrees/cli-agent-parity-validation` 的 source12 冻结树中逐一匹配。主工作区正在追加下一阶段修改，不能用当前主树替代已验证的冻结输入。逐文件散列、报告与日志散列、计数及边界保存在[独立审计](validation/macos-official-12-archive-audit.json)，原始日志未归档。

libtest SHA-256 由原始门禁报告记录为 `24b58021d1909185e3993d78c7ecf52941d21930ad37516435367cd81e45f036`，归档任务未重新读取大二进制计算散列。此记录不证明主程序已构建，也不证明真实夹具、应用重启、双语布局、SSH/tmux、Linux、Windows及最终同提交验证完成。

独立只读审查之后提出的三项收紧——Claude图片结果后必须正常退出、Grok未知嵌套字段及文件名的公开投影、实际Submit输入摘要与SQLite消息正文摘要逐条关联——仅纳入下一source13。source12通过不追认为这些收紧已实现或已验证；此前真实失败和外部能力限制继续保留，Goal尚未完成。
