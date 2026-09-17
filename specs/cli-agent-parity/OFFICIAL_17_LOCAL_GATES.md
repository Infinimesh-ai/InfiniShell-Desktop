# source17 本地 Cargo 门禁通过归档

source17 的三项本地 Cargo 门禁实际全部通过：`cargo check -p warp`、11 项 i18n 测试及 1,244 项受影响模块定向测试。此结论仅覆盖本地中间脏快照；本轮没有主程序构建、签名验证、新 Python 运行、真实 CLI/PNG、GUI 或同提交跨平台验收。

## 快照来源与历史边界

根代理执行的两份公开原始 JSON 按字节复制。基础提交为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，source17 清单包含 79 个唯一路径，SHA256 为 `addb4ca18ec6ab550e063bced9c879fe6bcba72da4bf138cd41a5d0c69ecb4bf`。相对 source16，仅 `app/src/ai/cli_agent_runtime/claude_image_tests.rs` 的 JSON 引用比较断言修正；该变化与生产逻辑、本轮在线夹具或语言资源的变动不可混淆。

[source16 失败报告](OFFICIAL_16_LOCAL_GATES.md) 保留原有 E0277 编译失败、i18n 未执行及定向测试未运行结论。source17 是修复后独立通过的新快照，不追认 source16 成功。source17 没有重新运行 Python；[source16 Python 原报告](validation/macos-official-16-python.json) 的 12 组、295 项、4.384 秒通过仅作为未变 Python 文件的历史证据引用，不计为 source17 新运行。

根代理在 source17 唯一 Cargo 进程结束后逐一核对 79 个冻结文件一致，再仅更新两个生产 PNG Python 文件并前进 source18（清单 SHA256 `3d539cc6282dc098346011f900140e2006af993aad93c7285e23fe4eedb2d3cf`）。本归档独立核对 source17 原清单哈希、79 条路径与哈希形态及原报告一致性；实际冻结文件一致性明确来自根代理确认，没有复读当前冻结树，也没有用 source18 字节替代 source17。source18 的在途门禁结果不属于此记录。

## 实际通过结果

| 门禁 | 退出码 | 时间 | 真实结果 |
| --- | ---: | ---: | --- |
| `cargo check -p warp` | 0 | 67.205 秒 | 生产编译检查通过 |
| `cargo test -p warp --lib i18n::tests` | 0 | 243.527 秒 | 11 通过，0 失败，6,766 过滤 |
| `cargo nextest run --no-fail-fast -p warp --lib`，受影响模块过滤 | 0 | 22.373 秒 | 1,244 通过，5,533 跳过 |

定向过滤为 `test(cli_agent) | test(local_cli_mailbox) | test(local_cli_tasks) | test(local_harness) | test(ai::orchestration::) | test(ai::agent_providers::chat_stream::) | test(ai::blocklist::action_model::execute::)`。准确命令参数见原 gates 报告；跳过项不计为通过。本轮没有启动需要显式在线隔离运行器的原生验收。

根 gates 报告记录 source17 lib-test 二进制 SHA256 为 `cc12e15bd9209d9c4ca3c935ba89d23f5af13d9e685be16af65795e33fb3a153`。本归档引用原报告的身份，不读取或重新哈希目标目录中的二进制，避免之后 source18 产物覆盖而混用来源。此 lib-test 身份不能代替尚未构建的 source17 主程序或签名证据。

## 逐字节原报告与独立日志核对

| 原报告 | 字节数 | SHA256 |
| --- | ---: | --- |
| [source17 inputs](validation/macos-official-17-inputs.json) | 13,821 | `addb4ca18ec6ab550e063bced9c879fe6bcba72da4bf138cd41a5d0c69ecb4bf` |
| [source17 gates](validation/macos-official-17-gates.json) | 2,107 | `a23426c75435f920b57657f1c22cc6f3ddbbd3f08571a483edcb17ee5fc72656` |

独立读取原 gates 报告明确列出的三份日志，仅保留哈希、大小、固定结果计数与凭据形态计数，不复制原日志：

| 日志 | 字节数 | SHA256 |
| --- | ---: | --- |
| check | 359 | `71810eb452040884f5ffbf8e84d7c8ecdbd957d7082042d7bab2c116e27d9e7c` |
| i18n | 1,259 | `ba13c88f04696d37b9e98f2b5eebac6943d70307c26d227b615b55f730ae49d0` |
| focused | 185,707 | `49e693ae8f767ce12ac428be63bf3e61f60b1c22f02d3cd9ad3e686047615301` |

三日志 SHA 均与根原报告匹配。i18n 成功摘要独立解析为 11 通过、0 失败、6,766 过滤；nextest 摘要独立解析为 1,244 运行且通过、5,533 跳过，另逐行计得 1,244 条 PASS。三日志均没有编译错误行。

[归档审计](validation/macos-official-17-archive-audit.json) 记录字节一致性、79 路径形态、结果计数、二进制身份引用来源及快照前进边界。两份原 JSON、三许可日志及四新增归档文件的 API key、JWT、Bearer、私钥、邮箱、凭据赋值六类形态计数均为 0，只记录计数，不输出候选。没有读取私有数据、认证、任意旧 stdout、目标目录或冻结树，也没有复跑 Cargo、CLI、模型或 GUI。

新增文件仅为验证记录，无需本地化变更。i18n 资源测试通过不代表英文/简体中文 GUI 布局已检查；真实生产 PNG、完整应用生命周期与同提交跨平台门禁仍需独立验证。
