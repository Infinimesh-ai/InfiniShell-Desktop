# Windows、Ubuntu 用户验收最终梳理（2026-10-19）

**本轮“流程演练＋多端验收”已完成证据复核，可以按演练范围收尾；不推进本候选正式发版。** 用户已说明另一会话正在周期性合并上游，后续另行组织正式发版。本次将 Windows PATH 修复尚未整合到正式资产、共用更新逻辑把旧版本误判为新版，转交为下一次正式发版前必须处理的事项。不再把“等待用户执行 Windows／Ubuntu 验收”列为当前任务。Mac 六项 GUI 和原 12 件资产的已验结论保留，严格绑定原字节。

本次只整理、复核和归档，不修改产品实现，不介入另一会话的上游合并，不合并 PR、不替换发布资产、不移动标签、不公开发版。无需本地化变更。

## 已确认的通过项

| 范围 | 最终认定 | 边界 |
|---|---|---|
| Windows R9 重发包六阶段 | 安装、真实 GUI、URI 新窗口及工作目录、正常关闭、设置发起官方卸载通过；PATH 原始字节／类型／存在性恢复，机器 PATH 不变，登记和快捷方式清理通过 | 只属于 R9 Setup `c18725c5…`；用户数据仅检查目录存在性；不是原 Draft Setup |
| Windows 原候选 URI 问题 | `WIN-RELEASE-URI-01` 关闭：嵌套历史 R3 原包 `1ad878fb…` 的六阶段、请求链及实际窗口截图证明旧失效关联已修复并可真实唤起 | 旧 `01.46` 候选失败记录保留；不把所有旧关联冲突组合都称为本次桌面实测 |
| Ubuntu 24.04 x86_64 DEB 六阶段 | 原候选 DEB `2442bf8b…` 在原生 GNOME Wayland 完成安装、shell、URI 新窗口、正常退出和官方 remove；程序、资源、desktop、链接和 MIME 清理通过，四个用户目录保留 | 只认定该平台／该 DEB；`rc` 表示 remove 后残留配置状态，不是仍安装；不是所有 Linux 包及 GPU 组合通过 |
| Mac 最终包 | 原签名 DMG 六项 GUI、重启恢复、签名、公证和装订结果继续有效 | 本次重核结果封存摘要，未重跑 GUI；不扩大为更新功能已通过 |
| 原 12 件资产 | 10 月 15 日全量三方下载核验继续有效；10 月 19 日实时 provider 的 12 个 ID、大小和摘要均未变 | R9 重发 Setup 不在这个集合；后续换包必须重新绑定并核验受影响资产 |

Ubuntu 原采集器遇到受保护进程读取失败后使用了明确标识的本地技术修订。复核确认包、资源、URI、退出及卸载断言未放宽；12 条 root helper 补证保留 PermissionError，并绑定 PID／启动时间／UID／nonce／文件摘要。可接受其证据来闭合本次生命周期，原 `formal_acceptance=false`、`issuer_approved=false` 现场记录保持原样，本报告才是后续独立审核结论。

## 下一次正式发版前必须处理

### WIN-RELEASE-PATH-01：通过的 Windows 修复尚未成为正式发行资产

原 Draft Setup 在 R3 实际卸载后没有精确恢复用户 PATH 原文，出现尾分号差异；这是已证实的产品问题。现场类型仍为 `ExpandString`，不能把后续隔离回归的全部类型／多产品场景都冒称为原包现场失败。“设置中不可见”属于验收执行宿主可见性，“OneDrive 路径被拒绝”属于采集器处理问题，不另列为安装器产品缺陷。

R9 修复已由 [CI 39134031520](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39134031520) 构建并完成独立桌面复验。provider ZIP 摘要、内含真实 EXE、六阶段及截图一致。其身份为：

| 身份 | 值 |
|---|---|
| 原产品源码 | `5068dae9c75b79628865a0872684113647c204d8` |
| R9 安装器／控制器源码 | `a4e279c745bbd67aed1f8a926f45a32bda9343c8` |
| 后续仅文档提交 | `2e3406e7bce33fab7fc292622c68239db77e0e88` |
| R9 Setup SHA-256 | `c18725c5326ce74e3407957606e7a55691a2eec28b755112a7e7ae29d170e503` |
| R9 Setup 大小 | 580325911 字节 |
| 当前 Draft Setup SHA-256 | `1ad878fb098c8cc7e49c7dc29aa6afec7ceb34d80fcb9fc8220643f35ec3086a` |
| 当前 Draft Setup 大小 | 580320096 字节 |

当前 [PR #26](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/26) 仍为 draft/open、未合并；150 个应用／资源文件保留原包字节，并不意味着安装器字节未变。R9 的生命周期通过可以保留，但不能用于宣布当前 Draft 的 PATH 缺陷已消失。后续须完成修复整合审核、冻结最终安装器来源并使 Draft 资产与实际验收包一致，再回下载复核。若最终包字节变化，应按影响补验，不能转贴 R9 结论。双方均为 `NotSigned`，继续如实披露既定 OSS 未签名边界。

### RELEASE-UPDATE-ORDER-01：旧版本被提示为新版

Ubuntu About 原图同屏显示当前 `v0.2026.10.14.16.45.oss_00`，却把 `0.2026.10.07.01.09.oss_00` 标为“发现新版本”；首启同时出现无法更新横幅。用户未点击更新，证据不证明实际降级或数据损失。

本次用仓库现有 `version_compare` 直接复现：去掉 `v` 的旧版字符串返回 `invalid version`；相同版本带 `v` 时正确判定当前版本较新，真正更晚的带 `v` 版本也能正确排序。`GithubRelease::version()` 去掉前缀，而当前格式的比较解析要求 `v`；`should_update()` 只在 `Ok(true)` 时阻止降级，解析错误会继续走可下载分支。相关源码与冻结候选逐文件 Git blob 相同。这是共用逻辑问题，不能作为 Ubuntu 特有文案问题，也不能通过公开候选来掩盖。

后续正式发版的关闭条件：修复实际发行版本格式与前缀归一化、解析失败处理；覆盖相同／更旧／更新版本及明确回滚策略；完成必要本地化审计、自动门禁和最终包更新界面定向复验。若因此重建统一候选，原 Mac／Windows／Ubuntu 结果只能按各自原字节保留，另绑定新包和受影响检查。

## 可延期项与证据边界

- `UBUNTU-SSH-EMPTY-CLIP-01`：默认窄 SSH 侧栏空态文字被裁切，拖宽后完整；可延期，不扩大为 SSH 连接失败。
- `UBUNTU-INPUT-MODE-01`：含中文的合法 `printf` 在一次输入中自动识别为 Agent，Ctrl-I 后正常 shell 输出；未发送模型请求。保留为需定向复现的非阻塞分类问题，不认定为 IME 失效，也不泛化为所有中文命令。
- `MAC-SETTINGS-SEARCH-ZH-01`：中文“连字”设置搜索未命中，英文 `ligature` 入口可用，继续延期。
- 物理中文 IME 按“用户实际确认＋提交和输出截图”计证；没有候选窗／组合过程录像，不借此关闭旧 V02 完整矩阵。旧 CLI V01／V02／V05 继续延期，G09 不重开，Linux 自动检查保留。
- Ubuntu `01-installed.png` 实际是聊天窗口，不能作为安装过程截图；安装由 apt 事务、installed 文件摘要和后续真实 GUI 交叉证明，无需仅为补图重装。原图含其他聊天信息，仅保留在本机受限归档。
- 用户数据只检查目录存在性，不宣称内容逐字节不变。干净 baseline 的人工验收不扩大为升级、任意旧关联恢复、并发或 all-users 安装均已通过。

## 归档与验证

原件、解压成员摘要、大小写同名文件映射、两平台独立报告、GitHub 只读快照、版本比较复现及本次主线门禁见[总复核归档](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/native-validation/user-acceptance-review-20261019/README.zh-CN.md)和[证据索引](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/native-validation/user-acceptance-review-20261019/evidence-index.json)。附件脚本未执行，用户原报告和历史失败未改写，原始本机证据未整体上传仓库。

本次 `cargo check --locked` 通过，保留 5 条既有警告；版本比较负例按实际解析失败归档，不冒充通过测试。未改变 Rust、Fluent 或构建输入，无需为文档复核重复调度远端检查。公开 Latest 仍为 `v0.2026.10.07.01.09.oss_00`，Draft `412823983` 未公开。
