# official-15 局部门禁与构建

source15 的本地编译、国际化、受影响模块测试、Python 离线回归及 macOS 构建通过；根代理实际 deep/strict 签名验证退出码 0。独立归档核对冻结 75 文件、五份日志 SHA、实际测试计数及两种语言资源的字节数与 SHA，没有重新执行 Cargo、Python 测试、CLI、模型、GUI 或验签，也没有读取大 main/lib 二进制。

基线为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`。这是包含未提交修改的中间脏快照，不计最终同一实际修改提交的跨平台验收。相比 source14，仅 `grok_sdk_origin_live_tests.rs`、`grok_sdk_origin_probe_runner_tests.py`、`run_grok_sdk_origin_probe.py` 三个 SDK 探针文件变化；没有新增或移除输入路径。探针收窄一次审批和重投账本不等于 SDK 能力已通过真实验收，SDK origin 仍只接受原生字段。本快照保留 source14 的生产 Claude 图片门禁及 FTL，不包含根工作区并行实现的生产 PNG 支持和新增 PNG 文案。

| 门禁 | 原始报告 wall 耗时 | 独立核对的日志结果 |
| --- | --- | --- |
| `cargo check -p warp` | 57.752 秒 | 退出码 0，完成 dev profile |
| `cargo test -p warp --lib i18n::tests` | 135.436 秒 | 11 项通过，0 失败、0 忽略；6,732 项 filtered out，实际测试 4.7 秒 |
| 受影响模块 `cargo nextest` | 18.624 秒 | 1,211 项运行且全部通过，5,532 项 skipped；实际测试 15.069 秒 |
| 11 组 Python 离线运行器回归 | 合计 4.201 秒 | 268 项通过，11 组均显示 OK |
| 本地 macOS `./script/run --dont-open` 构建 | 30.427 秒 | 退出码 0，日志确认 App 签名步骤完成 |
| 根代理 `codesign --verify --deep --strict` | 0.484381 秒 | 退出码 0，stdout/stderr 均为零字节 |

Python 各组为 Claude 适配器 11、Claude 权限 profile 7、Claude 协调器 56、Claude 批次取消 18、Claude 图片校准 25、Codex 来源 10、Grok 适配器 13、Grok 协调器 27、Grok 官方适配器 19、Grok SDK 来源探针 62、Grok 普通 PTY 准备 20。逐项日志独立确认 1,211 条 Rust PASS、11 条 i18n ok 和 268 条 Python ok；忽略或未选中的测试不计为已验证。实际退出码和 wall 耗时保留根代理原执行报告来源，本归档没有重跑门禁。

[冻结输入](validation/macos-official-15-inputs.json)、[Rust 原始门禁](validation/macos-official-15-gates.json)、[Python 原始门禁](validation/macos-official-15-python.json)、[原始构建报告](validation/macos-official-15-bundle.json)与[原始签名报告](validation/macos-official-15-signature.json)经八类定向凭据格式扫描后按原字节复制。manifest SHA-256 为 `6830223673d39301f2b9884d02661f22a4a19e5df27c85dbccdda4542fccdba2`，75 条路径唯一，冻结树在只读核对前后全部匹配。核对完成后向根代理发送 `frozen_reads_done`，后续仅处理报告与已归档 manifest，没有再读可能已推进到 source16 的冻结树，不将新树称为 source15。

[门禁归档审计](validation/macos-official-15-archive-audit.json)保存报告和日志源映射、SHA、逐源文件前后核对及实际计数。五份原报告及五份日志的 API key、JWT、Bearer、私钥、邮件地址、凭据赋值、私有 API 地址、私有 API 环境路径模式均为零命中；日志只记录方法、命中数、计数和 SHA，不复制原日志。libtest SHA `71e42409bc656cea8d050b30a1e4408576c9e46e6e45c775bf1bfde3860b79bf` 与 main SHA `d9a5590cc68c303aca53f5912df861b008063959d3c5ba002999776781d4e3db` 来自原报告，不冒充归档代理重算二进制散列。

构建使用 `local_cli_managed_tasks,rust-embed/debug-embed`，根代理原报告记录两份 FTL 的完整字节实际存在于 main。归档代理独立核对冻结语言资源的 SHA 和字节数，并确认与 manifest、source14 和 source15 构建报告相同，没有重读 main 验证嵌入。

| 已嵌入语言资源 | 字节数 | SHA-256 |
| --- | --- | --- |
| `app/i18n/en/warp.ftl` | 374,280 | `9442a4b062945cf3f5fbc5160193f4a14ae715ae9d35b7f5d5bc14be55980c3c` |
| `app/i18n/zh-CN/warp.ftl` | 361,234 | `2291d0796862e703a2869e174d38138f014183730d380d56d6d97ce4772a0d98` |

[构建归档审计](validation/macos-official-15-bundle-archive-audit.json)绑定 source15 manifest、构建与签名报告、日志及语言资源来源。签名原报告完整保存根代理实际 argv：`codesign --verify --deep --strict /Volumes/ORICO/CargoTarget/InfiniShell-Desktop-cli-agent-parity-dbee1ecae/debug/bundle/osx/InfiniShell.app`，目标为 App bundle 目录；空 stdout/stderr 的 SHA 独立核对为 SHA-256 空输入。构建日志记录的签名目标与验签 argv 字符串相同，但归档代理没有重新读取该目录或执行验证。签名不能证明公证、安装、发布或真实 CLI/GUI 生命周期通过。

SDK 第五轮真实 CLI 验收由根代理在同一 source15 快照独立执行，本归档没有读取其私有日志、协议、环境或认证资料，也没有纳入其结果。Claude 图片校准的独立真实记录不能据此宣称本快照生产 PNG 通过。[普通 Grok PTY 准备](GROK_OFFICIAL_PTY.md)的真实 live 入口在认证与网络前拒绝，20 项离线检查不计真实输入、响应或取消；[source13 Grok 协调器首轮](GROK_COORDINATOR_ACCEPTANCE.md)的真实成功属于另一快照。

SDK、子任务、父权限上限、生产 PNG 实际输入、完整双语 GUI 布局、应用重启恢复、活跃重关联、SSH/tmux、Linux/Windows、最终同提交平台验证和完整工作区门禁不能计为本轮通过。Goal 保持实施中。本次仅新增工程验证档案和文档，无需本地化变更。
