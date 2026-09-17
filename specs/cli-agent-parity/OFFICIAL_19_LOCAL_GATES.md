# Source19：macOS 本地门禁独立归档

本轮本地门禁全部通过，尚未证明最终 P0–P5 验收完成。source19 为基线 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 上含实际修改的冻结工作区，共 83 个清单路径，输入清单 SHA 为 `e1621ea312d45cc7b52b9e843b107020713dd7952a9c90767c46eecf506fe4ea`。相对 source18 有 5 个既有清单路径改变、4 个新纳入清单的路径；这些分类仅是冻结清单变化，不宣称四个路径都是新建产品代码。

## 改动范围与来源

本轮加入正常 stdin 关闭后保留控制连接直到真实监督进程退出与清理回执核验的 API，并分别调整 Claude、Codex、Grok 的有预算 stdout EOF 收尾。4 个新增纯 TCP 控制生命周期测试覆盖正常等待不发送停止字节、取消等待、超时不冒充清理、强制停止与缺失进程；它们没有伪造成功 ExitStatus 或内核退出证明。Codex 固定 runtime 准备器及其 27 项离线测试也纳入冻结清单。Claude 图片夹具的共享元数据字段澄清属于本次清单变化。

独立归档代理在通知根代理可以推进下次冻结前，连续两次读取 83 个 frozen 小源码并核对 SHA，全部与原清单一致；之后不再读取 frozen。三份原报告按原字节复制。Rust 三份日志仅用于独立 SHA 与摘要核对，没有复制原日志。测试二进制身份由根代理原报告给出，本代理未读取 target。

## 本地结果

| 门禁 | 原报告退出码 | 墙钟时间 | 独立摘要核对 |
| --- | --- | --- | --- |
| `cargo check -p warp` | 0 | 67.230 秒 | dev profile 完成，日志 SHA 一致 |
| `cargo test -p warp --lib i18n::tests` | 0 | 154.418 秒 | 11 通过、0 失败、6770 filtered out；测试内部 4.82 秒 |
| 受影响模块 `cargo nextest` | 0 | 18.074 秒 | 1248 通过、5533 skipped；测试内部 14.666 秒 |
| Python 13 组离线验证 | 全部 0 | 4.563 秒 | 原报告合计 324 项通过；独立核对组数、各组计数和退出码 |

focused 选择表达式为 `test(cli_agent) | test(local_cli_mailbox) | test(local_cli_tasks) | test(local_harness) | test(ai::orchestration::) | test(ai::agent_providers::chat_stream::) | test(ai::blocklist::action_model::execute::)`，不等于全工作区测试。独立日志核对还确认新增 4 项纯控制生命周期测试实际显示 PASS。

Rust check、i18n、focused 日志 SHA 依次为 `f1c5e402bb8637f1eaaeaffeb1cf27b15b81b88c43f3ca17ced10ce1419c2b01`、`705177be07d8122e1655e12d460881ae0ced3d8a14ddd0fda5f3dddfad5b6a3c`、`4fa1f372bb18d455bcdd3cf8fc87fc31d3eafda3aba2c643fe0cb4b4093c0f95`。Python 日志 SHA `7e5b2eafe4ded1f1ae6e23d151d686999719563f6611295ecfff494764c6d321` 来自根代理原报告，归档代理未读取或重新计算 Python 日志。

本轮 lib SHA `d426da4f2aac520c5bc6fc0e56e062cedcf31e0ba050e2e9b24308840a5ab32a` 来自 gates 原报告。source19 没有新的 main 构建、签名、真实 CLI、GUI 或平台运行证据。不得将 source18 main／签名追认为 source19，同样不得把本轮离线通过追认为正常关闭真实 CLI 验收通过。

## 本地化与未覆盖项

本次正常关闭控制生命周期、运行夹具元数据澄清与 Codex runtime 准备器修复没有改变 Fluent 文件，无需本地化变更；保留既有用户错误文案。i18n 门禁已通过，但 source19 完整英文／简体中文 GUI 布局尚未验证。真实三方 CLI 流程、应用重启恢复、原生退出与内核清理、SSH／tmux、最终包含实际修改的同一提交跨平台及全工作区门禁仍需要各自证据。

## 文件与审计

[输入清单](validation/macos-official-19-inputs.json)、[Rust 门禁](validation/macos-official-19-gates.json)、[Python 门禁](validation/macos-official-19-python.json)、[独立归档审计](validation/macos-official-19-archive-audit.json)。三份原报告和全部新增档案均通过固定六类凭据形态扫描，零匹配；此扫描不保证识别未知凭据格式。本代理未运行 Cargo、CLI、模型、GUI、网络或 Git，未读取 target 或其他私有文件，没有覆盖旧报告。
