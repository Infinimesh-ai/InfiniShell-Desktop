# Source18 macOS 本地门禁归档

Source18 的本地代码检查、国际化、受影响模块测试和 Python 离线回归均已通过。这是冻结的 79 路径脏快照，基线提交为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`；本地门禁之后的主程序构建与严格签名也已通过；真实 CLI 失败另列，本记录不计 GUI、最终交付提交或跨平台验收通过。

[输入清单](validation/macos-official-18-inputs.json) 原始报告 SHA-256 为 `3d539cc6282dc098346011f900140e2006af993aad93c7285e23fe4eedb2d3cf`。归档代理独立读取冻结树的 79 个清单路径，两次逐文件核对 SHA-256，前后均与清单完全一致；没有读取 target 中的测试或主程序二进制。原始三份 JSON 均按原字节归档，来源、摘要、计数和证据边界见[归档审计](validation/macos-official-18-archive-audit.json)。

| 本地门禁 | 实际结果 | 根执行报告耗时 |
| --- | --- | --- |
| `cargo check -p warp` | exit 0；独立读取日志确认 dev profile 构建结束 | 55.963 秒 |
| `cargo test -p warp --lib i18n::tests` | 11 passed、0 failed、0 ignored、6,766 filtered out | 5.240 秒 |
| 受影响模块 nextest | 1,244 passed、0 failed、5,533 skipped | 17.720 秒 |
| 12 组 Python 离线回归 | 297 passed；每组 exit 0 | 4.324 秒 |

Rust 命令、退出码、实际耗时及日志摘要保留在[原门禁报告](validation/macos-official-18-gates.json)。归档代理独立读取并核对三个 Rust 日志的 SHA-256，解析 i18n 与 nextest 的实际摘要；不复制日志正文。日志中的测试内部耗时分别为 4.280 秒和 16.049 秒，与表中的完整命令耗时是不同口径。

[原 Python 报告](validation/macos-official-18-python.json) 的逐组计数为 `[11, 7, 56, 18, 25, 19, 10, 13, 27, 19, 72, 20]`，独立求和为 297，全部退出码为 0。Python 日志 SHA-256 `7ec40c1d99a8d0f932f34079ced890b4a38984616cd09b6297a16893f1d24365` 仅保留根报告来源；归档代理没有读取该日志或重新运行测试，不能称为独立重算日志结果。

公有 source16/17 输入清单的精准对照确认：source17 相对 source16 仅 `app/src/ai/cli_agent_runtime/claude_image_tests.rs` 改变；source18 相对 source17 仅 `script/cli-agent-parity/run_claude_managed_image_live.py` 和 `script/cli-agent-parity/claude_managed_image_runner_tests.py` 改变，其他 77 路径同字节。Source18 的变更是 PNG 运行器保留可解析失败证据及两项离线负例，不把失败报告改为成功，也不放宽三输入、原生 ACK、两代正常退出和清理门槛。

根门禁报告记录测试库 SHA-256 为 `cc12e15bd9209d9c4ca3c935ba89d23f5af13d9e685be16af65795e33fb3a153`，与根提供的 source17 身份相同；这是报告来源身份，归档代理没有独立读取大二进制。首次代理归档只覆盖 Rust/Python 门禁；随后根执行的主程序与验签补充见文末。真实 PNG、SDK、Grok、普通 PTY、GUI、应用重启、SSH/tmux、Linux/Windows 及同一最终提交验收须各自以独立证据核对，不能由这些本地门禁推导通过。

对本次报告、允许读取的 Rust 日志及新增归档执行 8 类凭据形态扫描，命中数均为 0；不读取或归档认证、环境、配置或私有模型输出。无需本地化变更：本次只新增工程验证档案，没有修改产品功能或用户界面文案。归档代理未运行 Cargo、原生 CLI、模型、GUI 或 Git。

根执行补充：[主程序构建](validation/macos-official-18-bundle.json)实际 exit 0、264.779 秒，主程序 SHA-256 `0942eaadace89b284f249b4a20374d0dd2cb9803f805c363826c422b3ce409ba`；完整英中 FTL 在实际主程序中逐字找到。[严格签名验证](validation/macos-official-18-signature.json)实际 exit 0、0.524879 秒，stdout/stderr 均 0 字节。两份原报告逐字归档并核对构建日志摘要，见[补充归档审计](validation/macos-official-18-bundle-archive-audit.json)。这来自根执行，前述归档代理未读取二进制的边界仍保留；严格验证不代表发布身份或已发布。生产 PNG 与 SDK6 的真实失败见 [source18 真实记录](SOURCE18_NATIVE_FAILURES.md)。
