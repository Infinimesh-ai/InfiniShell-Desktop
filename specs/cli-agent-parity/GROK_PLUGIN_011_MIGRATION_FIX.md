# Grok 0.1.1 插件迁移修复准备记录

本记录只报告候选源码修复与非 Cargo 静态检查。状态为 **PREPARED_NOT_RUST_OR_NATIVE_VERIFIED**，不是已通过的升级交付。本轮没有执行 Rust 测试、Cargo、真实 CLI、GUI、网络或 Git，也未读取认证或原生私有产物；不能把本候选追认为 source31 已验证内容。根代理将在 source32 统一冻结后验证。

## 写入范围与输入身份

本轮只修改 [grok.rs](../../app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs) 和 [grok_tests.rs](../../app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs)，另新增本记录与 [安全 JSON](validation/grok-plugin-011-migration-fix.json)。0.1.1 manifest、当前 README、固定 0.1.0 README 夹具、旧审查报告和根三文档未修改。修复由前轮并发状态守卫缺口及旧源保持证据缺口触发。

| 源码 | 当前字节数 | SHA-256 |
| --- | ---: | --- |
| `app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs` | 43574 | `a37904802558e49cde49f585e94d6699fe84977345834c8a7b85ef3a2fab462c` |
| `app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs` | 51288 | `b4b9f3d29696d7cde8341aa315847e3af2bae9b810598b3735fdf20e763a93da` |

## 实际实现

`apply` 在只读运行时与插件验证的等待结束后，重新检查禁用意图和原注册信息；跨版本分支改用私有 `upgrade_plugin`。同版本文件修复与首次安装仍复用既有路径。新共享接口只封装本模块既有卸载与 `install_source`，生产依旧通过现有运行器派生进程；异步 mock 调用相同升级状态机，只写测试临时目录，不派生进程、不改全局环境。

`PluginMutationState` 在内存保存注册文件和配置文件原始字节、解析后的配置、注册身份、当前注册缓存完整文件树，以及原缓存目录在注销后的剩余文件树。快照结束再读原始字节，发现读取过程中漂移即拒绝。注册路径继续使用原来的唯一注册、Local 来源、缓存归属、普通目录和文件安全校验；未放宽路径或来源校验。

`PluginUpgradeGuard` 固定旧源、当前版本源和旧缓存备份的完整文件树。每次涉及原生修改的 `await` 前后，都重新检查上述状态、禁用意图、三个源的真实字节及权限、其他插件注册和其他配置字段。只允许本插件自身的 enabled/disabled 条目及注册项发生与当前命令相符的修改；配置的其他插件条目、权限及 UI 设置和注册表其他根字段保持原语义。边界之间保存的配置及注册原始字节也必须匹配，避免静默忽略用户编辑。

卸载后的残余缓存只能是上一已知缓存的子集；安装后注册源及版本必须属于该次固定安装请求，缓存中每个存在文件必须匹配固定配方的真实内容。新缓存原生复制权限没有提前声称与源相同，因此第一次观察后记录其实际权限，下一边界严格比较。原缓存即使不再注册仍被检查，不允许其出现未知字节或之后静默漂移。不存在的文件、完整或部分已知安装失败可以进入受控清理/恢复；无法识别的注册、目录、文件、源或配置变化立即停止，不继续卸载、安装或用备份覆盖。

成功返回要求当前安装命令成功、原来的完整 `verify_installed` 验证通过、最终状态边界仍一致。旧版恢复提示同时要求恢复命令成功、旧版注册版本和完整文件树验证通过、最终边界一致；即使恢复文件已完整但恢复命令返回失败，也只能提示恢复未确认。恢复成功仍返回更新失败错误，不计为升级成功。

`legacy_source_unchanged` 的原生夹具在升级前保存旧源完整 `plugin_tree`，升级后必须与快照逐文件内容及权限相等才写入 true。离线旧源/备份单测也补充相同的全树断言，原来只检查 README 的证据不足已消除。这里描述判据代码，**本轮未取得任何新的 native true 结果**。

## 回归定义与实际检查

新增以下 8 个 `#[tokio::test]`，均位于本模块 `migration_async_tests`，本轮执行数为 **0**：

- `disabled_at_upgrade_boundary_never_mutates_native_state`：禁用已存在时不执行原生变更，原配置、注册及缓存保留。
- `async_disable_stops_upgrade_without_reinstall_or_restore`：卸载等待期间出现禁用意图后停止，不安装当前版本或备份。
- `async_source_edit_is_preserved_without_installing_backup`：旧源在卸载等待期间被用户编辑后停止，保留用户内容。
- `user_cache_edit_during_failed_install_prevents_cleanup`：失败安装期间缓存被用户编辑时不清理或回退覆盖。
- `concurrent_registry_change_is_preserved_without_rollback`：失败安装期间新增注册元信息时保留，不继续回退。
- `failed_upgrade_install_restores_complete_legacy_recipe`：遍历安装前失败、部分写入失败、完整写入后失败三种情况，恢复旧配方并检查旧源全树、其他配置及真实调用序列。
- `failed_restore_command_never_reports_restore_success`：恢复命令返回失败，即使已写完整旧文件也不能提示恢复成功。
- `normal_upgrade_preserves_the_full_old_source_tree`：正常升级保留完整旧源、其他插件注册和包含权限字段的其他配置。

现有真实 `live_grok_production_installer_repairs_and_preserves_disable` 保留先前迁移候选的 6 段设计，并强化旧源全树判据；本轮没有运行这 6 段，也未修改旧证据或将旧 5 段证据回填成 6 段。

实际执行 `rustfmt --edition 2024 --config skip_children=true` 定向格式化，两文件均成功；最终 `rustfmt --check` 返回 0。纯 Python 文件检查核对 UTF-8、末尾换行、行末空白及冲突标记，全部通过。六类凭据形状扫描为 0：OpenAI 样式密钥、GitHub 样式令牌、JWT、AWS 访问键、私钥 PEM 标记及 Bearer 值。没有运行 `git diff --check`；本记录不把文件空白检查说成 Git 检查。rustfmt 只提供语法/格式检查，不证明 trait Send、类型或 cfg 编译通过。

## 用户可见语义与后续门禁

新增状态漂移、并发内容、恢复命令失败复用字面量 `crate::t!("cli-agent-plugin-grok-restore-failed")`；经过真实恢复验证的更新失败复用 `cli-agent-plugin-grok-update-restored`；原前置禁用路径保留既有错误键。没有新增硬编码界面字符串或新消息键，合成命令失败文本只属于测试日志。内存快照不加入错误字符串或日志；本轮没有扩展现有运行器的原生输出记录行为。

由于只授权两个 Rust 源码，本轮未读取或修改英文/简中文件。根代理已读取并核对既有英中 restore-failed 措辞：它表达更新失败且自动恢复无法确认，未断言曾执行恢复，因此准确覆盖并发停手场景。本变更无需本地化变更，不需新增翻译；这项语义确认来自根代理，本轮未独立读取两种语言资源。i18n 门禁及必要双语错误显示检查仍由 source32 验证记录确认。状态漂移分支主动停手，未继续清理或恢复；它保留已有操作日志，不新增状态细分原因标签，不能把这个错误说成恢复命令已失败或真实 CLI 缺陷已经定位。

仍有以下明确边界：

- 这是可观测原生命令等待边界的守卫，不是配置全局锁、原子文件系统事务或沙箱。其他进程在命令内部修改后又被 CLI 覆盖，若等待结束无法观察，本实现不能证明或保留该瞬时意图。
- 强状态守卫覆盖跨版本升级及其清理/恢复。首次安装分支只增加只读等待结束后的前置检查，不能声称已有同等完整并发防护。
- 真实 Grok 是否仅修改被允许的配置/注册字段、是否保留原缓存、复制权限和跨平台路径形状尚未验证。不符合预期时严格停止；不能用宽松比较让未知行为通过。注册仍存在但缓存目录丢失等无法可靠归属的失败也停止，不自动修复猜测状态。
- 本轮不涉及任务权限、完成判据、协议来源账本或 CLI 输入，不改变这些能力门槛。
- root 应在 source32 检查编译、执行新增 8 个回归及既有模块、i18n，并在固定版本隔离环境实际执行 6 段生产安装/迁移/修复/禁用/回退/恢复验收；相关平台验证必须基于包含实际修改的同一提交。未完成之前本候选不能标为已验证发布能力，也不能计为整个 Goal 完成。
