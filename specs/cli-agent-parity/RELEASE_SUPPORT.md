# CLI 支持与回退说明

当前版本和发布验收范围见[发布入口](../../docs/releases/README.zh-CN.md)。本页说明使用条件与故障回退，能力差异见[能力矩阵](CAPABILITY_MATRIX.md)。

## 版本与运行方式

固定验收基线为 Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41`。其他版本分别经过来源、版本和能力门禁；不能把可执行文件升级成功理解为新版本托管协议已验证。

普通终端使用 CLI 自己的交互界面；应用托管任务记录输入、接收确认、审批、结果和历史。两者分别判断就绪，安装成功不表示已经登录或拥有模型额度。桌面入口默认启用三款托管任务；TUI 与共用库的开关须单独核对。

## 输入、技能与权限

- 文件卡传递本地路径引用，发送前整批检查普通文件及可读性，失败保留草稿和卡片。远程普通文件卡不会自动上传；锁定 Shell 模式下的 Codex/Claude 文件卡在写入 PTY 前拒绝。
- Claude 托管图片支持 PNG、JPEG、静态 GIF、WebP；技能注册确认后再派发。纯图片、图片与技能、多技能、恢复分别受固定合同约束，不能推定所有组合可用。
- Grok 托管 PNG 限固定版本/模型及 Inherit 模式；技能组合要求私有独占 leader 和正确目录。空闲时才能热新增技能；固定读取/文件策略禁原生 shell、hooks 和技能，所选技能策略另按已登记集合判断。
- Grok 普通终端自动文本/PNG 桥只对已核验定制工件开放，见[原生桥接说明](../../native/grok-build/README.zh-CN.md)。无法使用时保留草稿，用户可切回普通输入手动提交；拖放不自动发送。
- 固定命令、文件及技能策略限制宿主工具权限，不构成操作系统沙箱。未知身份、来源、目录或权限状态不能被当作允许。

## 通知插件与授权

通过第三方 CLI 设置页安装或更新插件。安装、启用、完整性及原生 hooks 信任分别检查；禁用插件不因修复自动启用，未知文件及并发编辑不覆盖。

| CLI | 配套插件 | 特别要求 |
| --- | --- | --- |
| Codex | `codex-warp 0.4.0` 及固定通知修补 | 在 Codex 内通过 `/hooks` 审核信任定义；应用不代写信任摘要 |
| Claude | `warp 2.2.0` 及固定通知修补 | 保留禁用状态和无关用户设置 |
| Grok | `infinishell-grok 0.1.6` | Node.js 18+；安装缓存完整性单独核对，原生 update 成功不足以判定文件已更新 |

通知缺少回合标识时只能展示可信当前会话的提醒，不证明该回合阻塞、成功或获得工具权限。失败恢复只覆盖仍属于本次操作的状态，无法确认归属时保留恢复资料并报告失败。详见[插件兼容](PLUGIN_COMPATIBILITY.md)和 [Codex 持久来源](CODEX_PLUGIN_CACHE_REFRESH.md)。

Codex/Claude 通知修补可导出供目标环境安装：

```sh
python3 script/cli-agent-parity/apply_notification_patch.py --export /absolute/path/to/new-export
```

保留导出目录并核对 `SHA256SUMS.json`。SSH、容器及 WSL 使用目标环境自己的 CLI 和配置；本机检测不证明远端已安装，`--files-only` 不证明控制终端、审批或完整交互可用。

## CLI 自动升级与渠道

可执行文件升级与通知插件升级分别管理。三款 CLI 的自动升级默认开启，默认跟随已识别的安装渠道；用户可关闭或选择该 CLI 提供的渠道，偏好持久化。

| CLI | 渠道 | 跟随安装 |
| --- | --- | --- |
| Codex | latest / alpha | 保留已识别的正式或 alpha 渠道 |
| Claude | latest / stable | 读取 `autoUpdatesChannel`，未设置时使用 latest |
| Grok | stable / alpha | 按原生检查返回的渠道 |

只对来源与身份已核验的安装生成更新计划。忙碌时延期，失败保留原可用状态和可验证的恢复/重试入口；未知来源或特殊权限由原安装工具处理，不猜安装方式或强行覆盖。

Linux Codex npm 和三款 Linux Homebrew 的实际版本更换要求 Landlock ABI ≥ 3；同版本检查/配置同步与其他来源按各自合同判断。渠道切换可能涉及降级，不把不同 CLI 的渠道名称或策略混用。详细来源、事务及恢复条件见 [CLI_AUTOUPDATE](CLI_AUTOUPDATE.md)。

完整平台生命周期、物理输入法及真实审批布局的未验范围见[当前限制](KNOWN_GAPS.md)。普通 Stop、EOF 或一条通知不能证明模型完成、工具树退出或清理完成。
