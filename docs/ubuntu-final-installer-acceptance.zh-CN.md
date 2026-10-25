# Ubuntu DEB 桌面验收流程

本页是可复用流程，不绑定旧候选或待执行任务。当前版本和实际验收范围见[发布入口](releases/README.zh-CN.md)。新一轮先确定 Ubuntu 版本、架构、桌面会话及真实 DEB 的源码、版本、来源和 SHA-256；旧包或其他平台结果不继承给新包。

## 准备

- 在用户现有交互桌面执行，记录 GNOME/其他桌面及 Wayland/X11，不为满足旧工具假设自动切换桌面。
- 静态核对 DEB 元数据、ELF 架构、desktop/MIME、内嵌 SSH 资源和维护脚本。静态核验不等于执行安装、卸载或 GUI。
- 当前包名为 `infinishell`，主程序为 `/opt/infinishell/infinishell`，命令链接为 `/usr/bin/infinishell`，desktop 为 `dev.infinishell.InfiniShell.desktop`，协议为 `x-scheme-handler/infinishell`；执行前仍以本轮包内容为准。
- 记录已有安装、同名命令、URI 默认处理器及用户目录。发现冲突时先审核，不擅自升级、卸载或覆盖用户现有版本。

## 六阶段

| 阶段 | 操作与必要检查 |
| --- | --- |
| `baseline` | 记录系统/架构/桌面、原安装状态、URI 关联及用户目录存在性 |
| `installed` | 安装本轮 DEB，首次启动前核对版本、主程序/资源摘要、链接和 desktop |
| `launched` | 从桌面应用列表启动真实 GUI，确认交互 shell，按范围检查英中界面及输入法 |
| `uri_activated` | 经 `xdg-open` 打开本轮唯一 `infinishell://action/new_window?path=...`，确认新窗口中的 `pwd` 与请求一致 |
| `closed` | 正常关闭全部本轮窗口，核对候选进程及关联资源退出 |
| `uninstalled` | 使用官方包管理器移除本轮包，检查程序、资源、desktop、命令链接和 MIME 归属清理，保留用户数据 |

卸载使用 `apt remove`；不顺手执行 purge/autoremove，不手删残留文件掩盖缺陷。`rc` 表示包已移除但仍有配置记录，不直接判为仍安装。

## 判断边界

- URI 默认处理项可以为空；系统选择应用提示按实际操作记录，不强改默认关联。`xdg-open` 返回成功或进程存在不能代替新窗口及正确目录证据。
- 中文显示、粘贴文字和系统剪贴板不替代真实输入法组合输入及候选选择；未执行时明确记录未验。
- 读取受保护进程失败时保留错误与补证方法；修改采集器不允许放宽身份、来源或退出判据。
- 只查用户目录存在性不能证明配置内容未变。模型登录、完整 CLI 生命周期与 GUI 包生命周期分别验收。
- 一次 DEB 验收不扩大为其他发行版、架构、GPU、RPM/AppImage/Arch 或所有 Wayland/X11 组合均已通过。

证据按[证据约定](evidence/README.zh-CN.md)保存；清理遵守[测试存储门禁](local-test-storage.zh-CN.md)。Windows 的独立流程见[安装器验收](windows-final-installer-acceptance.zh-CN.md)。
