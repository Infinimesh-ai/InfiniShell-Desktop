# Claude 原生图片最小校准

状态：夹具已交付；23 项 Python 离线回归已通过。真实请求及 Rust 编译尚未在本夹具上执行；离线合成账本不属于真实图片验证。生产图片门禁保持关闭。改动仅涉及测试、隔离运行器和说明，无需本地化变更。

## 已知边界

固定 `Claude Code 2.1.273` 的 `--help` 提供 `--input-format stream-json`、`--replay-user-messages`、`--bare`、`--tools ""` 与 `--strict-mcp-config`。公开原生文本记录的 capability 只有取消回执、排队取消和消息生命周期；它们没有证明图片输入有效。

[官方 Agent SDK 图片示例](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode) 给出 `message.content` 数组中的 `text` 与 `image`、`source.type=base64`、`media_type=image/png`。该文档是待校准 payload 的依据，不能代替固定可执行的实际观察。

生产 `claude.rs` 当前拒绝 `LocalImage`，且 `UserTurn.expected_replay` 是字符串。本夹具使用独立测试 framing，复用生产 `write_message`、`managed_process::spawn`、`ManagedChild::finish` 与 `confirmed_exit`。它不经过生产图片编码、协调器、持久化或 GUI，不修改用户全局设置。

## 单次输入和安全策略

夹具以标准库在内存中生成 64×64 RGB PNG；四象限分别为红、绿、蓝、黄，顺序由新 UUID 随机排列。正确顺序只存在于像素和本地断言中，提示词列出候选颜色名但不携带答案。生成器使用 PNG CRC-32、无压缩 DEFLATE 与 Adler-32，不读取已有图片，不写项目或调用外部图片工具。

原生进程固定为 `--bare --tools "" --strict-mcp-config --mcp-config '{"mcpServers":{}}' --permission-mode default`，采用私有空 HOME/配置目录和显式 API 环境。初始化及原生 `system.init` 必须确认默认权限；后者必须确认工具和 MCP 列表均为空。任何工具块、原生权限请求或额外控制请求均拒绝并使校准失败，不追加输入或重试。

唯一的用户消息为 UUID `U`；新建会话首帧沿用生产已观察的空 `session_id`：

```json
{
  "type": "user", "uuid": "U", "session_id": "", "parent_tool_use_id": null,
  "message": {"role": "user", "content": [
    {"type": "text", "text": "只判断四象限颜色，按左上、右上、左下、右下输出四个英文大写颜色名"},
    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "<内存生成 PNG 的 base64>"}}
  ]}
}
```

上例文本仅说明语义；实际固定英文提示词在 Rust `PROMPT` 常量中。输入预算为一条原生用户消息，不声称等于一次 HTTP 请求；内部模型请求计数仍不可见。

## 真实通过所需证据

1. 固定版本校验和同代 `initialize` 的成功回执，原生状态 `idle`、权限 `default`。
2. 唯一输入 `U` 成功写入；原生 `isReplay=true` 的用户回放仍为数组，角色、整份数组及 `U` 精确相等。公开投影保留数组 SHA、块类型、文本 SHA、MIME、解码 PNG SHA 和字节数，不保留 base64 或正文。
3. 同一真实原生会话 `S` 的 `command_lifecycle queued/started` 对应 `U`；`started` 本身可作为原生接收确认，必须存在，不能以管道写入成功代替。
4. 主 assistant 的顶层 `user_message_uuid=U`、原生会话 `S`、唯一 assistant UUID 和无父工具归属。不得把当前活跃输入补作来源。只公开文本散列；思考块仅保留类型。
5. 原生唯一 `result` 对应 `S`，同时包含 `user_message_uuid=U` 和完整 `user_message_uuids=[U]`，`subtype=success`、`is_error=false`，无取消或错误终止原因。完整主 assistant 文本和 result 文本去除首尾空白后，都必须精确等于像素生成的四颜色答案；不能以子串或末尾匹配代替。
6. 关闭 stdin，同时保留并有界读取 stdout 至 EOF，与生产监督进程确认退出并行完成。收尾输出最多 8 MiB，只接受空输出或同一会话、同一输入的一条 completed 生命周期帧；超时、额外工具、未知帧或来源变化均失败。再读取同代真实退出回执，确认 containment、代次、`cleanup_confirmed=true`、`exit_code=0` 和 `exit_reason=stdio_closed`。生产 `finish` 返回值与再次读取的真实回执都必须满足该条件。图片协议结果正确和正常退出是两层独立证据；非零或缺失退出码、主机连接断开、请求停止或单纯原生退出均不能计为校准成功。

运行器按固定事件和字段白名单独立重算上述账本，不接受只有成功摘要、重复输入/终态、旧代初始化、会话错配、字符串摘要替代数组或额外正文/凭据字段。原生关联缺失时保留失败，不能推断接口已支持图片。

## 接线和执行

由根代理在 `claude_live_tests.rs` 注册：

```rust
#[path = "claude_image_probe_live_tests.rs"]
mod image_probe_live_tests;
```

精确测试名为 `ai::cli_agent_runtime::claude::live_tests::image_probe_live_tests::real_claude_image_input_probe`，测试默认 ignored。scope 为 `claude_native_image_input_probe`。沿用 `INFINISHELL_CLAUDE_LIVE_ROOT/CONFIG_DIR/EXECUTABLE/ARTIFACT/MODEL` 与 `INFINISHELL_CLI_SUPERVISOR_EXECUTABLE`；认证值仅由基础隔离运行器传给原生进程，不记入 argv、manifest、事件或公开 metadata。

根代理在冻结、编译及对应门禁完成后，可显式运行：

```sh
python3 -B script/cli-agent-parity/run_claude_image_probe.py \
  --test-binary /绝对路径/同提交-libtest \
  --claude /绝对路径/固定-2.1.273-claude \
  --supervisor /绝对路径/同提交-生产监督入口 \
  --api-environment-file /绝对路径/私有-API-环境.json \
  --model 固定原生模型 \
  --output /绝对路径/全新-图片校准.ndjson
```

离线审计命令：在 `script/cli-agent-parity` 下运行 `python3 -B -m unittest claude_image_probe_runner_tests`。运行器复用基础私有项目、版本验证、脱敏和进程隔离，不读取或复制原生登录资料。生成 `.ndjson`、`.metadata.json`、`.test-output.txt`，保留固定可执行、libtest、监督组件 SHA 和实际私有工作目录。

即使校准通过，也只可记为当前平台/模型/固定原生版本的一次结构化图片校准。`production_image_gate_open`、GUI/剪贴板/附件路径、应用重启、持久化、父权限上限、操作系统沙箱与 HTTP 计数始终为未验证；开放产品能力还需单独实现数组回放及完整产品验收。
