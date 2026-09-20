# source31 候选本地门禁记录

本轮本地门禁全部通过。验证对象是实际父提交 `0059ef1bf8438c5af7545c3101b880ec2d0b10e9` 上的独立 detached 候选树，未提交；111 个输入路径共 6,935,404 字节，仅 7 个授权 SDK 诊断、任务输入预览及中英文资源路径改变，其余 104 个路径在准备阶段逐字节核对父提交 Git 对象。保留 `dirty_candidate=true`、`same_commit_verified=false`，不能据此宣称新同提交或原生、GUI、跨平台验收已完成。

| 门禁 | 真实结果 | 总耗时 | 测试内部耗时 |
| --- | --- | --- | --- |
| `cargo check -p warp` | exit 0 | 138.701 秒 | — |
| `cargo test -p warp --lib i18n::tests` | 11 通过，0 失败，0 忽略，6902 过滤 | 305.205 秒 | 4.49 秒 |
| 受影响模块 nextest | 1430 通过，5483 未选中，1430 条 PASS | 20.24 秒 | 16.8 秒 |
| 15 组 Python 夹具 | 443 通过，无跳过，各组 exit 0 | 7.892 秒 | 分组记录见原报告 |

新增 7 项 SDK 私有响应形状诊断回归和 3 项已保存输入预览回归，已逐项核对各一条真实 PASS。SDK 测试覆盖闭合维护响应、u64 边界、伪身份、额外字段、首条捕获、深层结果值不复制以及未知响应仍拒绝且用户 pending 不被消费；它们使用合成协议数据，不能证明固定 Grok 原生已提供相同内层形状，也没有打开生产 SDK 或权限门禁。界面回归核验图片及重复附件、多块文本、文件与技能类型的预览；英文和简体中文资源纳入本轮 i18n 门禁，真实双语布局仍由根任务另行检查。

Rust 筛选沿用已授权表达式，包括实际模块 `terminal::model::session::test::`。Python 执行完整离线合成夹具，包括已授权的 localhost/socket/tunnel 与合成子进程；没有执行真实 CLI、调用网络模型或读取认证。本轮一次顺序执行 check、i18n、focused 和 15 组 Python，未重投失败或沿用旧结果。每项门禁前后核验 111 个 SHA、父提交和 exact 7M 候选状态；用户排除的两个源码文件及根主文档未复制。

实际测试二进制由本轮 i18n 日志观察，850,320,360 字节，SHA256 `c4c1c54f805ebc8dbdd745a0702bd883960ac28d1c9efdb609cb785af6037ca0`；Rust 门禁后及 Python 门禁后哈希一致。准备阶段的旧二进制元数据仅引用 official29 输入清单，不代表当前 shared target 或本轮通过。归档阶段只引用执行器公共报告，不重读 target、冻结树或二进制。

三份公共 JSON 按原字节归档，字段没有回填。归档核对四份编译/纯夹具日志的 SHA、测试摘要和六类明示凭据形状，原报告序列化字节、独立解码 JSON 字符串及新增档案均为 0 命中；日志正文、私有 envelope、native/test-output、模型帧均未归档。执行与归档由同一子代理完成，不能声称另一主体独立重跑。

本轮没有主程序构建、签名、真实 CLI/模型、认证、GUI、跨平台、完整工作区或 SSH/tmux 完整链验收；不回填 source27 等原生失败，也不将 source31 候选计为已提交平台通过。目标仍未达到全部完成门槛。

- [输入清单](validation/macos-official-31-inputs.json)
- [Rust 门禁原始报告](validation/macos-official-31-gates.json)
- [Python 门禁原始报告](validation/macos-official-31-python.json)
- [归档范围与安全复核](validation/macos-official-31-archive-audit.json)
