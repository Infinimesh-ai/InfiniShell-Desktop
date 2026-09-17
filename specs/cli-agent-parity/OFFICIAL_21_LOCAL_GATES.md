# Source21：macOS 本地门禁独立归档

本轮冻结源码的本地门禁全部通过，但 Goal 尚未完成。source21 为基线 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 上含实际修改的中间脏快照，87 路径输入清单 SHA `966dfd9ff2bced152808fb6ee75bb2a7b41e6f87eabb57157fdd0a947f24dff0`。相对 source20，清单记录 10 个既有路径改变、4 个新纳入路径；新路径为待定 Edit 取消 Rust 夹具、两个 Python 运行／审计文件及现代 Grok discovery 协议夹具。

## 独立核对

归档代理连续两次读取 87 个 frozen 小源码并重算 SHA，全部与原清单一致；随后停止 frozen 读取，`frozen_reads_done=true`。三份原 JSON 报告按原字节复制，复制后重新对照源文件。允许读取的 check／i18n／focused／Python 四份日志均独立重算 SHA、解析实际摘要，未复制日志全文，没有输出凭据匹配内容。

## 真实本地结果

| 门禁 | 实际结果 | 墙钟时间／内部摘要 |
| --- | --- | --- |
| `cargo check -p warp` | 原报告退出 0，日志完成 dev profile | 63.194 秒 |
| `cargo test -p warp --lib i18n::tests` | 11 通过、0 失败、6793 filtered out | 178.715 秒；内部 4.66 秒 |
| 受影响模块 nextest | 1270 通过、5534 skipped | 18.283 秒；内部 14.853 秒 |
| Python 离线回归 | 14 组／363 项，14 条 OK 摘要 | 4.794 秒；各组数量与原报告逐项一致 |

Python 各组实际数量依次为 `11, 7, 56, 18, 25, 25, 20, 10, 13, 27, 19, 85, 20, 27`。新增待定 Edit 取消组 25 项、PNG 组 20 项、Grok SDK 组 85 项、固定 Codex runtime 准备器组 27 项；离线通过不是真实 CLI 或模型流程通过。

focused 表达式仍为 `test(cli_agent) | test(local_cli_mailbox) | test(local_cli_tasks) | test(local_harness) | test(ai::orchestration::) | test(ai::agent_providers::chat_stream::) | test(ai::blocklist::action_model::execute::)`，不是完整工作区门禁。

四份日志 SHA 分别为 check `dc5fac0bdacea32414e7bac7884e3834a07ab4a27833e6ee203393a04a27a73a`、i18n `b07e46335c2c8666c9b706de6052e060f10700f0b176c97b3663e7714571a29c`、focused `c4ee5b13a6945a6d45f0c78ee91d1ee686ad4ed1b872eb578b2087f67a43b40c`、Python `49782b34c909f0533ccc091b918ffda6bc131113d6cb641783c593e06c11b301`，均独立重算一致。

lib SHA `10755f06d9e0d004c99551f24063660139f34bc8e9f07d08f3f9dd328d81a09e` 仅引用根代理 gates 原报告字段，本归档没有读取 target 二进制。

## 新发现与验收边界

根代理随后发现旧独立 Claude AgentDriver 入口仍含固定 bypass 和用户全局配置改写；该发现来自根代理，归档代理没有读取或复验此入口。主工作区正在独立修复，但未改变 source21 的 87 路径 frozen 清单。本轮门禁不能证明旧入口已修复，不能宣称所有 Claude 启动路径的配置与权限问题已经消除。source22 必须把该修复纳入新的清单并重新运行门禁，source21 原始报告保持不变。

source21 没有 main 构建／签名／真实原生验收／GUI／新平台验证证据，不沿用 source20 main 或原生结果。现代 MCP 业务来源、PNG 完整恢复、等待 Edit 取消、父子 GUI、三平台同一实际修改提交、全工作区、SSH／tmux、最新双语布局及用户其余门槛仍需独立证据。没有将本轮门禁视为 Goal 完成。

本次仅文档和证据归档，没有产品文案变化，无需本地化变更。三份原报告、四份原日志和全部新增档案固定六类凭据形态扫描均为零匹配，此扫描不能保证识别未知格式。没有读取认证／私有凭据或 target，没有运行 Git、Cargo、CLI、模型、网络或 GUI，没有改主计划、能力矩阵、主验证报告或旧档案。

## 档案

[输入清单](validation/macos-official-21-inputs.json)、[Rust 门禁](validation/macos-official-21-gates.json)、[Python 门禁](validation/macos-official-21-python.json)、[独立归档审计](validation/macos-official-21-archive-audit.json)。
