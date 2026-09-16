# Codex 0.147 缓存修补回滚与持久来源迁移

2026-09-16 的 macOS 原生隔离探测确认：仅修改已安装缓存不可靠。首次启动 Codex 的 marketplace 后台刷新会重新从 Git 来源复制插件，即使 Git ref 已固定、插件版本没有变化，四个通知修补文件仍会恢复为上游内容。同 ID 的完整受控本地来源能保留修补。这里记录原生事实和待实现生产约束；探测成功不等于产品安装器已经修复。

## 固定来源和根因

Codex 版本为 0.147.0，官方源码提交 `be6e8eac029b183056b7e4402879f15d2c85f61b`；codex-warp 为 0.4.0，来源提交 `31ce59d9011cfb1d78f265649a228dac5de58d76`。本机可执行文件摘要为 `19c4f144c5226a9f17c58e6f0fa854843b0f77a6eb420f40e2745a12f10f5d37`。

1. 原生 [marketplace add](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core-plugins/src/marketplace_add/metadata.rs#L32) 初建配置时写入 `last_revision: None`。
2. [启动任务](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core-plugins/src/manager.rs#L2121) 启动 `plugins-marketplace-auto-upgrade`。升级器只有在配置修订、安装元数据与真实来源全部一致时才跳过；[固定 SHA ref 直接充当 remote revision](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core-plugins/src/marketplace_upgrade/git.rs#L8)，仍无法补足缺失的 `last_revision`。
3. [升级判断](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core-plugins/src/marketplace_upgrade.rs#L216) 因此执行首次刷新。随后 [manager](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core-plugins/src/manager.rs#L2251) 对升级来源调用 `ForceReinstall`；[loader](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core-plugins/src/loader.rs#L664) 只在 `IfVersionChanged` 模式跳过同版本。
4. [store](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core-plugins/src/store.rs#L363) 从来源复制并原子替换缓存。来源的四文件尚为上游版本，所以不是用户授权或 hooks 执行改变了文件。

不能通过填写 `last_revision`、启动时重新修补、修改 `.tmp/marketplaces` 快照、自动授信旧哈希来规避这个来源问题。新的 hooks 仍须用户在原生 `/hooks` 中明确授权。

## 真实对照

新增 [独立 probe](../../script/cli-agent-parity/probe_codex_plugin_cache_refresh.py) 使用私有 HOME/CODEX_HOME，不继承认证变量。14 次 CLI 命令和七个 app-server 实例只操作插件、配置与 `hooks/list`，没有建立 thread/turn，也没有模型请求。

| 操作 | 真实结果 |
| --- | --- |
| 固定 Git 来源安装，然后仅修缓存 | 初始十文件匹配 rev3；启动 app-server 并读取 hooks 后四文件回到上游，配置补入固定 `last_revision` |
| 完整来源复制 | Git HEAD 与固定提交一致；全部 36 个跟踪文件校验，复制后只替换 `plugins/warp` 的四文件；orchestration、索引及其他文件摘要不变 |
| 已安装 orchestration | 原生安装，再原生配置接口禁用。后续迁移、重装、重启均保持其完整缓存及 `enabled=false` |
| 同名不同来源直接 add | 原生返回 1，配置字节不变；原生要求先 remove marketplace |
| 同 ID 迁移 | 原生 remove marketplace 不移除插件缓存/启用配置；add 完整本地来源，再 add `warp@codex-warp`，ID 不变，另一个 marketplace 配置保持 |
| 本地来源升级与重装 | 原生 upgrade 不选择本地来源；主动 native add 强制重装及两次重启均保留修补；五个 hooks 均 untrusted |
| 显式禁用 | 原生配置接口禁用 warp，重启无 hook，配置逐字不变 |
| remove 后 add 缺失来源 | 原生确实留下缺失 marketplace 的中间状态，必须显式恢复；不是原子事务 |
| 禁用状态来源恢复 | 原生 add 恢复原本地来源，不执行会重新启用插件的 `plugin add`；两个插件继续禁用，两个原始缓存不变 |

证据为 [完整脱敏原生报告](fixtures/codex-plugin-cache-refresh-0.147.0-macos.json)，只把本次临时根替换为 `<isolated-root>`。报告中的 `product_installer_fixed=false`、`native_pty_verified=false` 保持不变。GUI 原始故障另见 [缓存回滚现场](validation/macos-gui-pty-codex-patch-reverted.json)。

复验需要 Python 3.11+、Git、固定 Codex 0.147.0、能访问固定 Git 来源的网络。输出必须位于源树外：

```sh
python3 -B script/cli-agent-parity/probe_codex_plugin_cache_refresh.py --codex-executable /opt/homebrew/bin/codex --output /tmp/codex-cache-refresh.json
```

## 推荐生产迁移及失败恢复

产品保持 `warp@codex-warp` 和 `orchestration@codex-warp`，将来源切换为 CLI HOME 下持久、不可变版本目录中的完整已核验 snapshot。36 文件及许可原样随附，只有四个明确列出的通知文件可不同；清单记录固定上游提交、每文件原始/实际摘要、模式位、补丁版本和 CLI 契约。不能把任意本地路径视作本应用来源。

1. 预检实际 CLI/Bash/jq、显式禁用、配置可读性、原有缓存全树及来源。只迁移固定官方 Git 来源或本应用路径下清单完全一致的来源。用户 local override、未知来源、改过的缓存/来源、未知版本均拒绝，保留原文件。
2. 在独立临时目录准备并校验全部来源，再原子发布到受控版本目录；不先删除旧来源。现有 orchestration 缓存和配置也进入原始状态记录。
3. 记录配置摘要、受影响 marketplace/plugin 键、缓存摘要及迁移阶段。生产实现先在私有暂存 HOME 用原生命令注册最终持久来源路径及安装，再迁入验证后的缓存和限定字段；真实配置不经历 remove→add 空缺。暂存文件内不能留下对暂存 HOME 的来源引用，缓存必须与随附树逐字一致。每阶段验证仅预期键发生变化。真实 HOME 中显式禁用不得调用会重新启用的 `plugin add`。安装确认必须同时包含原生暂存注册状态、真实配置、持久来源和完整缓存摘要。
4. 中途失败只恢复本操作已确认且当前仍匹配的键/缓存。不能整份覆盖旧 config；无关并发更新必须保留。目标键或缓存出现未知变化时停止自动回退，保留旧来源与诊断，准确报告未完成，禁止返回成功。重试从真实状态重新预检，不把阶段日志当原生成功。
5. 来源切换后在独立验证中重新读取原生 hooks；授权仍由用户 `/hooks` 处理。必须在新原生进程验证，而不能借用旧进程已缓存的列表。产品安装函数本身的校验覆盖配置、来源及缓存，并不冒充新 PTY 生命周期验收。
6. orchestration 的安装/更新应识别同一受控来源、验证完整清单，不再走 Git 自动升级覆盖它。未来插件升级由应用显式交付新的固定完整来源版本，再按相同预检/切换流程迁移。原始 local override 继续属于用户，不自动接管；其他 marketplace 的原生升级策略不变。

SSH 手动包必须携带相同完整来源和迁移规则，不能继续把仅修缓存作为 Codex 默认成功路径。Windows 原生通知尚未通过验证，安装 gate 继续关闭。另起 marketplace 虽可避免切换既有来源，却会改变插件 ID 并可能同时加载两套 hooks，对当前兼容目标更复杂，因此不采用。

## 实现与并发边界

`codex_source.rs` 与导出包 `codex_persistent_source.py` 已按上述暂存方案接入。Unix 下完整来源还核对受测模式位；Windows 不把 Unix 模式位当作已验证原生权限语义。事务记录在实际步骤后更新 prepared/cache_written/configuration_written/verified 或 rolled_back/needs_review，记录只作诊断，不能授权重放。目标比较明确区分缺失项与空表，并比较带类型的 TOML 值；原生序列化新增空格不应被误报成配置漂移。合法普通表和内联表均保留支持，父表或目标启用字段类型非法时预检返回错误，不进入索引写入。GUI 配置写入仅修改 `marketplaces.codex-warp` 和目标插件的 `enabled` 字段，重新读取最新文档并保留无关修改和注释。标准库手动包由原生 CLI 在私有副本上生成完整 TOML，核对只有相同限定字段发生语义变化；真实提交要求整份输入仍相同，其他并发更新会拒绝提交。恢复时同样必须匹配本操作写入后的确切字节，不能整份盲覆旧配置。

两条路径都使用“读后比较 + 原子 rename”，并非操作系统提供的原子 compare-and-swap。其他不协作进程在最后比较与 rename 之间写配置，仍存在极窄竞态；不宣称完全消除跨进程写入窗口。固定官方 [config manager](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/app-server/src/config_manager_service.rs#L276) 的 `config/batchWrite expectedVersion` 也只在加载 user layer 时检查，随后 [ConfigEditsBuilder](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/config/edit.rs#L727) 重新读取并原子写文件，没有可复用的跨进程锁或真正 CAS，因此没有引入额外 app-server 平台去声称解决该问题。

原生 Git 后台升级在最终写入前另有 [ensure_configured_git_marketplace_unchanged](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core-plugins/src/marketplace_upgrade.rs#L293)，已经变成本地来源时会拒绝继续写入。该检查同样不是全局文件锁。

2026-09-16 已在新的私有 HOME 使用实际 Codex 0.147.0 验证手动产品 helper 的安装→check→更新→新 app-server `hooks/list`，来源与缓存一致，原有 `hooks.state` 值保持。该记录位于 `/tmp/infinishell-codex-manual-persistent-source.json`，不等于 GUI Rust 安装器已运行。十项独立 Python 真实文件事务回归与原有十三项通知测试通过；Rust 十二项回归待修正后统一 Cargo 门禁。

追加的[手动产品迁移原生证据](fixtures/codex-persistent-source-product-python-0.147.0-macos.json)已覆盖真正 Git marketplace 安装→已有 orchestration 禁用→手动产品 helper 迁移→新原生进程加载→warp 禁用→产品更新拒绝。原始 Git snapshot、orchestration 全树和禁用字段、用户 hooks trust 均保持；拒绝更新时配置逐字及缓存不变。它调用实际手动产品 helper，不覆盖 Rust GUI 安装器。

手动步骤已改为导出包命令并同步中英文；因导出包当前目录未知，按钮不自动执行该相对命令。产品 GUI 重启、真实新 PTY hooks 执行以及各平台同提交验证仍须单独验收。
