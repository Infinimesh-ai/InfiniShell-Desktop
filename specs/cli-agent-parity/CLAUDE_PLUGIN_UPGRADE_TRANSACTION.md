# Claude 插件升级与失败恢复

本项仅针对 Claude Code 2.1.273 和固定来源的 Warp 插件 2.1.0、2.2.0。2026-09-16 已通过本地编译、国际化、插件模块回归及三个真实 Rust 无模型场景。来源为 `c55385a69a4cd50364a6464dfbec1c965db2ac92` 加冻结文件，**不是最终同提交验收**；逐文件摘要见门禁报告。无账号、无模型探针与 Rust 文件回归分别记录，不能互相替代。

## 原有缺口的真实负向证据

使用原生 CLI 从官方提交 `bb6c1cf2f5cd7eb609678a2b175e7f4504ca61c2` 的 2.1.0 安装升级到 `8c28e936ae51cbb23a1a5657fca2bfd30cf06f12` 的 2.2.0；两个版本分别通过随附元数据中 17/18 个文件的完整摘要校验。固定提交经仅监听 `127.0.0.1` 的私有 Git 服务提供，没有发布或修改远端仓库。

原生 `plugin update` 返回 `updated` 后，第三个受控文件替换注入失败。既有 Python 修补 helper 将已经替换的文件还原，但原生 registry 仍指向新 2.2.0 缓存。旧 2.1.0 缓存仅保留并新增 `.orphaned_at`。因此，恢复受控文件不等于恢复旧活跃安装版本。明确重新执行修补能够成功；原生禁用后的同版本更新没有将插件重新启用。

证据：[claude-plugin-upgrade-patch-failure-2.1.273-macos.json](fixtures/claude-plugin-upgrade-patch-failure-2.1.273-macos.json)。这条原始负向证据执行了原生升级与既有 Python helper，没有执行新 Rust 安装器。

## 最小修复及边界

- Claude 安装/升级先获取本应用专属操作系统文件锁，检查并恢复前一份已知格式的发布记录。锁不使用 PID、进程年龄或超时判断存活；不约束原生 CLI 或用户手工编辑。
- 新原生安装只运行于立即持久化的随机暂存目录，`CLAUDE_CONFIG_DIR`、`HOME`、`USERPROFILE`、工作目录均指向该私有目录。应用被强杀后，即使原生命令仍存活，也只能继续写旧暂存目录；下一次使用另一随机目录。
- 原生安装结束并完整应用、验证通知补丁后，先迁入已修补的 2.2.0 缓存，最后切换 `warp@claude-code-warp` 的唯一 user 安装记录。原 2.1.0 缓存不会被改写或删除。
- 发布记录只覆盖 `known_marketplaces.json` 的本 marketplace 项、`settings.json` 的本 marketplace 来源和本插件启用项，以及 `installed_plugins.json` 的格式版本和本插件记录。其他插件、设置和无关并发修改保留。
- 初次注册的原生 marketplace clone 继续位于本次持久暂存目录，原生 `installLocation` 引用该路径，source 仍是原生 GitHub 来源。升级已有官方 marketplace 时保留其来源、路径、自动更新配置及其他字段。
- 发布记录采用原子、禁止覆盖已有文件的写入；记录、锁及受控路径拒绝软链接和硬链接，记录拒绝未知版本、字段、目标配置路径和不符合受测约束的值。恢复先检查所有前像/后像，冲突、用户禁用或未知内容均保留现场并失败。
- 配置写入使用读后比较和原子文件替换。这不是对不遵守应用锁的外部进程提供原子 CAS；能够观察到的并发变化会拒绝覆盖。恢复入口是用户再次执行本应用的安装或更新操作，不是后台盲目回滚。

**本轮不覆盖已有 2.2.0 的原地五文件修补在应用被强杀时的跨文件原子性。** 该分支仍使用既有受控文件事务及常规错误回滚，不能因为新升级路径使用暂存，就宣称它具有相同的崩溃恢复能力。

所有可能仍被旧原生命令使用的暂存目录和已迁入但未激活的缓存都会保留，不自动清理。这样避免误删仍在写入的现场，但反复失败会占用额外磁盘空间；本轮没有自动垃圾回收或凭 PID 判断可删除的逻辑。

## 验证入口

主代理执行的 [macOS 本地门禁](validation/macos-plugin-transactions-local-gates.json) 全部通过：`cargo check -p warp`、国际化 11 项、插件模块 163 项。真实场景使用同一 libtest，SHA-256 为 `b4d872e41fc96930b16df1b42ed492fed2647b2b5bd612855950b499dced506d`；CLI 二进制 SHA-256 为 `953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb`，版本命令在私有 HOME 实际返回 `2.1.273 (Claude Code)`。

| 真实场景 | 结果与证据 |
|---|---|
| 产品 manager 从 2.1.0 升级 | 实际 1 passed / 0 failed，4.66 秒；活跃 registry 为已完整修补的 2.2.0，发布 journal 已移除。[报告](validation/macos-claude-plugin-upgrade-1.json) |
| 原生暂存安装成功后补丁失败 | 实际 1 passed / 0 failed，4.87 秒；hooks 目录设为 `0500` 后生产修补入口遇到真实写入失败，此前脚本替换回滚；真实 HOME 的活跃 registry 保持 2.1.0。[报告](validation/macos-claude-plugin-patch-failure-1.json) |
| 暂存安装父进程被强杀后重试 | 等真实暂存 Git 请求进入受控阻塞，杀掉首个 Rust 父进程，退出码 `-9`；请求保持阻塞时新进程实际 1 passed / 0 failed，4.94 秒，通过新目录发布 2.2.0。旧目录保留，释放旧请求后 3 秒观察内配置和活跃缓存摘要未变化。[报告](validation/macos-claude-plugin-crash-retry-1.json) |

三个场景分别使用全新私有 HOME，没有复制凭据或提交模型请求。运行结束后另外只读对比旧 2.1.0 缓存的全部 17 个文件，三者均与运行前的完整树摘要一致；结果已附入各报告，不仅依赖测试中的单个 hook 断言。原始 `/tmp` JSON 和日志均保留，归档报告记录其摘要及冻结源码来源。

强杀场景证明的是父进程退出、旧请求仍阻塞、新目录重试和已发布状态保持；**没有用 PID 身份核验旧原生进程存活**。报告显式标记 `old_native_process_liveness_verified=false`，不能把 HTTP 阻塞当作旧原生进程仍活着，更不能据此声称所有后代都已退出。它也没有覆盖 registry 发布时的真实进程强杀；该阶段目前由文件回归中的中断恢复覆盖。

`claude_tests.rs` 新增真实文件回归，覆盖升级后保留旧缓存、补丁校验失败保持旧 registry、发布后失败恢复并保留其他插件、发布中断后的显式恢复、并发禁用、OS 锁、未知记录和软硬链接。完整原始文件夹具来自上述固定提交：[claude-warp-compatible-original-trees.json](fixtures/claude-warp-compatible-original-trees.json)。夹具包含上游许可证。

真实 Rust ignored 测试：`terminal::cli_agent_sessions::plugin_manager::claude::tests::real_claude_plugin_transaction`。必须由私有 marker、HOME、CLI 配置路径共同满足的独立探针启动，普通测试不执行原生安装。`upgrade` 直接执行产品 manager；`patch-failure` 执行真实暂存原生安装，再使 hooks 目录不可写，调用生产修补入口并检查此前替换文件回滚及旧活跃 registry 保持。后者不冒充完整 manager 的故障注入。

```bash
python3 script/cli-agent-parity/probe_claude_upgrade_transaction.py \
  --executable /path/to/verified/claude-2.1.273 \
  --test-binary /path/to/current/warp-lib-test \
  --rust-case upgrade \
  --output /private/output/claude-rust-upgrade.json
```

`--rust-case` 还支持 `patch-failure` 和 `crash-retry`。强杀场景只在真实暂存 Git 请求进入受控阻塞后杀掉 Rust 安装父进程，保留旧原生请求阻塞，要求新进程使用独立目录完成真实一项测试，再释放旧请求观察已发布状态。它覆盖原生 marketplace clone 阶段，不代表发布 registry 时的进程强杀、所有后代进程最终退出或跨平台验收。

本轮无需本地化变更：复用既有双语 `cli-agent-plugin-patch-failed` 提示，新增技术细节仅进入安装错误日志，没有新增 GUI/TUI 文案键或布局变化。主代理执行的国际化 11 项门禁已通过。
