# Grok 官方自动升级接口与真实无模型验证

观察日期：2026-09-19；末次收尾记录为 10:16:57 UTC。本文只新增接口与隔离升级证据，不修改既有 source41 失败记录，也不代表 Grok 最新版已完成托管任务验收。

## 1. 结论与来源

当前官方 **stable 为 1.0.34，alpha 为 1.0.38**。此结论来自本次直接读取官方通道、真实 `update --check --json` 和官方更新日志，未沿用旧 GUI 提示。消费版和开发版应共享同一稳定通道发现逻辑；不能将 alpha 的较大版本号当成稳定版。

| 官方入口 | 本次结果 | 原始响应 SHA-256 |
| --- | --- | --- |
| [stable 指针](https://x.ai/cli/stable) | 纯文本 `1.0.34`，6 字节 | `f793c92fb1aa67f65ee5a281b7acc0c7976cc29ec06ae255060d8027a0cf6855` |
| [alpha 指针](https://x.ai/cli/alpha) | 纯文本 `1.0.38`，6 字节 | `e591605acbab3fb074dcf9a31387ab360e463c165289c968d338c55558bb019b` |
| [官方更新日志](https://x.ai/build/changelog) | stable 1.0.34，2026-09-16 发布 | 网页佐证，不作为二进制完整性凭证 |

[官方安装文档](https://docs.x.ai/build/overview) 的脚本入口是 `https://x.ai/cli/install.sh`。本次仅下载审阅该脚本及 `install.ps1`，没有在用户环境执行安装脚本。

## 2. 固定二进制身份

| 二进制 | 真实 `--version` | 字节数 | SHA-256 |
| --- | --- | --- | --- |
| 已有受测 macOS arm64 | `grok 1.0.30 (04b7ffed98c6)` | 141869568 | `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb` |
| 官方 stable macOS arm64 | `grok 1.0.34 (3736acbc8658)` | 143016096 | `9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e` |

stable 来自 [官方压缩产物](https://x.ai/cli/grok-1.0.34-macos-aarch64.gz)，压缩包 60670915 字节，SHA-256 为 `0eb941ff9379d6921dd119b1751f49cc16d138a50766f7dd58a35304706e3511`。解压后实际运行版本/帮助；`codesign --verify --strict` 退出 0，签名为 X.AI Corporation，TeamIdentifier `5Y6N3AJ54S`。自行计算的摘要是本次产物绑定，不能冒充上游发布的签名清单。

其余平台官方固定 1.0.34 产物 HEAD 均返回 HTTP 200：macOS x86_64、Linux x86_64/aarch64、Windows x86_64/aarch64 `.exe`。这只证明下载入口存在，未运行这些平台的二进制。

## 3. 真实机器可读接口

两版本均在独立 HOME、GROK_HOME、工作目录中运行，无认证环境变量、无认证文件、stdin 为 null。`update --help`、`version --help`、`agent stdio --help` 的输出各自逐字相同；没有启动 ACP 会话或模型。

精确发现参数为 `['update', '--check', '--json']`。1.0.30 的真实 stdout：

```json
{"currentVersion":"1.0.30","latestVersion":"1.0.34","updateAvailable":true,"installer":"internal","channel":"stable","autoUpdate":false,"error":null}
```

1.0.34 返回同一结构，`currentVersion=latestVersion=1.0.34`、`updateAvailable=false`。两次退出 0，stderr 为空，合成 `config.toml` 字节不变；本次检查配置主动设置了 `auto_update=false`，不能据此推断用户默认值。应同时校验 JSON 字段及 `error`，不能仅凭退出 0 判成功。

`version --json` 的 `currentVersion` 含括号内 build；本次两者 `channel` 都为 `unknown`，因此版本 JSON 不能取代升级检查的通道结果。

帮助明确支持 `--check`、`--json`（用于 check）、`--force-reinstall`、`--version <V>`、`--alpha`、`--stable`。不需要构造 `--yes`。检查时不要附加通道切换参数；官方源码显示切换会持久化配置，甚至先于 check 执行。[CLI 参考](https://docs.x.ai/build/cli/reference)

## 4. 三次原生升级/回退实测

每案运行前独占登记，不隐式重试，单次进程期限 180 秒。复制真实旧二进制到合成安装目录，使用官方公开下载；未复制凭据、未读写用户日常配置。表中参数还附带独占 `--leader-socket <fixture/socket>`，防止与日常 leader 混用。

| 案例 | 原生命令参数 | 结果 | 配置副作用 |
| --- | --- | --- | --- |
| 固定升级 | `update --version 1.0.34` | 退出 0，8.917 秒；新 SHA 与独立官方下载完全一致 | `cli.auto_update: true → false`；删除合成注释并补 UI 默认值 |
| 固定回退 | `update --version 1.0.30` | 退出 0，7.941 秒；回到原固定 SHA | 保持升级后的配置，**不恢复**原来的开关或注释 |
| 全新 HOME 普通升级 | `update` | 退出 0，10.357 秒；1.0.30 → stable 1.0.34 | 全部已有 TOML 值不变，包含 `auto_update=true`、`yolo=false`、`compact_mode=true`；删除注释并新增两个 UI 默认值 |

普通升级新增的是 `ui.max_thoughts_width=120`、`ui.fork_secondary_model="grok-4.6"`。固定升级的初始合成文件没有 `[ui]`，升级后另补 `yolo=false`、`compact_mode=false`。没有把这些原生写入隐藏成“配置不变”。

| 配置状态 | 字节 | SHA-256 |
| --- | --- | --- |
| 固定升级前 | 111 | `97d05c6341abc71dca1493492e8d7e258ea15ebf756db3bf0f6f41262a3075e2` |
| 固定升级后／回退前后 | 167 | `4c2af61eaefc16ecd53b8aee7874b683c4251e821c6d2b3225fa3b5bc572c180` |
| 普通升级前 | 158 | `c54e5ed62113b147f11f91612b802569e3625ea1ce88c27b7ba0f8b305fd1673` |
| 普通升级后 | 165 | `ed4487f153b7e5225ec63fc50f986bc6affc5f1231d5d60ccdfd6c73a1634ebb` |

两套独立安装中，`GROK_HOME/bin/grok` 与 `agent` 都切换到 `../downloads/grok-<version>-macos-aarch64`；旧新两版包均仍存在且摘要正确。原生命令也写入安装目录中的补全、用户指南、版本/活动会话辅助文件，以及合成 HOME 的 fish 补全。收尾检查为零遗留原生进程、零 `auth.json`、零会话历史文件；模型输入数为零。上述成功是正常升级和显式回退证据，**不包含**强杀、磁盘写失败、断电或活动任务中升级的恢复验证。

## 5. 配置覆盖与安装布局边界

本次官方源码审阅固定为 [a28ee2b2063426e8816e380ccea528b9de95e5da](https://github.com/xai-org/grok-build/tree/a28ee2b2063426e8816e380ccea528b9de95e5da)，其 `SOURCE_REV=e8563f8f182296ebb53cadb3e1eab7615d76408e`，与上述两版二进制 build **均不匹配**。以下作为接口设计线索，不能替代精确版本动态证据。

- [配置 overlay](https://github.com/xai-org/grok-build/blob/a28ee2b2063426e8816e380ccea528b9de95e5da/crates/codegen/xai-grok-config/src/env_overlay.rs) 的 `GROK_CONFIG_PATH` 是额外只读 JSON/TOML 合并层，`GROK_CONFIG` 是 inline JSON；不是配置保存目标。实际两版二进制也包含这些字面量，但本次未执行覆盖实验。
- [设置保存路径](https://github.com/xai-org/grok-build/blob/a28ee2b2063426e8816e380ccea528b9de95e5da/crates/codegen/xai-grok-shell/src/util/config/mcp.rs#L1759) 固定取解析后的 `GROK_HOME/config.toml`；[保存实现](https://github.com/xai-org/grok-build/blob/a28ee2b2063426e8816e380ccea528b9de95e5da/crates/codegen/xai-grok-shell/src/util/config/persist.rs#L319) 不使用 overlay 路径。`GROK_HOME` 同时决定原生 updater 的 `bin` 和 `downloads`，不能用它只隔离配置而仍称升级了原安装。
- `GROK_BIN_DIR` 是 bootstrap 脚本选项；当前原生 updater 直接使用 Grok home 下的 bin。没有找到适合本任务的“保持原安装、只重定向 updater 配置写入”官方契约。
- 当前官方脚本 bootstrap 下载文件名不含版本；原生 updater 实测转为含版本下载包。macOS/Linux 使用链接，Windows 官方 PowerShell 脚本使用复制及占用时改名恢复，不能将 Unix 链接方案直接套到 Windows。
- [原生更新实现](https://github.com/xai-org/grok-build/blob/a28ee2b2063426e8816e380ccea528b9de95e5da/crates/codegen/xai-grok-update/src/auto_update.rs) 有下载临时文件、运行烟测、链接替换及失败回滚路径，但这些不是本次强杀/断电通过证据；`installer=internal` 在该源码存在缺省回退，不能独立证明 PATH 中可执行文件的安装归属。

## 6. 1.0.30 → 1.0.34 的兼容边界

已确认版本、升级检查 JSON 和三个帮助入口；未运行新版 ACP 初始化、认证、审批、取消、冷恢复或插件。不能据帮助相同宣称完整协议无差异。

官方更新日志明确列出：1.0.31 改善取消原因显示；1.0.32 使 `x.ai/plugins/list` 和 `x.ai/skills/*` 在首个会话前反映最新配置；1.0.33 涉及 MCP 结构化结果、取消/超时停止、会话历史保留和 SSH 换行提示；1.0.34 将 memory 标为正式可用。这些需要成为升级后针对性验收点，不是协议兼容证明。[官方逐版记录](https://x.ai/build/changelog)

当前仓库 `grok.rs` 的 ACP 版本判定、`grok_profile.rs` 的固定策略及插件 prerequisite 都精确绑定 `1.0.30`。自动发现 1.0.34 后，还必须更新并真实验证对应兼容记录；不能直接扩大白名单，也不能继续把 1.0.30 的证据标成 latest 通过。原普通 PTY 富输入安全降级也没有因版本发现而解除。

## 7. 给升级核心的具体实施依据

1. 消费版和开发版复用真实安装发现及官方稳定通道；安装版本、当前最新、后台任务使用的进程版本分别记录。探测 `update --check --json`，避免检查时改通道。
2. 已确认普通 `update` 是可用的非交互原生路径；无需长期保持 Unsupported。执行前做安装归属、活动任务及当前配置检查，持久化原字节与安装身份；执行后核对新的链接/文件身份及真实版本，而非只信退出码。
3. 配置保全必须处理本次已证实的重写。若恢复原字节，必须使用持久事务和冲突检测：拒绝覆盖用户并发修改、未知语义变化或变化的链接目标；崩溃后能区分已升级和待恢复配置。不能无条件覆盖，也不能把读取 overlay 当隔离保存。
4. 固定 `--version` 可用作显式版本恢复，但会关闭原生 auto-update；回退需要独立恢复配置状态。正常自动跟随 stable 优先使用已实测的普通 update，并处理检查与安装间通道更新的竞态。
5. 原生 updater 可能通知 leader 重启；本次没有活动 leader。应用必须选择安全空闲窗口，记录升级和恢复状态，避免破坏应用托管任务与旧会话。
6. 完成以上产品代码后，仍需真实配置冲突/失败恢复、最新三方协议、插件及同提交跨平台验证。本文无需本地化变更；没有替代根代理的 Cargo、i18n 或 GUI 门禁。

## 8. 可复核产物

独立证据目录：`/private/tmp/infinishell-grok-autoupdate-review/`。文件只含公开来源、合成配置、独立命令参数及摘要；认证未读取或复制。原始证据保留在目录中，由根代理选择归档。

| 产物 | SHA-256 |
| --- | --- |
| `evidence-manifest.json`（包含下载、帮助、源码及全部测试收据） | `d67dba34c5464da1dcbb6a109b2a8a0a3b73191f57ad9c10868fa4a46426b4c3` |
| `json-probe-runs.json` | `e8e33bc4894a477c99548ac28c4a76e2edce7e5ffb936914022edc0479a3a202` |
| `native-upgrade-report.json` | `20bc30d50259faac55b291863b4b12c6c637ce1c7fe3ecee12498bf9e01602da` |
| `native-rollback-report.json` | `779a84bbbed3c66f39efeef792d3edf554c97b976633fe1526410e067dd35ebe` |
| `native-plain-report.json` | `d7e51b1c9813560490f32cd27aab2e1160857788353ce9e7cb31230a32c35890` |
| `final-cleanup.json` | `a210041bdabb0ded89ab8814ecf9fe922950dc581876a8f081b37c30d9f9993f` |

另保留本次 source41 Files2 冷读诊断：`/tmp/infinishell-cli-parity-official-41-files2-cold-read-diagnostic.json`，SHA-256 `21872fe82738530436dfd19552a2f7fa02fa96232a094062d02b5027052c8967`。其前五阶段含严格待审批取消通过，第六阶段因测试代理 32 次 CONNECT 预算耗尽而失败；它与本文的零模型升级验收是不同证据，不能互相代替。
