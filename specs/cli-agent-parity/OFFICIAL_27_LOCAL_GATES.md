# source27 候选本地门禁记录

本轮本地门禁全部通过；Goal 仍为 ACTIVE。验证对象是实际父提交 `436cc739234061c092cedd106432bbbfa3c2d645` 上的独立 detached 候选树，未提交。输入清单 111 个路径，仅 5 个 Rust 与 4 个 Python 文件改变，其余 102 个路径保持父提交原字节；`dirty_candidate=true`、`same_commit_verified=false` 必须保留。

| 门禁 | 真实结果 | 总耗时 | 测试内部耗时 |
| --- | --- | --- | --- |
| `cargo check -p warp` | exit 0 | 131.160 秒 | — |
| `cargo test -p warp --lib i18n::tests` | 11 通过，0 失败，0 忽略，6882 过滤 | 306.312 秒 | 4.72 秒 |
| 受影响模块 nextest | 1410 通过，5483 未选中，1410 条 PASS | 22.438 秒 | 18.501 秒 |
| 15 组 Python 夹具 | 426 通过，无跳过，各组 exit 0 | 8.018 秒 | 分组记录见原报告 |

Rust 筛选沿用 source26 的表达式，并包含真实模块 `terminal::model::session::test::`。Python 覆盖完整本地合成夹具，包括已授权的 localhost/socket/tunnel 和合成子进程；SDK runner 101 项、权限预检 runner 31 项均通过。未调用外网、真实 CLI、模型 API 或认证接口。

每项门禁执行前后，实际执行器验证 111 个输入 SHA 与 Git exact 9M 状态。用户排除的 `web_runtime.rs`、`websearch_tests.rs` 不在复制清单中，原 source26 验证树未读写。归档阶段不重读冻结树、target 或二进制，只复核原始公共报告及四份本地门禁日志；执行与归档由同一子代理完成，不能作为另一主体独立复验。

实际测试二进制路径由本轮 i18n 日志观察，执行器报告 SHA 为 `050ff3f3a6bdf742ab628a19ddf618331b830bbaa9cabe20349e9c677156fdbd`，849690424 字节。归档阶段没有重新计算二进制哈希。这是 source27 候选身份，不能沿用到不可变 source26/436 的 GUI clone，也不能回填 source26 已失败的原生结果。

本轮没有主程序构建、签名、真实 CLI、GUI、跨平台、完整工作区或 SSH/tmux 完整链验收。纯诊断契约变更无需本地化文案变更；i18n PASS 不等于双语布局检查完成。此前其他源快照的真实证据与失败记录保持历史边界，Goal 尚未完成。

三份 JSON 按原字节复制，原属性没有回填。六类明示凭据形状扫描覆盖原报告序列化字节、独立解码 JSON 字符串以及四份门禁日志，均 0 命中；新增档案同样扫描。只归档 JSON 与本报告，不复制日志正文、test-output、私有诊断或模型帧。

- [输入清单](validation/macos-official-27-inputs.json)
- [Rust 门禁原始报告](validation/macos-official-27-gates.json)
- [Python 门禁原始报告](validation/macos-official-27-python.json)
- [归档范围与安全复核](validation/macos-official-27-archive-audit.json)
