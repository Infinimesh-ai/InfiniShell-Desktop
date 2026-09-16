# Codex 0.147 原生插件注册表负向生命周期

2026-09-16 在 macOS 使用实际 `/opt/homebrew/bin/codex` 完成下列隔离探测。仅覆盖原生注册表和失败重装；没有调用 InfiniShell 产品安装器，没有调用模型、thread/turn、原生 hook 执行或 TTY。Windows 自动安装门控及产品资源保持不变。

## 固定输入与原生接口

CLI 输出为 `codex-cli 0.147.0`，实际可执行文件 SHA-256 为 `19c4f144c5226a9f17c58e6f0fa854843b0f77a6eb420f40e2745a12f10f5d37`。其固定官方源码为提交 `be6e8eac029b183056b7e4402879f15d2c85f61b`。本探测记录本机二进制摘要，不把该摘要冒称其他平台发布资产的证明。

[官方 plugin_cmd.rs](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/cli/src/plugin_cmd.rs#L47) 只声明 `add`、`list`、`marketplace`、`remove`。实际运行 `plugin disable --help`、`plugin enable --help`、`plugin update --help` 均返回退出码 2 和未识别子命令错误。`--disable` 顶层参数控制 feature，不能当插件禁用命令使用。

禁用通过实际 app-server `config/value/write` 写入隔离配置的 `plugins."warp@codex-warp".enabled=false`，参数包含明确的私有 `filePath`。必须收到原生 `status=ok`，再读本次测试配置确认落盘，并启动独立 CLI `plugin list --json` 确认 `installed=true/enabled=false`。`hooks/list` 同时确认没有该插件的活跃 hook。没有手写注册表来代替原生成功结果。

插件来源为 [codex-warp 的固定提交](https://github.com/warpdotdev/codex-warp/tree/31ce59d9011cfb1d78f265649a228dac5de58d76/plugins/warp)，版本 0.4.0。复用现有完整十文件 SHA-256 清单校验原始目录；可从固定 raw URL 下载或由调用方提供目录。探测创建一个本地 marketplace 索引指向原始插件副本，由真实 CLI 安装。索引是受控测试输入，安装输出、注册表状态和文件快照均来自实际原生进程。

## 真实结果

| 阶段 | 实际操作与结果 |
| --- | --- |
| 缺失 | `plugin list --json` 无目标插件；`plugin add warp@codex-warp --json` 返回 1，配置逐字不变 |
| 安装 | 原生添加本地 marketplace，再 `plugin add` 返回 0；原生缓存的十文件摘要与原始来源一致，状态 installed/enabled |
| 禁用 | 原生 `config/value/write` 成功并落盘；独立 CLI 状态 installed/disabled；原生 hook 列表为空 |
| 失败重装 | 只损坏临时来源的 `plugin.json`；原生 `plugin add` 返回 1，指出 JSON 解析失败；配置与已安装缓存逐字不变，仍 disabled |
| 修复来源 | 恢复临时 manifest 原始字节并核对完整来源树；原生 `plugin add` 成功，缓存恢复核验通过 |
| 原生行为差异 | 成功的 `plugin add` 将原本 disabled 的插件重新启用。不能把这个操作当作“保留禁用的自动更新” |
| 恢复用户选择 | 再用原生配置接口恢复 disabled；整份隔离配置回到失败前的确切字节，已安装缓存仍一致 |
| 移除 | 原生 `plugin remove` 成功，注册表无已安装目标且缓存被删除；原生移除 marketplace，源插件和无关用户设置保留 |

失败前、恢复后的配置 SHA-256 均为 `47f7c870b0c3d9f79d83694b8d67de997a4690270926d9de4eae646f2cd27633`。该摘要包含本次临时目录值，因此重新运行会得到新的摘要；必须比较同一次运行的前后值。

证据为 [原生 JSON 报告](fixtures/codex-plugin-lifecycle-0.147.0-macos.json)，包含 20 次真实 CLI 调用、八个阶段的原生列表、两次原生配置写入与 hook 列表。仅把临时根目录替换成 `<isolated-root>`；没有合成原生响应。原始报告为 `/tmp/infinishell-codex-plugin-lifecycle-macos.json`，SHA-256 为 `ae47b5a1cd209b2d271d49239467173e60f03e85e39ee57a715f2efb12bf8762`。

## 复验

```sh
python3 -B script/cli-agent-parity/codex_plugin_lifecycle_tests.py
python3 -B script/cli-agent-parity/probe_codex_plugin_lifecycle.py --codex-executable /opt/homebrew/bin/codex --output /tmp/codex-plugin-lifecycle.json
```

需要 Python 3.11+ 和实际 Codex 0.147.0。省略 `--upstream-plugin` 时，从固定提交下载并核对十个文件；离线可传入已准备的原始插件绝对目录，摘要仍强制验证。输出必须在源树外。程序使用新 HOME、CODEX_HOME、USERPROFILE、APPDATA、LOCALAPPDATA，不继承认证变量；测试用户偏好只写入这些隔离目录，不读取真实用户认证文件。不产生源树 pycache，不改原始插件输入，不申请或使用模型凭据。

四项独立 Python 回归已通过，覆盖快照损坏/缺失/新增文件检测、无关设置保护、禁止模型入口和重复注册表条目。这些辅助测试不替代上述原生实跑。

## 验收边界

`plugin update` 在受测 CLI 中不存在。当前实证是原生 `plugin add` 对损坏来源的失败重装与恢复；不能计为新版本升级或 Git marketplace `upgrade` 失败恢复已验证。报告明确将 `new_plugin_version_upgrade_verified` 和 `git_marketplace_upgrade_verified` 保持 false。

产品安装器调用、版本兼容门控、真实原生 hook 信任交互及执行、Windows/Linux 同提交验证仍须分别验收。这里没有新增用户界面变化，无需本地化变更。
