# Windows URI 登记与卸载约定

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

自动检查通过后，以新源码冻结新的候选，交付包含安装器、身份绑定和采集器的便携 ZIP。用户按[最终安装器验收入口](windows-final-installer-acceptance.zh-CN.md)补实际 URI 唤起、退出和官方卸载，助手审核回执后再判关闭。Linux 桌面继续延期，自动检查保留；旧候选的 macOS 和 12 资产通过记录仅属于原字节。

无需本地化变更：没有新增或改变用户可见文案，变更限于 Windows 系统登记与卸载行为。
