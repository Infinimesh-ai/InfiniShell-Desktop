# Windows URI 登记与卸载约定

当前实现源码 `5acb349c7032c0fc07ba4fbbfaba93d87fa6407b` 已整合至主线 `cf75bdfddfd213215ded6ca27d484cfa628b7f2c`，本地 cargo check 与同 Rust／本地化源码 i18n 12/12 通过。[Windows 原生门禁 38512735757](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38512735757)全部成功：80 项 Inno 断言、6 项真实隔离注册表回归，以及应用 616、crate 492、Zap 528、completer 11 项测试；应用计数已包含该 6 项。原始依据为[原生审核](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/remote/38512735757/review.json)及[源码门禁记录](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/source-gates-5acb349c7.json)。早期 Inno 声明编译失败 `38512138949` 的原件保留，不重写为成功。

新候选 `v0.2026.10.14.13.00.oss_00` 的 tag 对象为 `9b5afd31aafa5bbda210356cf329d63ec696bc07`，固定上述产品源码，[正式 run 38514368665](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38514368665)正在构建。**源码缺陷已修复验证，最终 Windows 安装器和用户六阶段复验仍待完成**，不能据源码回归直接关闭发行验收阻塞。

`v0.2026.10.14.01.46.oss_00` 的用户回执确认：安装、GUI 启动、正常退出和官方卸载分项通过，但 `infinishell://` 命令始终指向旧失效目标。原件及审核保存在 [Windows 证据审核目录](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-user-review-20261014/README.zh-CN.md)。该候选不能发版，也不以修改旧包、重用旧标签来修复。

## 登记和恢复

- 安装器在安装结束时登记当前账户的 `HKCU\Software\Classes`；普通 GUI 初始化同步恢复同一关联并读回确认。开发隔离 profile 继续跳过系统 URI 登记。
- 协议根保存显示名、空 `URL Protocol` 和 `InfiniShellOwnerPath`；`shell\open\command` 使用 `"<本实例完整路径>" "%1"`。AUMID 保存既有 `DisplayName` 及相同归属字段。启动其他实例按既有行为接管关联，并同步更新归属。
- `REG_SZ` 写入包含 UTF-16 终止 NUL。旧依赖的 `set_hstring` 不计终止符长度，因此调用处显式补齐，不升级整个依赖。
- 错误保留失败阶段及 Win32 原因；不把注册失败静默丢弃，不把“开始后台任务”当作登记完成。错误日志不打印用户路径或 URI 内容。

现场缺少旧运行时 Win32 错误，首因仍未确定。Microsoft 的 [RegCreateKeyExW 说明](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regcreatekeyexw)明确区分父句柄访问掩码与创建子键时的安全描述符检查，因此不将 `KEY_READ` 单独认定为根因。[RegSetValueExW 说明](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regsetvalueexw)要求字符串长度包含终止符；这是独立可回归验证的修正，不据此声称已解释旧命令摘要未变。

## 卸载边界

仅当归属字段和当前 URI 命令同时匹配本安装，才清理本实例写入的值；AUMID 还须核对显示名，并受 URI 当前归属约束。别人接管、未知读取结果或额外字段不算本安装可删内容。删除空键逐层进行，保留未知值及子键；删除失败必须留出可审查日志。

这些登记是当前账户的集成，不扫描或清理其他用户的注册表。安装目录、PATH 和快捷方式沿用安装器已有生命周期；用户配置、历史和凭据不属于 URI 清理范围。

## 验证与交付

运行时回归使用随机 HKCU 私有子树，覆盖缺失登记、旧命令恢复、实例接管、中文和空格路径、原始字符串终止符及错误传播。安装器回归编译并执行生产共用的 Inno Pascal 函数，在独占子树验证清理和外来接管保护；不以文本搜索代替执行。

自动检查已通过，新候选已冻结并开始构建；实际安装器、同来源静态校验和身份绑定就绪后，再交付包含安装器、激活脚本和采集器的便携 ZIP。用户按[最终安装器验收入口](windows-final-installer-acceptance.zh-CN.md)完成 `baseline → installed → launched → uri_activated → closed → uninstalled` 六阶段，助手审核实际 URI 唤起、正常退出和官方卸载回执后再判关闭。Linux 桌面继续延期，自动检查保留；旧候选的 macOS 和 12 资产通过记录仅属于原字节。

无需本地化变更：没有新增或改变用户可见文案，变更限于 Windows 系统登记与卸载行为。

## 正式安装器只读工件验证

发行方在新的完整 release run 成功后，使用同一个 `cross-platform-preflight.yml` 控制面验证最终 Setup，不通过 SSH 另跑验证器，也不重复源码门禁。在 Actions 的 Run workflow 中选择包含验证工具的控制器分支，设置：

- `run_windows=true`、`run_linux=false`；其他开关保持默认。
- `windows_atomic_debug_scope=release-installer:<正式 release run 的正整数 ID>`，例如把冒号后的占位说明替换成实际 run ID。旧的 `all`、`upstream_essence` 等范围值仍按白名单接受，拼写错误会前置拒绝。

这里沿用范围字段，是因为既有 workflow 已达到 GitHub 的 25 个 dispatch 输入上限。该模式只有一个独立 Windows 只读 job，权限为 `contents:read` 与 `actions:read`；工具和临时文件只落在独占作业目录。

验证器分别记录 `artifact_source_sha` 和 `controller_sha`：前者由正式 run、注解 tag、生产作业、工件提供方摘要及六资源清单严格绑定，且必须是控制器提交的祖先或同一提交；后者是实际执行验证工具的提交。发行方还须将输出身份与已冻结 `candidate.json` 对照，不能把控制器后继提交说成候选产品源码。

验证包括 Windows 与 bundle 原始 ZIP 的提供方摘要、CRC、精确成员、上传/下载来源链，Setup 实际 Authenticode 状态，以及固定 innounp 对主 EXE、manifest 和六资源的八件只读解包。`NotSigned` 明确记为未签名；内层主 EXE 要求 AMD64，Setup 引擎可以是 PE32。只读验证不安装或启动产品、不改注册表，成功也不替代用户安装、真实 URI 唤起和官方卸载回执。失败保存原始证据，不自动重跑或放宽门禁。
