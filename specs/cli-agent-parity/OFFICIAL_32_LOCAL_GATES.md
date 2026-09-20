# source32 候选本地门禁

本轮在实际父提交 `0059ef1bf8438c5af7545c3101b880ec2d0b10e9` 的独立 detached 验证树执行授权本地门禁。119 个输入中 118 个有父 Git 对象，1 个为新增旧版 README 夹具；真实变更为 18 路径，其余 101 输入逐父对象核对。候选尚未提交，`dirty_candidate=true`、`same_commit_verified=false`，不代表最终提交或原生验收。

冻结清单 SHA-256 为 `bd9b16f9debe9dd1038e8203e64b3f4205f21203fea781850aff1a092de89998`，根最终授权 SHA 为 `784f675cda2b73e29d81c1325d04fdda00acaa0572781c889d617100e8f8f72e`。根完成源码写入代理 STOP、最终路径 SHA、plan pin 及 freeze 后，本代理执行唯一授权 gates 命令，工具 session 51337 最终退出 0，没有失败重试或源码修复。

## 实际结果

| 门禁 | 真实结果 | 外部耗时 |
|---|---|---:|
| `cargo check -p warp` | 退出 0 | 140.859 秒 |
| `cargo test -p warp --lib i18n::tests` | 11 PASS，0 failed/ignored，6930 filtered；内部 4.66 秒 | 320.445 秒 |
| 受影响 Rust nextest | 1458 PASS，5483 skipped，1458 条 PASS 行 | 25.620 秒 |
| 15 组 Python 纯夹具 | 453 PASS，0 failed/skipped；权限认证探针为 51 项 | 15.501 秒 |

Rust 筛选保留真实 `terminal::model::session::test::` 模块，并显式纳入 Grok 插件完整测试命名空间。新增测试名从父对象与最终源码按差集提取，38 个名字在实际日志中分别恰好出现一次 PASS，没有预设通过次数。按清单分类如下：

| 范围 | 变更路径 | 新 Rust 回归 |
|---|---:|---:|
| SDK 诊断及生产维护响应兼容 | 5 | 13 |
| 历史消息预览与双语资源 | 4 | 3 |
| 缓存认证握手及闭合审计 | 4 | 10 |
| 插件迁移与失败恢复 | 5 | 12 |

根最终冻结程序 SHA 为 `b23525b5bee60ab2cbbe723716e86950f4b6705e74220c0eb009009b95145b4b`，已在原准备程序上补齐 `tokio::test` 提取；原准备副本保留。8 个异步测试的属性数量来自根最终授权说明，本代理独立从日志核对全部 38 名的实际覆盖。gates 程序 SHA 为 `b4a370efbd12d88d6ce21953b8c998617c6dbab93bc7e2ed11d455b01c693298`，执行时未修改程序。

本轮测试库由真实 i18n 日志定位：`warp-3ad688c9d0ab1400`，850624232 字节，SHA-256 `edead012a79f8add9251eab300f87d552b282bfb8812eeb893f8bac7e2fd84a1`。执行程序在 i18n 后、受影响测试后及 Python 各阶段前后实际计算身份并保持一致；每步同时守护根授权、程序、清单、119 源码 SHA、父提交及冻结树真实修改集合。归档阶段不再读取 target、冻结树或二进制。

## 安全归档和验收边界

原始 [inputs](validation/macos-official-32-inputs.json)、[Rust gates](validation/macos-official-32-gates.json)、[Python](validation/macos-official-32-python.json) 三 JSON 按原字节复制，不回填属性。另有[归档审计](validation/macos-official-32-archive-audit.json)，记录闭合字段、重复键及非有限数值拒绝、日志 SHA 和实际计数复核。六类定向凭据形状在公共 JSON 序列化及解码字符串、四份临时测试日志中均为 0；日志正文没有入库。执行与归档由同一本代理完成，不冒认第二独立执行者。

用户的 `web_runtime.rs`、`websearch_tests.rs` 及其他未授权未追踪文件没有复制。旧 source31 和更早原生失败证据没有改写。本轮没有主程序构建、签名、policy4/SDK 原生重验、GUI 或跨平台验证，也没有全工作区 nextest。纯夹具可以使用本地合成进程及 loopback，不请求真实模型或账号 API。已知权限上限、业务 SDK 来源及产品能力门槛不因这些本地通过而提升，完整 P0–P5 与 Goal 验收继续待办。

界面变更的英文与简体中文 FTL 已在候选中同步，i18n 门禁通过；本轮不重演双语布局，source31 旧 GUI 专项保持自身范围。认证握手、维护响应与插件回归没有新增产品可见文案，无需额外本地化变更。`finished_reads_done=true`，本代理已停止读取并释放共享 target。
