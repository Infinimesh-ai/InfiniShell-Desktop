# 2026-10-19 发布修复整合续记

**本轮源码已在 main 吸收并推送，受测源码为 `8b46bcc3…`；第八轮 Windows CI 成功，readonly、PATH／reissue、通知链及四组 Rust 定向门禁通过。七次历史失败保留。Mac 更新器英中旧版拒绝、失败／重试及英文恢复已在冻结运行包限定范围通过，相关进程已清理；中文失败 AX 公告未捕获，实际 SSH 与正式新包未验，未公开发布，保持 Draft。** [第八轮 39256631029](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39256631029) actual head 为 `8b46bcc33c8c9d69a6bbecc8534edc39001ca867`；精确计数和限制见[审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39256631029/evidence-review.json)。源码吸收和自动门禁通过不等于新包验收通过。

本记录承接[原多端验收梳理](/Volumes/SanDisk/github/InfiniShell-Desktop/docs/release-acceptance-review-20261019.zh-CN.md)及[本轮上游巡检和吸收报告](/Volumes/SanDisk/github/InfiniShell-Desktop/docs/upstream-essence/2026-10-19.md)。已审 upstream 断点仍为 `ae60375ef1d3895908756878fe4a07cbe703ce1a`，没有扩大上游扫描范围。旧包验收只属于原字节，不转记为本轮新源码或新包通过。收拢前的五份未提交文档已完整保存在[进度快照及 SHA-256 清单](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/document-progress-snapshots/20261019T121920.731222Z/manifest.json)；原始失败证据及已提交历史不改写。

## 源码与包身份

实施分支为 `codex/upstream-essence-2026-10-19`。[源码合入回执](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/main-source-integration.json)确认：`2026-10-19T13:39:46.512850Z` 根仓库 main 从 `1e8142d6…` 快进并推送到 `8b46bcc33c8c9d69a6bbecc8534edc39001ca867`，当时 local／origin／remote 一致且根工作树干净。相对已审 upstream `ae60375e…` 的双向提交数为 main 侧 1535／upstream 侧 532，共祖 `b076027deb2e8d1b98e1073f9cd0f20fbf75a8ae`。[PR #26 回执](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/pr26-main-integration.json)为 `MERGED`，mergedAt 为 `2026-10-19T13:39:46Z`，mergeCommit `fd4c75fb…`；`isDraft=true` 只是保留元数据。原[文档收尾回执](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/final-closeout.json)已绑定 main／实施分支 `d18f9b354a53e64718fb3878d23feb51b617bc43` 与受测产品源码 `8b46bcc3…`；该回执中的 GUI 待验状态是当时快照，后续状态见本页 GUI 续验。本次仅续记文档，不改写原件。

| 对象 | 固定身份及范围 |
|---|---|
| Windows 来源整合 | `fd4c75fbf0a5dc0cd246aa957abe01ead60f6200`，第二父为 PR #26 的 `2e3406e7bce33fab7fc292622c68239db77e0e88` |
| 更新器产品源码 | `84fbb124744ecfd6d0044b6498e304b685b05848`；已随受测源码进入 main |
| 当前 readonly 修复源码 | `8b46bcc33c8c9d69a6bbecc8534edc39001ca867`；只改 `crates/warp_files/src/lib.rs` 与 `lib_tests.rs`，已进入 main；第八 Windows readonly 原生回归通过，PATH helper 不变 |
| 已验 PATH 修复源码／helper | `046f429aaa371893e26146a55c8f8b047dcb2113`；helper SHA-256 `0c56bb22d41007001f4a899502bd6f737f949629aaa87407d7d2c6ea91bae62a`；四处显式 `String` 和非 BMP 目标回归已在第七轮 PATH／reissue 通过；整轮另因 Rust readonly 失败 |
| 第六轮受测诊断源码 | `e4739c4748249edf2012c63130afcf0aec41fcf2`；在第五轮 `69d7…` 上仅增加测试诊断的只读 predicate；生产 helper、Rust 和资源不变。本行仅为第六失败的受测身份，当前修复身份见上一行 |
| 六轮失败所测 PATH helper | SHA-256 `d90a0a529d19c1aca5a329c0b9928434702f7a15f5f3f981dd0ac986a6af5fd9`；六轮测试诊断未修改它；当前修复 helper 摘要已更新，见上一行 |
| 原应用及 tag | 应用 `5068dae9c75b79628865a0872684113647c204d8`；tag 对象 `27d40029759b541dc8903a0deb9ec0948e9670fa`；版本 `v0.2026.10.14.16.45.oss_00` |
| 已受验 R9 Setup | 安装器源码 `a4e279c745bbd67aed1f8a926f45a32bda9343c8`；580325911 字节；SHA-256 `c18725c5326ce74e3407957606e7a55691a2eec28b755112a7e7ae29d170e503`；`NotSigned` |
| 本轮 Mac 运行包 | 产品源码 `84fbb124…`／冻结 checkout `661f3359…`；arm64、Apple Development、签名严格校验通过、未公证；只是定向运行 ZIP，不是正式 DMG |
| Mac 运行包实际字节 | EXE SHA-256 `5d3416466701497f26ff340c032e5fdeb2ca02e7df56e21884f7f9c1cb034649`；ZIP 156973880 字节、SHA-256 `27749685b0209e54dc7f0ec6a990d442052d2698701fe2730cca455099c24248`；[binding](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/macos-update-gui/binding.json) |

Draft `412823983` 已仅将旧 Setup asset `640654138` 换为受验 R9 asset `655995368`。`2026-10-19T10:49:59.681054+00:00` 回下载的大小和 SHA-256 与原件／provider 一致；其余 11 件的 ID、大小和摘要，以及 tag／应用源码均未变，公开 Latest 仍为 10.07，`draft=true`、`published_at=null`。见[R9 回下载收据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/r9-roundtrip.json)。

**R9 内应用仍为 `5068dae9c…`，不含本轮 updater 和上游整合。** 使用加强后的 PATH helper 或本轮产品源码生成任何新 Setup，都须冻结新字节并独立验收；不能继承 R9 整包结论。Mac 运行 ZIP 没有注入正式发行的六份 remote_server 载荷、没有生成发行 licenses，也不继承历史 DMG 的签名公证或多端验收。

## 本轮修复与本地化

[PR #26](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/26) 源码审核发现并修订两项问题；修订已进入 main；PATH／reissue 在第七、八轮通过，第八 Windows job 整体成功，第七后续 Rust 失败原件保留：

| 发现 | 本轮处理 | 边界 |
|---|---|---|
| P2：PATH 原始字节读取不完整 | 原 `RegQueryStringValue` 忽略原始 `cbData`，裁去尾 NUL 后写回补一个 NUL，可能改变零字节空值、缺终止或重复 NUL 的字节。改为 `RegQueryValueExW`，同句柄核对类型、长度、单个终止 NUL 及代理对完整性；不规范值在写 PATH 和 owner 前拒绝 | 保留异常值与有效代理对真实夹具；不能把源码用例数当作通过数，也不能撤销 R9 原场景的历史结论 |
| P3：重发门禁遗漏清理结果 | 将 `positive_test_registry_cleanup_passed` 纳入 `cleanup_passed` 和最终 status；从实际 AST 提取门禁做正负回归 | 保留 R4 失败负例；第七轮 reissue 已通过，负对照与未做桌面验收边界保留 |

`RELEASE-UPDATE-ORDER-01` 修复 macOS／Windows／Linux 共用更新链路：解析入口归一化可选单个 `v`，支持实际 `v0.YYYY.MM.DD.HH.MM.oss_NN` 与旧版本格式；严格校验日期、补丁和完整输入。`GithubRelease::version` 先校验再沿用原 URL 前缀约定。`should_update` 传播解析错误，拒绝同版及普通旧版；只有既有显式 `is_rollback=true` 且成功比较时才允许旧版，不新增自动降级入口。

检查错误进入 `CheckFailed`，不宣称最新、不发出可用更新事件、不将错误响应写入 `downloaded_update`，关于页及无障碍反馈提供失败和重试语义。GitHub 响应元数据缓存仍会在获取响应时写入；本轮未修改所有下载／安装失败的完整状态链。历史 Ubuntu 复现只证明旧版被误标新版，用户未点击更新，不记为实际降级。

英文和简体中文同步四个无变量消息键：`settings-about-update-check-failed`、`autoupdate-a11y-available`、`autoupdate-a11y-install-help`、`autoupdate-a11y-no-updates`。失败文案为 “Could not check for updates. Please try again.”／“检查更新失败，请重试。”；i18n 门禁已过；本轮 2560×1600 真实界面已核对英中无更新、检查中、失败及重试入口，无截断或重叠。英文失败 AX 已捕获，中文失败 AX 公告未捕获，后者不标通过；不扩大为 VoiceOver 语音或所有窗口尺寸。PATH helper、诊断及门禁没有新增用户界面文案，无需本地化变更。

## 已通过门禁与清理状态

| 范围 | 实际结果 | 证据与限制 |
|---|---|---|
| 更新器定向 nextest | `r-29864436`：61／61，8945 未选 | [收据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-29864436.json)及[输入绑定](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/updater-local-test-binding.json)；`channel_versions`／`autoupdate`／`i18n`，非全库 |
| 显式 i18n | `r-384c44cd`：`cargo test --locked -p warp --lib --features warpui/test-util,rust-embed/debug-embed i18n::tests`，12／12，8968 未选，exit 0 | [收据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-384c44cd.json)；代码摘要对应 `84fbb124…`，运行中仅两份状态文档变化；清理见下表 |
| 本地编译检查 | `r-9ce93977`：`cargo check --locked -p warp`；`r-700c727e`：`cargo check --locked -p warp_tui`，均 exit 0／cleaned | [warp](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-9ce93977.json)／[warp_tui](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-700c727e.json)；保留既有警告 |
| readonly 修复本地门禁 | `r-448defe1` warp_files nextest 33／33，0 skip；`r-85fd318a` `cargo check --locked -p warp` 通过，保留 5 条既有警告；均 cleaned | [输入绑定](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/readonly-fix-local-binding.json)及[33／33](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-448defe1.json)／[check](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-85fd318a.json)；两份 Rust 摘要前后与提交一致。check 期间仅两份 specs 文档变化；Mac 本地结果不替代 Windows 分支原生验证 |
| Linux CI | [39242160374](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39242160374)，head `84fbb124…`，五组 nextest 9／585／525／22／801 全过，合计 1942 次执行；skip 21／6／7／114／9055 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39242160374/evidence-review.json)；定向过滤及既有 ignore，非全库／非独立用例总数；801 内含 i18n 11，不另加总。Python 131 中 129 过、2 Windows-only skip；DEB 1 项及双 musl check 通过。84→046 的复用范围由[源码等价收据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/046f429aa-source-equivalence.json)绑定，未改变 Linux Rust／资源 |
| Windows PATH／非 BMP | 第八轮：1683 断言／366 verified case；1438 completed／1438 attempted actions、2572 step，200 矩阵／38 edge；30 raw case／90 assert，含 28 项异常拒绝及 2 项有效 UTF-16 往返；source gate 14 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39256631029/evidence-review.json)；16 项 fixture 预检类型／字节一致且清理通过；完整 PATH 清理通过，真实用户 PATH 未变；原路径及目标路径非 BMP、REG_SZ／REG_EXPAND_SZ 四个最终夹具已核对 |
| Windows reissue | 40 正例／2 负对照，342 actions／342 独立快照、2476 assert、source gate 4；清理及正例注册表清理通过，负例证据保留 | 同一[审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39256631029/evidence-review.json)；两负对照实际 1 fail／1 pass 均按既有历史边界保留。仅证明执行上下文 HKCU PATH 未变，未验证交互桌面 PATH；没有构建真实产品安装器 |
| Windows 检查／通知链 | cargo check 通过；登记 80 断言且清理通过；通知 Python 128 中 123 过／5 Unix-only skip；Codex 0.147.0／0.156.1 各 5 hooks 信任验证通过 | 同一[审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39256631029/evidence-review.json)；原生配置未变，未发模型请求；四组 Rust 与 app／i18n 已在第八运行通过；没有真实桌面或新产品安装器验收 |
| Windows Rust／readonly／i18n | 第八四组 574／528／11／789 全过，合计 1902 次执行；skip 9／7／116／8602。应用组 warp 779＋TUI 10，内含 i18n 11；readonly 回归通过 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39256631029/evidence-review.json)及[readonly PASS 日志](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39256631029/windows/crate-tests.log:498)；1902 非唯一测试数或全库。后两组 skip 含筛选与既有 ignore，2 项真实 SSH E2E 仍 ignore。compiler 准备 14 过／1 链接权限 skip，重发控制器 6／6 |
| TUI 真 PTY 冒烟 | `r-375a0354`：120×36 PTY 启动／资源、`?` 展开收回、显式 shell 固定 marker `printf`、空输入 Ctrl-D；TUI／tmux exit 0，run cleaned | [binding](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/tui-smoke/binding.json)／[result](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/tui-smoke/result.json)；实际 EXE SHA `03cae552c2dfdf3f4ac7aacd2e04850ec312c6876a5809c3da93d3b8d6d99ea7`。未发模型请求，非真实 SSH／GUI 验证；3 个 profile 目录保留 |
| Mac 构建与冻结 | `r-426c5f7f` release-lto 构建 exit 0；`r-f1cd2cff` 包冻结 exit 0／cleaned | [收据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-426c5f7f.json)／[收据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-f1cd2cff.json)；本行只证明构建与身份；后续 GUI 限定结果见下文独立续验 |

当前修复的三份本地收据及日志摘要已核对：[`r-a60f92db`](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-a60f92db.json) 的 gate 11 case／14 assert 通过；[`r-612224fc`](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-612224fc.json) 的实际 PowerShell AST 和 2 项 Unicode 目标 fixture 注册通过（未执行 Windows API）；[`r-c779bc6c`](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/r-c779bc6c.json) 的控制器 3 过／3 Windows-only skip。三轮均 `cleaned`，独立源码 review 通过；本轮无用户文案变化，无需本地化变更。

本地 PowerShell／AST、原失败源码负对照、控制器及 actionlint 结果按各自收据保留在 [机器可读整合记录](/Volumes/SanDisk/github/InfiniShell-Desktop/specs/cli-agent-parity/CURRENT_STATUS.json)，不替代 Windows API 实跑。第五／六诊断所用目标为 1635 断言／364 case、raw gate 14，未全部通过；当前修复新增 `4 + (22 × 2) = 48` 项，经独立复核，第七、八轮均实际完成并通过 1683 断言／366 case。工作流仅前移 Windows Python／PATH 步骤，19 步骤正文、条件、timeout 及其他 jobs 保持等价，见[顺序证据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/windows-path-step-order.json)。

| 原运行 | 原始保留状态 | 当前独立复查 |
|---|---|---|
| `r-426c5f7f` | 原收据 `cleanup=false`，受追踪子进程 55365 尚未退出 | 子进程退出后核对身份、所有已记录进程、launchd 和 lsof，[复查为 cleaned](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/r-426c5f7f-cleanup-recheck.json)；原件不改 |
| `r-384c44cd` | 原收据 `cleanup=false`，Device Hub launchd 身份未确认 | 原 labels 已消失，进程／lsof 为空且 owner／device／inode／祖先复核后，[复查为 cleaned](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/r-384c44cd-cleanup-recheck.json)；原件不改 |
| `r-b2fe8bf4` | 第二次 TUI 启动 exit 101，Little Snitch updater launchd 身份未确认，原 `cleanup=false` | [复查为 cleaned](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/r-b2fe8bf4-cleanup-recheck.json)；失败 pane 仍证明 feature 包选择问题，不改成产品故障或启动成功 |
| `r-7e0c826e` | 原启动曾等待钥匙串；原 `source_unchanged=false` 保留 | 用户处理后完成英中旧版拒绝，exit 0／cleaned；冻结 EXE 未变，[最终续验](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/macos-update-gui/resumed-validation/resumption-final.json)按字节身份计证 |
| `r-3a089b95` | 故障客户端重启曾再次等待钥匙串 | 用户处理后完成英中失败／重试及英文恢复，exit 0／cleaned；两次 503 夹具与恢复隧道三轮均 exit 0／cleaned，端口已关闭 |

TUI 前两次 exit 101 的原始收据保留；第三次额外选择 `-p warp` 后完成上述限定验证。重复 i18n／重叠运行不加总为新增独立覆盖。

第八证据 ZIP 为 3164162 字节，provider／本地 SHA-256 均为 `2093e1bc8c12b71bdd99822ea2d8e2fd61e218cd38281053506918d9b93b94e9`，artifact ID `12191526444`；它是测试证据，不是产品安装器。readonly 回归已覆盖恢复 readonly、无 TEMPORARY 属性、碰撞时既有内容／readonly 保留及无自持临时残留，见[审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39256631029/evidence-review.json)。

## Windows CI 失败简表

以下七轮均为当时失败快照，原件不改写；当前第八轮已成功。前七轮整体 CI 均已完成失败；前六轮的工件、日志、人工夹具快照和失败 HKCU 根保留，主测试 `cleanup=false` 与独立预检清理分别计证。第七轮 PATH／reissue 已通过并清理，转而在后续 Rust readonly 恢复断言失败。所有结论保留各自范围，不用部分通过替代未跑门禁。

| 轮次／实际 SHA | 首个错误 | 已执行范围及未跑边界 | 原件 |
|---|---|---|---|
| 1：[39242160374](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39242160374)<br>`84fbb124744ecfd6d0044b6498e304b685b05848` | 新夹具将 `SafeRegistryHandle` 传给 C# `IntPtr`，helper 调用前绑定失败，不能据此判生产 helper 故障 | Inno 1615、38 edge、218 completed actions／224 step；raw 0、矩阵 0。登记 80 过，compiler 准备 14 过／1 符号链接权限 skip；之后 reissue／Windows 通知和 Rust／i18n 未跑 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39242160374/evidence-review.json)；失败根 `fe2125473d34408996216eda07c59165` |
| 2：[39244460247](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39244460247)<br>`43388041d86a95ecf419e944c0865fef8f1f8561` | `missing-nul-add` 夹具读回不匹配，在 helper 前，旧记录无 actual 字节，具体原因未定 | source gate 4；zero-bytes add/remove 2 case／6 assert；Inno 1615、222 actions／226 step。后续 raw／矩阵／reissue／Windows Rust 未跑 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39244460247/evidence-review.json)；失败根 `36a237db8f784d82b9a7d444161a968b` |
| 3：[39245777003](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39245777003)<br>`661f335938ed8676d000faa521c34fcdb94866b8` | helper 前 `REG_SZ C:\raw`：预期无 NUL 的 12 字节，读回附一个 NUL 的 14 字节；当时尚未分离写入／读回边界，不归因 Out 或 OS | source gate 8；zero 2 case／6 assert；Inno 1615、222 actions／226 step，后续门禁未跑 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39245777003/evidence-review.json)；失败根 `cf78c9a21b60497395a24720d749e944` |
| 4：[39247315588](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39247315588)<br>`8e55aabdbaf33143c1636b98be039a160268630e` | 有效代理对 add A 预期成功却失败，根因当时未定。`native_actions=246` 是完成数，失败是尝试 247；246 是此前 low-surrogate-remove 清理成功 | gate 11；16 fixture 类型／字节全匹配且预检清理通过；REG_SZ 14 个异常 add/remove 拒绝，共 42 raw assert；Inno 1615、238 step。完整矩阵／reissue／Windows Rust 未全跑 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39247315588/evidence-review.json)及[失败 action-00247](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39247315588/windows/windows-path/full/action-00247.txt)；失败根 `d039d9da36ed45beb1241b21d047ae6e` |
| 5：[39249448503](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39249448503)<br>`69d7f0af0d39b3219c4c703ea7ab279c0ad45f5d` | Inno 5 项断言后 `unicode-roundtrip-1: add` 返回 false；初始及写后 PATH／元数据的合法 UTF-16 读写已证实，Projection 恢复／移交的具体 predicate 未定 | gate 14；16 fixture 精确往返且预检清理通过；native completed／attempted 均为 0。REG_EXPAND_SZ Unicode、剩余 1635 基线、1438 action 矩阵、raw production 边界、reissue、cargo check 和 Windows 通知／Rust／i18n 均未跑 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39249448503/evidence-review.json)；失败根 `1799f1c320dd4908a7fb9473af15416b` |
| 6：[39250927363](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39250927363)<br>`e4739c4748249edf2012c63130afcf0aec41fcf2` | 首个 false 为 `original-value.allowed=0`；此前读取／字段解析／类型与元数据判断成功。源码核对定位到内联 `Copy` 比较的 ANSI 临时转换，修复待验 | gate 14；16 fixture 精确往返且预检清理通过；Inno 5 项，native completed／attempted 均 0，后续完整门禁未跑 | [实际 job](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39250927363/windows-job.json)、[结果](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39250927363/windows/windows-path/full/result.json)、[predicate 日志](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39250927363/windows/windows-path/full/inno.log)；失败根 `f2bc1ee295c84a7ba19f5c968e69c1ca` |
| 7：[39252231214](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/39252231214)<br>`046f429aaa371893e26146a55c8f8b047dcb2113` | Rust `warp_files::tests::restore_preserves_readonly_and_cleans_its_temporary_file_after_collision` 在 `lib_tests.rs:352` 的 readonly 断言失败；审核定位 tempfile 3.23.0 Windows `persist_noclobber` 清掉属性，修复正在实施 | PATH／reissue／通知链及 cargo check 已通过；Rust 执行 574，573 过／1 失败／9 skip。迁移与同步 crate 组、v2 补全、app／i18n 后续组未跑，非全库通过 | [审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39252231214/evidence-review.json)；完整原件保留；未构建或验收新产品安装器 |

第五轮原件还证明：初始字符串长度 8，代理对 Ord 为 55357／56832；REG_SZ 原始 18 字节且单 NUL。add 后 PATH 为 68 字节，原非 BMP 文本与目标均在；`ReadEnvironmentValue` 对 Path、State、Projection、OriginalValue、AddedPath 返回 true，Pending 不在。Projection 留存，但 7 个字段按 UTF-16 单位正确解析、owner SHA 匹配目标。这些是第五轮的证据边界；当时尚未定位具体 predicate。第六轮进一步在 `original-value.allowed=0` 停止，前序读取与解析均成功。


第六轮后的两份独立源码复核定位了固定 Inno Setup 6.7.1 所用 PascalScript `d6e25439…` 的行为：`Copy` 声明返回 `AnyString`，其基类型为 ANSI `btString`；比较表达式先按声明类型分配临时值，后续虽按输入生成 Unicode 结果，仍复制回 ANSI 临时值，导致非 BMP 字符不能保真。显式 `String` 局部接收 `Copy` 可复用 Unicode 目标，避免这一步转换。依据为固定[编译器声明](https://github.com/remobjects/pascalscript/blob/d6e25439080c37e15af23badb48c39760a2b8d4d/Source/uPSCompiler.pas#L12458)、[Copy 返回声明](https://github.com/remobjects/pascalscript/blob/d6e25439080c37e15af23badb48c39760a2b8d4d/Source/uPSCompiler.pas#L13113)、[临时分配](https://github.com/remobjects/pascalscript/blob/d6e25439080c37e15af23badb48c39760a2b8d4d/Source/uPSCompiler.pas#L6319)和[结果寄存器处理](https://github.com/remobjects/pascalscript/blob/d6e25439080c37e15af23badb48c39760a2b8d4d/Source/uPSCompiler.pas#L9463)。这与实际日志中原文／Base／存储值 UTF-16 完整、内联比较却为 false 一致；不是初始 UTF-16 读取、`TArrayOfString` 或 `Ord` 的故障结论。

四处完整字符串内联比较在 PR #26 原来源 `2e3406e7…` 的 [327](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/2e3406e7bce33fab7fc292622c68239db77e0e88/script/windows/environment.iss#L327)、[484](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/2e3406e7bce33fab7fc292622c68239db77e0e88/script/windows/environment.iss#L484)、[524](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/2e3406e7bce33fab7fc292622c68239db77e0e88/script/windows/environment.iss#L524)、[705](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/2e3406e7bce33fab7fc292622c68239db77e0e88/script/windows/environment.iss#L705) 行已经存在，R9 reissue 也固定使用 6.7.1。**这是扩大非 BMP 覆盖后发现的既有边界，不是本轮原始 UTF-16 读取修订引入。** R9 六阶段通过仍只属于已测场景，不能宣称旧 R9 已覆盖非 BMP。当前四处显式 `String` 局部和非 BMP 目标路径回归已在 `046f429aaa371893e26146a55c8f8b047dcb2113` 推送，helper SHA 见身份表；第七轮实际 PATH 与 reissue 回归已通过，但整轮在后续 Rust readonly 恢复测试失败，不扩大为新包或全部门禁通过。

[源码等价收据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/046f429aa-source-equivalence.json)证明 `84fbb124…` 到 `046f429a…` 仅 9 份 Windows／测试／工作流文件变化，`app`、`crates`、`resources`、Cargo 清单与锁文件未变。因此只按既有范围复用 Linux 产品测试、Mac 更新器本地 Rust／i18n、已冻结 Mac 运行字节和 TUI 限定 PTY 输入；Windows helper 已在第七轮按上述范围独立原生验证。此等价收据仅截至 `046f429a…`，readonly 修复的条件编译等价与原生结果另见后文；源码等价本身不授予 GUI 或任何新包通过；冻结 Mac 运行包的限定 GUI 结果另由下文实际续验计证。

第七轮的 Rust 失败与前六轮 PATH 失败不同。[审核原件](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39252231214/evidence-review.json)定位 `restore_local_file` 在 `NamedTempFile.persist_noclobber` 前设置 readonly，而锁定的 tempfile 3.23.0 Windows 实现在移动前将属性设为 `FILE_ATTRIBUTE_NORMAL`，导致目标 readonly 丢失。Windows 专用修复已在 `8b46bcc3…` 进入 main，独立 review 和第八原生 readonly 回归通过；迁移／同步、v2 补全、app／i18n 均在第八补跑成功。第七的 9 项 skip 为 2 项显式忽略的真实 SSH、4 项既有补全、3 项 Windows 路径分隔补全，原失败及未跑快照仍保留；现态见[第八审核](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/ci-39256631029/evidence-review.json)。

修复使用 Windows `Builder.make_in` 配合普通 `create_new File`，先写内容并设置 readonly，再执行不覆盖目标的原生 rename；成功后禁用旧临时路径清理，失败时只通过自持句柄清除 readonly 后清理。Unix 仍沿用 `persist_noclobber`。原 readonly／碰撞测试保留，新增 Windows `FILE_ATTRIBUTE_TEMPORARY == 0` 与碰撞后既有 readonly 不变断言。源码独立 review 通过，且无用户文案变化，无需本地化变更；没有新增崩溃持久性保证，临时文件清理仍属尽力处理，不扩展到恶意同权限父目录替换保护。

[8b46 输入等价收据](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/8b46bcc33-source-equivalence.json)记录：两份 Rust 文件 blob 已变化，但 Unix 条件编译后的临时文件创建、写入、权限与 `persist_noclobber` 行为经两次独立 diff 复核等价；不能写成文件字节完全一致。Linux 原产品测试和 Mac 更新器既有门禁按原范围复用，冻结 Mac 运行包与 TUI 冒烟继续绑定各自原字节，没有重建为 `8b46…` 包。check 收据的 `source_unchanged=false` 仅因两份 specs 导航变化，相关 Rust 前后摘要一致，见[本地输入绑定](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/readonly-fix-local-binding.json)。

## Mac 更新 GUI 续验（本轮运行包）

[最终续验回执](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/macos-update-gui/resumed-validation/resumption-final.json)记录于 `2026-10-19T14:12:40.814640+00:00`，SHA-256 `6a75e6c7bab2583c827165c96215c41718b893eb88453e2d412add37da758eac`。本轮使用上表同一冻结 EXE 与 Apple Development 运行 ZIP，未重建成正式候选；续验开始时 `main` 为 `d18f9b354…`，产品受测源码仍为 `8b46bcc3…`。此前钥匙串等待与[第一阶段回执](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/macos-update-gui/resumed-validation/resumption-phase-1.json)均为历史快照；用户已完成两次处理，当前无需用户待操作。

| 本轮实际范围 | 结果及边界 |
|---|---|
| 英中旧版拒绝 | 手动检查后显示 `InfiniShell is up to date.`／`已是最新版本。`；AX 分别为 `No updates available.`／`暂无可用更新。`。公开 Latest 10.07 来自独立 GitHub API 观察；应用为 10.14 |
| 英中失败与重试 | 英文 `Could not check for updates. Please try again.`、中文 `检查更新失败，请重试。` 均保留检查更新入口；点击后分别捕获 Checking／正在检查更新，本机单次端点收到 `CONNECT api.github.com:443` 后延迟 5 秒返回 503，界面回到失败并可再次重试 |
| 无障碍及恢复 | 英文失败 AX 与英文恢复后的 `No updates available.` 已捕获；中文失败 AX 快照没有公告值，不标该项通过。英文恢复使用同进程重试，经只允许 `api.github.com:443` 的本机 TLS 密文直通隧道恢复真实请求，未解密 TLS 或保留请求头／正文 |
| 双语布局 | 2560×1600 原始截图中，本次状态、检查中、失败和重试入口无截断或重叠；切换语言后重建设置页，不外推全应用免重启切换、VoiceOver 语音或所有尺寸 |
| 退出与恢复 | 两个 GUI 及三轮夹具均正常 exit 0／cleaned，夹具端口关闭；GUI 恢复 System 后设置与原始 3543 字节完全相同，SHA-256 `6842e3695fcf85b847358c0d800a59b83650446a0834d0529771cb13d4a6dc52`，原不存在的代理字段仍不存在；自动化未修改系统网络或钥匙串设置，用户手动响应后的整个钥匙串状态未审计 |

[独立复核](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/macos-update-gui/resumed-validation/resumption-independent-review.json)通过并保留两项限缩：钥匙串结论仅指自动化未修改设置；夹具现存脚本已审，但原 run 只绑定路径与日志，未锁定运行时脚本 SHA，不倒记为当时锁定。23 件截图／AX 原件的摘要与长度均已复核；截图接口实际返回 JPEG，保留原始 `.png` 文件名和字节。`RUST_LOG=warn` 未保留应用解析版本的日志；手动 No updates AX 与冻结源码支持本次检查未进入新下载分支，不冒称网络抓包或整个历史进程／缓存没有下载。同版、新版、显式回滚和非法版本仍按源码单元测试计证，不修改正式 Release 元数据制造场景。`r-7e0c826e` 的原 `source_unchanged=false` 不改写，冻结 EXE 摘要前后一致；后续 `r-3a089b95` 及三夹具摘要按各自收据保留。

[GUI 后远端回执](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/macos-update-gui/resumed-validation/remote-after-gui.json)确认 PR #26 仍为 `MERGED`，Draft 仍 `true`／`published_at=null`，12 件资产的 ID／名称／大小／digest 与原收尾一致，tag 对象及 peeled 源码未变。原 `final-closeout.json` 保留不覆写；本次限定 GUI 通过不授予新三端正式包或实际 SSH 验收通过。

## 待验与发布边界

- 源码与前轮文档已主线吸收，定向自动门禁已闭合；本次 GUI 续记的实际文档提交与远端状态由独立[续验收尾回执](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/gui-resume-closeout.json)绑定，不覆盖原收尾。七次失败现场与原件保留，不把第八定向结果扩大为全库、实际 SSH 或正式包通过。
- 本轮 Mac 更新器限定 GUI 已完成，设置字节恢复、两次 GUI 与三个夹具清理均已核实，无钥匙串待操作。中文失败 AX 公告未捕获，不标该项通过；正式三端候选仍按下表补验，不能以本轮运行 ZIP 或源码单元测试替代最终包。
- 原 Mac 签名 DMG、Windows R9、Ubuntu 24.04 GNOME Wayland DEB 六阶段及原资产验收按[原多端验收梳理](/Volumes/SanDisk/github/InfiniShell-Desktop/docs/release-acceptance-review-20261019.zh-CN.md)保留；任何新正式候选仍须绑定源码、包字节、签名和受影响的多端补验。当前运行 ZIP、TUI 冒烟或 R9 资产替换都不能关闭新包验收。
- 旧 CLI `V01`／`V02`／`V05` 继续延期，`G09` 已关闭，Linux 自动检查保留。SSH 窄侧栏空态裁切、一次中文命令被识别为 Agent（Ctrl-I 可恢复）、Mac 中文“连字”搜索继续作为原非阻塞跟进。物理 IME 只按用户确认及提交／输出截图计证，不扩大为完整组合过程矩阵通过。
- 受测源码 `8b46bcc3…`、main 整合和第八 CI 已有回执；本轮限定 GUI 已另行续记，新 Setup 与正式三端候选的独立验收仍待后续安排。正式发布需要用户另行安排；保持 Draft，不预填发布或候选通过。

## 下次正式候选按影响补验

原包生命周期验收继续作为基线，不要求用户现在重复旧包六阶段；这些结论不能直接继承给新字节。正式构建、12 件资产核验、正式签名／公证及发布仍待用户另行安排。

| 受影响范围 | 新候选需要绑定与补验 |
|---|---|
| 三端桌面 | 绑定新桌面字节，定向核对更新器同版／旧版／新版、失败与重试的英中 UI，以及改动的 Agent 文件操作、输入顺序；handler／unit 通过不替代实际界面 |
| Windows Setup | 绑定包含新 helper 的最终 Setup，补 PATH／卸载与非 BMP 路径；原 R9 六阶段仅作原场景基线，不授予新 Setup 通过 |
| SSH daemon 与内嵌资源 | workflow 的六个 `infinishell-{macos,linux,windows}-{arch}` 压缩包是 remote-server，并非 Windows 桌面便携包。本轮 proto／server／FileModel 变化涉及六份新 daemon 及桌面内嵌资源，须绑定这些新字节并补文件删除／移动／撤销的实际 SSH 路径；现有 handler／unit 不等于真实 SSH 已验 |
| 正式发行集合 | 正式候选统一源码／版本／资产身份，核对 12 件资产及内嵌 daemon；按发布安排完成正式构建、下载摘要、签名、公证与装订，不以本轮开发签名运行 ZIP 或源码门禁代替 |
