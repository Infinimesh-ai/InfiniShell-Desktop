# source24 macOS 本地门禁独立归档

本次对象是基于 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04` 的 **111 文件脏候选快照**，不是包含本轮修改的新实际提交。正式输入清单 SHA-256 为 `4705e0753c8126490070a2e47dfe3c23d0d80663a69e9da77dd0ef1ec2404706`。本地准备、所选主门禁、Python 回归及独立 Session 补充门禁通过；原生 CLI、main 构建、签名、GUI、同一实际提交的平台验收和 Goal 完成均不在本报告的通过范围。

## 已核对的真实本地证据

| 步骤 | 报告墙钟秒数 | 独立日志摘要 | 证据 |
|---|---:|---|---|
| 准备预编译 | 229.418 | 日志显示完成 test profile 构建，无测试执行摘要；报告 exit 0 | [预编译报告](validation/macos-official-24-precompile.json) |
| 唯一 ignored 入口核对 | — | 报告 exit 0、唯一入口列出一次；命令含 `--exact --ignored --list`，仅列举，未执行测试 | [入口报告](validation/macos-official-24-entrypoint.json) |
| cargo check -p warp | 71.459 | 日志显示完成 dev profile 构建，报告 exit 0 | [主门禁报告](validation/macos-official-24-gates.json) |
| i18n::tests | 6.258 | 11 passed / 0 failed / 0 ignored / 6865 filtered；测试内部 4.34 秒 | [主门禁报告](validation/macos-official-24-gates.json) |
| focused nextest | 18.003 | 1378 tests run / 1378 passed，1378 条 PASS；5498 skipped；内部 16.159 秒 | [主门禁报告](validation/macos-official-24-gates.json) |
| Python 回归 | 4.970 | 15 组、410 项，15 条 OK、0 条 FAILED；逐组数量与报告一致 | [Python 报告](validation/macos-official-24-python.json) |
| Session 模块补充 nextest | 2.300 | 15 tests run / 15 passed，15 条 PASS；6861 skipped；内部 0.161 秒；含新增 4 项 | [独立补充报告](validation/macos-official-24-session-supplement.json) |

入口报告的 `exact_ignored_entrypoint_listed_once=true` 和 stdout/stderr SHA 来自根运行器；本归档没有读取或复制入口输出。其 `stderr_bytes=258` 原样保留，不回填为空。唯一入口列表通过只能解除运行器准备阻塞，不能证明 Grok 权限接口或原生生命周期已通过。

## 输入关联和覆盖边界

[准备清单](validation/macos-official-24-prepared-inputs.json) SHA-256 为 `3a0b00201651e650bff552184f4df64d67a2862504b010a188e8408845482814`；[正式清单](validation/macos-official-24-inputs.json) 关联准备清单和入口证明，两个门禁报告均精确关联正式清单。准备与正式清单都含 111 路径；两清单间改变的路径见独立审计，其原始 scope 和准备属性没有被改写。

归档者只读取冻结树清单内的 111 路径各一次计算 SHA，**111/111 匹配**，未读清单外文件；随后已向根代理声明 `frozen_reads_done=true` 并停止读取冻结树。根报告的 `source_unchanged_after_gates=true` 原样保留；本次冻结核对是门禁结束后的单次观察，不替代根运行器的前后检查。

主 focused 筛选曾遗漏新增 Session 回归：表达式选择 `terminal::model::session::tests::`，实际模块名为 `test`。根代理随后在同一正式 111 文件快照上执行 **独立 Session 补充门禁，15/15 PASS，包含新增 4 项各一条 PASS**。归档者独立核对日志摘要和逐条计数，覆盖 bootstrap 缺字段、重新 bootstrap、执行器切换及连接切换的命令快照完整性。补充报告精确关联正式输入 SHA，报告的 `source_unchanged_after_gates=true` 原样保留；未重新读取冻结树。主 focused 原始 **1378 PASS / 5498 skipped** 保持原记录，补充 **15 PASS / 6861 skipped** 单独记录，不合并或回填主计数。根主工作区 workflow 的两个模块筛选修正属于下一快照，不在本次 source24 manifest 内。

报告中的 lib SHA-256 为 `eb3fdde242fa992f8c9414d96a208878f059a1c3fdaefd18728c054413b53be8`，与入口报告的测试二进制身份字段一致；该一致性仅由公共报告字段核对。**未读取 target 或二进制，未独立验证当前二进制字节。** `base_commit` 为报告基线身份，`worktree_clean=false` / `worktree_dirty=true` 明确保留；本轮未执行 Git，也没有建立最终提交的身份链。

本轮没有新增或改动用户可见文案，按根代理变更范围说明记录为“无需本地化变更”。i18n 单元测试通过；英文和简体中文 GUI 布局、本轮完整资源嵌入仍待验，不能由单元门禁替代。

## 归档审计与未满足项

六份原始 JSON 加一份独立补充 JSON 均按原字节复制，所有属性原样保留。六份允许读取的日志只核对 SHA、摘要和凭据形状，没有复制日志正文。原始报告、日志和新公共档案的四组固定凭据形状扫描均为 0；扫描方式和逐文件 SHA 见 [独立归档审计](validation/macos-official-24-archive-audit.json)。

本次归档未运行 Cargo、CLI、模型、网络、GUI 或 Git；未读取认证材料、真实 HOME、policyDoc、target 或其他私有诊断文件。source24 的 main、签名、原生三方完整链、应用重启恢复、双语 GUI、同一新实际提交的 macOS/Linux/Windows、SSH/tmux 完整链、full workspace 和 Goal 最终验收均没有在本报告中取得通过证据。
