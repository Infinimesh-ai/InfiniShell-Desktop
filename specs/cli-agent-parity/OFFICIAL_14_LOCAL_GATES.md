# official-14 局部门禁

source14 的本地编译、国际化、受影响模块测试及离线运行器回归通过。独立归档重新核对冻结目录中的 75 个源文件、四份日志的 SHA-256 和实际测试计数，没有重新执行 Cargo、CLI、模型请求、GUI 或授权，也没有读取大 libtest/main 二进制。

基线为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，本轮是包含未提交修改的中间脏快照，不计最终同一实际修改提交的跨平台验收。source14 相比 source13 有六个变化或新增路径：跨平台工作流、Claude 图片夹具、图片运行器与其离线回归，以及普通 Grok PTY 准备运行器与离线回归。图片退出修正将关闭输入、有界输出 EOF 读取及监督退出确认并行处理；本轮只证明编译和离线检查通过，不据此开放图片能力。SDK 三个探针源文件与 source13 的全拒绝诊断逐字节相同，不包含后续并行实现的精确一次审批守卫。

| 门禁 | 原始报告 wall 耗时 | 独立核对的日志结果 |
| --- | --- | --- |
| `cargo check -p warp` | 64.996 秒 | 退出码 0，完成 dev profile |
| `cargo test -p warp --lib i18n::tests` | 134.341 秒 | 11 项通过，0 失败、0 忽略；6,720 项 filtered out，实际测试 4.65 秒 |
| 受影响模块 `cargo nextest` | 18.646 秒 | 1,199 项运行且全部通过，5,532 项 skipped；实际测试 14.879 秒 |
| 11 组 Python 离线运行器回归 | 合计 4.232 秒 | 253 项通过，11 组均显示 OK |

Python 各组为 Claude 适配器 11、Claude 权限 profile 7、Claude 协调器 56、Claude 批次取消 18、Claude 图片校准 25、Codex 来源 10、Grok 适配器 13、Grok 协调器 27、Grok 官方适配器 19、Grok SDK 来源探针 47、Grok 普通 PTY 准备 20。逐项日志独立确认 1,199 条 Rust PASS、11 条 i18n ok 及 253 条 Python ok；忽略或未选中的测试不能计为已验证。退出码与 wall 耗时保留原执行报告来源，本归档没有重跑测试。

[冻结输入](validation/macos-official-14-inputs.json)、[Rust 原始门禁](validation/macos-official-14-gates.json)与[Python 原始门禁](validation/macos-official-14-python.json)通过八类定向凭据模式扫描后按原字节复制。manifest SHA-256 为 `587c446d737259bc48a913256008558c87b450d5f29304015a10841ed31eec93`；75 条路径唯一，在 `.worktrees/cli-agent-parity-validation` 冻结树中全部匹配，归档前后分别复算并确认未变化。没有用主工作区后续修改替代本轮输入。

[归档审计](validation/macos-official-14-archive-audit.json)保留报告/日志源映射、SHA、逐文件前后核对、六个变化路径及计数。三份 JSON 和四份日志的 API key、JWT、Bearer、私钥、邮件地址、凭据赋值、私有 API 地址及私有 API 环境路径模式命中均为 0；原日志不归档。libtest SHA `ec8e9c0f1c5281b7dd35050dd1c6383b0ff854bfe7bf2b29609f69a109024880` 来自原报告，本归档没有重新读取大二进制计算散列。

局部门禁审计不包含 main14 构建与签名，相关证据另行追加。Claude 图片第二轮已由根代理启动，但本局部门禁归档没有其真实验收结论，图片能力门禁继续关闭。[普通 Grok PTY 准备](GROK_OFFICIAL_PTY.md)的实际 live 入口在认证与网络前拒绝，20 项离线检查不能计为真实终端输入、响应或取消。[source13 Grok 协调器首轮](GROK_COORDINATOR_ACCEPTANCE.md)的真实成功属于另一冻结快照，不能改写为 source14 真实验收成功。

SDK、子任务、父权限上限、图片实际输入、完整 GUI 双语产品验收、应用重启恢复、活跃重关联、SSH/tmux、Linux/Windows、新增修改的最终同提交验证及完整工作区门禁仍不能计为本轮通过。既有失败与能力限制保留，Goal 保持实施中。本次仅新增工程验证记录和文档，无需本地化变更。

## main14 构建与签名补充

根代理随后完成同一 source14 输入的本地 macOS 构建，原始报告退出码 0、wall 26.199 秒；构建日志独立核对完成 dev profile，日志内编译耗时为 0.95 秒，两者计时范围不同。[原始构建报告](validation/macos-official-14-bundle.json)经八类定向凭据扫描后按原字节复制，SHA-256 为 `647dc19eecb4661114e5b9ccd0d622a8f29bc2ecce71e8e67afa6cdc3f9de91a`。main SHA `0993d97e13317064d25895a0791c884cb00363684a9e8f6f5675640cd3ccbd2b`保留原报告来源，本归档没有重新读取大二进制。

构建参数包含 `local_cli_managed_tasks,rust-embed/debug-embed`。根代理原报告记录两份 FTL 的完整字节存在于 main 二进制；归档代理独立核对冻结 FTL 的字节数与 SHA，并确认其与 75 文件 manifest 及构建报告相同，没有重新执行二进制资源扫描。

| 已嵌入语言资源 | 字节数 | SHA-256 |
| --- | --- | --- |
| `app/i18n/en/warp.ftl` | 374,280 | `9442a4b062945cf3f5fbc5160193f4a14ae715ae9d35b7f5d5bc14be55980c3c` |
| `app/i18n/zh-CN/warp.ftl` | 361,234 | `2291d0796862e703a2869e174d38138f014183730d380d56d6d97ce4772a0d98` |

[签名来源记录](validation/macos-official-14-signature.json)保留根代理实际 `codesign --verify --deep --strict /Volumes/ORICO/CargoTarget/InfiniShell-Desktop-cli-agent-parity-dbee1ecae/debug/bundle/osx/InfiniShell.app` 验证的报告：目标为 App bundle 目录，退出码 0、wall 0.379989 秒、stdout/stderr 均无输出，与本轮 main SHA 报告同源。根代理随后补充精确 argv，已据此补正早期目标留空记录；归档代理仅将报告目标与已记录的构建 App 路径做字符串核对，没有读取 target、冻结树或二进制，也没有执行验签。签名不能证明公证、安装、发布或真实 CLI/GUI 生命周期通过。

[构建归档审计](validation/macos-official-14-bundle-archive-audit.json)绑定原始报告、构建日志 SHA、main 报告 SHA、语言资源和 source14 manifest。75 文件的再次核对及两份 FTL 核对在根代理获准推进冻结树之前完成；后续只读原报告与已归档 manifest，没有再读可能已前进的冻结树，不将后续 source15 称为 source14。构建和签名补充仍属于未提交的中间快照，不补足最终同提交跨平台、双语 GUI 布局、真实图片输入或普通 Grok PTY 验收；图片第二轮真实证据由独立子代理另行归档，本页不计入其结论。
