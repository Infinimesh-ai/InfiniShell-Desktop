# source29 候选本地门禁记录

本轮本地门禁全部通过；Goal 仍为 ACTIVE。验证对象是实际父提交 `e6873e2cbb12a12ffce0b8283ee2ac96b940da39` 上的独立 detached 候选树，未提交。输入清单 111 个路径，仅 2 个 Rust 与 2 个 Python 文件改变，其余 107 个路径保持父提交原字节；原 `dirty_candidate=true`、`same_commit_verified=false` 保留。本轮没有沿用 source28 的测试结果。

| 门禁 | 真实结果 | 总耗时 | 测试内部耗时 |
| --- | --- | --- | --- |
| `cargo check -p warp` | exit 0 | 131.245 秒 | — |
| `cargo test -p warp --lib i18n::tests` | 11 通过，0 失败，0 忽略，6892 过滤 | 314.571 秒 | 4.74 秒 |
| 受影响模块 nextest | 1420 通过，5483 未选中，1420 条 PASS | 22.966 秒 | 18.872 秒 |
| 15 组 Python 夹具 | 436 通过，无跳过，各组 exit 0 | 8.151 秒 | 分组记录见原报告 |

Rust 筛选沿用已授权表达式，包括真实模块 `terminal::model::session::test::`。Python 执行完整本地合成夹具，包括已授权的 localhost/socket/tunnel 与合成子进程。新范围仅包含两个 Grok policy 预检 Rust 文件和两个 policy Python runner 文件；没有扩大源码复制域。本地通过不回填旧平台或原生失败，也不代表修复后的平台已经通过。新增 10 项 Rust 回归已逐项核对各一条 PASS。

实际执行器在每项门禁前后核验 111 个输入 SHA 与 Git exact 4M 状态。用户排除的 `web_runtime.rs`、`websearch_tests.rs` 不在清单中，根文档未复制，原 source28 树未读写。编译前共享 target 既有测试二进制只记录基线身份，不能当成本轮候选通过证据。

本轮测试二进制由实际 i18n 日志观察，执行器读取 SHA 为 `7e9606c1468d4cb8cfb116b4a578c23293ba2d38089693198dec0c9b17f42a19`，849545480 字节；Rust 门禁后与 Python 门禁后哈希一致。归档只引用公共执行器报告，没有重读 target 或二进制。此身份属于 source29 候选，不能沿用到不可变 436 的 GUI clone，也不能回填 source26 已失败的原生结果。

归档复核读取三份公共原报告与四份本地门禁日志，独立核 SHA、摘要与测试计数；执行与归档由同一子代理完成，不能作为另一主体独立复验。三份 JSON 按原字节复制，原属性没有回填。六类明示凭据形状扫描覆盖原报告序列化字节、独立解码 JSON 字符串、四份日志与新增档案，均 0 命中；日志正文、test-output、私有诊断与模型帧均未归档。

本轮没有主程序构建、签名、真实 CLI/模型、认证、GUI、跨平台、完整工作区或 SSH/tmux 完整链验收。无需本地化文案变更；i18n PASS 不等于双语布局检查完成。Goal 尚未满足全部完成门槛。

- [输入清单](validation/macos-official-29-inputs.json)
- [Rust 门禁原始报告](validation/macos-official-29-gates.json)
- [Python 门禁原始报告](validation/macos-official-29-python.json)
- [归档范围与安全复核](validation/macos-official-29-archive-audit.json)
