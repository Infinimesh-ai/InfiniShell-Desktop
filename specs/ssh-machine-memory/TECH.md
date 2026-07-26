# ssh-machine-memory — Tech Spec

功能定义见 `PRODUCT.md`。本文按可独立执行的任务拆分（Task 1–6），每个任务
自带验收标准。执行会话请只领取一个 Task，读完本文全文与 PRODUCT.md 后再动手。

**任务依赖**：Task 1 → Task 2 → Task 3 → Task 4；Task 5 / Task 6 依赖 Task 1，
可与 3/4 并行。Phase 1 = Task 1+2+3；Phase 2 = Task 4；Phase 3 = Task 5+6。

## 0. 模块放置总览

```
crates/persistence/
├── migrations/2026-07-XX-000000_add_ssh_machine_memories/   (NEW — Task 1)
├── src/schema.rs                                            (修改 — Task 1)
└── src/model.rs                                             (修改 — Task 1)

crates/warp_ssh_manager/src/
├── db.rs                           (修改 — Task 2：首次打开失败可降级)
├── lib.rs                          (修改 — 导出新模块)
├── memory.rs                       (NEW — Task 1：类型 + repository + key 归一化)
└── sync_provider.rs                (修改 — Task 6：同步新表)

app/src/ai/machine_memory/          (NEW — Task 2/4)
├── mod.rs                          (加载辅助)
└── review.rs                       (Task 4 — 后台复盘)

app/src/ai/agent/api.rs             (修改 — Task 2/5：RequestParams 增字段)
app/src/settings/ai.rs               (修改 — Task 2：AI 设置项)
app/src/ai/agent_providers/
├── chat_stream.rs                  (修改 — Task 2 注入；Task 3 工具拦截)
├── tools/mod.rs                    (修改 — Task 3：REGISTRY 注册)
├── tools/machine_memory.rs         (NEW — Task 3)
└── prompts/
    ├── tool_descriptions/update_machine_memory.md   (NEW — Task 3)
    └── tasks/machine_memory_review_system.md        (NEW — Task 4)

app/src/ssh_manager/server_view.rs  (修改 — Task 6：记忆区块 UI)
app/i18n/{en,ja,zh-CN}/warp.ftl     (修改 — Task 6)
```

分层规则不变：`warp_ssh_manager` 保持纯 Rust、不依赖 `warpui`；
所有 UI / AppContext 相关代码在 `app/` 侧。

---

## Task 1 — 数据层：表、repository、machine_key 归一化

### 1.1 Migration

新目录 `crates/persistence/migrations/2026-07-XX-000000_add_ssh_machine_memories/`
（XX = 实际日期；格式对齐 `2026-05-04-120000_add_ssh_manager_tables`）：

`up.sql`：

```sql
CREATE TABLE ssh_machine_memories (
    machine_key    TEXT PRIMARY KEY NOT NULL,  -- 归一化 "host:port"，见 1.3
    content        TEXT NOT NULL DEFAULT '',   -- Markdown 记忆全文
    hostname_alias TEXT DEFAULT NULL,          -- DCS 回报的远端真实 hostname（可空）
    ssh_node_id    TEXT DEFAULT NULL,          -- 可选关联 ssh_servers.node_id
    last_review_at TEXT DEFAULT NULL,          -- 上次后台复盘时间（RFC3339）
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
```

`down.sql`：`DROP TABLE ssh_machine_memories;`

`schema.rs` / `model.rs` 按 `ssh_servers` 现有行结构照抄式扩展
（`SshMachineMemoryRow` / `NewSshMachineMemory`）。注意 `schema.rs` 中
`diesel::table!` 手写声明需与 up.sql 严格一致。

### 1.2 Repository

`crates/warp_ssh_manager/src/memory.rs`，风格对齐 `repository.rs`
（方法接受 `&mut SqliteConnection`，事务边界由调用方决定，错误复用/仿照
`SshRepositoryError`）：

```rust
pub struct MachineMemory {
    pub machine_key: String,
    pub content: String,
    pub hostname_alias: Option<String>,
    pub ssh_node_id: Option<String>,
    pub last_review_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub struct MachineMemoryRepository;

impl MachineMemoryRepository {
    pub fn get(conn, machine_key: &str) -> Result<Option<MachineMemory>, _>;
    /// upsert：不存在则插入；存在则更新 content/updated_at。
    /// content 超过 MAX_MEMORY_CHARS 时按 char 截断（保护 CJK）。
    pub fn upsert_content(conn, machine_key: &str, content: &str) -> Result<(), _>;
    pub fn set_hostname_alias(conn, machine_key: &str, alias: &str) -> Result<(), _>;
    pub fn set_last_review_at(conn, machine_key: &str, at: DateTime<Utc>) -> Result<(), _>;
    pub fn list_all(conn) -> Result<Vec<MachineMemory>, _>;   // Task 5/6 用
    pub fn delete(conn, machine_key: &str) -> Result<(), _>;  // Task 6 用
}

pub const MAX_MEMORY_CHARS: usize = 16_000;
```

### 1.3 machine_key 归一化（纯函数，同文件）

```rust
/// 输入：InteractiveSshCommand 解析出的原始 host（可能是 "user@host"、
/// ssh_config 别名、IP）与可选 port 字符串。
/// 规则：剥离最后一个 '@' 之前的用户名前缀 → trim → 小写 →
///       拼 ":{port}"，port 缺省/解析失败取 22。
/// host 为空/全空白时返回 None（无法定位机器，调用方跳过记忆功能）。
pub fn resolve_machine_key(host: Option<&str>, port: Option<&str>) -> Option<String>;
```

示例：`("root@Web-01", None)` → `web-01:22`；`("10.0.0.5", Some("2222"))` →
`10.0.0.5:2222`；`(None, _)` → `None`。ssh_config 别名不展开（别名本身即 key；
真实 hostname 通过 `hostname_alias` 列辅助归并，本任务只需存取该列，不做归并逻辑）。

### 1.4 验收标准

- [ ] `cargo check` 全绿；migration 可 up/down 往返。
- [ ] `resolve_machine_key` 单元测试覆盖：user@ 前缀、大小写、缺省端口、
      显式端口、空 host、纯空白 host。
- [ ] repository 单元测试（参考 `warp_ssh_manager` 现有测试用内存 SQLite 的方式）：
      upsert 新建、upsert 覆盖、超长 content 截断到 16 000 字符、get 不存在返回 None。
- [ ] 不引入任何 `warpui`/`app` 依赖到 `warp_ssh_manager`。

---

## Task 2 — 读路径：把机器记忆注入 system prompt

### 2.1 加载辅助（app 侧）

新模块 `app/src/ai/machine_memory/mod.rs`：

```rust
pub struct MachineMemoryContext {
    pub machine_key: String,
    /// 注入用内容，已截断到 INJECT_MAX_CHARS(6_000)。空记忆 => content 为空串。
    pub content: String,
}

/// 仅 legacy SSH 会话返回 Some（Phase 1 范围，见 PRODUCT.md 决策 H）。
/// 内部：session_context.is_legacy_ssh() && resolve_machine_key(...) 成功
/// → warp_ssh_manager::with_conn(|c| MachineMemoryRepository::get(...))。
/// 机器无记忆时也返回 Some（content 空串）——工具可用性（Task 3）依赖 machine_key 存在。
/// DB 错误：log warn + 返回 None，绝不 panic、不阻塞请求。
pub fn load_for_session(
    session_context: &SessionContext,
    ctx: &AppContext,
) -> Option<MachineMemoryContext>;
```

设置项：在 `app/src/settings/ai.rs` 的现有 AI 设置处（`AISettings`，
`is_memory_enabled` 所在体系）新增
`ssh_machine_memory_enabled: bool`，默认 `true`。`load_for_session` 在
总开关 `is_memory_enabled` 或本开关为 false 时直接返回 None。

`crates/warp_ssh_manager/src/db.rs` 的连接惰性初始化必须传递
`open()` 错误，不得在 `OnceLock::get_or_init` 内 `expect`；这样未初始化、
数据库打开失败或表缺失都能在 app 侧统一 warn 后降级。

### 2.2 RequestParams 增字段

`app/src/ai/agent/api.rs` 的 `RequestParams` 新增
`pub machine_memory: Option<MachineMemoryContext>`，在 `RequestParams::new`
中调用 `load_for_session`（已持有 `session_context`）。

### 2.3 渲染注入

`app/src/ai/agent_providers/chat_stream.rs`：仿照
`render_ssh_session_block`（L164）新增 `render_machine_memory_block`，在
`build_chat_request`（L1196 附近，ssh_block append 之后）追加：

```
<machine_memory machine_key="...">
  <fact>Accumulated notes from previous sessions on this same remote machine.
  They may be stale — verify before relying on them for destructive actions.</fact>
  <content>
  {content 或 "(no memory recorded for this machine yet)"}
  </content>
  <rules>
  - When you learn a durable fact about THIS machine (OS/services layout,
    deploy conventions, gotchas, non-standard paths), call `update_machine_memory`
    with the full revised memory document.
  - Never store credentials, tokens or private keys in machine memory.
  </rules>
</machine_memory>
```

属性值过 `xml_attr`、正文过 `xml_text`（两者已存在于 chat_stream.rs）。

### 2.4 验收标准

- [ ] 单元测试：`render_machine_memory_block` 空记忆/非空/None 三态快照
      （对齐 chat_stream.rs 现有测试风格）。
- [ ] legacy SSH 会话中发起 Agent 对话，请求日志（network log）里 system prompt
      末尾能看到 `<machine_memory>` 块；本地非 SSH 会话看不到。
- [ ] 关闭 `ssh_machine_memory_enabled` 后不注入。
- [ ] DB 读失败（如表不存在的降级场景）不影响正常发请求。

---

## Task 3 — 写路径 A：`update_machine_memory` 工具

BYOP 本地工具，**完全照抄 webfetch/websearch 的拦截模式**（不映射 protobuf
executor，chat_stream 在 `parse_incoming_tool_call` 之前按 name 拦截本地执行，
见 chat_stream.rs L4308 起的现有实现与 tools/mod.rs 头部注释）。

### 3.1 工具定义

`app/src/ai/agent_providers/tools/machine_memory.rs`：

- `TOOL_NAME = "update_machine_memory"`。
- 参数 schema：`{ "content": string }` — 修订后的**完整**记忆文档
  （模型已在 `<machine_memory>` 块里看到旧文，全量替换，无 append 模式）。
- `from_args` / `result_to_json` 照抄 webfetch 对这两个字段的处理方式
  （被拦截的工具不会真正走到 protobuf 转换）。
- 描述文件 `prompts/tool_descriptions/update_machine_memory.md`：说明用途、
  全量替换语义、16 000 字符上限、禁存凭据、"仅记录对未来会话有用的持久事实，
  不记录一次性操作流水"。

注册：`tools/mod.rs` 的 `REGISTRY` 追加，位置放在 webfetch/websearch 旁并加注释
（同为 BYOP 本地拦截工具）。

### 3.2 可用性 gating

`chat_stream.rs` 构建工具数组处（现有 web 工具 gating 在 L2678/L2737）：仅当
`params.machine_memory.is_some()` 时把该工具加入 tools；否则过滤掉。

### 3.3 本地执行

拦截点（L4308 起的分支）新增 `update_machine_memory` 分支：

1. 解析 args；`content` 按 char 截断到 16 000。
2. 从 `params.machine_memory` 取 `machine_key`；为 None 时返回错误 JSON
   `{"status":"error","message":"not in an ssh session with machine identity"}`。
3. `warp_ssh_manager::with_conn(|c| MachineMemoryRepository::upsert_content(...))`。
   注意拦截点在异步流里：与 webfetch 一致处理阻塞问题（with_conn 为快速小写入，
   若现场发现阻塞风险，用与 webfetch 相同的 spawn 手段隔离）。
4. 成功返回 `{"status":"ok","stored_chars":N}`。
5. **本轮请求内**后续注入内容不需要刷新（下一轮 `RequestParams::new` 自然读到新值）。

成功、参数错误、机器 key 缺失和数据库错误的结果 JSON 都必须包含
`"_byop_intercepted": true`。该 sentinel 与现有 todowrite/web 工具一致，由
controller 触发自动续轮，避免本地拦截工具没有 `AIAgentAction` 入队时对话停住。

carrier `ToolCall` 的 `tool` 为 None，现有转换层默认不生成 UI。允许在 app 侧增加
专用于机器记忆更新的可见 output message：由 `update_machine_memory` carrier 转换，
复用现有文本渲染显示结果中性的 `Updating machine memory`；不得为此新增 protobuf
executor variant。

### 3.4 验收标准

- [ ] SSH 会话中对 Agent 说"记住这台机器 nginx 装在 /opt/nginx"，模型调用工具，
      UI 出现该工具调用，`ssh_machine_memories` 表出现对应行。
- [ ] 下一次在同一机器新开会话，`<machine_memory>` 块含上述内容。
- [ ] 本地非 SSH 会话的请求 tools 数组中**无** `update_machine_memory`。
- [ ] 超长 content 截断不报错；机器 key 缺失时返回错误 JSON 且不崩。
- [ ] 成功与所有错误结果都带 `_byop_intercepted`，controller 能自动续轮；UI
      能看到机器记忆更新工具调用。
- [ ] 单元测试：args 解析、截断、gating、缺 key、sentinel 与 app 侧 carrier
      转换（对齐 web 工具现有测试）。

---

## Task 4 — 写路径 B：会话结束后台复盘

### 4.1 触发

`ModelEvent::ExitShell` 的 view 侧处理处（`app/src/terminal/view.rs` L10735
附近的 ModelEvent 分发区）：当退出的是 legacy SSH 会话，且本会话期间该终端的
Agent 对话有过至少一次完整交互（controller 可查询），调用
`machine_memory::review::spawn_session_review(...)`。

补充触发（防漏）：终端 view 关闭时若存在符合条件的 SSH 会话对话，同样触发一次。
用 `last_review_at` + 对话内容 hash 或"本会话已复盘"标记去重，避免同一会话
ExitShell 与 view 关闭双触发写两次。

### 4.2 复盘实现

`app/src/ai/machine_memory/review.rs`。**整体照抄 `start_title_generation`
的 spawn 模式**（`app/src/ai/blocklist/controller.rs` L3210：ctx.spawn +
后台 oneshot 调用 + 主线程回调）：

1. 收集输入：当前记忆（repository get）+ 会话摘要 digest。digest 从该终端的
   `AIConversation` 提取：每轮的用户 query、执行的命令 + 截断输出（每条 ≤500 字符）、
   Agent 的结论性文本；总量按 char 截断到 20 000。
2. oneshot 调用：`resolve_active_ai_oneshot`（`agent_providers/oneshot.rs`），
   `OneshotOptions { max_chars: Some(20_000), temperature: Some(0.2),
   response_format_json: true, allow_reasoning: false }`。
   system prompt 新文件 `prompts/tasks/machine_memory_review_system.md`：
   - 角色：合并旧记忆与本次会话事实，输出该机器的修订版记忆。
   - 输出 JSON：`{"changed": bool, "memory": "<markdown>"}`；无新知识时
     `changed=false`（此时不写库）。
   - 结构引导（不强制）：`## 系统画像` `## 服务与部署` `## 操作惯例` `## 踩坑记录`。
   - 硬性规则：≤16 000 字符；合并去重、旧事实被推翻时更新而非并存；
     **禁止**写入密码/token/私钥/一次性操作流水。
3. 解析响应：JSON 解析失败或 `changed=false` → 静默放弃（log debug）。
   成功 → `upsert_content` + `set_last_review_at`。
4. 失败策略：任何环节失败均静默放弃，**不重试**（下次会话还有机会）。
   全程不弹 UI。
5. gating：设置项（Task 2）关闭时不触发；`resolve_active_ai_oneshot` 返回
   None（无可用 BYOP 配置）时不触发。

### 4.3 验收标准

- [ ] SSH 会话中让 Agent 执行若干运维操作（不显式让它记忆），`exit` 退出远端，
      数秒内 `ssh_machine_memories` 该机器行的 content 出现合并后的复盘内容，
      `last_review_at` 更新。
- [ ] 同一会话不会被复盘两次（ExitShell + 关 tab 只写一次）。
- [ ] 无 Agent 交互的纯手工 SSH 会话退出：不发任何 LLM 请求。
- [ ] oneshot 返回非法 JSON：无写库、无 crash、无 UI 报错。
- [ ] 单元测试：digest 构建（截断、多轮拼接）、响应解析（合法/非法/changed=false）。

---

## Task 5 — 机器索引注入（意图直达，Phase 3）

在**本地非 SSH** 会话的 system prompt 注入已知机器索引，使"去 web-01 重启
nginx"能直接关联目标机器。

- `RequestParams` 增 `machine_index: Option<String>`：会话非 SSH 且设置开启时，
  `list_all()` 取全部记忆，每台渲染一行
  `- {machine_key}: {content 首个非空行，截断 120 字符}`，
  最多 30 台（按 updated_at 降序），总量 ≤3 000 字符。空表 → None。
- chat_stream 渲染为 `<known_ssh_machines>` 块，说明：用户提及某台机器时可
  据此定位；连接方式为让用户执行/建议 `ssh <machine_key 的 host 部分>`
  （Agent 不自动发起 SSH 连接——连接仍由用户或 SSH Manager 发起）。
- 验收：本地会话说"web-01 上面跑了什么"，Agent 能引用索引内容回答并建议连接；
  SSH 会话内不注入索引（避免与 `<machine_memory>` 冗余）。

---

## Task 6 — zap_sync 同步 + SSH Manager UI（Phase 3）

### 6.1 同步

`crates/warp_ssh_manager/src/sync_provider.rs`：把 `ssh_machine_memories`
并入现有 `section_key() = "ssh"` 的 collect/apply（随既有加密通道走）。
合并策略：按 `machine_key` 对齐，`updated_at` 新者胜（last-write-wins）。
需处理远端有/本地无、本地有/远端无、双方都有三种情况；不做逐字段 merge。

### 6.2 UI

`app/src/ssh_manager/server_view.rs`：服务器详情增"记忆"区块（与 notes 并列）：
只读展示 content（滚动）、显示 updated_at、提供"清空记忆"按钮（confirm 后
`delete`）。服务器条目与记忆的关联：用该 server 的 host/port 走
`resolve_machine_key`。i18n 三语言补 key（`warp.ftl`）。

- 验收：面板可见记忆、清空生效；两台设备通过 gist 同步后记忆一致，
  冲突时新 updated_at 胜出（单元测试覆盖 merge 三种情况）。

---

## 7. 通用工程约定（所有任务）

- 遵循 `WARP.md` / `AGENTS.md`：不跨层倒挂依赖；注释风格与所在文件一致
  （本 fork 大量中文注释，新代码注释用中文，注明 `Zap:` 前缀的惯例照旧）。
- 每个任务独立分支 + PR，PR 描述引用本 spec 对应章节。
- 提交前跑 `cargo check` 与所涉 crate 的 `cargo nextest run`
  （workspace 惯例 `--exclude command-signatures-v2`）。
- 不改动 `render_ssh_session_block` 现有行为，只做追加。
- 所有新增 prompt 文本（注入块、工具描述、复盘 system prompt）用英文
  （与 `prompts/` 目录现状一致）。

## 8. 总验收清单（验收人使用）

Phase 1 端到端：

1. 连接一台测试机（`ssh user@host`），让 Agent 排查一个服务问题；
   观察它调用 `update_machine_memory` 记录事实。
2. 退出并重新连接同一台机，新开 Agent 对话问"这台机器你了解什么"；
   Agent 应直接答出记忆内容，且请求日志证实注入来自 `<machine_memory>` 块。
3. 换端口/换用户名连接同一 host：命中同一份记忆（key 归一化生效）。
4. 本地会话：无记忆块、无该工具。
5. 关闭设置项后重复 1-2：完全无记忆行为。

Phase 2 追加：不显式要求记忆的运维会话，退出后记忆自动出现复盘增量；
纯手工会话无 LLM 调用。

Phase 3 追加：本地会话按机器名下达任务可被正确关联；双设备同步一致；
面板查看/清空可用。
