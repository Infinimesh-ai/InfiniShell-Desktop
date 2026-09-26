# CLI 支持与能力对齐：验证结论

本报告保留截至 2026-09-27 的固定版本验收结论、源码来源和未验边界，不保存逐轮日志或截图。**这是阶段交付，完整 Goal 尚未完成。** 当前状态以 [CURRENT_STATUS](CURRENT_STATUS.json) 为准；剩余功能、优先级和关闭条件统一维护在 [KNOWN_GAPS](KNOWN_GAPS.md)。

目录整理本身没有修改产品或重跑历史测试；此后继续实现的输入增量及新证据单列如下。历史通过、失败与跳过仍按原提交计证；新增工作区收据也不能外推到最终 SHA、其他版本或全部平台。

**2026-09-27 远程图片恢复与 Windows npm 模板兼容**：`5aac3f7d0` 补齐 Claude 延迟审批后的图片状态查询／回收，以及 Grok 明确未派发取消后的主动重试。Claude 原生事件最多触发三次只读补查，固定 `2.1.280` 的原字节证明工具完成 hook 可早于最终历史写出；无精确回执仍保留 Unknown。Grok 仅在 Submit future 尚未创建时保存绑定完整 ticket／message／subject 的未派发终态，原领取字节保留；真正开始 RPC 后仍不重发，两路径均不清除新草稿。新 Grok 提示英中同步，布局待验。

`97649deb2` 修正真实 npm `10.1.0` 对命名空间 Windows 路径的递归问题，转换前核验同一文件对象；诊断采用固定官方 Node/npm 原字节的独立纯函数复现，未冒充 Windows 新安装通过。`bd80cc9ed` 另接 `cmd-shim 6.0.1` 与既有 8 的整组三件套精确校验，旧事务三份 SHA 固定模板，候选／退出／恢复不得切换到另一模板；产品仍复制真实入口原字节，不升级 Node/npm、不改写验收 shim，也不放宽 AppContainer。此部分无需本地化变更。

本地 Mac ARM64 第二轮编译检查、i18n 11 项、远程图片 36 项、更新模块 199 项（6 忽略）、footer 47 项、Grok 上下文 7 项及 actionlint 全通过，包含新增 19 项回归；Python 运行器 15 项分别在 3.12.13／3.14.6 通过。19 个冻结文件与 `bd80cc9ed9a6dbc032602e457a64d1c55e7998ba` Git blob 一致，绑定收据 SHA-256 `6265114b19a1c4b517ae4f56f7cf5ae2f00689b089f0c6ff63b6debc6129e1f3`；原始日志、固定 Claude 时序复核和清理收据位于 `validation/remote-image-recovery-20260927`。第一轮测试链接因空间峰值失败，832 字节坏产物及日志完整保留，未进入测试；第二轮通过不覆盖旧失败。仅清理确认停用的项目编译缓存，真实原生程序、应用和证据保留。

同提交 [Actions 36266539274](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36266539274) 已启动 Linux／Windows 定向验证与 Windows 真实 npm 五场景，结果待补；macOS Intel 明确排除。实际远程延迟 Allow／Read 回收、GUI 双语、认证模型及完整生命周期仍后置，G08／G09 与 Goal 不关闭。

新版 Mac ARM64 联测包已按 `8876fa3c5a3950154c8569ea1344507f38f964eb` 构建，其产品源码与 `bd80cc9ed` 完全一致；路径为 `/Users/zhishi/Library/Caches/InfiniShell-Desktop/cli-parity-apps/8876fa3c5/InfiniShellParity-8876fa3c5.app`。签名前摘要 `98244b1f7e9686e64f3c45381e5f4ae73260925f8303b850e00ccccae15d06de`，签名后摘要 `e3cf04b650d0b719075e45d270a5b03ea4c25f3bbe8d049d90cf7cd22b6d3423`；四份完整英中 FTL、131 项资源及 ad-hoc 签名核验通过。包装收据位于本运行 `prepared-app-8876fa3c5/bundle.safe.json`，摘要 `5ba79365f640e5e5da5f5133f792f31972609e32a6d4c6457b66ac63af3b8622`。应用未启动，独立 profile `parity-code-8876fa3c5`；未检查认证、未计 GUI／模型通过，旧包不动。此轮仅构建交付 app，默认 feature 的编译／i18n 门禁按同产品源码复用，不冒充 debug-embed feature 的运行测试。

**2026-09-27 Grok 上下文与升级空闲通知补接**：功能提交 `debed8c51cc54f1d017fda809c9254fbc658b80b`，独立夹具修正 `c7ef8d95b48df4a01f64985471b42e80beeda1eb`，均已推送。有效本地／远端专属 Grok 的代码、评审和 diff 上下文现在统一先打开富输入、等待原草稿恢复，再按同代次顺序追加；已经打开的专属会话也走同一队列，连续上下文不会抢在恢复前被覆盖。取消、关闭、换代或失效时旧回调不写入，未绑定普通会话不因此获得 PTY 发送能力。Grok 操作提示已同步英中，实际双语布局仍待共同验收。另将升级器既有 sessions model observe 对齐三支持平台，使 Linux／Windows 无存活 pane 的专属 Grok 清理后能够重新同步空闲条件；没有新增终端锁或第二份观察订阅。

本地 Mac ARM64 编译检查、i18n 11 项、Grok 视图 7 项、footer 47 项、更新模块 190 项（6 忽略）及 actionlint 通过，含四项新增顺序／旧回调／失效／未绑定回归。十份源码／资源／工作流 Git blob 与冻结快照一致，原始日志、独立静态复核和绑定位于 `validation/grok-context-routing-20260927`。旧 Mac `2b59c8c62` 联测包保留但不含本批 UI 修改，新包及真实三平台／远端发送仍待补；G01/G09 与 Goal 不关闭。

`56f6da217` 的 [Actions 36262495627](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36262495627) 已结束为失败：Linux／Windows 基础编译及定向 Rust 分别 958／818 项通过，Linux 固定 Node 原文件的只读依赖闭包 1 项通过。两平台离线合集原始日志均确认 `Reparse` 测试夹具缺 `st_mode`，与已提交的 `c7ef8d95b` 修正一致；CI 实际 Python 3.13.15。Windows 真实 npm 私有登记另返回 `Maximum call stack size exceeded`，尚未生成真实 shim 或进入产品升级事务，原因继续诊断，五场景不能记作执行。完整原始日志 ZIP 摘要 `17bd8e61eec57354bfc196ec02fe065fc91a7d30794c43bc11328cb9eb8494c0`，运行结论收据摘要 `174bda32c1dee1a1fa33f470dd38c20cd60c2aa54ef8d002ba2f3250364bb918`，统一归档于 `validation/g09-followup-20260927/ci-36262495627`。本轮 Windows 定向无 `LEAK`，不据此关闭旧运行的泄漏疑点。此运行不覆盖后续 `debed8c51`。

本地 47 脚本／1,164 项 Python 3.14 全通过，Python 3.12 独立复现上述夹具错误；仅修夹具后 11 项在两个版本通过，生产检查未改。此前诊断字段的 CI 3.11 推断已另存更正，原字节保留；原始批次与更正在 `validation/g09-followup-20260927/offline-batch-diagnosis`。本轮还按已授权范围，仅清理两份停用 Cargo 测试链接输出共 1,604,934,608 字节；身份、摘要和无占用均核验，原始验收程序／日志／源码／工作树保留，清理收据为同归档 `retired-test-link-cleanup-01.safe.json`。

**2026-09-27 G09 Linux ELF 修正与 Windows npm 实测入口**：实现提交 `56f6da21789c2aa90abf2fdff6a2e10aea03283d`。旧 Linux Codex 失败的直接原因是官方 Node 20.9.0 的 `DT_STRSZ=5,291,274` 被误套 1 MiB 动态记录表上限；本次保留整段唯一映射和文件边界，改为每个已引用依赖名至多读取 256 字节。新增七项边界回归及固定 Node 原字节的只读系统依赖闭包测试；依赖名、加载器、网络和 Landlock 门禁均不弱化。ABI1 仍独立限制候选隔离，不把解析修正等同于该主机可以执行 Codex npm 更新。

Windows 新入口通过真实 npm 在私有前缀登记固定 `0.155.1`，并调用产品后端更新至 `0.156.1`；包含正常更新、OldMoved 冷恢复、发布后缺收据冷恢复、外部改动保留、候选改动拒绝。cmd、PowerShell 两候选各需真实 AppContainer／Job 退出收据；三种 shim 保留 npm 原产字节，不为通过而改写。固定渠道和断点只进入测试构建。初始 npm 登记不是产品 Job 隔离证明，超时不得记作已清理；消费者渠道、忙碌／插件重检及 GUI 均不在此入口结论内。

本地 Mac ARM64 `cargo check -p warp`、i18n 11 项、更新模块 190 项（6 忽略）、Python 11 项及 actionlint 通过，八份源码／工作流与提交 Git blob 一致。原始日志及审查位于 `validation/g09-followup-20260927`；此前 actionlint 的 SC2155 失败保留，拆开赋值与 export 后独立通过。同提交 [Actions 36262495627](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36262495627) 的实际终态与失败已在上文记录；本批无需本地化变更，不重包或重跑 Mac GUI，不选择 macOS Intel，G09 与 Goal 均不关闭。

**2026-09-27 G09 Linux 隔离能力前置增量（本地门禁通过，平台及布局待验）**：实现提交 `2b59c8c62c7c46bc30211a9872f966c52317202e`，基线 `6db38e0da2fd0c066eeb31f4f2de9f7b446fd4fb`。仅 Linux Codex npm 和三款 Linux Homebrew 在版本确需更换、且既有来源／版本／渠道检查无错误时查询 Landlock ABI≥3；不可用时不生成计划，既有状态机清除旧计划并阻止手动或自动派发。Grok npm、其他来源、macOS／Windows以及同版本无需候选执行的检查／配置同步不加此门槛；原 worker 的 ABI 拒绝与实际隔离规则安装保留，查询通过不代替执行时核验。新增错误提示已同步英文／简体中文，实际双语布局仍待共同验收。

原始收据位于 `/Users/zhishi/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/g09-isolation-preflight-20260927`：`source-commit-binding-01.safe.json`（SHA-256 `14e2ea8349e7273c568ecdd34f601ad750a2104228bb9a669e68ddc30d21ddf9`）确认 9 个 Git blob 与 `source-snapshot-01.safe.json`（`16fa1247cbaf7db9d74db7ea5f2ef7f83f0c52dce530be768e0f38bca46a616e`）一致。macOS arm64 `cargo-check-01` 通过（98.56 秒）、`i18n-01` 11 项通过（252.42 秒）、`updates-01` 190 项通过／6 忽略（4.67 秒，含新增 4 项行为测试）；`supervisor-build-01` 通过（177.29 秒，日志 SHA-256 `c265c002e38c944e2253a86ce79d735e82f08bfb2eb9e902f0a9b1d448834113`）。另以显式 `rust-embed/debug-embed` 运行 `embedded-i18n-01`，11 项通过（262.10 秒），收据 SHA-256 `407e119ffb4c3f16840b9172f51d71672d5cbd43dfff9fbea8e29d8e5b10263b`；嵌入资源应用已按下述独立收据完成包装；实际双语布局仍未计通过。这些本地门禁不证明 Linux 内核能力或真实包事务通过，`2b59c8c62` 的相关平台同提交 CI 仍待补。

同源码的 **Mac ARM64 联测包已准备，尚未启动**：`/Users/zhishi/Library/Caches/InfiniShell-Desktop/cli-parity-apps/2b59c8c62/InfiniShellParity-2b59c8c62.app`。交付构建显式启用 `rust-embed/debug-embed`，四份英文／简体中文 Fluent 文件的提交原字节均在签名前后程序中核对；131 个外置资源与原账本一致。签名前 SHA-256 `2b8525dd45177214659f11d4fed71c763bc30a54029b1ade835b2e96944c6afc`，临时签名后 `a42a0439886fc5174ba6db7d51211949133c4a9232e3e448d22d946bbb6029fd`；严格签名验证通过，使用独立 `parity-code-2b59c8c62` profile，不查询开发者证书，不改旧配置。收据 `prepared-app-2b59c8c62/bundle.safe.json` SHA-256 `6d706e45584115fce4feef72120921937969356820358bb587f77f71e20af6e4`，索引 `92a3d9bfc3f94cb1ac60cbc452430e5910b22c6387aebe9f2b4ffca57e8fef50`。默认后端与交付程序同源码、不同 feature 与摘要，不将后端正常升级结果当作此包 GUI／模型通过。

首次 `embedded-build-01` 虽返回 Cargo exit 0，日志含 `rust-objcopy` 输出空间不足，3280 字节文件不是 Mach-O；包装器在创建应用前拒绝，失败日志和两份无效原字节保留于同归档。按用户授权，仅清理已核对摘要／身份且无进程占用的停用单测及旧主程序编译缓存；`completed-test-cache-cleanup-01.safe.json` 与 `relink-cache-cleanup-01.safe.json` 分别为 `f401c7442cac2a7a161ac6784ee0693ffb213490cc8d06dbb0360631e11f9fd3`／`078ea5d47e36c0c349b790bd1b21d889872604fead82b5fa32509a38b135e4f4`。复用已编译库后的 `embedded-build-02` 用时 7.94 秒，链接及后续 ARM64／资源／签名验证通过，日志 SHA-256 `44b83bcafd01a532a4f3add33b5a5708c2d70fde7c06692074cfb1441b98737e`。原始失败不覆盖，真实运行 worker／supervisor 与旧应用仍保留；此处无需本地化文案变更，也未开展 GUI 布局检查。

`2b59c8c62` 的 **macOS arm64 Codex npm 私有正常升级已独立复核通过**：`codex-live-09-normal` 固定官方 `0.155.1→0.156.1`，运行器 exit 0（354.88 秒），真实生产来源识别、npm 只读 prefix 查询、Node→`bin/codex.js --version` 候选探针和包交换完整执行；原始 stdout 为 `codex-cli 0.156.1`。18 个运行器追踪源码逐个匹配该提交 Git blob 和运行后字节；`working_tree_dirty=true` 对应同时编辑的文档，不能省略此记录。worker SHA-256 `0ecd5d870874e3eb64b89d773a11a664cdf97afa5c4f34a28c8e921389f96d9e`，监督程序 `974cea9a90df40a3c57fbcf0106db8bcbdb12f0fdd4e20db266d6ad3a57e27df`；二者及 Node／npm 摘要再次核对一致。四份官方新旧归档 SRI、47 个发布文件和 69 个候选绑定文件逐项复核；根 inode `93465119→93465346`，uid/gid/mode及公共 symlink 身份保留。generation `59b819cb-c243-4d70-b27b-14ff4625c4a5` 的 exit/binding/manifest/coalition/native 摘要相符，`exit_code=0`、`cleanup_confirmed`／`job_removed`／`resource_cid_destroyed=true`，原 PID 和 launchd 项已不存在；旧 stage 和活动 journal 已移除。私有 fixture 原本缺失的 `.codex/config.toml` 仍缺失，不声称验证了用户真实配置。

该成功的 46 份原始小文件（411,026 字节）逐字节复制到上述新归档的 `codex-cache-normal-success-receipts`，原件、旧失败和大包／程序原位保留；`index.safe.json` SHA-256 `1277e2bb2e980bad3281738115205d51cfe7d223092dcfc390831b32436dfdff`，独立复核记录 `independent-review.safe.json` 为 `29ea4436b3ee4379f7347cb3bc3b30a37649a493924a3b946061edee3e239ec1`，原 `summary.safe.json` 为 `4e2badeb87bf60fbb66b73aaecdb17574863712988e530bff9f9842a216d35a2`。本轮 `recover_invoked=false`、零模型输入、未执行 npm install／生命周期脚本；目标来自固定测试钩子，不覆盖消费者实时渠道、GUI、忙碌／插件重检，以及其余三种恢复／故障场景。不能据此关闭 G09 或 Goal。

旧提交 `39941b281` 的 [CI 36257833211](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36257833211) 已结束：Windows 作业成功，Linux 作业失败且唯一失败为 Codex npm；Linux／Windows 编译和定向模块通过，不能继承给 `2b59c8c62`。其 `ci-36257833211/linux-environment.safe.json` 只读确认 euid 1000、Landlock ABI1／`abi3_available=false`、`/home/linuxbrew` 缺失；ABI1 不满足 Codex npm 和三款 Linux Homebrew 的候选隔离条件，Grok npm 不使用该门槛，须看实际事务回执。`unshare` 返回 127 只说明命令不可用，不证明内核禁止 namespace。Windows 预检的 `winget.command_found=false`、HKCU 可读和非管理员身份，也不能单独证明已登记 portable 来源事务可用或不可用；来源、ACL、依赖、候选隔离和真实事务仍须验证。Linux Codex npm 已取得的实际失败为 `ProbeFailed`，generation `8ace5170-0ab8-4bed-8fc8-300ef3323612` 的 `supervisor.stderr` 为 `managed_process.linux_glibc_elf_invalid`，退出收据为 `stdio_closed`／exit 1／`cleanup_confirmed=true`；尚未观察到该次执行的 Landlock 拒绝，不能把 ABI1 环境事实替代实际错误。原件位于 `ci-36257833211/codex-npm-artifact-10911429927`，ZIP SHA-256 `938191725ef7c9a273f13cb5d546adc169519a326d9acf280ebfe48a1b497fcb`，`failure-review.safe.json` 为 `1c803d57fdd6f0ecd90707b16a09fcd3825b39f52626fcede31a47cf0ae291eb`。Windows 作业虽 success，`windows-job.raw.log:4136` 的 `supervised_cancel_before_control_handshake_confirms_process_tree_exit` 标为 nextest `LEAK`，该步骤为 5 passed（1 leaky）；未扩展诊断或重跑，不能宣称完全无残余进程／句柄。安全拒绝不计升级成功，G09 与 Goal 保持未完成。

`39941b281` 的 **Linux x64 Grok npm 四场景已独立复核通过**：正常 `1.0.40→1.0.41`；交换后缺收据由新进程冷恢复至 `1.0.40`；外部改动由冷恢复保留在 `1.0.41`；候选改动在启动前拒绝并保持 `1.0.40`。三份真实退出收据与 manifest／exit-binding 摘要相符，均确认清理；两次恢复分别有 execute/recover exit 0 及对应前后树。147 个原始 artifact 文件和原 ZIP 保留，ZIP SHA-256 `4ab0266dc82cdf588b2e28cc3d9c58e335d295faa390cceff61fcf9ac21d6428`；16 个追踪源码逐个对应该提交 Git blob。完整收证 `ci-36257833211/completed-review.safe.json` SHA-256 `e7bbda8643cd2ddb8593baab1b66a7a55a5658ad24a37c516ed57807104310ab`，Linux 原日志 `2bc92dd48e6fb761574351725a64e7cfaa7f122900edae90f61a573d61b49791`。Linux 定向模块 947 项和五项无模型清理边界通过；Windows 原 `LEAK` 不改变。此结果限私有固定候选后端，不包含消费者渠道、GUI、模型、插件复检或 G09 整体完成。

**2026-09-27 G09 私有 npm 真实验收入口增量（进行中，未关闭）**：基线 `acc29e1549fb09e6e0dce1549af5ade469df30ef` 的增量已提交并推送为 `39941b2815506997198091082b92d19e11d92761`；`source-commit-binding-01.safe.json` 确认 17 个代码／工作流／脚本 Git blob 与 `source-snapshot-05.safe.json` 一致。新增 Codex `0.155.1→0.156.1`、Grok `1.0.40→1.0.41` 私有 npm 后端正常更新、交换后缺收据冷恢复、外部改动保留、候选改写拒绝四场景入口；Linux 窄工作流复用同源码 libtest／supervisor，Linux／Windows 包来源环境预检只读、独立归档。两新 artifact 仅收集私有白名单，包含隐藏 `.local` 和最新 `codex-npm.json`／`grok-npm.json` 账本，不扩大到用户 HOME。产品修正包括 Grok 新版本 Mirror 继承旧 uid/gid/mode、Codex macOS Node 裸加载路径解析，以及离线探针的根目录 literal 只读、精确系统 dyld 缓存目录读取和空 OpenSSL 配置；签名、来源与身份核验未放宽，未开放 OS／Rosetta 整目录或网络。

该提交的最新 macOS arm64 本地门禁：`cargo-check-07` 通过（97.97 秒）、`i18n-06` 11 项通过（260.46 秒）、`macho-paths-03` 3 项通过（2.74 秒）。此前 updates 186 项／6 忽略、Python 52 项与最终工作流 `actionlint-03` 通过，原始运行记录保留；`supervisor-build-01` 主动中断，build-02／03／04 通过，build-05 亦通过（178.10 秒，日志 SHA-256 `a04e074aa243a6cd8b2b4c93f3566d1871ad5974b37510e1202afab959f12654`）。[Actions 36257833211](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36257833211) 绑定该 `39941b281` 提交，Linux／Windows 编译和定向模块通过，最终整轮 failure；仅选择 Linux／Windows，开启两款私有 npm 运行器与 `native_acceptance_only`，关闭全工作区测试和 Mac Intel。Grok Linux 四场景结果见下文；Codex Linux 失败保留，不能覆盖后续 `2b59c8c62`。

旧失败全部保留：Codex 验收模块误用 Stable，生产按真实 Latest 渠道拒绝，仅修验收模块；`codex-live-03-normal` 的 `UnsupportedSource` 不改写。依赖闭包修正后的 05／07 仍为 `ProbeFailed`／Node dyld 初始化 SIGABRT，`native_wait_status=6`，两轮精确代次清理均已确认，不计升级成功。07 的 PID `12908`、generation `29ce7311-7a17-4066-8b29-e5fee11fb238` 及 `cleanup_confirmed`／`job_removed`／`resource_cid_destroyed=true` 见 `codex-cache-normal-03.safe.json`、原日志和 `codex-live-07-dyld-diagnosis-20260927`；小收据逐字节副本位于 `codex-cache-third-failure-receipts`，索引 SHA-256 `9d9db256d171f8170395391b774168fa039bf864671c74e5041e0d27d0642278`。05 的诊断、32 文件副本和更早原始输出仍保留。Grok Documents 内首轮候选监督首次连接 10 秒失败，候选未执行、仍为 `cleanup_unknown`；PID／launchd 后来退出不补造清理确认。相同监督程序错误入口约 0.0884／0.0847 秒返回不证明加载慢，未扩大 timeout。

后续独立实测将读取范围收敛为根目录 `literal "/"` 与精确系统 dyld 缓存目录：`dyld-private-node-start-02` 的 `root_only` 返回 Node 版本并退出 0；`dyld-private-codex-entry-01` 的原包入口因全局 OpenSSL 配置访问拒绝退出 1，失败保留，同轮绝对路径／openat／符号链接越界读取、包／Node 快照写入、网络及额外执行七项负例均返回 EPERM。`dyld-private-codex-entry-02` 仅将同一私有字节包的 `OPENSSL_CONF` 设为 `/dev/null` 后返回 `codex-cli 0.156.1`、exit 0，原包字节未变。该环境设置只用于 macOS 离线版本探针，不修改全局配置或正常模型会话，也不证明其 TLS／FIPS 配置兼容。静态依据和精确收据索引为 `codex-libignition-static-evidence-20260927.safe.json`，SHA-256 `7c0edf67fcdbf0f180b826d071793fe00e8558b0e58260faacccb997c1a5109e`。**独立入口成功不等于完整更新成功**：Mac 修后完整 Codex npm 正常升级此前因内置盘不足运行器 3 GiB 门槛而暂停，该历史记录保留；精确清理后，`2b59c8c62` 的 `codex-live-09-normal` 正常场景已通过下述独立复核，旧失败不回填，也不降低运行器门槛。

Grok macOS arm64 的 `grok-live-03-normal` **仅正常升级路径已独立复核通过**：固定官方 `1.0.40` 私有 npm 安装升至 `1.0.41`，完整包与用户镜像、npm 只读来源查询、公共入口保留及候选正常退出／清理均有收据；零模型输入，没有执行 npm install 或生命周期脚本，`recover_invoked=false`。该轮 worker SHA-256 `23c0e6639faac76634e22b91b41287a63b9207014492fa9e18be08264bb54e3c`、监督程序 `0b68f31f0bd678357cb7f6ff42803bd6826c12e11ae727352ba1cc7e8c12444e`；`/Users/zhishi/Library/Caches/InfiniShell-Desktop/cli-parity-npm-live-20260926/grok-live-03-normal/summary.safe.json` 摘要为 `d9f7a6b51850be70a3318773f071954dae7fbdf89e56c2d954eac6f4cf713145`。原 Documents 失败不回填，不能外推其余场景通过。

本批主归档为 `/Users/zhishi/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/g09-npm-live-integration-20260926`，历史成功、失败及二进制摘要保留在原收据和 `CURRENT_STATUS.json`；源码提交绑定、本地构建及同提交 CI 收证已完成；CI 整体失败，平台与场景结果分别见上文。固定测试候选仅覆盖包更新后端，不包含消费者实时渠道解析、GUI 更新入口或模型生命周期；Homebrew／WinGet 真实升级与恢复仍欠，Windows 本批运行器只纳入脚本可移植性及 cfg 门禁，Mac 仅 Apple Silicon，Goal／G09 不关闭。`39941b281` 增量**无需本地化变更**；后续 `2b59c8c62` 新增原因提示及布局边界见上文。用户授权的两轮精确清理分别保留于 `g09-unused-build-cache-cleanup-20260926/result.safe.json`（6 个旧中间文件、2,051,019,161 字节）和 `rlib-rmeta-unused-cleanup-20260927.safe.json`（2,253 个旧 `.rlib/.rmeta`、1,690,137,365 字节）；依据实际空间监测保留必要余量，不声称始终保留至少 12 GiB，这两轮未删除二进制、证据或夹具。后续 `unit-test-cache-cleanup-20260927.safe.json`（SHA-256 `be9a5e8064eba29e713b5ceadd48aef097f4746d06b3a5083ea60f12c1848f11`）仅清理 7 个已停用单测可执行缓存，逻辑字节 4,936,347,344，实际可用空间增加 4,921,823,232 字节；保留 5 个当前或原生验收角色、全部原始收据／日志／元数据。清理后当时可用 6,597,607,424 字节，不代表后续实时余量，也不改变旧失败。

当前代码批次以 `76dcfdc60254077edaa86175722591333526c05c` 为基线，按用户要求先完成功能接线；代码已提交为 `704bb33bb2943f8a686b24b843c3b3a1ba656c19`，macOS arm64 内置盘 `cargo check -p warp` 及 i18n 资源门禁 11 项通过，该代码检查点当时未运行功能测试、GUI 或在线模型；后续无人值守回归单列如下。三平台 Grok 专属普通终端、远端图片协议、包管理器事务和受限技能的代码范围见 `CURRENT_STATUS.json.current_work_order`；不继承下文历史门禁的通过结论。用户已明确 macOS 仅支持 Apple Silicon，Mac Intel 不再属于实现或验收范围；本轮新增的 Intel 清单与适配已撤回，仓外静态下载资料保留，不计成功验收。

本轮另外归档固定 Grok npm 三平台、Homebrew cask 与完整依赖的官方静态原始材料：仓外 `validation/grok-package-static-contract-20260926`，34 个文件，索引 SHA-256 为 `bab479385537d2a73069e27a8ce832041d3b6b5e9370b6475e41e71a716fec12`。Windows npm 原生映像与官网映像的所有 PE 节内容一致，移除官网尾部签名并规范化 checksum/security directory 后逐字节相同；两种发行映像仍各自绑定摘要。此归档只证明源码/包/映像静态合同，没有执行 CLI、安装脚本、升级、模型或 GUI，不扩大历史验收范围。

本批 Windows 受审命令、Grok 普通历史继续、受限降级及 Linux Codex cask 的代码/静态原始资料分别存入仓外 `validation/g10-windows-mcp-code-20260926`、`grok-owned-history-code-20260926`、`claude-downgrade-code-20260926` 和 `codex-linux-brew-static-20260926`，索引摘要见 `CURRENT_STATUS.json` 的 `current_work_order.code_archive_indices`。Linux Codex 固定官方归档为 145,976,992 字节，SHA-256 `8b711520beddf385467b8da4d2c93736637c6ba1e46811cf0d8606b7c490b6f6`；44 文件/10 目录与固定清单的字节、类型、模式一致，原生 ELF64 x86_64 没有 PT_INTERP/DT_NEEDED。这里只列静态合同，未执行归档内程序或重跑模型，不能据此关闭 G09。Homebrew 的 `LATEST_DOWNLOAD_SHA256` 只在 `cask.version.latest?` 条件下写入，数字固定版本不因此增加该文件；`@latest` 名称不等同 `version :latest`。

Linux Claude cask 和其余 Linux cask/Grok WinGet 官方材料已分别复制归档至 `validation/claude-linux-brew-code-20260926`（29 文件）和 `validation/linux-brew-grok-winget-static-20260926`（25 文件）；原临时资料保持原字节，索引见 `CURRENT_STATUS.json.current_work_order.code_archive_indices`。三款 Linux Homebrew 与 Grok WinGet 的最后共享接线已经合入工作区，源码绑定包含 69 个唯一且存在的文件。上述均为静态资料／源码归档，未运行候选、安装器或测试。

G10 后续增量已接通三平台宿主逐命令监督、同会话顺序多命令、逐次审批和原生 Submit；原生 shell 保持关闭。Unix 独立执行器原始代码存入 `validation/g10-unix-command-code-20260926`，整批代码检查点及编译日志位于 `validation/code-integration-g10-sequential-20260926`。第一次编译因缓存参数不一致主动中止，不记失败或通过；恢复原参数后的第二轮 exit 101，24 条类型/可见性/借用/Send 错误保留，相关接口修正后第三轮 `cargo check -p warp` 通过。i18n 第一轮在测试程序编译时出现 3 条错误，补齐测试构造字段及两处字节断言后第二轮 11 项通过、0 失败；原失败均保留。223 个代码、插件与资源文件逐个核对 Git blob 与本地门禁快照一致，绑定收据为 `implementation-commit-binding.safe.json`，摘要见当前状态。这里仅确认本地编译与英中资源约束，不包含功能、模型、双语布局或跨平台通过结论。代码检查点先按用户指示完成实现，后续仅推进无人值守门禁；GUI／在线模型联测仍后置，Mac Intel 不再选择。

代码检查点后的无人值守本地回归以 `c72313648895e7cb1d0d25a721ea8346403324d5` 为来源：CLI 组 1,832 通过／17 失败／67 忽略；远端图片服务端 1 通过／25 失败；Grok owned 组 50 通过／2 忽略；图片客户端 5 通过。Codex 来源 Python 首轮 10 通过／3 错误，历史 payload 夹具修正后 13 项通过。原始日志保存在 `validation/post-code-local-gates-c72313648-20260926`。失败发现远端图片暂存目录未在创建时显式设为0700，同时旧技能／插件测试夹具和 npm 支持断言未同步；修正未削弱权限或未知版本门禁。首轮修正后编译与 i18n 11 项通过，受影响回归 1,880 通过／1 失败／67 忽略：原失败均消除，仅跨进程重关联夹具的事件断言失败。源码核对发现夹具在 ACK 与后续 Progress 之间即可保存游标，需明确等待两者后再交接；原输出未含实际违规事件，保留证据边界。只修测试 helper 后，受影响的活跃重关联、宿主崩溃和离线回收三条进程用例全部通过，i18n 11 项再次通过；原整批失败记录保留，不改记为整批通过。原始后续收据存入 `validation/post-code-handoff-sync-20260926`，索引摘要见当前状态；无需本地化变更。图片客户端 5 项、通知 Python 20 项、插件兼容 Python 14 项、修后来源 Python 13 项及 actionlint 亦通过。上述修正提交并推送为 `bc9bb1f28d08717f6a5ded8a562571779a653ea0`，同提交 [Actions 36244024291](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36244024291) 首轮 Linux/Windows 定向门禁均失败：Linux 图片验收夹具 4 项失败及权限校验函数私有可见性 E0603；Windows 运行器仍要求旧 rev5、历史候选混用当前 payload，另有 AppContainer 继承标志 ACE_FLAGS 类型错误。两平台编译后的定向 Rust 测试未运行。完整日志和 7 份原始产物 ZIP 存入 `validation/cloud-post-code-bc9bb1f28-20260926`，索引摘要 `c89e7f2edc1182bcd3b2e714c18fd841d27093838abf14136381009ec2cc4b2b`。两平台已完成的无认证原生探针仍按其各自范围有效，不代替模型生命周期。Mac Intel 与全工作区测试不选择；此轮没有 GUI、输入法或已认证模型验收。

首轮云端失败后的最小修正：Linux 仅将既有权限校验函数开放给父模块，Windows 继承标志改为与原数值一致的命名 ACE_FLAGS；Codex 正式运行器精确同步 rev6，旧候选仍核归档 payload 的原 SHA；Claude 图片夹具仅补其 Python 加载器环境，生产 CLI 环境白名单不变。内置盘 `cargo check -p warp` 与 i18n 11 项通过，Codex Hook Python 48 通过／2 项 Windows 专属跳过、Notify 6 项通过，Claude 图片运行器 9 项通过。新增修订混用拒绝与夹具环境回归，未放宽权限、摘要或 UTF-8 原字节断言。本地门禁和源码快照在 `validation/cloud-round1-fixes-local-20260926`，Python 原日志在首轮归档的 `local-script-diagnosis/`；修正已提交推送为 `b78b62a48223235e8c29157e3786747c8db2ff68`，同提交 [Actions 36245112104](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36245112104) 已完成 Linux／Windows 复验：两平台 `cargo check -p warp --lib` 通过，Windows 固定 Codex 0.156.1 原生 Hook／ConPTY 通过；Linux 主应用定向回归 3,166 通过／4 失败／4,840 筛选或忽略：四项均在同一 Grok 输入夹具初始化失败，测试进程未拥有新建 PTY 的控制终端，真实身份校验拒绝；相应功能断言尚未运行。图片客户端 5、共享 action/skills 75、warp_cli 129、TUI 9、命令归属 1 及 IPC 2 项通过，通知 worker、固定 Grok 生产通知插件安装和监督清理等无认证原生探针亦通过。Linux 作业结论仍为失败；Windows 作业全部通过，主应用定向回归 2,973 通过／0 失败／4,772 筛选或忽略，图片客户端 5、共享 action/skills 75、warp_cli 129、TUI 9、命令归属 2、宿主崩溃监督 5、IPC 2 和 rust-genai 81 项通过；固定 CLI 原生通知、恢复/崩溃边界与 Grok 监督清理探针亦通过。完整日志及 10 份原始产物 ZIP 的索引为 `second-run-evidence-index.safe.json`，114 文件，SHA-256 `e71bc6d7c2f4f6f22cc0feb0c8020070351d7c65855295c943d37bc9acfbca05`；不包含已认证模型、GUI/IME或安装来源真实升级。无需本地化变更，不关闭功能缺项。

同一 `b78b62a48` 代码还通过本机 `cargo build -p warp --bin infinishell`，生成 arm64 可执行文件并复制到内置盘后核对摘要 `ca2d6a34cab2e98f810158571fe4d8e5eacb122c382d9131f3067e26eb325997`。源码在构建期间未变，动态依赖列表没有外置卷路径。证据为 `validation/code-checkpoint-app-b78b62a48-20260926`，索引摘要 `c89b0c49199b65a45360e3c5b27777ed4b651f776b13fa7ca0e260f42ea33418`。程序未启动，未制作签名应用包，不算 GUI 或模型验收。

该 arm64 二进制另已组装为内置盘联测 `.app`，资源 131 文件中 bundled 128 文件与当前仓库逐个摘要一致；显式 `codesign --sign -` 临时签名及严格签名核验通过，没有查询开发者证书。签名后程序摘要 `6ff75465683af7d5bf49f765800840e4f4ee526b54c03812b974d70213019327`，原始程序未变；使用独立 `parity-code-b78b62a48` 数据 profile，不改旧配置。归档 `validation/code-checkpoint-bundle-b78b62a48-20260926`，索引摘要 `ea4da1ca09c09161248e86d476976323a4b24264f05d43b3e94de7212896b17c`。应用未启动，此包仅供后续共同联测，不算新 GUI 或模型证据。

Linux 四项失败后，仅修改 `grok_owned_input_tests.rs`：通过统一 command 封装派生持有控制 PTY 的 shell，用真实子进程身份初始化，测试结束或断言退出时 kill/wait；生产校验和四项功能断言不变。本地 `cargo check -p warp`、四项定向测试及 i18n 11 项通过，原 Linux 失败保持；修正已提交推送为 `17c1dcc516a7f45c2ce61243a0bf82487b3afec0`，仅补跑相关 Linux 平台的 [Actions 36248869156](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36248869156) 已全部通过：主应用 3,170 通过／0 失败／4,840 筛选或忽略，四项 Grok PTY 用例逐项 PASS；图片客户端 5、共享 action/skills 75、warp_cli 129、TUI 9、命令归属 1、宿主崩溃监督 5、IPC 2 及 rust-genai 81 项通过，固定 CLI 无认证原生边界亦通过。完整日志与 2 份原始产物 ZIP 位于 `validation/cloud-linux-pty-17c1dcc51-20260926`，60 文件索引 SHA-256 为 `4f6c41baa1889aeaf50e94ff54634f83dd7e793a3e019af07e4b2f4274a80901`。Windows 生产与测试源码未变，复用 b78b62a48 的独立通过结果，不冒充在17c1dcc51重跑。原始收据在 `validation/linux-pty-fixture-fix-local-20260926`，索引摘要 `b5babc7f3476e2fc22c8e3b73d774fc2506103abf25fd0c14ea6019ee93913f6`。无需本地化变更，不扩大任何功能验收结论。

## 验收基线与追溯

| 项目 | 固定来源与解释 |
| --- | --- |
| CLI 版本 | Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41` |
| 历史产品行为冻结 | `ee839b4fc7dacd58bac0806aa549500af5776b30`；包含桌面 Grok 默认开放、启动取消修复和双语说明修正 |
| 历史最新相关验证提交 | `50e1515bcd3f20cf120edf5565766975b6a68d5d`；只完善测试与运行器，未再次修改产品行为 |
| Mac 模型与 GUI 实链 | 来自 V2–V13 等实际构建批次及源码摘要；后续按变化域复验，不能重标为 `ee839b4fc` 或 `50e1515bc` 全量重跑 |
| 历史档案固定提交 | `e3ef39689cd8b686dfe040b90217ed1067b4d268`；保存原过程专报、失败、通过、截图与摘要索引 |

原始材料从当前目录移出后，仍可从[固定历史目录][history]追溯，或使用 `git show e3ef39689cd8b686dfe040b90217ed1067b4d268:specs/cli-agent-parity/<历史路径>` 读取。历史总体“全部完成”判断已被当前状态撤回；保留原文件不表示继续认可其总体判断。

关键来源是[固定版本验收表][acceptance]、[Mac 交付记录][mac]和[同提交补验报告][final]。这些固定提交链接用于复核旧结论，不承担当前待办管理。

## 2026-09-26 Claude npm 单包更新增量

以 `fe135e3436c22eaa7857c31bfaf94969eb1aa11f` 为基线的工作区接入 Claude npm 官方完整包树事务，目标限定 `2.1.280`，旧安装验收起点为 `2.1.278`。独立核对 wrapper/platform 归档 SRI、完整树及公共 native 入口，不执行安装脚本；原有忙碌预约、配置约束及监督清理保持。候选使用私有空配置、固定 `--version` 和拒绝网络；新增删除前逐成员校验及保留外部变化的回归。Codex npm、Windows npm、musl、额外 ACL、Homebrew、WinGet 和合法降级仍未完成，G09 不关闭。

macOS arm64 本地 `cargo check -p warp`、i18n 11 项、更新模块 182 项（4 跳过）、监督模块 85 项（17 跳过）、Python runner 16 项及应用构建通过。首两轮测试编译的导入错误已修，原日志保留。工作流 actionlint 在显式登记现有自托管 runner label 后通过。英中版本探测失败提示已在同一签名 GUI 构建实际显示，完整文本、渠道说明、按钮与下拉框可读；合成无效版本只计布局，不能替代更新事务。应用、测试程序、监督程序及缓存均从内置盘运行，两种语言启动均未等待人工授权。

第一轮真实 npm 事务在 `/private/tmp` 私有子目录中构造官方包后，被已有 macOS 原子执行祖先检查拒绝：`managed_process.atomic_macos_ancestor_unsafe`。原始 native exit 1、cleanup_confirmed 收据保留，旧公共程序摘要不变，暂存树已回收。未放宽权限检查。第二轮在用户私有内置目录通过祖先检查，但 15 秒输出等待到期：原子镜像验证发生在监督者就绪之后，空输出和 `stdio_closed` 收据不能证明已进入 CLI 主程序。该轮同样确认清理、旧公共摘要不变和暂存树回收。候选版本探针改用既有更新事务的有界 300 秒预算，并单独返回 `TimedOut`；新源码门禁及后续五个真实场景均已通过，旧失败保留。

本轮 macOS arm64 在私有前缀物化官方包，实际执行 npm 只读来源查询、生产 Rust 更新后端及受监督候选 `--version`：正常更新到 `2.1.280`、交换后缺收据的冷进程恢复到原 `2.1.278`、外部改动及 journal 保留、候选改写后未启动和未审核 `2.1.278` 降级拒绝，五场景均通过。独立 Python 检查逐份核对 SRI、归档成员与最终目录；公共入口链接保留，所有已启动候选正常退出并确认清理。版本拒绝不能外推为合法降级已实现。

最终监督程序 SHA-256 为 `63c34a50b1ce86d3bfdd8774aee3ee9ab070bef348c9d3bc8cda2e9e5c30499b`，测试程序为 `ebc56ba3d43ccb0f8b92838cd777831e4ea8d484b6fcf68aeaebd4eb7f041a96`，零模型输入。忙碌范围仅为另行运行的既有产品状态机测试，未覆盖 GUI 更新点击或插件重检。GUI 文案与资源未随超时修复变化，复用 v3 的独立布局原图，不冒充同一程序全量重跑。349 个原始记录、旧失败、源码与截图保存在仓外 `validation/claude-npm-20260926/`，索引 SHA-256 为 `7f0a70056a1edfafd746a962699d8f2ae70685ce909658bf745c217b7a2ec022`；实现提交为 `529512a354d66fdcf48f70db1bbe63216c36a3da`，独立提交绑定 SHA-256 为 `6d2b4d5c7836803e3d1890998d924edf6af214761145f1ba42475b784ed51be8`。[Actions 36217724224](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36217724224) 已通过同提交 Linux/Windows 定向门禁，Linux 官方 Claude npm 五场景原始收据逐份核对通过，零模型输入。云端 253 份记录归档于仓外 `validation/cloud-g09-529512a35/`，索引 SHA-256 为 `97ca36b14ecbd9eecf2d6313ab4a3fc51a662a26c27953934d0ba2b207a2e991`。本次仅补验收结论，无需本地化变更；G09 的其余安装来源仍未关闭。

## 2026-09-26 文件卡片同提交门禁

文件选择器增量已提交为 `83e5115839b301697f3322092f487ddce9d72dfe` 并推送。本地 `cargo check -p warp`、i18n 11 项、editor 6 项、footer 46 项与应用构建通过；实际 macOS Codex 两张卡片投递、原生 shell 读取及双语布局沿用该增量的原收据。Grok 自动提交、Claude 首次向导后的普通终端链仍待补，G07 不关闭。

[Actions 36210156960](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36210156960) 绑定同一提交，Linux x64 与 Windows x64 定向门禁均通过，Intel 作业未选择。它覆盖相关编译、回归、无认证原生协议/通知/清理；不证明已认证模型生命周期、真实中文输入法或 Grok 完整双语 GUI。47 个原始日志和产物保存在仓外 `validation/cloud-g07-83e511583/`，索引 SHA-256 为 `20373845cd0a8228ae23fca12f57db91aec0e8bb6dc62986d462fa302678a364`。此前 `6790b185a0c0bc581f804edf975960047df85db7` 的 Linux/Windows 整合门禁亦通过，归档索引为 `8239de43dd32999ba0348146d923df701e9d7b97e0b5e079022beff9d2abcdc3`。

## 2026-09-25 继续实现增量

实现来源标记为**输入实现提交 `84102bb1c687f87a2425bc1937784e77250c416c`**。G02 接通固定 Grok PNG ACP 图片；G04/G05 扩展 Claude 静态格式、纯图与精确注册单技能的图片编码；G07 接通普通终端本地文件卡片到明确路径引用。G09 的 Codex/Claude npm 管理器/包清单/前缀/真实入口绑定已提交为 `b6f93f6627738fb83b6be776dd225666a900ac90`，但 npm 更新仍为 `ManualOnly`，尚未完成真实升级、降级及恢复事务。以下均不关闭 Gap。

### 图片生产适配器与原生合同

| CLI、版本、模型、平台与模式 | 本次实际结果 | 证据边界 |
| --- | --- | --- |
| Claude `2.1.280`、`claude-opus-5-5`、macOS arm64、Inherit 生产适配器 | JPEG、WebP、静态 GIF 分别新建识别随机象限色；原生 user 回放的 MIME、字节和数组摘要与持久图片一致；关闭后同原生 ID 冷恢复正确回忆，均自然退出并清理。每用例 2 次模型输入 | WIP 二进制 `6e3e13f07aef46612c90c35b0c67c84163313f00f5e594701746f1f06215d284`；不是 GUI、应用重启或跨平台链 |
| 同上，纯 PNG | 先文字定义后续图片回答格式，再提交没有文字块的纯图片；实际识色、字节回放、持久引用恢复及冷恢复回忆通过，共 3 次模型输入 | 不外推所有纯图片格式；不是图片加技能的生产验收 |
| Grok `1.0.41`、`grok-4.7`、macOS arm64、Inherit、无已选技能的真实生产 connect | 新建识别红/蓝 PNG，关闭后同原生 ID 冷恢复，提交独立绿/黄 PNG 并正确识别。真实最终历史 user_message_chunk 的图片字节、MIME、typed content 摘要与持久输入一致，promptIndex 为 0/1，代次和回合准确关联 | WIP 二进制 `ff821a38c0260c35e9a3210f39090156c6814d2b1327499bb8992669b2b3f8b0`，未用 candidate flag；两代各重发同一消息 ID 共 3 次，仍各仅执行 1 个原生回合。两次退出均 `stdio_closed`、exit 0 且 cleanup_confirmed；不是 GUI/应用重启或其他策略/平台 |
| Claude `2.1.280`、`claude-opus-5-5`、macOS arm64、直接原生 stream-json 图片加单技能 | v5 新建与冷恢复各有精确注册命令的真实 Skill tool_use 及 `SKILL_OK LEFT=RED RIGHT=BLUE` 正例 | 原生合同；保留原生权限流程，审批次数未单独计证，不是完整生产适配器/GUI链。裸 slash 数组未触发技能的失败和 v4 探针“禁止工具”指令冲突导致的失败原字节保留 |

两款生产用例使用同一内置盘监督者，摘要 `a8aae1653910a4625c879caa62f730e5131207eb08bfb68877334f4eb3b1d4b8`。收据明确记录基线 HEAD、工作区脏状态、关键源文件和运行器摘要及运行期间源码不变；**HEAD 只是基线，不能把 WIP 二进制记为该提交已验证**。其后又补严 Grok 已选技能图片门禁，以及仅 Claude 的静态 GIF 首次/恢复校验；上述原始收据没有覆盖这两项后续修正，不回填为修正后通过。最终重建与受影响模块回归已通过（见下节），静态 GIF 后续已用 `84102bb1c` 对应 libtest 完成在线窄复验；20 个构建源文件均与此提交 Git blob 一致，旧 WIP 记录不回填。 Grok Python 运行器后来增加 Linux 临时目录回退，但没有本次 Linux 在线结果；Windows 运行器明确拒绝执行，认证副本 ACL 与在线验收另待实现验证。Rust ignored 测试可跨平台编译不等于 Windows 产品链可用。

### 普通终端、技能与独立图片判断

本次普通终端探针均为 macOS 手动驱动的原生 PTY，产品源码改动未在该探针执行：Codex `0.156.1/gpt-6-luna`、Claude `2.1.280/Opus 5.5`、Grok `1.0.41/grok-4.7`。

- G07：三款分别通过原生 exec/read 工具读取中文与空格路径的独立标记；Claude 第二文件的 No 拒绝未读取。它证明路径引用合同，不证明修改后的文件卡片 GUI 链、拒绝后草稿恢复或 Linux/Windows；Grok 产品仍受 G01 自动发送守卫阻止。
- G01：新空会话超过 66 秒没有 idle_prompt；help 模态、未提交中文草稿及后台 sleep 仍活跃时却可能出现 idle_prompt。审批等待 76.7 秒只有 permission_prompt。该通知不能作为 Enter 安全或编辑器为空的证明，守卫继续保留。
- G03：Ctrl+V 后 bracketed text 粘贴造成同一回合两张图片，负例保留；Ctrl+V 后普通 UTF-8 两行配合 Alt+Enter 得到恰好一段文字和一张图片，并正确识别红蓝。无剪贴板消费 ACK 与可靠输入就绪，尚未接通自动链。
- G06：Claude `2.1.280/Opus 5.5` 的原生 PTY 与 stream-json 取得同轮两技能依次审批/调用、运行中新增技能、reload_plugins 返回准确命令列表及冷恢复读取新技能标记的正例。只限无副作用技能原生合同；产品实现由后续增量处理，注册事务、并发、父权限上限、Grok 多技能及两平台均未计通过。
- V04：Codex `0.156.1` 独立原生 exec 的 `gpt-6-luna` 位置错误保留；同图 `gpt-6-sol` 返回 `YELLOW BLUE RED LIME`，独立目视确认 LIME 对应亮绿且顺序正确。原收据 strict_match=false 不修改，语义判断单列；该正例不是 GUI/产品适配器链，也不解释原 GUI 错误的全部原因。

### 当前门禁与双语布局

集中定向测试曾为 1,600 通过、16 失败、60 跳过；不能称全绿。Grok 旧“所有图片拒绝”断言已修正，内置临时目录复验 atomic 15/15、runtime host 31 通过/1 跳过。2026-09-26 最终输入源码通过 `cargo check -p warp`、`cargo build -p warp --bin infinishell` 与 `cargo test -p warp --lib i18n::tests`（11/11）。复制到内置盘并核对 SHA-256 后，受影响模块通过：managed_input 27、Claude 169、Grok 383、普通终端 footer 45，合计 624；另有 21 个在线用例跳过，不能计通过。该轮覆盖新 GIF 边界和 Grok 已选技能门禁，libtest SHA-256 为 `e2ab62925d6aae589eaf00d6d3d1f3b4feb4056f3b242551b42ec35c5fe0fe44`；构建的 20 个源文件与 `84102bb1c` Git blob 一致，绑定证据见 `input-final-local/commit-binding.safe.json`；Linux／Windows 相关验证运行 [36158594977](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36158594977) Linux 成功，Windows 两项新增附件投递测试失败，整体失败，不计作在线模型验收。Windows 的 `cargo check` 通过；定向测试 2,778 通过／2 失败／4,769 未选或跳过。两失败把原始 Windows 路径与 JSON 转义路径比较；工作区已改为准确数组/字符串断言，本地 footer 45 项复验通过，Windows 同提交复验待补。Linux 主应用定向 2,916 项通过，未选、零模型和旧版 Codex 原生脚本均在各自 scope 中明确区分。

macOS 重启各语言进程后，Claude 图片格式说明与 Grok 顶部新说明的 en/zh-CN 原图均可读，完整换行且相关按钮可见，原图 2560×1600、逻辑窗口 1280×800；只验此视口布局，无模型输入。GUI 使用上述 WIP 二进制（签名后摘要 `8f42725d3ab4c9f4c2ea8d4efb7e71edb29010abf3f0303e3edc3cbbe4d73715`）动态读取当前 Fluent，早于最后两项 Rust 门禁修改。旧 live-switch 混语言截图保留且不计中文完整布局；不计最终 SHA、错误提示/审批全视口、Linux/Windows 或 V05 完成。

### 2026-09-26 后续实现与窄复验

- `84102bb1c`：Claude `2.1.280/claude-opus-5-5` 静态 GIF 新建和冷恢复窄复验通过，原收据因无关未跟踪探针记 dirty=true；另附关键源码与 Git blob 的逐字节绑定，不改原收据。
- 同提交 macOS GUI：JPEG 原始 MIME／字节和四象限回答正确；第二轮 READY 后应用退出再启动，重新关联原进程，消息仍为两条。断开确认原 PID 消失后，历史继续使用新进程和同原生 ID；此时尚未新增消息，随后显式发送纯 PNG，原生内容仅一个 image 块、无 text，独立象限回答正确，持久消息变为三条。区分重关联、冷恢复、发送，不把恢复按钮当作新输入投递。范围为 Inherit 根任务，非父权限证明或三平台完整生命周期。
- 同提交 Grok GUI 选择 PNG 被旧通用选择器拒绝的负例已保留；托管运行时已支持图片，但入口遗漏。当前工作区已修正入口，仍要求固定版本、继承模式、无已选技能和原生模型再次核验，后续修正后的双图 GUI 与恢复正例见下文，旧拒绝不改记通过。
- G06 实现来源 `3b4d8e8a3`：空插件槽、无图多技能、严格 reload_plugins 确认、事务回滚、旧回调隔离及接收后持久化已接入。该来源的 macOS 生产适配器三轮在线通过（多技能→新增→冷恢复，6 次真实 Skill 单次审批），两代正常退出。原模块首轮 1,022 通过／3 失败／52 跳过，单线程复核其中两项通过，一项监督控制通道 os35 仍失败；不称全模块全绿。后续整合门禁已通过；Claude Mac GUI 双技能及同会话热新增实际 native Skill 逐次允许通过，后续热技能恢复修复与 GUI 复验见下文，跨平台仍待补。
- G10 实现来源 `9f5967f62`：独立 V2 文件创建策略保持父权限版本匹配，逐次审批、路径范围和允许时再次校验；固定 `2.1.280` 的原生 Write 允许／拒绝／越界／符号链接探针已通过。V2 父子与 V1 禁升级的生产协调器验收夹具来自 `de135f92c`，后续两条生产协调器链已通过，范围见下文；原生探针与真实父子链分别保留来源。

- 整合工作区内置盘首轮：i18n 11 项、运行时 1,038 项通过／53 跳过，`cargo check -p warp` 与应用构建通过；之前 os35 的恢复场景本轮单线程通过，原失败不改写。外置盘断连导致的编译 SIGBUS/os5 和主动中止重试仍保留；经用户授权改用内置缓存，并仅清理已核对无占用的 3.18 GiB 旧运行程序。后续修正须另补门禁。
- G05 整合工作区生产链：固定 Claude `2.1.280/claude-opus-5-5`、macOS arm64、Inherit 根任务的图片＋单技能新建与同会话冷恢复通过。两次精确 Skill `AllowOnce`、两组独立象限颜色、原生图片字节/工具身份、两代 `stdio_closed/exit 0/cleanup_confirmed` 对应完整；libtest `ec718473…`、监督者 `48018a00…`。不外推多技能图片、父权限上限、GUI、应用重启或跨平台。
- G10 首轮 V2／V1-ceiling 生产父子链均失败：G06 新逻辑把正常邮箱 `MessageAccepted` 当作 self `user_input`，返回错误的启动不匹配。子进程未取得原测试完整清理收据，之后按精确身份补救清理另记；已修正为只有自身用户输入登记技能，其他邮箱保留原回执校验；后续还修正已持久化邮箱 joined 重放的同类误判，两条新构建生产链已独立通过，见下文；原失败和当时缺失的子清理收据不回填。补救前仅确认 launchd 项已登记但 not running，不能推断原生进程当时仍存活或由补救导致退出。
- Grok 整合 GUI 已能选择 PNG 并保留图片卡片，但新建时出现 `post-response setup phase arrived before session/new response`，没有进入模型或确认原生会话，缺项仍开放。原始 journal、未确认输入和清理收据保留；不能把卡片显示算作图片投递成功。原生正负对照已定位为缺省配置走 direct 模式：仅有 `--leader-socket` 不强制 leader。工作区对固定 1.0.41 根任务显式传 `--leader`，SDK／固定策略仍为 `--no-leader`，协议阶段检查不变；后续新构建双图 GUI 已通过，见下文；原始握手失败不改记通过。

### 2026-09-26 整合修正后的限定验收

以下仍为 `84102bb1c` 基线上的工作区摘要绑定，**不是最终提交或三平台完整验收**。内置盘 `fixed` 轮来源摘要为 `b8c832473e548e8213eb83cd29422f6ecebf8879e8ed1718677dda1073e92632`，i18n 11 项、runtime 1,044 项通过／53 跳过、check 与应用 build 通过。随后邮箱 joined 重放修正的 `replay-fixed` 轮来源摘要为 `d5a8790a63fdcf7c267dd8bf99c525f59781f9ca85102226d550c066478177bc`，coordinator 91 项通过／4 跳过，i18n 11 项、check 与应用 build 通过；未无变化重跑整套 runtime。该轮 libtest SHA-256 为 `c7cf03f14893dd82eceb80cdf39fabf1cfe2f2e155076f27f2d86f2922a2f595`，监督者为 `8484a0c900e2c596f95d3d2ff4c473e1814f18713540c8fef8e0191e8e405e95`。跳过项、Windows 原失败与外置盘构建失败保持原结论；同提交相关平台验证待补。

恢复清单、身份隔离及 ACK 顺序修正后的源码快照为 `05e7fcbb8d4e8ef98e924ccf1b17426564239b3b2313639ab34a4fc100306461`（36 个构建源文件）。i18n 11、coordinator 101 通过／4 跳过、`cargo check -p warp`、应用 build 全部通过；此前未再修改的 task-manager 46 项通过。新增真实 SQLite 故障测试验证 ACK 失败不改技能、ACK 成功但清单失败后重放零发送、错误会话不提前 ACK。libtest `7a3955070f01470a00cb2fb970ed2b0757c14325384ac82b6d94b84c3021e05d`，监督者 `db931fe468a13b7a7d30d4850ddee763606f7527b15d24e87af24bd8b6c1f884`。较早恢复构建被主动中止以整合隔离补丁的记录保留，不记作编译失败或通过；GUI 和在线结果仍使用下列各自原二进制来源。

- **G02／V04 Grok GUI**：macOS arm64、`1.0.41/grok-4.7`、Inherit 无技能，两张独立 PNG 的原始字节和实际象限色回答分别通过。首次断开并退出应用后，历史继续使用新宿主/原生进程和同原生 ID，发送新输入前消息仍为 1 条；第二轮完成后退出应用，宿主仍存活，重启精确关联同宿主、同原生进程和代次 2，消息仍为 2 条，没有重投。两代均 `stdio_closed/exit 0/cleanup_confirmed`。两轮分别绑定对应源快照与签名后应用摘要，不能把第一轮改记为后一轮构建。英文／简体中文权限、附件、技能限制、消息及历史结果完整滚动区无截断遮挡；不计 Linux/Windows、真实 IME 或未执行的审批交互。
- **G05／G06 Claude GUI**：固定 `2.1.280/claude-opus-5-5`、macOS arm64、Inherit 根任务，三轮真实 GUI（双技能、热新增、PNG加单技能）共5次精确Skill AllowOnce，原生历史模型为claude-opus-5-5且图片字节摘要匹配；恢复修复前错误及原始记录保留。修复构建重关联同宿主/原生进程、同会话和代次3，仍3条输入，原生历史字节不变；正常断开exit0并确认清理。英文和简体中文相关说明、技能列表、消息与历史结果可读。模型轮次与重关联程序分别绑定各自源码/二进制，不宣称新构建重跑模型。 旧宿主签名后摘要 `b014a98f…`，恢复应用为 `8dd332a3…`（源码快照 `e01dee11d49901d81b1a427916aef5d87e78709546847a2de30f140615694f76`，未签名程序 `b4f56cb5…`）；持久 OwnerClaimed epoch 1→2，原生会话 `33ae1e46-47b6-4865-bf8d-008d6630e0b9` 不变。恢复后通过显式“刷新”加载 alpha/beta/gamma 目录。原生历史摘要 `c6668675b2002c0e53d61c457643d6cbda70375497da4dbf9223551a6fd013af`；原 PNG 与原生图片块同为 `ea9dc58742e14577738c00e60ab53214bf9863b7a0ee6b57545426d79d742c9f`。不证明父权限上限、图片加多个技能或跨平台。
- **G07 GUI 入口复核**：macOS arm64、Codex `0.156.1`、应用摘要 `8dd332a3…`，真实“附加文件”可以选择文本文件，但只把路径插入正文，没有生成 PendingFile 卡片。旧点击未生效时 Open 禁用的截图保留；随后键盘选中并点击 Open 成功，撤回“文本文件不可选”的误判。两会话均零模型输入，清空草稿后正常退出；不计文件读取、投递或 G07 完成。
- **G06 Grok 原生合同**：默认 profile、显式 `agent --leader`、固定 `1.0.41/grok-4.7`、macOS arm64，2 次模型输入证明同轮首技能原生展开、后技能通过 read_file 读取完整正文，以及同会话新增技能须显式 reload 后才注册/展开。另一次零模型双目录探针证明 reload 会刷新同 leader 两会话，即使参数仅写一个 sessionId；不能当作局部刷新。两轮 client 自然 exit 0，按精确归属清理各自 leader。零审批请求，不计 InfiniShell 多技能/热新增产品接通、严格串行多技能执行、父权限上限或 GUI。未信任第二目录的早期作用域观察和离线校验器失败均保留。
- **G10 Claude V2／V1-ceiling 生产链**：固定 `2.1.280/claude-opus-5-5`、macOS arm64，使用上述最终内置 libtest/监督者。V2 精确 Write 允许生效、拒绝后文件不变；实际 5 次允许／1 次拒绝。另一条真实 V1 父记录请求 V2，因 `claude_profile_parent_mismatch` 在建立原生连接和发送输入前拒绝，父记录不变；该链保留 V1 Edit 效果。两链分别 4 次 MCP 工具、5 次输入、3 次原生执行、2 次 joined，双向消息和自动结果 ACK 完整；四代共 12 张原始清理收据齐全，host/native PID 全部消失，launchd 项均注销，无补救清理。
- **G10 运行器修正范围**：V2 Rust 测试及完整私有树已通过，但首版公开投影误把 inspect 明确标记截断的嵌套 JSON 当完整 JSON 解析，wrapper 失败。修正后只对布尔 `body_truncated/evidence_truncated=true` 的有界副本保留原字节摘要，未标记的坏 JSON 和损坏完整证据仍拒绝。原失败 metadata/原始树保持不变，同一次 V2 原生执行重新投影通过，没有新增模型执行；两个 Python 脚本 before/after 摘要另记，Rust blob 未变，离线测试 25+70 项及 py_compile 通过。待审批 Write 取消、活跃宿主重关联、冷恢复、GUI、Linux/Windows 及命令/技能扩展不在本轮范围，G10 仍开放。

### 6790b185 提交绑定与文件选择器后续增量

技能、V2 文件策略和恢复修正已提交并推送为 `6790b185a0c0bc581f804edf975960047df85db7`。最终快照 `05e7fcbb…` 的 36 个构建源文件与此提交 Git blob 一致；绑定收据摘要 `613cb19968667392a08829f4f831030af6da4a2fca7d9fa58ef538e4e6b80060`。相关平台运行 [36176479030](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36176479030) Linux x64／Windows x64 两项作业均成功，Intel 作业跳过。该定向工作流不包含已认证在线模型、物理 IME 或 Grok 专属双语完整布局，不将相关门禁外推为完整验收。

随后 G07 四文件增量接通展开富输入的文件选择器与 PendingFile 卡片，保留 CLI 锁定输入模式、取消和旧会话回调边界。源码快照摘要 `e02d7239b30e43019a209688348eb23bf7b76246b0798267d291d00ebb73477e`，基线为 `6790b185`、工作区 dirty；内置盘本地 i18n 11、editor 6、footer 46、check 和应用构建通过。libtest `a2de24af1e607ae5b6a479fb100cc61521a27cd4e0e85fa8480f675b414670e8`，签名后应用 `d383765f2e24f0f49747e25a1ec3f8d69a9f2237bd9b0586c8a986eeb17d2688`。无需本地化变更，既有“附加文件”语义适用。

- macOS arm64、Codex `0.156.1/gpt-6-luna`、普通终端 read-only/on-request：真实选择器添加两卡片，中文/空格路径以一个 JSON 数组随正文一次到达原生请求。中文首轮模型转向 TextEdit，拒绝后未读取内容，原失败保留。英文独立正例仅关闭官方 computer_use/browser_use 工具功能，保留原安全策略；模型调用原生 shell，工具输出及最终回答均匹配两份文件的独立标记。会话分别为 `01a0d9f8-a040-7950-b03e-82202114b1ec`、`01a0d9fd-cb30-7bf3-8a07-b4d1aef1b6b7`。两语言卡片和正文布局可读，不计物理 IME、允许/拒绝完整生命周期或跨平台。
- Grok `1.0.41/grok-4.7`、普通终端 default：实际创建两张文件卡片，Enter 后未发送提示完整且卡片/正文不丢；零模型回合。该结果仅证明选择器和保留行为，G01 未接通前不算 G07 投递成功。
- Claude `2.1.280` 认证状态仍 loggedIn=true，但普通 TUI 首次启动向导未完成；本轮未发送模型输入。未改全局 Claude 配置跳过向导，托管模式已有证据范围不变。
- 本轮所有执行程序和缓存均在内置盘；应用 PID 55371 及受控进程树未发现打开的外置卷路径。旧实例误选截图、一次命令输入丢下划线导致的零模型启动错误分别保留操作边界，不能算新构建缺陷或有效验收。

### 仓外证据入口

本机持久归档根为 `/Users/zhishi/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/`。以下为根下相对路径；原件逐字节保留，认证工作目录未归档，跨机器不能仅凭本机路径认定已验。

| 证据 | SHA-256 与范围 |
| --- | --- |
| `g07-picker-gui-20260926/index.safe.json` | `1432fbbf90e6b9b0e82a2cffb6826e6082f112c705c74d07eb872e6ca6d59162`；43 文件，GUI 原图、Codex 原始失败/成功会话、内置执行路径与本地门禁 |
| `input-final-local/commit-binding.safe.json` | `84102bb1c` 的 20 个构建源文件与 Git blob 一致；本地门禁原件与运行程序摘要同目录保存 |
| `claude-gif-84102bb1c/receipt.json` | 收据 `8de6ba3946bfb5b4108516898990d49670e55f7712400b3ec46a69283538f5e6`；同目录源码绑定 `88935b4822d11f13a5c0579be07390ef0fda7751e82f83fd9fc64625f95df5d7` |
| `gui-input-84102/index.safe.json` | `53054d274bee66768574f1a556d1c6cd4b3270992e740b57f968bed2cb8a8ef5`；27 个文件，含 JPEG／纯 PNG 原生行、SQLite 投影、英文 GUI 和独立恢复证据；scope `1b7f054843239e966dcc9c6da9a58c93d6e1d4b55432583ee6525856882a2b88` |
| `claude-skills-adapter-3b4d8e8a3/index.safe.json` | `fb448a1a509ec50871993eda2c65c74dd5ff2c5e5c72fd839de244270379b0ba`；12 个原始文件，三轮真实适配器及模块失败/复核日志；scope `6f016b661f7ff58eedabb154b36fcd6abd2098e64cbe68f56a9f979ee818b8ed` |
| `claude-image-skill-integrated-wip-v1/index.safe.json` | `24a2b2367eb5125b05842e1793a11eda54129a8854b6c279492487660f4c7b85`；G05 20 文件，收据 `a1a015dec9718047f26cea2d4cffa0819e3a752c4c43527a548ff18b27a8feb1` |
| `cloud-input-84102/index.safe.json` | `abe10089eb66d5518a7f49bf572e669bd4e4aa1665519bb913c9df70e7b6f282`；Linux 23 文件；采集时 Windows 尚在运行，旧索引不回填 |
| `cloud-input-84102/windows-job-108149210250/index.safe.json` | `181418248d53df176440faabc6a56c74341ee53511451ef58e007ea12e830384`；Windows 46 文件、8 个 Actions 产物摘要一致，整体失败 |
| `internal-integrated-gates-20260926/index.safe.json` | `785f001b60cbb04bbf243261b6981c436e3be6f91809d19d3f7654e1f733933a`；29 文件，fixed 与 replay-fixed 的各自源码、构建和门禁；不是最终提交/云端验收 |
| `g07-picker-negative-20260926/index.safe.json` | `f41dad825484fc447a7fe03a534808011341eaa97b1143d591da4e2475ae5ea1`；原截图/AX保留，类型过滤判断已撤回 |
| `g07-picker-recheck-20260926/index.safe.json` | `653e648cd37a62a1fa1b74d19f726d3edde34af6a20788c0e097b70cbc926b7e`；键盘可选的反证及路径正文无卡片的真实入口缺口 |
| `claude-recovery-local-gates-20260926/index.safe.json` | `c446536dc2eb113f75941666bc562963619aabc48ec975799ba4064e0a9baa0a`；36文件，三轮各自源码/门禁、主动中止、最终ACK故障回归及程序摘要 |
| `gui-claude-hot-skills-20260926/index.safe.json` | `a13e13a571adec83aae6361398313dda3e7cf47342301c091f1840d70079d737`；70文件，三轮技能/图片原生历史、重启原失败、修复后零重投重关联、正常清理及英中原图 |
| `gui-grok-two-images-20260926/index.safe.json` | `64726f010ae77e59c480df88efc704fb5597b984832807c94c5bc755135bd735`；47 文件，双图、两个独立恢复模式、原生字节、两代清理与 macOS 双语滚动区 |
| `grok-g06-leader-default-20260926/index.safe.json` | `c9f7a13d0ca0c63fe4b22d4f78254c390b2dbcce494641b82b11390871bc6413`；368 文件，默认 leader 多技能原生调用及跨会话 reload 作用域；不计产品接通 |
| `g10-coordinator-post-fix/index.safe.json` | `8d30cf54cc0fa3d6c39a0037a020787229364200950aea3d28dd7408f718b42c`；41 文件，V2／V1 父上限两条生产链、投影原失败及零新增模型重验、四代完整清理 |
| `g10-coordinator-pre-fix/index.safe.json` | `6e7921157149f50062b7e39c76c36cebf86142149a8a2e54fb2c7b853dbd6a3e`；V2／V1-ceiling 原失败、源码绑定和补救清理分别保留 |
| `gui-grok-pre-fix-20260926/index.safe.json` | `abaf004d0efefb645718add14529c0575696f087ec8d25ea103414ffdff1a1e7`；GUI 图片卡片正例及握手负例、原始未确认输入与已确认清理，零模型输入确认；不计物理 IME |
| `grok-setup-mode-1041-20260926/index.safe.json` | `08a82a439f5e6cf1e85e912f0e07b9d41da7d5eec73e5fea2318ecb48f3d74cc`；8 次零模型原生模式探针，缺省 direct 负例与显式 leader 正例，56 个索引文件 |
| `image-evidence-sha256.json` | `ad4bb6a4da73f41b44c25c08844a259386a63bbcacda304fd1bd02bd0d288308`；55 个图片证据文件的源/目标字节摘要，含两个完整证据目录 |
| `infinishell-claude-rich-images-evidence/production-image-evidence-wip-index.json` | `3ef53b8a10968af6a9549f4986b2a01228f38e88bb1ada109cd239639b90a39f`；四个格式/纯图生产用例；同目录 native-skill-v4/v5 保留失败/成功 |
| `infinishell-grok-managed-image-wip-v1/receipt.json` | `53f51826fc669de485ebc8a0a1c33f803feca9ea2ceaa0737b3caf8267421154`；原生图片投影文件摘要 `d0ba084e5c437f1b9383378dfdf3ee499861b3cfc7b5ed793ed1f0e4e2fcb05a` |
| `terminal-files-native/scope.safe.json` | `25285aecbd815b9cc3389acd6f22025a6b5491c296fd313013183c9ebcfb2987`；原生 G01/G03/G07 的来源和负例；原 index 为 `25a22584b2ec0cabc3f7143e035c866a188e6f6c5cb4aae9dfaa9bfa8e5e7f3f` |
| `claude-skills-native/scope.safe.json` | `10d7e33219112755c0c72a076ef5941ed48cd1daeb0aa8419686f560be45acf0`；原生 G06；原 index 为 `271c28ffb75cd75c46f09149ad5d4b57cda3eff4648ad62633e6bca8a1162f19` |
| `codex-image-luna-native.archive-index.json`、`codex-image-sol-native.archive-index.json` | 分别 `2c8c10286e4d581f7cbb390520481442ba40c90e143ef929820f02047d5e5259`、`47f7905eadcc9ce7e46c23b991597aa004cf1a4f88bcc88f993d753a88005e56`；原错误及独立语义正例 |
| `gui-layout-wip.archive-index.json` | `feea309ee4d7e61054d6987f17c4838fd8280af0b30dda0de6e8250dfe4bc90f`；四张通过原图、旧混语言负例及 layout-review.safe.json 的 WIP/资源来源边界 |

## 历史三平台相关门禁

`ee839b4fc` 的 macOS arm64 本地门禁与 [Linux／Windows 集中验证 36103244022](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36103244022)记录如下。该次 Windows 作业整体失败，不能由其他通过项覆盖。

| 范围 | macOS arm64 | Linux x64 | Windows x64 |
| --- | --- | --- | --- |
| 桌面工作区测试 | 10,895 通过、96 跳过 | 10,923 通过、100 跳过 | 10,680 通过、112 跳过 |
| `cargo check -p warp`、i18n、默认入口 | 通过；i18n 11 项 | 通过；i18n 11 项 | 通过；i18n 11 项 |
| 实际双语 GUI 目视 | 6 张有效原图通过 | 4 张原图通过 | 4 张原图通过 |
| 原生 CLI 原子升级 | 复用各自固定构建的三款实链 | 12 场景通过 | 三款正式升级通过 |
| 直接 Grok ACP 五秒 EOF | 不与托管清理混计 | 按原探针来源保留 | 5,005 ms 时未退出，之后自然退出 0；五秒观测仍失败 |

桌面工作区集合排除 `command-signatures-v2`、`integration`、`warp_tui`；GUI 与受影响 TUI 单列，不等于所有仓库测试。macOS 的 TUI 消息状态 9 项通过。Windows 4 项 leaky 属于通过子集，不证明后代进程已退出；macOS 3 项、Windows 1 项 slow 按原记录保留，跳过项不计通过。

双语 GUI 只覆盖原收据列明的视口。Linux／Windows 使用 Codex 页面与系统剪贴板，不证明 Grok 专属权限界面或物理中文输入法；Windows GUI 使用实际 Mesa GL/llvmpipe 窗口，不外推为 DXGI 全功能验收。

## 真实 Grok 进程清理补验

`50e1515bc` 的 macOS 提交后实测和 [Linux／Windows 集中补验 36116931025](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36116931025)均通过。该次只补相关原生模式；工作区、GUI、原子升级沿用上节的原来源，没有再次执行。

四场景分别在私有 leader／无 leader 两种拓扑执行显式结束与标准输入关闭后结束，经过生产 `ManagedChild`，核对真实进程身份、代次、stdout EOF、旧代次拒绝及已绑定进程退出。

| 平台 | 已通过结果 | 不能据此推定 |
| --- | --- | --- |
| macOS arm64 | 提交后四场景通过，源码干净；之后同提交 `cargo check` 也通过 | 二进制是核对摘要后复用的内置盘副本，不是四场景前重新编译 |
| Linux x64 | 定向 847 项、原监督 5 项、真实 Grok 四场景通过；取得 `linux_subtree` 清理回执 | 无 leader 场景退出码为空，不能称全部自然退出 0 |
| Windows x64 | 定向 747 项、原监督 5 项、真实 Grok 四场景通过；真实进程句柄与严格 Job 清理 | 两个无 leader 场景为退出 1，只证明清理，不是模型任务成功 |

此补验没有认证或模型输入，不替代审批、用户 Stop、历史继续与应用重启的完整在线链。独立直接 ACP 的自然 EOF 与 leader 强制清理也分别计证，不能冒充生产监督器证明。macOS Intel 备用作业未执行。

## 历史固定版本产品链已验范围

以下主要来自 Mac 的已认证原生执行及真实 GUI，均保留原构建与模型来源。Grok 模型为 `grok-4.7`，Claude 父子链为 `claude-sonnet-4-6`；不同模式不能相互替代。

| 能力 | 已验结论 | 范围限制 |
| --- | --- | --- |
| Codex 托管父子任务 | V9 双向原生 ACK、父权限上限、自动结果 ACK、显式继续后持久结果回收通过 | 不是完成态父任务自动唤醒证明，也没有该用例的子文件效果或 GUI 证明 |
| Claude 托管父子任务 | 审批、工具、子文件效果、双向消息 ACK、inspect、结果 ACK 与清理通过 | 仅相应固定权限策略，不代表任意工具或原生 shell 可用 |
| Grok 托管根与子任务 | 根任务两轮、允许／拒绝、排队、取消和同会话恢复通过；V8 父子 ACK、结果回收及跨宿主冷恢复通过 | 固定读取／文件策略与继承设置模式分别验收，不能混称操作系统沙箱 |
| 三款 GUI 恢复 | 存活宿主重新关联、同原生会话冷恢复及结果回收有实链；重启不重投旧输入 | 源码代次与宿主身份必须核验；不等于所有平台均完成该链 |
| Grok 多轮重启 | V10 两轮后应用重启，输入数未变，第三轮正确回忆；V11 空闲目录刷新不新增回合 | 目录刷新不等于任意新技能均能在运行中使用 |
| Codex 富输入 | 附件类型化通道、技能、评审意见及文件引用实际进入原生请求 | 图片颜色回答错误，不能计图片理解成功 |
| Claude 富输入 | PNG 原始字节摘要一致，图片回答正例、单技能执行及重启恢复通过 | 当时仅 PNG；本次静态格式/纯图的新增范围见上节，组合与完整验收仍有缺项 |
| macOS 中文输入 | 物理拼音、marked text、数字／空格选词、中英混输及多行保留通过 | 此 IME 用例未提交模型，系统候选浮窗未单独截图 |

普通 PTY 生命周期补验共 16 次输入，三款允许／拒绝、继续及应用重启后同会话恢复有证据。Claude／Grok 取消使测试进程退出；Codex Escape 仅中断对话，测试 shell 自然结束，因此保留 Unknown 与显式关闭终端，不能称工具取消清理通过。

插件与原生通知已有三款实链。Grok `0.1.4` 的通知桥接由产品安装器管理，保留配置禁用、冲突与去重门禁；Windows 安装、`0.1.3 → 0.1.4` 更新、同版本修复、禁用保全、失败回滚和再修复在 [36028161259](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36028161259)通过。插件安装测试为零模型请求，模块导出不证明普通 TTY／GUI 通知全链。

## SSH、tmux 与升级

三款固定版本已在 Mac 本地 PTY、Mac 到同机隔离 OpenSSH、tmux 透传开启且断连重接场景取得真实 hook 与产品会话配对。不是远端 Linux／Windows／WSL 验收；Grok 该批模型输入为 ASCII，不计中文输入通过。

关闭 tmux 透传时，Claude／Grok 已观察到内层 SessionStart 与外层零接收；Codex 补输入后外层零接收，但内层原生发送未独立观察，负例不完整。Codex 首轮 tmux 的 Return 换行疑点未独立确认原因，不宣称已修复。

三款官方原生安装的固定升级为 Codex `0.155.1 → 0.156.1`、Claude `2.1.278 → 2.1.280`、Grok `1.0.40 → 1.0.41`。Mac 正式事务及 Linux／Windows 后续原生事务按各自提交通过，包含目标版本／摘要、配置保全与清理；包管理器来源的缺项以 G09 为准。

Claude Mac `2.1.280 → 2.1.267` 的 Latest → Stable 真实渠道降级通过；其他渠道切换只保留各自历史版本和源码域结论。更新偏好在 GUI 重启后恢复，不能由偏好持久化推定升级事务成功。忙碌延期、启动预约、回滚及恢复有对应回归，不等于正式升级样本曾在运行中的模型会话内执行。

## 失败、修正与未验结论

| 原始问题 | 后续结论 |
| --- | --- |
| `9af6de393` Linux Grok 中断升级退出超时 | `ee839b4fc` 修复启动取消窗口并取得 12 场景独立通过；原超时未被追改，也未声称唯一因果已证明 |
| `ee839b4fc` Windows 直接 ACP 五秒 EOF 失败 | 迟到自然退出单列；随后新增真实监督清理门禁通过，仍不承诺原生五秒 EOF |
| `14e0d7d42` Linux 身份绑定／Windows 输入路径检查失败 | `50e1515bc` 修正线程 TID、路径与实际对象身份检查后通过；原失败不能计作已执行的模型场景 |
| Mac 外置盘 worker 启动受阻 | 栈采样显示未进入 Rust main；用户说明启动需要授权，之后核对摘要并从内置盘启动，不放宽超时或权限 |
| 早期 Windows 编译、更新和 Grok 迁移失败；Linux 中文方框 | 分别修正后取得窄范围原生补验及 GUI 正例；旧失败与其真实来源仍在固定历史提交 |
| Grok 技能旧 runner 为 false、早期冷恢复失败 | 技能原生两轮由独立离线复核确认；冷恢复另有 V8 实链。不能把原 runner 或旧构建改记通过 |

完整在线生命周期的 Linux／Windows 覆盖、两平台物理 IME、远端 SSH/tmux 组合及 Grok 专属双语布局仍未完整满足。原生图片、技能、普通终端富输入、远程附件、包管理器升级和子任务工具范围的当前缺项统一见 [KNOWN_GAPS](KNOWN_GAPS.md)，本报告不重复维护其关闭状态。

本报告不计为合并、发布或 Goal 完成。后续结论更新应同时写明提交、CLI 版本、平台、模式和实际通过／失败／未验范围；新增原始日志与截图置于仓库外，只将必要结论与可追溯引用留在专题目录。

[history]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/tree/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity
[acceptance]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/ACCEPTANCE_FIXED_VERSIONS_20260924.md
[mac]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md
[final]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/FINAL_SAME_COMMIT_VERIFICATION_20260925.md

## 2026-09-26 Grok 多技能与会话内新增增量

本增量基于 `83e5115839b301697f3322092f487ddce9d72dfe`，仅开放 Grok `1.0.41/grok-4.7`、Inherit、无 profile/model 覆盖／本地父子工具的私有独占 leader，最多 32 项本地技能。首技能原生展开，其他技能由原生工具读取；不保证工具严格串行。热新增等待准确 reload ACK、同会话目录及路径／内容核验，原生接收与技能清单落盘分别持久化。

macOS arm64 生产适配器 v3 三轮通过；真实 GUI v3 首轮双技能、v4 同会话新增 gamma、两次应用重启重新关联活跃任务、停止旧进程后新进程继续同原生会话并执行 gamma/beta 均通过。三轮原生输入、技能正文或 read_file 原字节、模型最终标记、NativeProtocol ACK 与产品结果逐项核对；零自动重投，两代退出码 0、资源组和后台任务清理已确认。v4 原始构建 SHA-256 `29278201780c87a6869efc34729d16e40d71025ea2995bdd0093568df6e3c599`，内置盘签名运行文件 `9ff3a223d5d45610f9ade22e4836c4c2576708aecfcb7d783e3c7c17827fb939`。两次重启无权限弹窗；英文与简体中文长说明、权限、技能卡、消息及结果完整滚动区检查可读。

本地通过：i18n 11、Grok 454（15 个在线专项默认忽略，三轮专项另有真实运行）、协调器 88、本地技能 12、宿主 31（1 个在线专项忽略）、任务面板 59，`cargo check -p warp` 与应用构建。v4 仅修正界面将已完成输入关联误判为忙碌；未变协议／持久化测试按源码摘要复用。v3/v4 的源码、二进制和各自运行范围独立绑定，不冒称单一二进制全量重跑。

保留旧握手阶段顺序失败、未信任项目技能目录失败、热新增入口隐藏、输入工具丢中文的未发送草稿及第三轮尚在运行的中间快照；最终成功不覆盖这些记录。原始日志、截图、负例、审计和源码索引共 284 文件位于仓外 `resume-20260925/validation/grok-g06-20260926/`，`index.safe.json` SHA-256 为 `3606b77b9c1e716e1d85f4a1705213dea5170c8b4ca694ea029635482acb3710`；对应实现已提交为 `fe135e3436c22eaa7857c31bfaf94969eb1aa11f`，独立提交绑定 SHA-256 为 `3419077aa75a1af26e05c9f5114c43cf8190ac179df1ae11ff27b7937dc2e9d2`。[Actions 36213944594](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36213944594) 的同提交 Linux/Windows 定向门禁均通过，47 个原始日志及产物归档于 `validation/cloud-g06-fe135e343/`，索引 SHA-256 为 `323bb6af8d3df103331f0069657e2c6c30bafeee60e7b54e0a85ee4f4403fcfb`。两平台已认证模型与 GUI、用户级来源和固定权限模式仍未完成；G06 及原 Goal 保持未完成。

## G01 owned 普通终端基础增量（2026-09-26）

固定 macOS arm64 Grok `1.0.41/grok-4.7/default`：应用隐藏入口在真实 shell PTY 核对设备、父进程和前台进程组后 exec；生产 leader 侧车和 SQLite 一次领取完成两轮中文输入，每轮均有独立原生 prompt ID、精确最终 ACK 和 hook 模型标记。再次提交同一消息被持久层拒绝，恢复旧启动记录不能再次派发。PTY 身份直接从现有 master 文件描述符查询，不改变启动服务的二进制消息格式。插件 `0.1.5` 增加原生权限观察；通知本身不认证进程、不承担审批。

原始证据归档 `~/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/grok-owned-20260926`，146 文件索引 SHA-256 `d49082ee7e6290bf007f2b316c3db5c96b80967fcc40377e38df7cf407fbec33`。v1 两轮输入通过，但运行器关闭 PTY master 的顺序导致 shell 回收卡住；代理收尾及原失败均保留。仅修改验收运行器，v2 57.04 秒完成两轮和 TUI／leader／shell 回收，私有认证副本已移除。独立检查确认恰好两个原生输入和两个正确标记，SQLite 两条消息均已 ACK。复用相同程序和 libtest 摘要，未改产品源码来放行复验。

本地 v7 `cargo check -p warp`、i18n 11、PTY 内核身份 2、CLI 会话 413（6 项专门环境测试跳过）及应用构建通过；独立运行随后显式执行其中一项原生在线测试。未变更的持久化模块复用 v4 78 项，修复后的运行器 6 项通过。构建源文件摘要已逐文件绑定实现提交 `922308af60cbdd7f84ea69fb3e465ae08b303213`，独立提交绑定 SHA-256 `7e88b8fea31de65fa5d58639f8295083cebb78d67b706349d4b3c17579432ed5`；[Actions 36220234098](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36220234098) Linux 通知/安装验收因脚本仍要求 0.1.4、实际插件为 0.1.5 失败；Windows 安装前 `verify_runtime` 的 3 秒版本检查超时，原因与 Linux 不同。两平台原失败另存 `validation/cloud-g01-922308af6`，102 文件索引 SHA-256 `f2846ce145b2e1214b34f5c7a7688eaa70313ee2a2b21d475a5ce67198bde0f0`。后续整合提交独立复验，旧失败不改写。**不是 GUI、真实 IME、工具审批或跨平台在线生命周期验收。G01 保持开放，尚不能解除普通终端无条件拒绝。**

无需本地化变更：该增量尚未改变消费者入口和现有显示文案；后续 GUI 接线须同步英文／简体中文并分别检查实际布局。

本次恢复增量在上述基础上把侧车事件改为异步接收；真实退出先留清单再释放占用，SQLite 读盘前同步置忙，后台恢复仅关联已有启动占用，并隔离旧回调和重复定时器。普通 TUI 记录禁止进入托管新建、继续、自动重连及未确认宿主统计，避免第二个原生进程或重复模型输入。

macOS arm64 v4 `cargo check -p warp`、i18n 11、PTY 身份 2、CLI 会话 423（6 忽略）、持久化 78、完整协调器 111（4 忽略）、运行器 6 和应用构建通过。恢复专项原生两轮 53.09 秒通过；独立审计核对恰好 2 次 prompt_submit、2 次 stop、独立 prompt ID、正确模型标记及 SQLite 两条 NativeProtocol ACK。TUI、leader、shell 退出和私有认证副本删除确认。构建二进制 SHA-256 `f630b6362389f021b9b1c98578378adfc2e7767e220fc039572707d3ef606e02`；libtest `5f3238e4d6b34777deff6b08384d0b38e2ebb8118f514800b17dccf0cbdd07bf`。

内置盘签名后 GUI SHA-256 `0e5d6f237062bbd99a3b810e33a9db8f4b47a205c47f9d8c2ccf67664ca6e174`，独立空白 profile 英文/简体中文设置与 Grok 版本区域实际可读，未出现人工授权弹窗；两测试 GUI 进程均正常退出。无需本地化变更：内部恢复和通道调整复用既有文案。本次不是 GUI 输入、真实 IME、工具审批或有模型会话的应用重启验收；GUI 新启动登记/派发/握手、启动前期异常回收及平台验收仍欠。

原始证据 159 文件存于仓外 `validation/grok-owned-recovery-20260926/`，索引 SHA-256 `0c1fbc47dec6f0a7cd9c2bc7c8c6a6cc59c73a707dc0c95518096cb4386f8402`，含各轮源码和二进制；提交后追加独立 Git blob 绑定。v1 Timer 返回类型编译失败、v2 门禁通过后主动中止构建、v3 未确认宿主统计遗漏的失败均保留，v4 修复后复验。首张名为 chinese 的图实际仍为英文，重启后的中文正例另存，未覆盖原图。G01 与原 Goal 均未完成。

恢复实现提交 `044263a61b38fa95af32de2b758c1101aae08226` 已逐文件绑定上述 11 个编译源文件，提交绑定 SHA-256 `3491a456e67a69582ee2333d8314a78bdb7dca4e46c5f6a0deef4a0d0f1b6338`。基础提交 Linux 的两个失败门禁已定位为 Python 仍硬编码插件 0.1.4：实际安装步骤退出 0，原始 `passed=false` 收据保留。`4ed5b3d80` 同步严格目标至 0.1.5，旧版/未知版/缺失/混合版本均拒绝；本地验证器 15 项、真实原生 worker 17 项中 15 通过/2 个缺少 shell 场景跳过。稳定步骤键沿用历史名称，真实目标由 `current_plugin_version` 核对。没有重跑 Linux 安装或回填旧失败；整合后同提交跨平台复验。无需本地化变更。

G01 与原分支 G09 整合后再次通过本地 `cargo check -p warp`、i18n 11、PTY 身份 2、会话 423/6 忽略、协调器 111/4 忽略、升级 182/4 忽略、受监督版本探测 7/1 忽略、持久化 78、运行器 6、插件验证器 15 及应用构建。105 文件索引位于仓外 `validation/g01-g09-integration-20260926/`，SHA-256 `67b875a435b5408373166375e16adbe24b65cdeaa485b8f72146491666f18c64`；没有重跑在线模型，原先各轮来源不变。整合提交将从原分支执行同 SHA 跨平台门禁。

G01 持久目录增量：新 v2 清单写入当前任务数据库的数据域，socket 另存短路径私有目录并绑定设备号/inode；旧 v1 清单仍按旧路径合同恢复。v2 本地 check、i18n 11、PTY 身份 2、CLI 会话 433（7 忽略）、运行器 6 和应用构建通过。未修改的共享协调器/升级路径沿用 `32a932f95` 整合门禁；该提交 [Actions 36223768480](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36223768480) Linux x64 与 Windows x64 相关定向门禁均已通过；不含已认证模型或实际 GUI/IME。140 文件原始日志与产物归档 `validation/cloud-g01-g09-32a932f95`，索引 SHA-256 `6e9f25d861b94d6927099a734b92577f1e392e4efb309da9da472cb3b6a4c0cc`。后续未提交 GUI 增量不沿用此通过结论。

固定 macOS arm64 Grok `1.0.41/grok-4.7/default` 的持久目录 v1 两轮原生输入通过，独立审计确认 2 个 prompt ID、2 个正确模型标记和 SQLite 两条 NativeProtocol ACK；TUI/leader/shell 与临时认证副本清理确认。新增退出恢复检查因原生 `leader.lock` 残留报目录非空，原 `passed=false`、panic 和文件字节均保留。v2 仅补精确锁归属清理：要求目录身份未变、leader 真实生存期已结束、私有单链接文件原字节等于绑定 PID；未知文件或别名不删除。新 libtest 在同一真实退出现场恢复成功、socket 目录移除、持久清单保留，旧记录仍不能派发；零新增模型输入。两轮来源分开，不把 v1 整体失败改写为成功，也不宣称同一二进制重跑模型。

v1 应用 SHA-256 `e0bbfaa089f795ff92573a2a3b2735540730d350dceddbe73febc53c2f19efde`、libtest `cd34e4ff95be2d05295235806768dcd8939779c6eb2fba60aa2ab82f0bad3d53`；v2 应用 `9cfa85844f368467a34e98edbaac27834eb56da2024d00d36543f41bb4730222`、libtest `f54d7d283807bc4ae4fc7e5fd9fb5e965cd5477fd1b6eab96e63c55d650f34a9`。79 文件索引位于仓外 `validation/grok-owned-durable-20260926/`，SHA-256 `257d7844508552621e8c10637c3c5149642b4d7ac7a6eb5c03b7aafaebc7f050`。二进制采用 gzip 无损保存，并核对解压后的原始摘要；原始失败及各轮源码保留。

生产改动仅在 macOS arm64 条件模块内；Linux/Windows 不执行本项原生路径，共享基础的同提交矩阵单独记录。无需本地化变更，现有用户界面语义未改变。本项未接通 GUI 提交、新启动注册与真实应用重启模型链，G01 仍开放。

## Claude 图片验收运行器跨平台修正（2026-09-26）

生产验收不再要求 `/private/tmp` 或 Pillow；明确 UTF-8 管道、原始文本收据与仓库工作目录。新增八项离线回归覆盖 Windows 旧编码、中文失败/超时原字节、缺少临时目录及 Git 来源绑定，并加入 Linux/Windows 定向工作流。与关联回归合计 75 项通过，主仓 `cargo check -p warp`、脚本语法、工作流检查通过。工作流初次 actionlint 的既有自托管标签未知告警与登记标签后的通过均保留。

原字节归档 `validation/claude-image-runner-portability-20260926`，索引 SHA-256 `89b652a8f47b67cd6e33e9c739c5b35f3c1a4cd62ea3a47b447dca4f93dc4e0f`。本增量零在线模型输入，无需本地化变更；不补记 Linux/Windows 图片理解、恢复或 GUI 验收，G04/G05 保持原边界。同提交云验待与下一实现增量整合后执行。
