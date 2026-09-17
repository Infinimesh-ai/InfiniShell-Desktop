# source22 本地门禁独立归档

source22 的本地 check、i18n、受影响模块测试与 Python 测试均通过。这是基于 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 的脏工作树中间快照，不能计作最终同提交验收或 Goal 完成。

本归档只读取明确授权的三份原始 JSON、四份日志，以及冻结树清单中的 93 个文件用于两次 SHA 校验。没有执行 Git、Cargo、真实 CLI、模型、网络或 GUI 操作；没有读取 target、私有凭据、真实用户 HOME 或其他私有文件。日志只用于摘要和 SHA 核对，未复制正文。

## 输入与独立校验

- 输入清单包含 93 个文件，原字节 SHA-256：`2179e1a3add0f5507440f565b9fe2b4382ab68112555c313d9fc758073cb1482`。
- 冻结树 93 个文件连续两次重算 SHA，均与清单完全一致，两个轮次各 0 个差异。`frozen_reads_done=true`，之后不再读取冻结树。
- 相对 source21 输入清单，3 个已有输入变化，6 个路径首次纳入本轮清单。这里的“首次纳入”不表示仓库中新建文件。
- 三份原始 JSON 按原字节复制，复制后再次核对源字节；gates 与 Python 报告都关联相同输入清单 SHA。
- 根报告声明门禁后源码保持不变。本归档的独立证据是上述两次冻结文件 SHA 校验与日志核对，没有额外冻结或构建。

## 本地门禁结果

| 门禁 | 原报告耗时 | 独立日志结果 |
|---|---:|---|
| `cargo check -p warp` | 85.881 秒 | exit 0，dev profile 完成 |
| `cargo test -p warp --lib i18n::tests` | 169.478 秒 | 11 通过、0 失败、6783 筛除；测试内部 4.83 秒 |
| 受影响模块 nextest | 18.693 秒 | 1301 通过、5493 跳过；内部 15.087 秒 |
| Python 回归 | 4.831 秒 | 14 组、363 项、14 个 OK，各组 exit 0 |

focused 表达式本轮包含 `ai::agent_sdk::driver::harness::`；日志独立计数该范围 31 个 PASS 行。Python 各组计数依序为 11、7、56、18、25、25、20、10、13、27、19、85、20、27。

check 日志包含 `agent_events` 的两条 unused-import 警告，另有 Cargo 汇总警告行。这是删除旧 runner 后相关测试导出缺少生产引用的警告，解释来自根代理；本归档只独立核对两条警告及其位置数量。警告没有导致 check 失败，也未在归档中消除或隐藏。

## 本轮变更与验证边界

根据输入清单及根代理报告，本轮关闭缺少审批回路的旧 Claude 独立 AgentDriver：私有 runner/build_runner 直接报告不可用，删除固定危险审批绕过与用户全局配置写入器，保留普通终端 pane 验证、托管任务入口和 Claude 身份检测。新增合成配置保留／不创建、分派与认证回归，相关 harness 纳入 focused 范围。上述变更描述来自原输入与根代理报告，本归档没有复读源码正文，也没有扫描或修改真实用户配置。

输入包含英文与简体中文同步的 `cli-agent-standalone-harness-unavailable` 消息，变量为 `$cli`，本轮 i18n 11 项通过。文案和入口变化仍需真实英文、简体中文 GUI 布局检查；不能以 i18n 单元测试或旧快照布局代替本轮检查。本归档自身只新增文档和原始报告复制，无需额外产品本地化变更。

测试二进制身份 `752f2bd512cbe2bc874904a0b647350979a2b46c702254c43e37beecc96f450a` 仅来自根报告字段，本归档没有读取二进制。没有 source22 main 构建、严格签名、真实 CLI 全链、GUI、最终同提交平台、SSH/tmux 或全工作区测试证据；所有这些项目仍待验收。本地回归通过不证明旧入口的真实审批链已经可用，也不追认先前 FAILED 原生报告通过。

## 原始档案与审计

- [输入清单](validation/macos-official-22-inputs.json)
- [Rust 门禁报告](validation/macos-official-22-gates.json)
- [Python 门禁报告](validation/macos-official-22-python.json)
- [独立归档审计](validation/macos-official-22-archive-audit.json)

原始三报告、四日志及新归档按固定六类凭据形状扫描，只记录匹配数量，均为 0；不输出匹配正文。零匹配仅表示这些固定形状没有命中，不能保证识别所有凭据格式。旧报告和根三份计划／矩阵／验证文档未修改。
