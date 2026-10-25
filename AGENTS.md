# AGENTS.md

> 本文件是给在本仓库中工作的 AI/自动化 agent 的导航文档。它汇总了仓库的整体架构、Cargo 工作区的主要 crate 职责、`app/` 主二进制下各子模块的边界,以及在做改动前必须遵守的工程约定。
>
---

## 1. 仓库总览

InfiniShell 是基于 Warp 的本地优先 **agentic 终端 / 开发环境**，以 Rust 和 WarpUI 为主，提供终端、BYOP AI、CLI Agent、SSH/SFTP、项目管理、代码评审、Notebook 和设置。上游兼容模块仍保留部分 Warp 命名；是否启用能力以 OSS 入口、运行时 feature flag 和当前实现为准。

顶层目录:

| 目录 | 作用 |
|------|------|
| `app/` | 主二进制 crate(`warp`),装配所有子系统、UI、数据库迁移、平台粘合层 |
| `crates/` | 按职责拆分的库及 TUI crate，成员由根 `Cargo.toml` 的 `crates/*` 定义 |
| `crates/command-signatures-v2/` | 命令签名工作区成员，在常规 workspace nextest 命令中显式排除 |
| `script/` | 跨平台 bootstrap、构建、presubmit 脚本 |
| `resources/` | 字体、图标、shell 集成脚本、shaders 等运行时资源 |
| `docker/` | 容器化构建相关 |
| `specs/` | 产品/技术 spec 文档 |
| `.agents/skills`, `.claude/skills` | agent 工作流的 skill 描述(创建 PR、修复错误、特性灰度等) |
| `.warp/`, `.config/`, `.cargo/`, `.vscode/` | 各类工具配置 |

构建系统:Cargo workspace,`resolver = "2"`,`default-members` 故意收敛到经常需要编译/测试的子集(见 `Cargo.toml`)。`serve-wasm` 与 `integration` 默认不在 `default-members` 内。

许可证拆分:
- `crates/warpui` 与 `crates/warpui_core` → MIT
- 其余 → AGPL-3.0-only

---

## 2. 顶层架构分层

从底向上大致是 4 层。在新增代码或定位 bug 时,先确定改动属于哪一层,**不要跨层倒挂依赖**。

```
app/  (主二进制:装配、入口、平台粘合、持久化迁移、UI 视图根)
  ↑
产品域 crate:ai / computer_use / vim / onboarding /
              warp_completer / languages / infinishell_projects / infinishell_sftp …
  ↑
框架 crate:warpui / warpui_core / warpui_extras / editor /
            ui_components / sum_tree / syntax_tree
  ↑
基础设施 crate:warp_core / warp_util / http_client /
                websocket / ipc / jsonrpc / persistence / mcp /
                managed_secrets / virtual_fs / watcher / asset_cache …
```

关键架构模式:

1. **Entity-Handle 系统**:`App` 全局拥有所有 view/model entity,View 之间通过 `ViewHandle<T>` 引用,而不是直接拥有。
2. **Element / Action**:UI 由声明式 Element 树 + Action 事件系统组成(Flutter 风格)。
3. **跨平台**:macOS / Windows / Linux 原生实现 + WASM 目标;平台代码用 `#[cfg(...)]` 隔离。
4. **AI 集成**:Agent Mode、BYOP 与 CLI Agent 集成集中在 `app/src/ai` 与 `crates/ai`；TUI 入口位于 `crates/warp_tui`。
5. **持久化与同步**:项目、SSH 和可选同步分别见 `crates/infinishell_projects`、`crates/warp_ssh_manager`、`crates/infinishell_sync`。`app/src/drive` 等上游兼容模块的存在不表示 OSS 启用了 Warp 云服务。
6. **Feature Flag**:运行时灰度优先于 `#[cfg]`,枚举与通道列表定义在 `crates/warp_features/src/lib.rs`，`warp_core::features` 重导出。

---

## 3. `crates/` 一览

下表按主题分组列出主要 crate。完整成员以根 `Cargo.toml` 和 `crates/*/Cargo.toml` 为准；实现细节见对应 `src/lib.rs` 或二进制入口。

### 3.1 UI 框架 / 视图层

| Crate | 职责 |
|-------|------|
| `warpui_core` | WarpUI 框架核心(MIT):`App` / `Entity` / `ViewHandle` / `AppContext` 等基础设施 |
| `warpui` | WarpUI 上层组件、Element 树、布局、渲染管线(MIT) |
| `warpui_extras` | WarpUI 的可选扩展件,默认不启用全部 features |
| `ui_components` | 跨视图复用的高层组件库(按钮、输入、列表、模态等) |
| `editor` (`warp_editor`) | 文本编辑器:缓冲区、选择、光标、键映射、撤销栈 |
| `sum_tree` | 持久化平衡 B-树,编辑器 / Notebook / 大列表的核心数据结构 |
| `syntax_tree` | Tree-sitter 封装与语法高亮支持 |
| `markdown_parser` | Markdown 解析(用于 AI 消息、文档视图、Notebook 等) |
| `vim` | Vim 模式键绑定与操作语义 |
| `voice_input` | 语音输入支持 |
| `warp_tui` | 无图形窗口的 InfiniShell TUI 前端，默认二进制为 `infinishell-tui` |
| `ipynb_parser` | 将 Jupyter Notebook 内容解析为只读富文本展示 |

### 3.2 终端

| Crate | 职责 |
|-------|------|
| `warp_terminal` | 终端模拟核心:PTY 管理、ANSI/VT 解析、grid、滚动、shell 集成钩子 |
| `input_classifier` | 终端输入意图分类(纯命令 / 自然语言 / AI Prompt) |
| `natural_language_detection` | 自然语言识别(配合 `input_classifier`) |

### 3.3 AI / Agent

| Crate | 职责 |
|-------|------|
| `ai` | AI 模型客户端、Prompt 编排、Agent 协议、工具调用框架 |
| `computer_use` | "Computer Use" 工具能力(截屏、点击、键入等)的 Rust 端实现 |
| `command-signatures-v2` | 命令签名 v2 元数据工作区成员；常规测试命令通过 `--exclude` 排除 |
| `onboarding` | 新用户引导流程数据/状态 |
| `warp_multi_agent_client` | 保留多 Agent 协议辅助类型与纯逻辑，当前不发起云端 RPC |

### 3.4 网络 / 协议 / IPC

| Crate | 职责 |
|-------|------|
| `http_client` | 工作区统一 HTTP 客户端封装 |
| `http_server` | 内嵌 HTTP server(本地 RPC、登录回调等) |
| `websocket` | 原生与 WASM 共用的 WebSocket 抽象,适配 `graphql_ws_client` |
| `ipc` | 通用类型化 IPC 请求/响应协议(进程间) |
| `jsonrpc` | JSON-RPC 实现 |
| `mcp` | MCP 运行时、OAuth 和协议集成 |
| `local_control` | 本地应用控制协议、发现、认证和客户端类型 |
| `remote_server` | 远端 sshd 模式下的服务端逻辑 |
| `serve-wasm` | 把 WASM 构建产物 host 出来的辅助 server(默认不参与编译) |

### 3.5 持久化 / 文件 / 资源

| Crate | 职责 |
|-------|------|
| `persistence` | Diesel + SQLite 持久层基础；迁移在 `crates/persistence/migrations/`，schema 在 `crates/persistence/src/schema.rs` |
| `warp_files` | Drive 文件、Workflow、Notebook 等可同步文件对象 |
| `virtual_fs` | 抽象文件系统(测试用 mock 与生产用真实 FS 同接口) |
| `repo_metadata` | 仓库元数据:文件树构建、`.gitignore` 处理、文件系统监听 |
| `watcher` | 文件系统监视器(对 `notify` 的封装) |
| `asset_cache` | 资源磁盘/内存缓存 |
| `asset_macro` | `bundled!` / `theme!` 等资源引用宏 |
| `warp_assets` | 嵌入资源与 WarpUI 资源提供接口 |
| `cloud_objects` / `cloud_object_models` | 上游云对象的公共元数据、类型与具体对象模型 |
| `infinishell_projects` | 项目及项目与 SSH 服务器关联的持久化数据层 |
| `infinishell_sync` | 可选同步的通用基础设施 |
| `infinishell_sftp` | SFTP 会话与远程文件操作 |
| `warp_ssh_manager` | SSH 服务器树、凭据存储与命令构造的数据层 |
| `managed_secrets` / `managed_secrets_wasm` | Keychain / DPAPI / Linux Keyring 抽象 + WASM 代理 |

### 3.6 配置 / 设置

| Crate | 职责 |
|-------|------|
| `settings` | 设置存储与变更分发 |
| `settings_value` | `SettingsValue` trait:控制 TOML 序列化语义 |
| `settings_value_derive` | `#[derive(SettingsValue)]` 过程宏(枚举变体转 snake_case 等) |
| `warp_features` | FeatureFlag 枚举、运行时 API 和各通道功能列表 |
| `channel_versions` | 发布通道版本与版本比较 |
| `warp_channel_config` | GUI/TUI 共用的通道配置加载 |

### 3.7 命令 / 补全 / 语言

| Crate | 职责 |
|-------|------|
| `command` | 跨平台进程派生的安全封装,**特别处理 Windows 的 `no_window` 标志**;新派生子进程一律走这里 |
| `warp_completer` | 补全引擎(支持 `--features v2`) |
| `languages` | 语言/扩展名/Tree-sitter grammar 注册 |
| `warp_ripgrep` | 给 `warp_cli` 用的 ripgrep 薄封装 |
| `warp_cli` | 二进制内的 CLI 子命令解析(`warp <subcmd>`) |
| `fuzzy_match` | 模糊匹配 + glob 风格通配,用于路径搜索与命令面板 |

### 3.8 平台 / 系统服务

| Crate | 职责 |
|-------|------|
| `app-installation-detection` | 检测系统中已安装的 app(用于 launcher 联动) |
| `prevent_sleep` | 抑制休眠(长任务/AI Agent 期间) |
| `isolation_platform` | 在 Docker / GitHub Actions 等沙箱中运行的兼容层 |
| `node_runtime` | 自动安装/管理 Node.js 与 npm(macOS/Linux/Windows × 多架构) |
| `warp_js` | 在 Rust 侧操作 JavaScript 值/函数的助手抽象 |

### 3.9 通用工具 / 通信

| Crate | 职责 |
|-------|------|
| `warp_core` | 工作区内最底层的"core":平台抽象，并重导出 `warp_features` 的功能开关 API |
| `warp_util` | 跨多个 crate 复用的通用工具函数 |
| `warp_logging` | 日志配置统一入口 |
| `warp_errors` | 错误报告宏与错误类型扩展 |
| `warp_search_core` | 共享搜索数据源、候选项和结果组合逻辑 |
| `build_cache` | 沙箱 Agent 的持久化构建缓存管理 |
| `simple_logger` | 给 `remote_server` 等 stderr-only 进程用的简易异步文件日志 |
| `warp_web_event_bus` | Web 端事件总线(给嵌入的 web view) |
| `field_mask` | gRPC/Proto 风格 FieldMask 工具 |
| `string-offset` | 偏移量基础类型(byte/char/utf16) |
| `handlebars` | Handlebars 模板引擎封装 |
| `integration` | 集成测试框架,只用于测试 |

> 命名小坑:`crates/editor` 的 package 名是 `warp_editor`;`crates/isolation_platform` 是 `warp_isolation_platform`;`crates/managed_secrets` 是 `warp_managed_secrets`;`crates/virtual_fs` 是 `virtual-fs`(短横线);`crates/string-offset` 是 `string-offset`(短横线)。

---

## 4. `app/` 子模块导航

以下按主题列出 `app/src/` 的主要入口。新增模块和文件名以当前源码为准，不以静态文件数量判断模块边界。

### 4.1 启动 / 装配 / 全局
- `bin/` — 多个二进制入口(主程序、附带工具)。
- `lib.rs` / `app_state.rs` / `app_state_tests.rs` — 应用状态根。
- `app_menus.rs`, `app_services/`
- `appearance.rs`, `gpu_state.rs`, `font_fallback.rs`, `global_resource_handles.rs`
- `dynamic_libraries.rs`, `alloc.rs`, `tracing.rs`, `profiling.rs`
- `crash_recovery.rs`, `crash_reporting/`
- `features.rs` — `app/` 内对 `warp_core::FeatureFlag` 的消费;新增 flag 时通常需要在两处都接好。
- `channel.rs`, `autoupdate/`

### 4.2 终端
- `terminal/` — 主体:shell 进程、PTY、grid、blocks、shell 集成、命令执行、I/O 流水线。
- `default_terminal/` — 默认终端启动逻辑。
- `shell_indicator.rs`, `prefix.rs`(命令前缀解析),`vim_registers.rs`

### 4.3 AI / Agent
- `ai/` — 包含 Agent UI、对话模型、Agent 管理、工具/MCP、Cloud Agent、Plan/Diff 视图、artifacts、blocklist、execution profiles 等。改动前先在该目录内搜索具体子主题(`agent_*`, `conversation_*`, `cloud_agent_*`, `mcp`, `tool_*`)。
- `ai_assistant/` — 旧版 AI 辅助入口/适配。
- `chip_configurator/`, `context_chips/` — Agent 上下文 chip 选择/构造。
- `coding_entrypoints/`, `coding_panel_enablement_state.rs`
- `prompt/`, `tips/`, `voice/`, `completer/`

### 4.4 编辑器 / 代码 / Review
- `editor/` — 主编辑器集成。
- `code/` — 代码视图、diff、navigation。
- `code_review/` — Code Review 流。
- `notebooks/`, `workflows/`

### 4.5 搜索
- `search/` — 多目标搜索(文件、命令、Agent 历史等)。
- `search_bar.rs`

### 4.6 服务端通信 / Drive / 同步
- `server/` — 服务端通信及上游兼容适配，实际启用范围以当前 OSS 入口为准。
- `drive/` — 上游 Drive 兼容入口，不代表 OSS 已启用 Warp 云同步。
- `cloud_object/` — 云对象抽象层(workflow、notebook 等)。
- `remote_server/` — 客户端侧连接 remote 模式 sshd 的 glue。

### 4.7 设置 / 用户配置 / 主题 / Onboarding
- `settings/`, `settings_view/`
- `user_config/`, `themes/`, `appearance.rs`
- `experiments/`, `tab_configs/`, `launch_configs/`
- `tips/`, `banner/`, `quit_warning/`, `wasm_nux_dialog.rs`

### 4.8 上游兼容边界
- `auth/`、`pricing/` — 保留的上游兼容代码；OSS 的实际账户与云功能边界见当前入口和 feature flag。

### 4.9 持久化
- `persistence/` — 应用持久化集成，并重导出 `persistence` crate 的模型和 schema。
- 迁移文件在 `crates/persistence/migrations/`；生成的 schema 及配置分别见 `crates/persistence/src/schema.rs`、根 `diesel.toml`。

### 4.10 平台 / 系统集成
- `platform/`, `system/` / `system.rs`
- `login_item/`, `antivirus/`, `network.rs`
- `external_secrets/`, `env_vars/`
- `keyboard.rs`, `safe_triangle.rs` / `safe_triangle_tests.rs`(菜单悬停安全三角)

### 4.11 视图根 / 面板 / 通用 UI
- `root_view.rs` / `root_view_tests.rs`
- `pane_group/` — 分屏分块布局。
- `tab.rs`, `command_palette.rs`, `modal.rs`, `menu.rs`
- `palette.rs`, `notification.rs`, `resource_center/`
- `view_components/`, `ui_components/`
- `workspace/`, `workspaces/`, `voltron.rs`(多窗口/多 workspace 协调)
- `session_management.rs`, `undo_close/`, `word_block_editor.rs`
- `suggestions/`, `input_suggestions.rs`
- `plugin/` — 插件系统接入。
- `uri/` — `infinishell://` 及兼容 URI 的路由。
- `debug_dump.rs`, `interval_timer.rs`, `throttle.rs`
- `linear.rs`, `resource_limits.rs`, `warp_managed_paths_watcher.rs`
- `preview_config_migration.rs` / `preview_config_migration_tests.rs`
- `window_settings.rs`, `projects.rs`

### 4.12 InfiniShell 与 TUI 集成
- `ssh_manager/`、`sftp_manager/`、`project_manager/` — SSH/SFTP 和项目管理视图。
- `skill_manager/`、`storage_cleanup/`、`local_control/` — Skill 管理、存储清理、本地控制集成。
- `tui/`、`tui_export/` — 应用层 TUI 适配；独立前端见 `crates/warp_tui`。

### 4.13 测试基建
- `integration_testing/` — 端到端集成测试支撑。
- `test_util/` — 单元测试公共 util。

---

## 5. 工程纪律(给 Agent 的强约束)

> 以下为项目自定义规则;本文件对 agent 的验证要求以 `cargo check` 为准。

### 5.1 必读约定
- **注释/回复一律使用简体中文**(用户规则)。
- **新增 Git 提交的标题与正文统一使用英文**；上述中文要求仅适用于代码注释和交流回复，不延伸到提交信息。
- 提交标题统一使用 `type(scope): description`，`scope` 可省略；`type` 使用 `feat`、`fix`、`docs`、`refactor`、`perf`、`test`、`build`、`ci`、`chore` 或 `revert`。`scope` 使用稳定的模块或子系统名称（如 `cli-agent`、`terminal`），不使用临时验收编号。
- `description` 使用简洁的英文祈使句，概括实际变更，不以句号结尾；标题不足以解释原因时，正文与标题空一行，说明问题、动机或取舍。例如：`fix(cli-agent): preserve PowerShell 5.1 script encoding`。该约定适用于后续新增提交，不要求改写历史提交。
- 在 git 索引内的搜索/grep 使用 `fff` 工具或 `rg -n "<关键词>" <路径>`;`read_file` 仅用于图片/二进制。
- 提 PR / 推新 commit 之前必须通过 `cargo check`;新增或变动用户功能还必须满足 [5.9 中英文同步](#59-中英文同步强制门禁) 的额外门禁。
- 改动需精准:**每一行修改都能溯源到用户请求**,不要顺手"改进"无关代码、注释、格式。
- 简洁优先:不要为单点使用引入抽象、配置、错误处理、多余特性。
- 多解释方案、暴露不确定性,而不是默默替用户做选择。
- worktree路径：.worktrees/<worktree_name>/

### 5.2 Rust 风格
- 闭包参数不要写多余类型注解。
- 顶部统一 `use`,不要写一长串路径限定;`#[cfg]` 分支内例外。
- 上下文参数命名为 `ctx` 且放在最后;若同时有闭包参数,闭包放最后。
- 未使用参数**直接删除**而不是加 `_` 前缀,同步更新调用点。
- `println!` / `format!` 等宏使用内联格式参数(`"{x}"` 而不是 `"{}", x`)以满足 `uninlined_format_args`。
- `match` 语句**禁止使用 `_` 通配**(除非确实需要),保持穷尽匹配。
- 不要因为不相关的修改去删/改既有注释。

### 5.3 终端模型锁(高优先级!)
- 调用 `TerminalModel::lock()` 极易死锁(macOS 上表现为 UI 卡死/沙滩球)。
- 新增 `model.lock()` 前必须确认调用栈中没有上层已经持锁;尽量把已锁定的引用沿调用栈往下传,而不是再次加锁。
- 持锁范围最小化,持锁时不要调用可能再次加锁的函数。

### 5.4 Feature Flag
- 新增:在 `crates/warp_features/src/lib.rs` 的 `FeatureFlag` 枚举里加 variant;按需把它加入 `DOGFOOD_FLAGS` / `PREVIEW_FLAGS` / `RELEASE_FLAGS`。
- 使用:**优先**用运行时 `FeatureFlag::Xxx.is_enabled()`,而不是 `#[cfg(...)]`;只有当无 `cfg` 就无法编译(平台/可选依赖)时才用 `cfg`。
- 包裹整段产品功能,而非每个调用点都加;上线稳定后**清理 flag 与死分支**。
- UI 入口要与代码路径用同一个 flag。

### 5.5 数据库
- ORM:Diesel + SQLite。
- 新增/改 schema 必须走 migration:在 `crates/persistence/migrations/` 加新目录(`up.sql` / `down.sql`),不要手改 `crates/persistence/src/schema.rs`(由 `diesel print-schema` 生成)。

### 5.6 测试
- 用 `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`。
- 单元测试放到 `${文件名}_tests.rs` 或 `mod_test.rs`,在原文件末尾用:

  ```rust
  #[cfg(test)]
  #[path = "filename_tests.rs"]
  mod tests;
  ```

- 集成测试用 `crates/integration` 的框架,样例在 `app/src/integration_testing/`。

### 5.7 跨进程命令
- 不要直接 `std::process::Command::new(...)`(尤其在 Windows 上会弹窗),统一走 `crates/command`。

### 5.8 子代理 / 多代理
- 大任务拆分为**写入域不重叠**的子任务并行下发;信息收集类任务可以并行。
- 简单任务直接做,不要过度拆分。

### 5.9 中英文同步(强制门禁)
- **任何新增或变动的用户功能都必须完成本地化审计**,不是仅在新增消息键时才检查。若功能改变了既有界面语义,必须同步复核并修改原有英文与简体中文文案;若确认没有用户可见文案变化,交付说明中应明确记录“无需本地化变更”。
- 用户可见文案包括 GUI/TUI 中的标题、菜单、按钮、提示、占位符、工具提示、空状态、加载/错误信息、通知和无障碍文本。日志、遥测稳定值、协议字段、配置键、命令名与测试夹具等例外边界见 `app/i18n/PROGRESS.md`。
- 英文是源语言。先在 `app/i18n/en/warp.ftl` 或 `warp_tui.ftl` 编写自然英文,再在对应的 `app/i18n/zh-CN/` 文件中添加或更新同一消息键;两种语言的 `$variable` 必须完全一致。
- 调用方使用字面量 `crate::t!("message-id")`;禁止新增硬编码界面文案,禁止依赖英文回退代替简体中文翻译,禁止为调整语序而拼接本地化字符串。
- 提交前必须运行 `cargo test -p warp --lib i18n::tests` 和 `cargo check -p warp`;涉及 TUI 资源时还应运行对应 TUI 测试。任何门禁失败都不得视为功能完成。
- 文案可能影响布局时,必须分别在英文和简体中文界面检查截断、换行和控件尺寸;检查结果应在交付说明中记录。

### 5.10 本机测试与临时目录
- 在本机执行构建或测试前，必须先读取未纳入 Git 的 `.local/README.md` 及其引用的机器配置。若当前 worktree 没有该文件，使用 `git worktree list --porcelain` 定位主检出，再读取其 `.local/README.md`。未找到适用机器配置时不得猜测目录或放松安全门禁，应先取得必要的机器配置。
- 按机器配置选择安全测试根，每轮创建短名称子目录，仅为该轮命令设置 `TMPDIR`，不改 `HOME` / `CODEX_HOME`。核实真实路径、owner、device/inode、设备约束及祖先权限，不能靠修改既有目录权限绕过检查。
- 每轮记录状态、目录/进程身份、归档位置和默认 `false` 的 `cleanup_ready`。仅在命令、后代及关联服务均退出、打开文件已释放、小证据已归档后，才可标记可清理并删除。个人定时补查计划保存在本机配置。
- 活动、归属不明、挂载点、符号链接或清理证据不足的目录保留；不删除源码、`target`、缓存、凭据、外置归档或唯一证据。删除前重新核对记录身份，不追随链接、不跨设备、不按宽泛进程名结束进程。
- 本机专用功能开关和已验证的测试规避措施按机器配置执行；不将个人路径或开关硬编码进产品及其他平台 runner，不为通过测试启用 `release_bundle` 或改动产品超时。通用约定见 [本机测试存储约定](docs/local-test-storage.zh-CN.md)。

---

## 6. 常用入口速查

| 想做的事 | 起点 |
|---------|------|
| 改终端 grid / shell 集成 | `crates/warp_terminal/src/`,联动 `app/src/terminal/` |
| 改 Agent UI / 对话 | `app/src/ai/` 内按 `agent_*` / `conversation_*` 分主题 grep |
| 改命令补全 | `crates/warp_completer/`(注意 `--features v2`) |
| 改 AI 模型 / 工具调用协议 | `crates/ai/` |
| 加新设置项 | `crates/settings_value*`、`crates/settings`,UI 在 `app/src/settings_view/` |
| 加 Feature Flag | `crates/warp_features/src/lib.rs` + `app/src/features.rs` / TUI 消费点 |
| 改云端同步对象 | `crates/warp_files` + `app/src/drive/` + `app/src/cloud_object/` |
| 改持久化结构 | `crates/persistence/migrations/` 加迁移 + `crates/persistence` |
| 加新二进制工具 | `app/src/bin/` |
| 平台特定代码 | 用 `#[cfg(target_os = "...")]`,UI 平台胶水在 `app/src/platform/` |
| Vim 模式 | `crates/vim` + `app/src/vim_registers.rs` |
| Notebook / Workflow | `app/src/notebooks/`、`app/src/workflows/`、`crates/warp_files` |
| 跨平台进程派生 | `crates/command` |
| 文件搜索 / 监听 | `crates/repo_metadata`、`crates/watcher`、`crates/warp_ripgrep` |

---

## 7. 修改前的检查清单

在动键盘改代码前,自问一次:

1. 这件事属于哪一层 / 哪个 crate / 哪个 `app/src/<子模块>`?改动是否会跨越层界?
2. 是否需要新增依赖?如已存在的 workspace 依赖能复用,优先复用 `Cargo.toml` `[workspace.dependencies]`。
3. 这是产品功能吗?是否需要 Feature Flag 包起来?
4. 涉及终端模型?当前调用栈是否已经持有 `TerminalModel` 锁?
5. 涉及子进程?是否走了 `crates/command`?
6. 涉及持久化?是否需要 migration?
7. 新增或变动功能是否已完成中英文审计,同步 Fluent 消息与变量,并通过 i18n 门禁及必要的双语布局检查?
8. 已经写了对应的 `${file}_tests.rs`?
9. `cargo check` 是否绿?
10. 改动的每一行能否一一对应到用户请求?顺手做的"小重构"是否应该回滚?

把上面 10 条都过一遍,再交付。
