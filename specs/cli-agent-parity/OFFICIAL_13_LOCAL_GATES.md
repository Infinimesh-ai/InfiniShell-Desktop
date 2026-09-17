# official-13 局部门禁

source13 的本地编译检查、i18n 门禁、受影响模块测试和离线运行器回归通过。本次独立归档核对了冻结目录中的 73 个源文件、四份日志的 SHA-256 和实际测试计数，没有重新执行 Cargo、原生 CLI、模型请求或 GUI，也没有读取大 libtest/main 二进制。

基线为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`。这是包含既存修改的中间脏快照，不是包含本次修改的干净提交。source13 纳入 Claude 图片校准的真实正常退出判据、Grok 未知嵌套字段与文件名的公开投影、真实 Submit 摘要与 SQLite 消息正文摘要关联，以及有界私有原生工具帧诊断。收紧已进入冻结输入并通过所选单元/离线门禁，不能因此将真实图片或生产协调器生命周期验收记为通过。

| 门禁 | 原始报告 wall 耗时 | 独立核对的日志结果 |
| --- | --- | --- |
| `cargo check -p warp` | 56.671 秒 | 退出码 0，完成 dev profile |
| `cargo test -p warp --lib i18n::tests` | 154.996 秒 | 11 项通过，0 失败、0 忽略；6,720 项 filtered out，实际测试 4.49 秒 |
| 受影响模块 `cargo nextest` | 18.000 秒 | 1,199 项运行且全部通过，5,532 项 skipped；实际测试 14.771 秒 |
| 10 组 Python 离线运行器回归 | 合计 4.035 秒 | 231 项通过，10 组均显示 OK |

Python 各组计数为：Claude 适配器 11、Claude 权限 profile 7、Claude 协调器 56、Claude 批量取消 18、Claude 图片校准 23、Codex 来源 10、Grok 适配器 13、Grok 协调器 27、Grok 官方适配器 19、Grok SDK 来源探针 47。独立日志计数分别确认 1,199 条 Rust PASS、11 条 i18n ok 和 231 条 Python ok；未选中或忽略的测试不计为已验证。退出码和 wall 耗时来自原始执行报告，归档任务没有重跑测试进程。

[冻结输入](validation/macos-official-13-inputs.json)、[Rust 门禁](validation/macos-official-13-gates.json)和[Python 门禁](validation/macos-official-13-python.json)通过凭据模式扫描后逐字节复制，保留原始字节。输入 manifest SHA-256 为 `419e6a6fac30237c4577061cb753078360fdf42e5833de29266363e3cdd5dd82`；73 条路径唯一，在 `.worktrees/cli-agent-parity-validation` 的 source13 冻结树中全部匹配。归档前后都重新核对冻结源文件 SHA；不能用后续主工作区修改替代这份输入。

逐文件散列、原始报告与日志散列、独立计数及边界保存在[归档审计](validation/macos-official-13-archive-audit.json)。三份 JSON 及四份日志的 API key、JWT、Bearer、私钥、邮件地址和凭据赋值模式扫描计数均为 0；只记录扫描计数，原始日志不归档。

libtest SHA-256 由原始门禁报告记录为 `a857fbb82541ff572591bb9ab526a187e1a96325063636a7f72f928d6864e471`。归档任务只核对该报告字段，没有重新读取二进制计算散列。main13 生产 bundle 由根代理独立构建，不属于本归档范围；此记录不证明生产 bundle 或新真实夹具通过。

Grok SDK、本地子任务和父权限上限门禁没有开放。真实新夹具、应用重启、双语布局、SSH/tmux、Linux、Windows、全工作区测试及最终同提交验收均不能计为本次通过；此前真实失败与外部能力限制继续保留，Goal 未完成。本次只新增验证记录和说明，无需本地化变更。

生产构建另存为独立[构建原报告](validation/macos-official-13-bundle.json)和[构建归档审计](validation/macos-official-13-bundle-archive-audit.json)。上文局部门禁审计不覆盖生产 bundle；以下追加证据单独记录该构建，原局部门禁审计文件保持原样。source13 在同一份 73 文件冻结 manifest 上执行 `./script/run --dont-open --features local_cli_managed_tasks,rust-embed/debug-embed`，退出码 0，报告 wall 耗时 187.054 秒。主程序报告 SHA-256 为 `0c9e05fe2c75aaf909e31c2dfb4508e5f7ea3ee9da6c7a8e167713ae8205d77f`；本归档没有重新读取大主程序计算散列。构建原报告逐字节复制，SHA-256 为 `d49f3180dc1f2f697ebfeb0e4b405185a8907bb8ca529cf02b918b6610a68cd6`；构建日志 SHA-256 `07e8579e75aa4e286abfca6758ac3552fc9d5aa174e622b71db5e89c43cfa12e` 独立核对匹配，日志不归档。

原构建报告确认下列完整资源字节存在于固定二进制，并记录 `locale_resources_immutable_in_binary=true`。归档独立核对完整冻结 FTL 源文件的字节数、SHA 和 manifest；二进制内嵌结论保留原始扫描来源，没有重新扫描主程序。

| 资源 | 完整源文件字节数 | SHA-256 |
| --- | --- | --- |
| `app/i18n/en/warp.ftl` | 374,280 | `9442a4b062945cf3f5fbc5160193f4a14ae715ae9d35b7f5d5bc14be55980c3c` |
| `app/i18n/zh-CN/warp.ftl` | 361,234 | `2291d0796862e703a2869e174d38138f014183730d380d56d6d97ce4772a0d98` |

[严格签名记录](validation/macos-official-13-signature.json)取自根代理本轮明确确认的 `codesign --verify --deep --strict` 退出码 0、无输出证据；归档没有重跑 codesign，原核验耗时未提供并保留为 `null`。签名记录绑定同一主程序报告 SHA 和 manifest SHA；这只证明本地严格签名核验通过，不证明公证、发布、安装或 GUI 实际显示。

构建和签名归档的凭据模式扫描计数均为 0。运行中的 SDK4 和其他原生新夹具结果不纳入本构建归档；完整双语资源内嵌不能代替双语布局、图片输入、SDK 来源、权限或完整 CLI 生命周期验收。该 bundle 仍来自中间脏快照，不能计为最终同提交跨平台通过，Goal 仍未完成。
