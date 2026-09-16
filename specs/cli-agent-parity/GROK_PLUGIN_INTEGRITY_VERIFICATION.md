# Grok 已安装插件完整性验证

本记录针对普通 PTY 通知插件 `infinishell-grok 0.1.0`，不改变 Grok 托管执行、审批、成功模型回合或恢复的产品门控。

## 真实无模型复现

在 macOS arm64 使用固定 `grok 1.0.30 (04b7ffed98c6)`，可执行文件 SHA-256 为 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。新建私有 HOME/GROK_HOME、工作目录和来源目录，未提供凭据、未读取用户 Grok 配置、未提交模型输入或创建 ACP 会话。CLI 每次调用超时为 30 秒，均已正常退出，没有常驻探针进程。

步骤与输出保存在 [脱敏原生记录](fixtures/grok-1.0.30-installed-integrity-macos.json)：

1. 将仓库随附的四文件 Grok 插件复制到私有来源目录，执行原生 `plugin install --trust <source>`，成功。来源和安装缓存的完整文件清单与 SHA-256 相等。
2. 读取原生 `plugin list --json`，保存配置和注册表的原始字节。
3. 仅将已安装缓存的 `hooks/notify.cjs` 替换为不发送通知的注释，再次执行原生 `plugin list --json`。两次列表完全相同，仍为 `installed`、版本 `0.1.0`；四文件仍存在，manifest 与 registry 版本仍相等，配置和注册表字节不变。
4. 执行原生 `plugin disable infinishell-grok`，成功；`config.toml` 的 enabled 为空、disabled 含插件。原生列表仍显示 installed，说明禁用与安装必须分别读取。记录另保存禁用后的真实注册表，只脱敏绝对路径。

复现源树 HEAD 为 `dbee1ecae81da54a1749de88a7caffb3099d7eb7`；证据生成时 Grok 修复尚未实现。旧生产检查只依赖 manifest/name/version 与四文件存在，上述真实磁盘状态会通过这些检查；同版本安装/更新提前走验证返回，不恢复脚本。**本次原生复现没有调用旧或新 Rust 安装器**，因此不将其计为产品安装器端到端通过。

## 修复范围

生产安装器把“唯一原生注册记录”与“完整缓存可用”分开。注册路径必须属于当前 GROK_HOME 的 `installed-plugins/<repo_key>`；多插件或重复注册、外部路径、未知注册类型仍拒绝。

完整性检查要求恰好为随附四文件、已知两个子目录和匹配的文件内容。额外文件、额外空目录、符号链接、非普通文件、超限文件、缺失文件及未知未来版本不能报告可用。Unix 另拒绝硬链接。Windows 当前没有 nlink 检测，不能声称会拒绝硬链接；修复使用独立临时文件后替换目录项，不原地截写已存在文件，因此不会通过写入原 inode 修改共享链接目标，但 Windows 原生文件与句柄行为尚未验证。Node 按固定 hooks 定义读取脚本，不依赖脚本的可执行位；读取失败同样不能通过验证。

用户触发当前版本安装/更新时，仅在原生注册唯一、插件仍启用、来源位于应用受控目录且来源完整匹配时修复缓存。损坏的已知文件和缺失文件通过同目录临时文件、文件同步和原子重命名替换；已有文件权限保留。正常文件不重复写入。此分支不运行原生卸载、安装或 enable，不写 config.toml 或 registry.json，避免同版本修复重新启用插件。

每次替换前和完成前重读禁用状态、注册表及缓存内容。普通写入错误后，只回退仍等于本次写入值的文件；原先缺失的文件恢复为缺失。用户中途禁用不会被撤销；并发编辑或注册变化导致无法证明所有权时保留现场并返回失败，不盲覆整份配置或来源。

旧版升级保留现有原生命令事务，但升级前增加已安装缓存及受控来源的完整性校验。目前仅允许随附同一脚本配方、manifest 仅版本号不同的旧版本，不将任意旧版本号当作已验证兼容配方。没有真实旧版来源证据时不能宣称历史升级通过；未知来源或脚本内容保守拒绝。恢复备份也必须有完整已知文件结构。

上述检查是写入前后重读与原子文件替换，**不是跨进程原子 compare-and-swap 或四文件整体事务**。检查与替换之间仍有外部写入窗口；本轮回归不冒充真实 SIGKILL 试验。若进程在部分文件替换后结束，下一次检测保持不完整状态，后续用户明确触发修复可补齐，不能预先报告成功。

无需本地化变更：现有安装失败、无效插件状态、禁用和更新未生效文案继续使用；没有新增用户界面文本。公共插件接口、其他 CLI 与随附 Grok 插件内容均未修改。

## 回归与验收边界

`grok_tests.rs` 中的安装测试使用本记录的真实原生 registry 形状，只重定位路径与 repo key。新增真实文件回归覆盖：

- 损坏通知文件、未知未来版本不能通过完整性判断。
- 同版本损坏与缺失修复、正常文件不重复写入、config/registry 字节不变。
- 第一次替换完成后第二次失败，恢复原损坏内容或原缺失状态。
- 开始前和修复中发生禁用，保留原生 disabled 状态。
- 并发修改已修复文件或来源时，不覆盖用户内容、不报告成功。
- 原生注册变更、外部缓存路径、未知文件/目录、Unix 符号链接和硬链接拒绝。
- 明确构造的部分替换磁盘状态仍不可用，只有显式补齐后才通过；这不是进程崩溃的原生证据。

子代理冻结时完成定向 rustfmt、差异空白检查和 JSON/摘要静态核对。根代理随后在隔离 validation 树完成插件、i18n、cargo check 及真实 macOS 生产安装器验证，结果见下节。Linux/Windows 文件系统行为、真实进程中断，以及最终同提交产品端到端验证仍未通过本记录验收。

## 显式真实生产安装器入口

新增默认忽略的 `terminal::cli_agent_sessions::plugin_manager::grok::tests::live_grok_production_installer_repairs_and_preserves_disable`。它要求独立 test binary 进程、`--exact --ignored --test-threads=1`、私有目录标记、空 GROK_HOME，以及运行前指定的 Grok/Node 可执行文件摘要；不在测试进程内修改全局环境。整体限时 180 秒，生产原生命令还有各自超时。

此入口真实调用 `GrokPluginManager::install()` 和 `update()`，依次验收安装、同版本损坏修复、原生禁用后 update 拒绝且配置/缓存不变。随后在同一份原生缓存上，通过生产 `repair_current_plugin` 的文件故障注入点验证第一项替换后的失败回滚，再次调用生产 update 恢复。**故障注入段明确不是 apply 失败或 SIGKILL 试验**；报告用 `production_apply_call=false` 区分，其他生产路径不会由 Python 实现代替。

入口仅编译于 macOS/Linux。仓库 `dirs 6.0.0` 在 macOS 使用私有 HOME 下的 `Library/Application Support`，Linux 使用 XDG_DATA_HOME；测试在首次调用安装器前断言实际 data_local_dir 位于私有目录。Windows 的同一库使用 Known Folder API，单设 LOCALAPPDATA 无法证明重定向，因此本轮不提供可能落入真实 profile 的 Windows live 入口。

以下运行器只创建私有环境和启动已构建的 Rust 测试，不执行安装、修复或配置回写。先由根代理将 `WARP_TEST_BINARY` 设为当前 validation 树构建出的绝对 test binary 路径，再运行；给定摘要和 Node 版本适用于本次已核实的 macOS 工具，其他平台必须先核实对应固定资产后显式更新预期值，不能改成运行时自行接受任意摘要。

```sh
python3 - "$WARP_TEST_BINARY" /Users/zhishi/.grok/bin/grok /opt/homebrew/bin/node <<'PY'
import hashlib, json, os, pathlib, subprocess, sys, tempfile

binary, grok, node = (pathlib.Path(value).resolve(strict=True) for value in sys.argv[1:])
expected_grok = 'd53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb'
expected_node = 'a46ed02589ca3af795237111ff854262064f8ff5c5b58d75c1509f37311eb15e'
assert hashlib.sha256(grok.read_bytes()).hexdigest() == expected_grok
assert hashlib.sha256(node.read_bytes()).hexdigest() == expected_node
root = pathlib.Path(tempfile.mkdtemp(prefix='infinishell-grok-installer-live-')).resolve()
paths = {
    'HOME': 'home', 'USERPROFILE': 'home', 'GROK_HOME': 'home/.grok',
    'APPDATA': 'home/AppData/Roaming', 'LOCALAPPDATA': 'home/AppData/Local',
    'XDG_CONFIG_HOME': 'home/.config', 'XDG_DATA_HOME': 'home/.local/share',
    'XDG_CACHE_HOME': 'home/.cache', 'TMPDIR': 'tmp',
}
for path in [*paths.values(), 'home/Library/Application Support', 'work', 'bin']:
    (root / path).mkdir(parents=True, exist_ok=True)
# 固定下载文件名未必为 grok，只在私有目录建立本次进程的命令入口。
(root / 'bin/grok').symlink_to(grok)
(root / 'bin/node').symlink_to(node)
(root / '.infinishell-grok-plugin-live').write_text('isolated unauthenticated Grok plugin verification\n')
(root / 'empty-gitconfig').touch()
environment = {key: str(root / path) for key, path in paths.items()}
environment.update({
    'PATH': os.pathsep.join([str(root / 'bin'), '/usr/bin', '/bin']),
    'LANG': 'en_US.UTF-8', 'TERM': 'dumb', 'GROK_AUTO_UPDATE': '0',
    'GROK_DISABLE_AUTOUPDATER': '1', 'GIT_CONFIG_NOSYSTEM': '1',
    'GIT_CONFIG_GLOBAL': str(root / 'empty-gitconfig'),
    'INFINISHELL_GROK_PLUGIN_LIVE_ROOT': str(root),
    'INFINISHELL_GROK_PLUGIN_LIVE_GROK_SHA256': expected_grok,
    'INFINISHELL_GROK_PLUGIN_LIVE_NODE_SHA256': expected_node,
    'INFINISHELL_GROK_PLUGIN_LIVE_NODE_VERSION': 'v25.9.0',
})
test = 'terminal::cli_agent_sessions::plugin_manager::grok::tests::live_grok_production_installer_repairs_and_preserves_disable'
print(json.dumps({'private_root': str(root), 'artifact': str(root / 'grok-production-installer.json')}), flush=True)
result = subprocess.run([str(binary), '--exact', test, '--ignored', '--test-threads=1', '--nocapture'],
                        cwd=root / 'work', env=environment, timeout=200)
assert result.returncode == 0, result.returncode
report = json.loads((root / 'grok-production-installer.json').read_text())
assert report['passed'] and len(report['steps']) == 5
print(json.dumps(report, ensure_ascii=False, indent=2))
PY
```

正常结束后原生配置、缓存和报告保留在打印的私有目录，便于核对；运行器不删除任何已有用户目录。产物记录实际 test binary 摘要、固定 CLI/Node 摘要与版本、逐阶段结果，不自动计入整体模型生命周期验收。本机已完成构建与实际执行，结果见下节；其他平台仍待验。

## macOS 生产路径实跑结果

隔离 validation 树以 c55385a69 为基线同步冻结插件变更，已通过 `cargo check -p warp`、国际化 11 项及插件相关 163 项，见 [本地门禁](validation/macos-plugin-transactions-local-gates.json)。新测试二进制 SHA256 为 `b4d872e41fc96930b16df1b42ed492fed2647b2b5bd612855950b499dced506d`。

上述显式真实入口 **1 项通过，27.17 秒**，五阶段全部通过：生产安装、同版本损坏修复、原生禁用后生产更新拒绝、生产文件事务中第一次替换后的故障回滚，以及生产更新恢复。修复前后原生配置/注册表字节保持；禁用后配置和缓存保持。固定 Grok/Node 摘要、逐阶段断言及脱敏 libtest 输出见 [真实记录](validation/macos-grok-production-installer-1.json)。私有原生目录与原始输出保留在本机。

该结果首次覆盖当前 Rust 生产安装器作用于真实 Grok 缓存；它仍是未提交冻结代码的 macOS 中间快照，不覆盖模型生命周期、真实 SIGKILL、Windows/Linux 或最终同提交验收。故障注入阶段仅覆盖文件事务，未冒称完整 apply 强杀。
