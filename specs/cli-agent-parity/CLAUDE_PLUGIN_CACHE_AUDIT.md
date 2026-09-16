# Claude 原生插件缓存与更新审计

本记录独立于 Codex 的缓存回滚问题。2026-09-16 在 macOS 上使用 Claude Code 2.1.273、`warp@claude-code-warp` 2.2.0 和 InfiniShell 通知补丁 revision 3，未向模型提交任何输入。

## 已确认的固定版本行为

原生 `plugin marketplace add warpdotdev/claude-code-warp` 和 `plugin install warp@claude-code-warp` 成功，marketplace HEAD 为 `8c28e936ae51cbb23a1a5657fca2bfd30cf06f12`，与随附 `PATCH_METADATA.json` 一致。原始缓存的全部 18 个文件通过受控来源校验；使用现有修补 helper 替换五个文件后，逐步对比完整树摘要。

| 实际操作 | 结果 |
|---|---|
| 无模型 stream-json `initialize`，观察 20 秒后退出 | 18 个文件的修补摘要保持；原生日志实际注册 7 个 hooks |
| `plugin marketplace update claude-code-warp` | 修补摘要保持 |
| `plugin update warp@claude-code-warp --json` | 原生返回 `up_to_date`，版本仍为 2.2.0；修补摘要保持 |
| 再次 `plugin install warp@claude-code-warp` | 原生返回 already installed；修补摘要保持 |
| 原生 `plugin disable` 后无模型重启 | `enabledPlugins` 仍为 false；原生注册 0 个 hooks |
| 禁用期间原生同版本 `plugin update` | 没有重新启用，也没有覆盖修补；产品 Python 预检明确拒绝禁用插件 |

这组操作没有复现 Codex 的同版本启动缓存回滚。原生用户设置和 `known_marketplaces.json` 均没有显式 `autoUpdate` 字段。官方说明第三方 marketplace 默认关闭自动更新；不能把“字段缺省”写成用户显式关闭。参见[原生自动更新规则](https://code.claude.com/docs/en/discover-plugins#configure-auto-updates)。

相同版本不会自动继承未来版本的行为。Claude 以插件版本作为缓存键，同版本 update 会跳过更新；这是本次实测结果的适用边界。参见[官方版本规则](https://code.claude.com/docs/en/plugins-reference#version-management)。

## 受控新版本实验

另建独立无账号 HOME 和仅监听 `127.0.0.1` 的 Git HTTP 服务。夹具来自同一固定提交，只更名 marketplace 为 `infinishell-claude-update-fixture`，初始插件树仍为原始 2.2.0；原生下载到 cache 后再应用相同五文件补丁。随后只在实验来源修改 manifest 和 marketplace 的版本为 `2.2.1-fixture`，脚本仍为未修补的原始内容。这不是已发布的 Warp 新版本，也没有推送任何远端仓库。

| 实际操作 | 结果与边界 |
|---|---|
| 私有原生设置和 marketplace 记录写入 `autoUpdate: true`，无模型 initialize 后观察 60 秒 | 没有观察到自动更新；活跃版本仍为 2.2.0，修补摘要保持 |
| 原生 marketplace update，再 plugin update | 返回 `updated`，原生安装记录切到 2.2.1-fixture 的新 cache 路径 |
| 比较新活跃缓存全部 18 个文件 | 与未修补实验来源完全一致，五个受控通知文件不含 InfiniShell 修补 |
| 比较旧 2.2.0 缓存 | 原有 18 个文件仍保持修补；原生额外添加 `.orphaned_at`，旧目录已不再是活跃安装路径 |

因此，**原生跨版本更新可以让缓存修补失效**，具体机制是切换活跃缓存目录，而非本次固定 2.2.0 重启时原地还原。这个风险可以独立验证，无需把 Codex 的回滚机制外推给 Claude。

**后台自动更新是否会触发这条路径仍未验证。** 官方允许启动后最多十分钟随机延迟；60 秒无模型、非交互观察窗口没有覆盖完整调度边界，也没有获得后台更新触发日志。不能把 `autoUpdate: true`、initialize 成功或文件暂未变化计为后台更新通过。参见[后台更新时序](https://code.claude.com/docs/en/discover-plugins#configure-auto-updates)。

## 实现边界

- `plugin_manager/claude.rs::notification_operation` 先检查用户禁用和本地 marketplace 覆盖，再做 exact CLI 与固定插件树预检；已处于目标 2.2.0 时只应用通知补丁，不调用重复安装。
- `notification_patch.rs` 校验唯一 user 安装记录、预期缓存路径和完整文件树，然后原子替换受控文件。它不改原生 marketplace 来源或自动更新开关。
- `ClaudeCodePluginManager::needs_update` 通过当前安装路径的通知补丁状态判断需要修补，不能把原生注册中的 installed 当作补丁存在。
- 本次没有调用 Rust GUI 安装器；实际原生操作与 Python helper 的证据不替代 GUI 安装验收、模型交互或 Rust 门禁。

## 隔离与可复现产物

所有原生命令使用新建的 `HOME`、`USERPROFILE`、`CLAUDE_CONFIG_DIR` 和空 Git 全局配置。没有复制账号、API 密钥或普通用户配置。设置 `DISABLE_AUTOUPDATER=1` 保护共享的受测 CLI 二进制，并设置 `FORCE_AUTOUPDATE_PLUGINS=1`，避免这个保护变量同时掩盖插件更新行为。

- 探测器：[probe_claude_plugin_cache_refresh.py](../../script/cli-agent-parity/probe_claude_plugin_cache_refresh.py)。只允许原生版本/插件管理命令，会话只发送 `initialize`，拒绝 `user` 输入。
- 固定来源证据：[claude-plugin-cache-refresh-2.1.273-macos.json](fixtures/claude-plugin-cache-refresh-2.1.273-macos.json)。包含原生命令输出、完整树摘要、隔离配置快照、实际握手和插件加载日志；临时根路径已替换为占位符。
- 受控新版本证据：[claude-plugin-controlled-update-2.1.273-macos.json](fixtures/claude-plugin-controlled-update-2.1.273-macos.json)。包含只读回环 Git 请求、两个实验提交、自动更新观察窗口、原生 `updated` 回包，以及新旧缓存完整摘要。

```bash
python3 script/cli-agent-parity/probe_claude_plugin_cache_refresh.py \
  --executable /path/to/verified/claude-2.1.273 \
  --output /private/output/claude-plugin-cache-refresh.json

python3 script/cli-agent-parity/probe_claude_plugin_cache_refresh.py \
  --executable /path/to/verified/claude-2.1.273 \
  --controlled-update --wait-seconds 60 \
  --output /private/output/claude-plugin-controlled-update.json
```

没有修改 Rust、FTL 或共享脚本，也没有运行 Cargo。
