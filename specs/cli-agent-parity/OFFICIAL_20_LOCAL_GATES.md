# Source20：macOS 本地门禁独立归档

本轮授权归档的本地门禁全部通过，不能计为最终 P0–P5 完成。source20 是基线 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 上含实际修改的冻结工作区，83 个路径的输入清单 SHA 为 `b3b3e7a61530b1314ef5e4f8de5f2c2b5bfb62f883e7b3e6109d42d2590fc693`。相对 source19 只有三个清单路径改变，没有新增路径：`app/src/ai/cli_agent_runtime/grok.rs`、`script/cli-agent-parity/grok_sdk_origin_probe_runner_tests.py`、`script/cli-agent-parity/run_grok_sdk_origin_probe.py`。本轮离线校验不开放 Grok SDK 产品 gate。

## 独立归档范围

归档代理连续两次读取 83 份 frozen 小源码，全部 SHA 与输入清单一致，随即发出 `frozen_reads_done`，之后未再读取 frozen。三份原报告原字节复制；指定的三份 Rust 日志仅读取并核对 SHA 与完成摘要，不复制日志。lib 身份来自根代理原报告，没有读取 target。Python 日志 SHA 同样标为原报告来源，没有读取或重新计算该日志。

## 本地结果

| 门禁 | 原报告退出码 | 墙钟时间 | 独立摘要核对 |
| --- | --- | --- | --- |
| `cargo check -p warp` | 0 | 60.240 秒 | dev profile 完成，日志 SHA 一致 |
| `cargo test -p warp --lib i18n::tests` | 0 | 146.350 秒 | 11 通过、0 失败、6770 filtered out；内部 4.45 秒 |
| 受影响模块 `cargo nextest` | 0 | 17.632 秒 | 1248 通过、5533 skipped；内部 14.685 秒 |
| Python 13 组离线验证 | 全部 0 | 4.565 秒 | 原报告合计 329 项通过；独立核对组数、各组计数与退出码 |

SDK 专用 Python 组从 source19 的 72 项增加至 77 项，本轮原报告实际为 77 项通过。其余组的计数详见 Python 原报告。focused 选择表达式为 `test(cli_agent) | test(local_cli_mailbox) | test(local_cli_tasks) | test(local_harness) | test(ai::orchestration::) | test(ai::agent_providers::chat_stream::) | test(ai::blocklist::action_model::execute::)`，不等于全工作区测试。

三份 Rust 日志 SHA 依次为 `b05de9b18264f76a9e8a40bdfe4deb345480bb107886afa86918625f8373cf45`、`71dc2b7dc29aaf9d25f92f09f9c4f7a994749b2e2b1abb1e34e098dc5da0bda5`、`f2918fe239dd4c0634831867e092546c722808771bdd4223f3c8181b18310c2d`，均独立重算且与原报告一致。Python 日志 SHA `7c23d8a43d10a0a8fbf51be7e9222444fd63debbd48c46dd733b49bb03797bfd` 来自根代理原报告；lib SHA `c5819a0d7e73031b3ce19716e27e143b3188e03a5c6e7b0bc5e1a8b2a88c7716` 也只由原报告提供。

## 验收边界与本地化

本次授权的档案不包含 source20 main 构建或签名结果。根代理授权时 main 构建正在进行，本报告不为其作通过结论，不沿用 source18 main 或签名。没有新的真实 CLI、GUI、SSH／tmux、跨平台或最终同提交证据；SDK 诊断离线通过不能替代真实反向请求、原生来源账本或父子任务验收。

source19→20 的变动不包含 Fluent 文件，无需本地化变更。i18n 门禁通过，但 source20 英文与简体中文完整 GUI 布局未验。全工作区门禁及用户要求的其余流程仍需各自实际证据。

## 档案

[输入清单](validation/macos-official-20-inputs.json)、[Rust 门禁](validation/macos-official-20-gates.json)、[Python 门禁](validation/macos-official-20-python.json)、[独立归档审计](validation/macos-official-20-archive-audit.json)。原始报告和全部新增档案固定六类凭据形态扫描均为零匹配；此扫描不能保证识别所有未知凭据格式。没有覆盖旧报告，没有运行 Cargo、CLI、模型、Git、GUI 或网络，没有读取 target 或私有文件。

## 后续补档：实际 main 构建与严格 codesign 验证

前述“授权时 main 构建正在进行”是首次归档时的状态。本次后续授权提供的根代理实际 source20 构建报告已结束：`./script/run --dont-open --features local_cli_managed_tasks,rust-embed/debug-embed` 退出码 0、墙钟 196.531 秒，构建日志 SHA `10650e3dd0cc81980f2de80f18c4fe42d0dcb72359aa2f5caa06451cb04c418b` 已独立重算并与原报告一致。报告内完整 source_manifest 与首次归档的 source20 inputs JSON 对象逐字段一致。构建前后均为脏工作区，结果是实际修改的中间冻结快照。

main／监督 worker SHA `8b1ad22b400b71b13e08ca6d9f8fdcc3b78da69c965d3fd9366a8cfa1ed3ddea` 由根代理实际构建报告提供，本归档没有读取二进制。报告记录英文 374,753 字节、SHA `2dad34fc7ba6ef63d9bf072e5464719fd650d931206038a147aca4e153d614b9`，简体中文 361,658 字节、SHA `2ffab1ab84d464d06288f2f9f278c297dd3ed0575e2ee39c2a267cde2b62d4b9`，两份完整资源在二进制内找到且不可变。这些 hash 与 source20 清单中的资源 hash 一致；“完整字节在二进制内找到”来自根报告，本归档没有独立扫描二进制。资源嵌入不等于双语布局验收。

根代理的实际签名报告记录 `codesign --verify --deep --strict` 退出码 0，墙钟 0.480609 秒，stdout／stderr 均为 0 字节且 SHA 均为标准空摘要；报告的 source_manifest_sha256 与 source20 inputs 原字节摘要一致。本次归档只核对原报告，不重新执行 codesign。此结果是调试 bundle 的严格签名验证，不证明发布者身份、公证、公开发布或最终交付提交。

新增 [实际构建原报告](validation/macos-official-20-bundle.json)、[严格验证原报告](validation/macos-official-20-signature.json)、[补档独立审计](validation/macos-official-20-bundle-archive-audit.json)，两份原报告按原字节复制。本次没有读取 target、frozen 或私有文件，没有运行 Cargo、CLI、模型、GUI、Git 或网络；没有归档仍在运行的原生验收部分。真实 CLI、完整 GUI、跨平台、最终包含实际修改的同一提交及 P0–P5 验收仍需后续完整证据，不能从本次构建推定通过。
