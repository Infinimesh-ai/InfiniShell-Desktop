# 固定版本同提交补验与完成结论更正（2026-09-25）

> **2026-09-25 完成结论更正：阶段交付，完整 Goal 尚未完成。** 已通过的固定版本功能、三平台相关回归及原始收据继续有效；功能缺项、模式限制与未覆盖验收不计为完成。统一待办见[已知缺项](KNOWN_GAPS.md)，当前机器可读状态见[CURRENT_STATUS](CURRENT_STATUS.json)，阶段合并范围见[合并准备说明](MERGE_READINESS_20260925.md)。本更正覆盖下方“全部完成”的历史总结，不改写原始通过或失败记录，也不表示已经合并或发布。

本报告处理最终复核发现的两项收尾：同一提交的三平台必要门禁，以及交付支持说明与实际产品能力不一致；同时落实用户追加的 Grok 正式能力默认开放。本轮选定的相关回归已通过；未覆盖的功能与验收仍须跟进。产品实现、测试、配套插件与源码／原生证据按下列固定版本和平台范围交付；不表示已经对外发布。

## 冻结范围

- 分支：`codex/cli-agent-parity`。
- 开始补验时基线：`d8759fbe1f8c763a82732c984aa2d20980f8d1b0`。用户随后要求默认开放 Grok；首轮受测提交为 `9af6de393a8f9fb108eb4fc8a3c871550ff2daf5`。产品改动冻结于 `ee839b4fc7dacd58bac0806aa549500af5776b30`；新增真实 Grok 清理门禁与原生 EOF 观测分类的首个验证提交为 `14e0d7d4218e53cfc73cad9d2f8ece6751af0104`；后续夹具修正冻结为 `50e1515bcd3f20cf120edf5565766975b6a68d5d`，均已推送并核对远端一致。
- 固定版本：Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41`。
- 三平台 InfiniShell 桌面共用托管入口默认开放。Grok 与 Codex／Claude 使用同一入口，版本、权限策略、审批及原生能力门禁保持。共用库和 TUI 不自动启用此桌面配置。
- 新候选保留 OS 派生后的控制握手任务；调用方取消时仍完成所有权确认，再通过真实监督者清理进程。新增固定单线程队列取消回归、超时诊断及双语 GUI 验收；不延长产品超时，不改变退出证明与恢复的判断。
- GUI 验收复用正式桌面默认配置，移除用例单独强开托管功能的操作；收据区分默认配置与测试覆盖。

## 当前结果索引

| 范围 | macOS arm64 | Linux x64 | Windows x64 | 实际来源 |
| --- | --- | --- | --- | --- |
| 桌面工作区集合 | 10,895 通过／96 跳过 | 10,923 通过／100 跳过 | 10,680 通过／112 跳过 | `ee839b4fc`，失败、slow、leaky 和跳过另列，不包含所有仓库测试 |
| i18n 与桌面默认入口 | 通过 | 通过 | 通过 | `ee839b4fc`；两种语言源文案同步 |
| 双语 GUI 与原图目视 | 6 张有效原图通过 | 4 张原图通过 | 4 张原图通过 | `ee839b4fc`；仅各收据列明的真实视口及输入方式 |
| 三款原子升级 | 既有固定版本实链通过 | 12 场景通过 | 三款正式升级通过 | Mac 保留原提交；Linux／Windows 为 `ee839b4fc` |
| 真实 Grok 监督清理 | `50e1515bc` 提交后四场景通过 | `14e` 身份绑定失败；`50e` 四场景清理通过 | `14e` 用例准备失败；`50e` 四场景清理通过 | 不同提交与候选收据分别保留 |
| 完整模型生命周期、父子消息与 SSH／tmux | 既有对应模式实链通过 | 未重跑完整模型链 | 未重跑完整模型链 | 原版本／构建／平台和源码域关联保留，不重标为本轮执行 |

## 50e1515bc 的最新补验

`50e1515bc` 仅修改三份测试／运行器文件：统一 Windows 输入路径表示，按实际文件和目录身份核对持久清单，排除 Linux 线程 TID，并导出有界的 pidfd 数值错误码。真实 root／leader、完整 SHA256、实际测试进程与 worker、代次、stdout EOF、旧回执拒绝和全部进程退出仍是硬性要求。读取身份失败保持失败，不用内容相同的文件副本冒充原输入。

[最终本地候选](validation/final-same-commit-20260925/grok-platform-final-macos-candidate/local-gates.safe.json)通过安全回归 19 项、check 和真实四场景；其中 Rust 二进制来源为[此前合并夹具构建](validation/final-same-commit-20260925/grok-platform-fixtures-macos-candidate/local-gates.safe.json)，该次构建、check、i18n 11 项和四场景均通过，之后只完善 Python 对象身份检查与第 19 项回归。两份记录保留各自源码摘要，不重标为重新构建。冻结前三份源码逐一核对暂存 blob，与本地验收摘要相同。[新提交源码与二进制审计](validation/final-same-commit-20260925/candidate-50e1515bc/source-binding.safe.json)另核对 56 个源码域、10 份原生运行输入源码及内置二进制，确认生产仍冻结于 `ee839b4fc`，构建复用与提交后运行的实际来源一致。

[集中复验 36116931025](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36116931025)已成功结束，headSha 精确为 `50e1515bcd3f20cf120edf5565766975b6a68d5d`，仅开启两平台相关原生模式。完整工作区、GUI 和原子升级没有变更，不重复执行；原失败分别保留。Linux／Windows 原生门禁均通过，[最终作业状态](validation/final-same-commit-20260925/candidate-50e1515bc/run-final.safe.json)确认两个选定平台均为 success。macOS Intel 两项备用作业按选择跳过，不计为通过。

[Linux 原收据及索引](validation/final-same-commit-20260925/run-36116931025/linux/archive-index.safe.json)已核对通过：定向 847 项、原监督 5 项和[真实 Grok 四场景](validation/final-same-commit-20260925/run-36116931025/linux/grok-supervised-exit.json)全部通过，34 步成功／10 跳过／零失败，没有重试、slow 或 leaky。四场景实际绑定进程数为 2、2、1、1，均取得 `linux_subtree` 清理回执并确认 stdout EOF、旧代次拒绝及所绑定进程退出；pidfd 错误与 panic 列表为空。私有 leader 两项为 `stdio_closed/0`，无 leader 分别为 `stop_requested/null`、`stdio_closed/null`，后者没有数值退出码，不能称全部自然退出 0。独立 ACP stdio 此次 2,120 ms 自然退出 0，leader 五秒内未自然退出、随后专属清理为 -9，仍与生产监督证明分开记录。

[Windows 原收据及索引](validation/final-same-commit-20260925/run-36116931025/windows/collection-index.safe.json)已核对通过：定向 747 项、原监督 5 项和[真实 Grok 四场景](validation/final-same-commit-20260925/run-36116931025/windows/grok-supervised-exit.json)全部通过，47 步成功／13 跳过／零失败，没有重试或 leaky。四场景均使用真实进程句柄和严格 Job，观察进程数依次为 2、2、1、1；退出、stdout EOF、当前代次回执与旧代次拒绝全部确认。私有 leader 的显式结束为 `stop_requested/0`，EOF 为 `stdio_closed/0`；两项无 leader 均为 `stdio_closed/1`，只证明清理，绝不称全部自然退出 0 或模型任务成功。独立直接 ACP 此次 EOF 观测为 4,685 ms、stdio 自然退出 0，leader 另行强制清理；旧五秒负例仍保留。本轮不重跑 full／GUI／原子升级或独立 i18n，原来源保持。

[提交后 macOS 实测](validation/final-same-commit-20260925/candidate-50e1515bc/macos-postcommit.safe.json)与[四场景原收据](validation/final-same-commit-20260925/candidate-50e1515bc/macos-grok-supervised-exit.safe.json)均通过，`source_dirty=false`，HEAD 绑定 `50e1515bc`。当时环境提示 ACASIS 未挂载；此前本地 check 与构建已完成，这次复用经过摘要核验的内置盘副本，不要求用户处理桌面授权，也不声称在缺少构建缓存时重跑了 Cargo。环境恢复可用后，[同提交 cargo check](validation/final-same-commit-20260925/candidate-50e1515bc/macos-postcommit-check.safe.json)也实际通过；该后续检查单独留证，不追改先前四场景收据中的未重跑 Cargo 标志。

## 14e0d7d42 的真实 Grok 补验

本提交仅增加验证代码、测试注册、固定 Mac 输入及 CI 门禁，没有再次修改产品监督器、协议适配、权限、默认入口或界面文案。[源码逐项核验](validation/final-same-commit-20260925/candidate-14e0d7d42/source-binding.safe.json)确认八个变更文件均属验证范围；`grok.rs` 整个文件对象有变化，但唯一增量是 `cfg(test)` 模块注册，不称其 blob 未变。[Mac 候选收据](validation/final-same-commit-20260925/grok-supervised-macos-candidate/summary.safe.json)确认 check、i18n 11 项、受影响模块 453 项、共享监督 5 项和真实 Grok 4 场景通过；8 个候选源码摘要逐一与提交 blob 对应。二进制为预提交候选构建，不称为提交后重新构建。

[提交后 Mac 核验](validation/final-same-commit-20260925/candidate-14e0d7d42/macos-postcommit.safe.json)再次通过 check 和[真实四场景](validation/final-same-commit-20260925/candidate-14e0d7d42/macos-grok-supervised-exit.safe.json)。执行记录明确 `source_dirty=false`、HEAD 为 `14e0d7d42`；复用摘要已绑定相同候选源码的内置盘二进制，没有伪称重新编译。

四场景分别调用私有 leader／无 leader 拓扑的 `finish` 与 `finish_after_stdin_close`，观察真实原生进程身份、当前代次的清理回执、stdout EOF 和旧代次拒绝。自动 leader 的无认证 setup 为五阶段，无 leader 为七阶段且允许最后通知晚于认证拒绝响应；先验证实际接口，再按拓扑精确断言。此检查没有认证或模型输入，不替代运行中审批、用户 Stop、历史继续和应用重启的既有实链。

用户确认外置盘启动需要桌面授权，未在电脑前导致等待。后续本机执行统一使用内置盘的同字节副本，外置盘只保留构建缓存；每份输入均核对摘要，不改系统权限或产品超时。原外置盘失败和进程采样保留。五项监督的通过采用串行 libtest，与此前 nextest 分进程失败的执行方式也有差异，不把这一对照扩大为全部历史超时均已唯一归因。

[最小相关平台补验 36113156742](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36113156742)已在精确 `14e0d7d42` 上派发 Linux／Windows 原生模式，包含新增四场景及五项监督回归。没有再次改动的 GUI、全量工作区与正式原子升级使用 `ee839b4fc` 的实际来源证据，未重复派发或改写旧 SHA；Linux／Windows 均以失败结束，当轮未满足收尾门槛；后续修正与独立通过见 `50e1515bc` 小节。

### 双平台用例失败与夹具修正

`36113156742` 的 [Windows 原始归档](validation/final-same-commit-20260925/run-36113156742/windows/collection-index.safe.json)确认唯一失败是新增四场景的入口检查：`grok_supervised_exit_live_tests.rs:600:5` 比较输入 Grok 路径和 Rust 规范化路径失败，退出 101、没有超时，`started_scenarios=[]`、`cases=[]`，未创建场景事件文件。`verify_binary` 内的普通文件、大小和 SHA256 校验已先行通过，不能将本次当作 Grok 已运行或生产清理失败。原始 libtest 日志未上传，仅保留 1,100 字节与摘要；具体路径两侧的原文不可见，不把 Windows 扩展路径前缀猜测记作独立观测。

候选只修正测试入口：逐一核验三份输入文件后统一使用 Rust 规范化路径，继续比对实际测试进程和监督 worker；输入清单原字节、运行前后摘要、代次、原生身份、EOF 和清理回执要求不变。没有修改产品代码、权限或超时。[Windows 路径修正的本地候选](validation/final-same-commit-20260925/grok-path-normalization-macos/local-gates.safe.json)先从内置盘通过 check、i18n 11 项与真实四场景；这不代替后续 Linux 修正合并后的候选构建。两项夹具修正合并后，再对包含修正的提交补双平台原生门禁；旧失败保留。

本轮 Windows 的 check、747 项定向、原五项监督、IPC 2 项、所有权 2 项和 rust-genai 81 项通过；没有重试、flaky 或 leaky。直接 Grok ACP 探针此次在 4,561 ms 观察到 stdio 自然退出 0，仍单独强制清理 leader，且 `product_supervisor_verified=false`；它不能覆盖新增四场景失败，也不能改写上轮五秒 EOF 负例。

[Linux 原始归档](validation/final-same-commit-20260925/run-36113156742/linux/archive-index.safe.json)确认五项原监督和 847 项定向通过，新增四场景的首例经过 startup、ACP、native_identity 后，在 `codex_idle_crash_identity_tests.rs:31:13` 的 `pidfd_open` 失败，零场景完成。没有取得原始 PID／errno，不能宣称精确系统错误已经定位。[源码诊断](validation/final-same-commit-20260925/run-36113156742/linux/failure-diagnosis.safe.json)确认锁定 sysinfo 0.37 的默认刷新包含线程 TID，而本测试仅按同一映像筛选，存在将线程交给进程身份绑定的明确缺陷。同轮 Codex 原生空闲崩溃使用同一 pidfd helper 已通过，全局不支持 pidfd 不符合该证据。

候选为测试的四处刷新关闭线程收集，并在最终原生候选中排除线程；保留 root 必须在集合、leader 至少两个真实进程、逐个 pidfd 绑定及全部退出要求，不跳过绑定失败。运行器仅新增已知身份夹具 panic 的有界数值 errno 投影，原文仍私有；18 项离线安全回归通过。两项更改均属测试和诊断，不影响产品实现与本地化，其后由 `50e1515bc` 的双平台真实补验验证，原失败保持不变。

## 产品改动 ee839b4fc 的门禁

`ee839b4fc` 的[源码绑定](validation/final-same-commit-20260925/candidate-ee839b4fc/source-binding.safe.json)与[提交前本地门禁](validation/final-same-commit-20260925/candidate-ee839b4fc/precommit-local.safe.json)已归档：check、integration check、监督器构建、真实监督 5 项、i18n 11 项、CLI 定向 1,786 项全部通过，源码摘要与提交一致。[提交后 macOS 门禁](validation/final-same-commit-20260925/candidate-ee839b4fc/macos-local.safe.json)确认 check、桌面工作区 10,895 项、TUI 消息状态 9 项及桌面构建全部通过；工作区 96 项跳过、3 项 slow，没有失败、重试或 leaky。

[新一轮集中验证 36103244022](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36103244022)已绑定该 SHA 并结束，Linux 通过、Windows 失败。独立验证引用保留首轮任务，未因新任务取消旧结果。Windows 的固定 Grok 未登录 ACP 探针失败：五项协议边界已完成，但关闭标准输入后的 5,005 ms 快照显示进程尚未退出；经过既有 leader 观察后，stdio 在自持清理前自然退出且返回 0，没有强制终止 stdio。原失败不能改记为五秒 EOF 通过，也没有证据将原因归于基础设施。该探针直接运行固定 CLI，未调用本轮修改的 Rust 监督器；其他独立检查的通过不能覆盖该失败。

[Linux 最终日志汇总](validation/final-same-commit-20260925/run-36103244022/linux/linux-final.safe.json)确认桌面工作区 10,923 通过、100 跳过，监督 5 项、默认入口 3 项、两轮 i18n 各 11 项通过，无重跑、slow 或 leaky。12 项真实原子升级全部通过，原先失败的 Grok `interrupted_recovered` 此次完成严格退出、恢复和 journal 清理；原失败仍保留。双语四张原图已逐张目视并核对实际语言、默认开关与摘要，见[归档索引](validation/final-same-commit-20260925/run-36103244022/linux/linux-collection-index.safe.json)。

[Windows 最终日志汇总](validation/final-same-commit-20260925/run-36103244022/windows/job-summary.safe.json)确认唯一失败为上述 Grok 探针。桌面工作区 10,680 通过、112 跳过，4 项 leaky 为通过子集、1 项 slow、没有重跑；定向 2,737 项、监督 5 项、默认入口 3 项及两轮 i18n 各 11 项通过。三款正式原子升级和双语四张原图也已归档，见[归档索引](validation/final-same-commit-20260925/run-36103244022/windows/collection-index.safe.json)。Linux／Windows 截图当前选择 Codex，只证明系统剪贴板、默认入口及所见布局，不扩大为 Grok 专属权限或物理输入法验收。

该负例暴露两项必须分开的合同：原生 Grok 的五秒 EOF 行为，以及应用托管进程的可靠清理。现有 Windows 真实升级收据只证明升级进程的严格 Job 清理，合成监督器夹具也不能代替实际 ACP 进程。因此新增四场景真实 Grok 监督清理门禁：私有 leader／无 leader 两种 stdio 拓扑，各自执行显式结束和标准输入 EOF 后结束，必须经过生产 `ManagedChild`、核对代次及平台进程树退出。无 leader 拓扑不冒充固定权限策略的完整验收；本补验不提交模型输入。

本轮探针保留原有五秒观测和失败历史，将迟到自然退出单列为 `delayed_natural_exit`；只有 stdio 最终自然退出 0、从未强杀、输出读取线程结束、所有权和完整协议记录均通过才可接受观测合同。不会新增等待或延长产品超时，未确认退出、强杀 stdio 或非零退出仍失败。新增真实监督门禁通过前，不能据此关闭剩余验收。

新增门禁的[首轮本地失败](validation/final-same-commit-20260925/grok-supervised-first-attempt/diagnosis.safe.json)已定位为测试夹具目录错误：隔离 HOME 未按生产要求放在 `state/grok-managed/<UUID>`，监督 worker 在执行 Grok 前拒绝。该轮 check、构建、i18n 11 项和受影响模块 453 项通过，但四场景验收尚未开始 ACP，不能记为通过；修复夹具布局后再验，不放宽生产目录边界或超时。

修正目录后仍有[真实启动超时](validation/final-same-commit-20260925/grok-supervised-corrected-startup-failure/observation.safe.json)。一次专门的[进程采样](validation/final-same-commit-20260925/grok-supervised-corrected-startup-failure/loader-stack.safe.json)观察到 launchd wrapper 的 420 次调用栈均处于 `dyld → mapFileReadOnly → __open`，尚未进入 Rust `main`，没有资源域 claim 或 Grok 启动证据。当时只定位到外置卷上的调试 worker 在加载阶段受阻，未独立观察授权弹窗。后续用户说明外置盘启动需要授权；位置对照使用摘要完全相同的内置盘副本，不修改权限、超时或退出证明。

[同字节内置盘对照](validation/final-same-commit-20260925/grok-supervised-internal-worker-diagnostic/comparison.safe.json)已跨过启动握手并完成四个有响应的 ACP 请求，表明先前超时与此次 Grok 协议执行不同。该次随后在新测试的阶段序列断言失败：自动 leader 返回五项前置阶段；旧 Python 探针预先启动 `--relay-on-demand` leader，观测到七阶段。不能把两种拓扑的内部通知数量混同。随后独立核对两种生产拓扑的真实接口并修正夹具，见[阶段观察](validation/final-same-commit-20260925/grok-supervised-internal-worker-diagnostic/setup-observation.safe.json)。该诊断本身不计四场景验收通过，后续正式通过另列于前文。

[本地诊断记录](validation/final-same-commit-20260925/candidate-ee839b4fc/native-diagnosis.safe.json)保留原失败及后续证据边界：单次定点取消已得到真实资源域清理证明；新候选 5 项全部通过，首例包含实际系统扫描。观察器未成功收集本次内部代次文件，不能冒充已采集；此前两个控制通道超时仍未完整归因。

[macOS 双语原图审计](validation/final-same-commit-20260925/candidate-ee839b4fc/mac-gui/gui-review.safe.json)绑定本提交构建与签名后二进制：两种语言分别使用独立配置启动，实际界面显示固定三款 CLI 版本及私有路径。6 张有效原始截图证明默认入口、Claude 更新后的说明和 Grok 三种权限选项在当前视口内清晰可读；两个配置的任务、代次与消息表均为空。首组英文 3 张截图因路径和 CLI 版本来源不符，保留但明确排除。原始截图实际为 JPEG，索引记录编码与原扩展名差异，归档不转换字节。本轮未提交模型输入，不宣称 HTTP 监控、物理输入法或未滚动区域的布局通过。

## 首轮受测提交 9af6de3

首轮候选产品的 `cargo check -p warp`、i18n 11 项和 CLI 定向 1,786 项全部通过。双平台集中补验 [Actions 36095732136](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36095732136) 实际 headSha 为上述受测提交；Linux／Windows 均已结束：Linux 失败，Windows 通过，整体不能计通过。

本轮 [Linux 原子升级收据](validation/final-same-commit-20260925/run-36095732136/linux/linux-atomic.safe.json)的 12 场景中有 1 项失败：Grok `1.0.40 → 1.0.41` 的 `interrupted_recovered` 返回 `interrupted_exit_timeout`，尚未进入后续恢复。其余 11 场景通过，包括三款正常升级。诊断发现控制握手工作尚未开始时取消启动，可连带取消持有控制监听器的任务；现有收据与该路径相容，但未记录具体调度，不能断言它就是本次失败的唯一原因。新候选已修复该启动取消窗口并补确定性回归，原超时及中断时点保持。原失败不能改记为恢复通过；后续 `ee839b4fc` 对应场景已取得独立通过收据，见前文。

Windows 已取得[正式升级事务](validation/final-same-commit-20260925/run-36095732136/windows/atomic-updates.safe.json)与 [GUI 默认配置收据](validation/final-same-commit-20260925/run-36095732136/windows/gui-review.safe.json)。两张 GUI 原图均为英文界面，分别证明可读的中英混合多行输入及图片附件标签；Grok 选择按钮可见，使用产品默认配置且没有测试专用开关覆盖。该检查仅覆盖当前滚动位置，不代替完整简体中文界面布局或物理输入法验收。Windows [最终步骤汇总](validation/final-same-commit-20260925/run-36095732136/windows/job-summary.safe.json)确认桌面工作区 10,680 通过、110 跳过，定向 2,737 通过，没有重试。3 项 leaky 为既有斜杠命令／补全测试的通过子集，名称保留于收据，不作为进程清理证明。

[macOS 同提交门禁](validation/final-same-commit-20260925/macos-same-commit.safe.json)已全部通过：check、i18n 11 项、桌面工作区 10,895 项（94 项跳过）和 TUI 消息状态 9 项。工作区记录 1 项 leaky：`ai::agent_providers::request_budget::tests::multimodal_attachment_remains_intact_when_tool_results_are_trimmed`；该项属于 passed 子集，不作为进程树清理证明。三个宿主恢复／崩溃／重连用例较慢但均在期限内通过，无失败重跑。

首轮基线定向有 12 项原子更新测试被临时目录祖先权限检查拒绝；改用权限符合要求的私有目录后全部通过。原失败保留，不改产品的祖先权限验证。

Linux 最终[步骤与日志汇总](validation/final-same-commit-20260925/run-36095732136/linux/linux-final.safe.json)记录：桌面工作区 10,923 通过、98 跳过，定向 2,871 通过，没有重试标记；i18n 11 项与默认入口测试实际执行通过。Job 因上述 Grok 中断退出失败而失败，不能记为平台整体通过。

启动取消修复的[首轮 macOS 本地验收](validation/final-same-commit-20260925/startup-cancel-initial-failure.safe.json)中，check 和监督器构建通过，5 项真实监督测试有 3 项失败。定点诊断证明排队握手取消可以收敛，但未完整解释两个已有用例的 macOS 控制通道超时；原失败保留；新增超时诊断后的候选验收与后续结果分别列于前文。

## 本轮门禁

macOS 先运行 `cargo check -p warp`、`cargo test -p warp --lib i18n::tests` 和 CLI 相关定向回归。通过后以同一远端提交集中执行 Linux x64／Windows x64 的编译、回归、原生 CLI 安装升级、进程清理、真实 GUI 剪贴板及桌面工作区测试。

桌面工作区集合排除 `command-signatures-v2`、`integration`、`warp_tui`，GUI 和受影响 TUI 测试单列。测试跳过、重试和失败按原记录保留，不把选定集合称为整个仓库全部测试。

## 既有真实验收的来源

三款 CLI 的真实模型生命周期、父子消息与结果、Mac 真实输入法、双语布局和 SSH／tmux 按[固定版本验收表](ACCEPTANCE_FIXED_VERSIONS_20260924.md)与[Mac 交付记录](MAC_FIXED_VERSION_DELIVERY_20260924.md)保留原始版本、构建和来源。这些历史收据不改写成此次提交的重新实测。

[首轮源码绑定记录](validation/final-same-commit-20260925/source-binding.safe.json)仅比对上一最终产品 `b3acf2a69` 与首轮 `9af6de393`：除验收文档外，仅六个桌面默认配置与对应测试文件发生变化；九个原生实现域的 Git 对象一致。该结论不能扩展到 `ee839b4fc`，因为新候选已修改 `managed_process.rs` 的启动取消行为。历史协议交互收据保留原始来源，新监督器与平台检查另列证据；代码一致性不能当作新的模型交互实测。

[候选最终源码域复核](validation/final-same-commit-20260925/candidate-ee839b4fc/final-source-domains.safe.json)已直接比较 `b3acf2a69 → ee839b4fc`：三款协议、权限、协调器、历史恢复和插件的列明对象一致；升级生产实现一致，但所在目录因测试诊断改变而不同。共享监督代码的真实改动单列，不能称整个运行时未变。Windows PowerShell 测试脚本的工作区为 CRLF，按仓库属性转换后的摘要与预提交收据相符，不错误宣称其原始 Git blob 与工作区字节相同。

支持范围、安装来源、渠道、默认入口和降级行为见[交付支持与回退说明](RELEASE_SUPPORT.md)。首轮 GUI 原图发现 Claude 的旧功能开关工程提示过时，因此先前“无需本地化变更”的判断已撤回。候选源码同步英文／简体中文的受支持版本说明，并让两种语言分别启动独立 GUI 进程，收据验证实际界面语言。三平台相关可见区域的双语布局及原图审查已通过，分别保留实际语言、来源及视口范围；最后的进程测试补验不改变产品文案，无需新增本地化变更。

## 交付结论

固定版本已有功能及本轮默认开放、启动取消修复、文案同步和相关三平台复验的证据继续有效；**原“P0–P5、自动升级、REOPEN 必需项均已闭环”结论撤回**。[历史最终汇总](validation/final-same-commit-20260925/final-acceptance.safe.json)中的 `all_required_acceptance_satisfied=true` 与空 `remaining_required_items` 是当时过宽的总结，不作为当前完成判据。该文件与索引保持原字节；[CURRENT_STATUS](CURRENT_STATUS.json)明确取代其总体完成结论，[已知缺项](KNOWN_GAPS.md)逐项列明剩余能力和验收。产品实现仍冻结于 `ee839b4fc`，最后相关原生验证提交为 `50e1515bc`。

[归档原字节索引](validation/final-same-commit-20260925/ARCHIVE_INDEX.safe.json)记录本轮 173 份产物的大小和 SHA256，包含通过、失败及原始截图；索引自身不计入。全部 JSON 可解析，最终汇总的 19 份证据摘要与原文件一致。

不计作通过的边界继续保留：Grok 托管图片不支持；Claude／Grok 子任务限相应固定策略，固定策略禁用原生 shell／hook／技能；未知来源与非原生安装保持手动升级；普通 Codex PTY Escape 不保证工具退出；原生 Grok 五秒 EOF 不可靠。Linux／Windows 系统剪贴板不代替物理 IME，Mac→本机 SSH／tmux 不扩展为所有远端系统，历史模型链不重标为本轮重跑。Windows 无 leader 的退出 1、Linux 无 leader 的空退出码均只作为清理完成记录，不作为模型成功。
