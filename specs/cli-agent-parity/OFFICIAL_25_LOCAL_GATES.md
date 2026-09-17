# source25 macOS 候选本地门禁

本次是基于 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04` 的111路径脏候选，输入 SHA-256 为 `3a722bb3f6f2c0c4282b628c4e0671f6a8e4c17c972b2b407b667b681eba1101`。相对 source24 只改变三份文件：Windows Python 路径断言、Rust 私有JSON转义断言、两平台 Session 实际模块筛选；生产权限与协议规则没有因测试失败而放宽。

| 门禁 | 真实结果 | 墙钟秒数 |
|---|---|---:|
| cargo check -p warp | PASS | 52.750 |
| cargo test -p warp --lib i18n::tests | 11 PASS，0失败，6865 filtered | 142.748 |
| 定向 nextest | 1393 PASS，0失败，5483 skipped；内部16.639秒 | 19.892 |
| Python 回归 | 15组410项，全部退出0 | 4.999 |

本次实际筛选 `terminal::model::session::test`，四项新增命令快照回归各有一条PASS，包含缺少bootstrap字段、重新bootstrap、执行器切换与连接切换。source24原1378及独立补充15计数保持原记录，不回填历史。

输入、门禁和Python报告分别保留于 [输入清单](validation/macos-official-25-inputs.json)、[Cargo报告](validation/macos-official-25-gates.json)、[Python报告](validation/macos-official-25-python.json)，按原字节复制。日志只保存摘要与SHA，未复制正文。三个报告关联同一输入清单，根运行器前后验证全部111路径未改变。lib SHA-256为 `1c4d7fcf6cfd6aca2be85b28d953a03c08e3a26ae22c741dd25dcede24210e2c`。

这次两项修复仅改变测试断言，另一个改动仅修正筛选表达式，**无需本地化变更**。i18n单元门禁通过，最终英文／简体中文GUI布局仍待验。

本地候选通过不等于 CI11修复后平台通过，也不计新实际提交、main构建、签名、原生、GUI、SSH／tmux、全工作区或 Goal完成；下一步必须验证包含这些修改的同一实际提交。凭据形状扫描和原始文件摘要见 [归档审计](validation/macos-official-25-archive-audit.json)。
