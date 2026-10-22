# 当前正式发布结果（2026-10-22）

产品源码固定为 `255849d04ffe6a8645f7ec31f3cd6fd58004c4be`，annotated tag `v0.2026.10.21.15.36.oss_00`（object `bfbf1a35de5f84c403b9cc6b31b7898ac5637db5`），正式构建 run `39627212929/1` 已成功；PR #29 已合入。已于 **2026-10-22 06:05:02（北京时间）** 正式公开为 [Latest](https://github.com/Infinimesh-ai/InfiniShell-Desktop/releases/tag/v0.2026.10.21.15.36.oss_00)，Release ID `420079568`，精确12件资产。后续文档提交不会移动冻结产品 tag。

- 同源码完整预检：Linux 12,212 PASS / 123 skip，Windows 11,932 PASS / 160 skip；五项新回归及两端适用12项i18n各三阶段通过。Mac本机115项定向、13项i18n及 `cargo check --locked -p warp` 通过；条件跳过不算通过。
- 本轮10/10 CI原件已归档。Mac最终DMG为512,352,949 B，SHA-256 `f46b144a6ce12c9b27263f1e5e5ffa7b9afeca1909ab171537fb98f3f8a890d0`，签名、Apple Accepted、装订、Gatekeeper、版本和六资源已验。Linux四包只解包及原生版本核对通过；Windows Setup只读来源、AMD64、内部版本与八成员核对通过，实际 **Authenticode=NotSigned**，没有执行Setup或桌面程序。
- 最终DMG英中U1–U4、A1原字节/CRLF/0600恢复后同一非空对话续问、A2失败保留原卡并同卡重试、A3依次接受A/B后C、A4当前进程A/B/B/A路由均有独审支持。使用受控provider，不扩为真实模型或重启持久化；审批仍挂起时输入C仍为 `not_executed_not_waived`，不能用审批后C替代。
- Linux/Windows最终SSH各10真实场景已限定接纳。Windows原20秒 `failed_proxy_exit` 保留，同一本地SSH随后自然退出0；两端后续精确端点缺席不证明远端全部后代/句柄或清理。两语言GUI正常退出、provider退出、镜像正常detach和profile精确恢复已记录；父会话历史EADDRINUSE/exit1与所有 `cleanup_ready=false` 保留。
- 用户决定Windows交互安装/PATH/非BMP/官方卸载/GUI与Ubuntu GUI不补验（`user_skipped`，`not_required/not_tested`）；Mac发现未来新版GUI延期（`user_deferred/not_tested`）。旧CLI矩阵延期和历史失败不变。

Draft精确12件全量回下载与公开后12件无凭据匿名全量下载均通过，名称/大小/SHA-256三方一致；Latest和tag/source保持。发布后同一最终DMG的英文、简体中文手动检查均显示已是最新版本、无下载安装入口，正常网络且无测试provider/proxy；检查中瞬态未单独捕获，不记作该瞬态已验。语言恢复System default，设置原始字节一致，正常Quit和精确detach均完成。最终正文回写已通过元数据复核。最终证据入口为[本轮台账](/Volumes/SanDisk/InfiniShell-Releases/release-2026-10-20/release-state.json)、[验收计划](/Volumes/SanDisk/InfiniShell-Releases/release-2026-10-20/acceptance-plan.json)、[EN独审](/Volumes/SanDisk/InfiniShell-Releases/release-2026-10-20/macos-en-gui-audit-255/result.json)、[ZH独审](/Volumes/SanDisk/InfiniShell-Releases/release-2026-10-20/macos-zh-gui-audit-255/result.json)、[Linux四包](/Volumes/SanDisk/InfiniShell-Releases/release-2026-10-20/linux-final-packages-20261021T1536Z/final-review.json)及[Windows安装器](/Volumes/SanDisk/InfiniShell-Releases/release-2026-10-20/windows-installer-verification-20261021T1536Z/final-review.json)。正式归档为 `/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.21.15.36.oss_00`。

`CURRENT_STATUS.json` 当前入口为 `main_acceptance.release_20261021`；下列10月19日退役说明完整保留为当时历史，不代表本轮无候选或未获发布授权。本次仅文档事实更新，无需本地化变更；文档提交前 `cargo check --locked -p warp`（r-4df159da）实际exit0，8分46秒、5条既有警告；期间仅JSON文档格式调整，Rust源码未变，原source_unchanged=false如实保留。

发布后证据：[匿名12件回环](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.21.15.36.oss_00/evidence/public-roundtrip-01/result.json)、[EN同版](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.21.15.36.oss_00/macos-gui/postpublication-en-01/result.json)、[ZH同版](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.21.15.36.oss_00/macos-gui/postpublication-zh-01/result.json)、[最终正文回执](/Volumes/SanDisk/InfiniShell-Releases/release-2026-10-20/publication-body-closeout-255/execution-01/result.json)。文档提交及远端一致性记录于 `/Volumes/SanDisk/InfiniShell-Releases/release-2026-10-20/revisions/final-publication-closeout-20261022/git-closeout.json`。

---

# 未发布候选退役与后续正式发布准备（2026-10-19）

**按用户授权，已删除未公开的候选 Draft、tag、对应构建产物及已合并分支。目前没有活动候选或 Draft，本项目本地和 origin 分支均只保留 main；没有创建新 tag、构建新正式包或公开发布。** 公开 Latest 仍为 `v0.2026.10.07.01.09.oss_00`。原“保持 Draft”的指令在本次退役范围内由删除授权替代，正式发布仍待另行安排。

本轮起点 main 为 `ea8f136605a8a90a13b7ec8dd24431abc1d39b1f`；受测产品源码仍为 `8b46bcc33c8c9d69a6bbecc8534edc39001ca867`，本次仅清理和续记文档，没有产品代码、界面或文案变更，无需本地化变更。[清理结果](/Volumes/SanDisk/InfiniShell-Archives/release-retirement/2026-10-19/result.json)的 SHA-256 为 `6613c72251aa34eb18c95e724aa4757544a05018c21126a9679a013bbca1e90e`；提交与远端最终一致性见[收尾回执](/Volumes/SanDisk/InfiniShell-Archives/release-retirement/2026-10-19/closeout.json)。

## 已完成清理

| 对象 | 结果 |
|---|---|
| 未公开 Draft | 删除 `412209685`（01.46）与 `412823983`（16.45），两份共 24 件资产、6,748,962,505 字节 |
| 候选 tag | 下表 6 个远端及同名本地 tag 已删除；未移动任何已发布 tag |
| Actions 构建产物 | 删除 6 次候选工作流的 42 个 artifact、8,033,583,882 字节；工作流运行／日志历史保留 |
| 分支引用 | 删除 7 个 origin 非 main 分支、10 个本地非 main 分支；全部已进入 main，独立提交数为 0；清理时开放 PR、stash 均为 0 |
| 工作树 | 9 个工作树在原 HEAD 分离，目录及其中 `target/`、ignored 测试资料完整保留；另一个原 detached 候选目录及主目录也保留，共 11 个工作树 |

上表远端资产大小合计 14,782,546,387 字节；这是已删除对象的报告大小，不代表 GitHub 账单即时减少额。清理不涉及本地验收原件、唯一证据、构建缓存或凭据。

| 已退役 tag | 原源码 |
|---|---|
| `v0.2026.10.13.15.44.oss_00` | `9d761130d70f…` |
| `v0.2026.10.13.16.19.oss_00` | `36abcbb5b027…` |
| `v0.2026.10.13.22.25.oss_00` | `f429fe2c0426…` |
| `v0.2026.10.14.01.46.oss_00` | `5360f5d6cf7e…` |
| `v0.2026.10.14.13.00.oss_00` | `5acb349c7032…` |
| `v0.2026.10.14.16.45.oss_00` | `5068dae9c75b…` |

| 已清理分支 | 原本地头 | 原 origin 头 |
|---|---|---|
| `codex/cli-input-default-20261014` | `b4e59a8cda49…` | 无 |
| `codex/fix-byop-cli-routing-20261013` | `5360f5d6cf7e…` | `5360f5d6cf7e…` |
| `codex/fix-linux-deb-cleanup-20261014` | `5068dae9c75b…` | `5068dae9c75b…` |
| `codex/fix-windows-uri-20261014` | `422734eee128…` | `203d08e50534…` |
| `codex/macos-gui-smoke-fixes` | `8deb402c32d1…` | `8deb402c32d1…` |
| `codex/release-candidate-20261013` | `36abcbb5b027…` | 无 |
| `codex/release-candidate-20261013b` | `f429fe2c0426…` | 无 |
| `codex/release-candidate-20261014` | `5360f5d6cf7e…` | 无 |
| `codex/release-musl-fix` | `36abcbb5b027…` | `36abcbb5b027…` |
| `codex/upstream-essence-2026-10-19` | `ea8f136605a8…` | `ea8f136605a8…` |
| `codex/windows-uninstall-entry` | 无 | `2e3406e7bce3…` |

Windows URI 分支本地比远端多一个文档提交，但两者都已在 main；CLI 输入默认行为分支也已完整进入 main。远端引用使用精确 SHA lease 与原子 push 删除；本地先核对 main 祖先关系，再解除 worktree 分支占用并删除引用。保留 `upstream/*`、`review-zap/main` 及 `archive/acceptance-linux-v01-v02-v05-2026-10-13` 等历史来源，未按名称泛删。

## 保留与恢复

10 个已公开 Release 的正文、tag、发布时间和资产 ID／名称／大小／digest 均与清理前一致；146 个其他 Actions artifact 保留。候选工作流仍保留运行记录，未重写七次 CI 失败或任何旧验收结果。

[恢复归档](/Volumes/SanDisk/InfiniShell-Archives/release-retirement/2026-10-19/README.zh-CN.md)包含完整前后清单、原 Release 元数据、6 份候选工作流元数据与 release-body ZIP、Git bundle 和逐项删除回执。`retired-refs.bundle` 已通过校验，保存原 tag 对象与本地／origin 分支头。[24 件资产恢复清单](/Volumes/SanDisk/InfiniShell-Archives/release-retirement/2026-10-19/release-assets-recovery.json)已重新完整读取原件 SHA-256，全部与 GitHub digest 匹配；当前 R9 Setup 使用 `c18725c5…` 原件，旧 `1ad…` Setup 仍仅保留为历史。

没有为了清理重新复制或重新验收旧包。本地原件和证据继续保留；42 个 Actions 中间构建 ZIP 不承诺全部可逐字节恢复。重新上传资产可恢复字节及可编辑元数据，但不能恢复 GitHub 原 ID、时间与下载计数。恢复本地旧 ref 不等于允许再次推送旧 tag 或公开发布。

## 后续正式发布入口

1. 从届时的 main 冻结新的产品源码和唯一新版本；本次未选择新版本，也不复用已退役 tag 作为新的候选身份。
2. 沿用[已整合源码及验证结论](/Volumes/SanDisk/github/InfiniShell-Desktop/docs/upstream-essence/2026-10-19-release-integration.zh-CN.md)，重新绑定正式三端桌面、最终 Windows Setup、六份 SSH daemon 及内嵌资源和 12 件资产。Windows Setup 的最终签名状态须按实际结果记录，原 R9 `NotSigned` 不改写。
3. 新包按影响补验：共用更新器同版／旧版／新版、失败和重试的英中 UI；Agent 文件操作与有序输入；Windows PATH／卸载／非 BMP；真实 SSH 删除／移动／撤销。原 Mac、Windows R9、Ubuntu Wayland 生命周期及本轮 Mac AX 结论仅绑定各自原字节，复用未变部分，不要求立即重跑已完成的旧包验收。
4. 正式构建后完成最终资产回下载摘要、签名、公证及装订核验；新候选先保持 Draft，公开发布另等用户明确安排。旧 CLI V01／V02／V05 延期、G09 关闭、Linux 自动门禁和既有非阻塞跟进不变。

`CURRENT_STATUS.json` 的 `main_acceptance.release_retirement_20261019` 是当前候选存续状态入口；原 `current_release_candidate`、历史候选和 `release_integration_20261019` 保留为原阶段记录，不能据其内部 Draft 字段推断候选仍在线。

## 本轮验证

`cargo check --locked -p warp` 已通过，保留 5 条既有警告。检查在已保留的 `upstream-essence-2026-10-19` 分离工作树执行，HEAD 与本轮文档修改前 main 相同（`ea8f1366…`），产品源码未变；[运行回执](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-e0f7ff5b.json)记录退出码 0、源码未变及临时目录清理完成。JSON、文档差异和历史记录保留检查见[文档验证回执](/Volumes/SanDisk/InfiniShell-Archives/release-retirement/2026-10-19/documentation-validation.json)。

[独立复核](/Volumes/SanDisk/InfiniShell-Archives/release-retirement/2026-10-19/postcleanup-independent-review.json)重新核对了远端删除差集、24 件本地原件的完整 SHA、可恢复 Git 引用及工作树目录身份。目录与 ignored 条目保留检查不等同于对全部 `target` 缓存逐字节审计。本轮检查只证明清理和文档收尾，不扩大旧包验收范围。
