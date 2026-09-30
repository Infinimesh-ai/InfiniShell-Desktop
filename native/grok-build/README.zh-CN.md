# Grok 普通 TUI 原生输入桥

这是 G01 的原生能力补丁。当前 `.3` 的原生编译、74 项定向回归及独立构建已通过；Mac 标准首页普通 TUI 的 InfiniShell GUI 已取得八条真实原生输入、精确回执与结果，包含中文长文本、审批保护和同会话冷恢复。随后文本粘贴已修为本地草稿路由，七门禁及英中各一条真实输入通过；中文手动回退提示截断已由单行缩短及新工件布局复验解决。Mac 功能与双语审计已满足；Linux 宿主普通桥接入、Windows 原生传输及宿主接入已补入本轮源码，仍待目标平台源码门禁与实际工件绑定，G01 保持开放，PR #22 保持草稿。各轮源码与二进制分别绑定，不继承旧定制构建或官方发行版的验收。

## 固定来源与构建

- 官方源码：<https://github.com/xai-org/grok-build>
- 基线：`07e35a3dfeed2f200d319ef6c893b5ea286d9a51`，版本字段 `1.0.41`。
- `SOURCE_REV`：`84745de98b3d3996729aefcefd518890ffb73930`，不同于已验官方发行版 `4220f3b224a6`。
- 当前定制源码：`a5d1e2449179bae2f2c51d05120f212f92fa4d03`，版本 `1.0.41+infinishell.terminal-bridge.9`。Mac 已验工件仍绑定原 `.3` 提交 `1491b486fbaa4bff3b124db2893a413c7019e2fd`。
- 补丁身份及门禁范围见 `source.json`；许可证和修改声明见 `LICENSE`、`NOTICE`。

在该精确基线的独立干净工作树内，核验补丁 SHA-256 后运行以下命令。不要覆盖系统安装或默认 Grok 配置；当前二进制仍作为独立验收工件管理。

```sh
git apply --check /absolute/path/to/InfiniShell-Desktop/native/grok-build/terminal-bridge.patch
git apply /absolute/path/to/InfiniShell-Desktop/native/grok-build/terminal-bridge.patch
GROK_VERSION=1.0.41+infinishell.terminal-bridge.9 cargo check --locked -p xai-grok-pager-bin
GROK_VERSION=1.0.41+infinishell.terminal-bridge.9 cargo test --locked -p xai-grok-pager -p xai-grok-shell -p xai-grok-tools -p xai-grok-shell-terminal --lib terminal_bridge -- --test-threads=1
GROK_VERSION=1.0.41+infinishell.terminal-bridge.9 cargo build --locked -p xai-grok-pager-bin
```

Rust 工具链由上游 `rust-toolchain.toml` 固定为 `1.94.0`；`protoc` 沿用上游查找方式。用户本机执行时必须遵守仓库 `docs/local-test-storage.zh-CN.md`：每轮短 `TMPDIR`、身份记录、原件归档与退出后清理；不修改 `HOME` 或 `CODEX_HOME`。

## 已验工件的使用方式

Mac arm64 已验 `.3` 工件已独立保存在 `/Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/resume-20260930/g01-cross-platform/mac-terminal-bridge.3/grok`。SHA-256 为 `edcdc3d8729cc657080e6a266e26a6590ec2b275f4545a5b93c1dd5e26bf08f1`，CDHash 为 `356fe77c29f339fcaa90e48094fc7af7e0571ec5`；复制后的完整摘要和代码签名再次核对通过。此副本不随后续 `target/debug` 构建覆盖。Linux x64 `.6` 已通过 CI 的 78 项原生回归与构建，独立工件为 `/Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/resume-20260930/g01-cross-platform/linux-terminal-bridge.6/grok`，SHA-256 `e2cb765c093fe6381eecfbb3ba8329e4b4edb76ecb6f98f38f88fc8203c76ffb`，权限 `0700`。宿主已写入该摘要，绑定后源码门禁尚未完成；Windows `.9` 待实际构建复验，不宣称 G01 完成。

1. 取得对应平台、版本和摘要的独立工件，复制到自己的独立目录；Linux／Mac 将副本设为 `0700`（CI 下载可能丢失可执行位），并重新核对完整 SHA-256。Mac 还须核对代码签名及上述 CDHash。不要覆盖系统安装，也不要给重建文件直接套用旧摘要。
2. 产物 `xai-grok-pager` 必须复制命名为 **`grok`**；Windows 对应为 **`grok.exe`**。宿主按这个文件名识别普通 CLI，直接运行原产物名不会进入 Grok 富输入。
3. 在 InfiniShell 启用 CLI 通知后，新建普通本地 shell，让宿主设置本次发现目录。使用独立私有 `GROK_HOME`，按正常登录流程授权，并在该配置中设置 `[cli].auto_update = false`。不要手工复用其他终端的 `GROK_TERMINAL_BRIDGE_DIR`。
4. 从该普通 shell 以绝对路径运行 `GROK_HOME="<私有配置目录绝对路径>" "<独立工件目录>/grok" --no-auto-update`（占位路径须替换；Windows 使用等价的环境设置及 `.exe` 路径）。无需 owned、leader 或 `--minimal` 参数；富输入仅在原生空闲且身份验证通过时提交，审批与原生草稿仍受保护。

上述源码重建命令面向最新 `.9`，不是取得 Mac 已验 `.3` 身份的方法；新的构建须独立审核绑定后才能使用。

## 当前跨平台实现边界

2026-10-01：`.8` 在 Windows 已通过 protobuf 6 项与登录环境解析 2 项；资源测试在编译时被上游 Windows 管道测试的 `String + &String` 类型错误阻断，没有二进制。`.9` 仅将该一行改为显式格式化相同长路径，保留原断言，并在 Windows 构建中显式运行该边界测试。Mac 完整原生 check 通过；Windows `.9` 目标复验、工件绑定及最终源码门禁待完成。

2026-10-01：`.7` 在 Windows 已通过 protobuf 6 项和完整原生 check，随后库测试因 Unix-only 解析辅助函数缺失而编译失败，未生成工件。当前 `.8` 仅向测试开放纯解析函数，并让资源路径测试使用各平台原生符号链接 API，保留全部断言；真实登录 shell 捕获仍仅 Unix。本机解析 2 项、资源路径 3 项和完整原生 check 已通过。Linux `.6` 的实际构建、78 项原生回归、宿主 1469 项和 command 14 项通过；Windows `.6` 宿主 1308 项、command 36 项及原 ConPTY 辅助验证通过。具体摘要与取消范围见 `source.json`；Windows `.8` 及绑定后源码门禁仍待完成，G01 保持开放。

2026-10-01：`.6` 的 [目标门禁 36752136629](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36752136629) 已通过两平台宿主 check。Windows 原生已越过旧锁文件问题，但在公开 `xai-proto-build` 的 `/dev/stdout`／`/dev/null` 处失败；真实失败日志摘要 `ee50ae82a4c51fb6ad2d8365b861c4931c8aa7a899f76970a43196fe36cb5fe5`。`.7` 仅将构建辅助改为临时实体文件并精确解析依赖路径；Mac 6 项 protobuf 回归与完整原生 check 通过，Windows 实际复验仍待完成。普通桥运行时代码未变，旧 `.6` 的 74 项共享测试仍按原轮计证；本轮不新增功能／GUI 验收。

`.6` 已修正 Linux 连接身份握手和 Windows 目录句柄；Mac 原生编译及共享库 74 项、宿主编译及 55 项定向回归、i18n 11 项通过。新增平台路径和固定工件仍待 Linux／Windows 源码门禁，不计 G01 关闭。

2026-10-01：`9fdce3e3e` 的两平台宿主 check 与 Windows 真实原 ConPTY 辅助程序步骤通过；Linux `.4` 原生构建通过，Windows 原生在上游 `cc 1.2.48`／`find-msvc-tools 0.1.13` 常量类型不兼容处失败。`.5` 仅将后者精确锁到 `0.1.10`，并修复 Windows 日志失败测试的 Unix `File` 注入；Mac `--locked` check 与 74 项原生回归通过，目标复验仍待完成。原始失败不覆盖或改写。

`.4` 源码加入 Windows 命名管道及私有持久回执。Windows 响应帧读完后，宿主发送单字节 `1`，服务端在同一有界期限内等待该字节再关闭；它只确认传输读取完成，不是输入 ACK。避免使用会等待客户端读取而无界阻塞的 `FlushFileBuffers`，亦不依赖断开后仍保留未读缓冲。[Windows 官方管道合同](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-disconnectnamedpipe)

Linux `.6` 原生在每条连接读取正式请求前，同次 `sendmsg` 发送一字节 `0x01`、自有 pidfd 与 `SCM_CREDENTIALS`。宿主连接前设置 `SO_PASSCRED`，单字节 `recvmsg` 核对内核发送凭据与 `SO_PEERCRED`、pidfd 类型／活性及进程代际，保留收到的原 pidfd 和实际 `/proc/<pid>/exe` FD；不再要求普通桥使用本轮 runner 缺失的 `SO_PEERPIDFD`，也不按数字 PID 重开授权。多余 FD、缺失凭据、截断与超时均拒绝；身份前导不构成输入 ACK。Windows `.6` 同步修复宿主与原生目录句柄：增加实际目录读取权限，使不共享 DELETE 的句柄真正阻止重命名；保留 DACL、reparse、对象身份和原拒绝断言。Windows 宿主通过关闭 kill-on-exit 后的短调试观察取得实际映像 `hFile`；原 ConPTY 的专属只读辅助程序从真实 HPCON 派生，GUI 自身不切换控制台。两平台只接受由现有 `cross-platform-preflight.yml` 从公开固定基线加补丁构建、经过审核后绑定的摘要；当前仍待构建与目标门禁，G01 保持开放。

Mac 继续绑定 `.3` 的原工件和既有功能／双语证据。本轮只验证公共协议及 Unix 接线未回归，没有生成新的 Mac 功能验收。`source.json.previous_native_artifacts` 保留原 `.3` 源码与工件身份，不能把其结果移植给新的跨平台二进制。

## 接入合同

仅普通 TUI 进程显式设置 `GROK_TERMINAL_BRIDGE_DIR` 时开启 Unix socket。该根必须预先创建，属于当前用户、权限 `0700`，真实绝对路径及祖先不可由组或其他用户写入。每次 TUI 启动产生独立 `t-<UUID>` 目录，包含私有 `control.sock` 和 `manifest.json`；清单控制令牌不进入日志或验收归档。退出清理 socket/清单，保留仅含身份、摘要与状态的 `receipts.jsonl`。

调用方必须将 socket 对端的内核进程身份、UID、实际二进制及当前终端 PTY 对应核验；清单里的 PID、进程名或存活检查不能独立证明身份。Mac 宿主接线使用内核 audit token、真实前台 PTY、固定 SHA-256 及动态 CDHash 核验，只接受 `source.json` 中的独立工件。自行重建所得二进制必须重新审核身份及验收，不能仅改版本号沿用信任。

macOS Unix socket 正常回包并关闭后不能再次读取 `LOCAL_PEERTOKEN`；宿主改为每次连接建立后捕获该连接的内核对端凭据，再于写入前后、读取后复核存活进程身份、签名、文件身份及前台 PTY。读取响应改用非阻塞 socket 与 `poll`，共用绝对截止时间，避免对端已关闭时设置 `SO_RCVTIMEO` 返回 `EINVAL` 而丢弃已缓冲的回包。发现时仍完整读取并校验 SHA-256；主仓仅将 `sha2 0.10.9` 的开发配置优化级别设为 `3`，不缓存验证结果、不缩减校验范围。

启用通知的本地 Mac shell 从系统账号主目录取得按渠道／profile 隔离的短 `0700` 发现目录；调用者传入的旧路径会清除，Docker 不继承该变量。用户仍从普通 shell 启动定制 Grok，无需 owned 包装进程。发送先持久领取一次，只有原生精确 ACK 才清除对应编辑器快照；未知只查询，不自动重投。确定尚未发送的尝试允许下次用户提交；已确认的相同文本须明确点击提示作为新一轮发送。新编辑和附件不被旧回调清除。

帧为四字节大端 JSON 长度及 UTF-8 JSON；请求外层为 `{"token":"<private>","request":{...}}`。响应使用相同帧格式。`request.operation` 支持：

- `state`：传 `instance_id`，只读返回可信可用状态及绑定 agent／会话／代际的短期一次性租约；不创建或切换会话。
- `prepare_session_if_idle`：仅由用户本次提交在标准首页触发，传 `instance_id/input_epoch`，复用原生首页 Enter 的创建／工作区确认流程，不携正文。返回固定 agent、预期会话及首次绑定代际；宿主之后仅查询该目标，取得新租约后才领取正文。拒绝或响应丢失不重试创建，也不代用户回答确认。
- `submit_if_idle`：传当前租约的 `lease_id/session_id/binding_epoch/input_epoch`、`instance_id`、相同且规范的非空 UUID `message_id/prompt_id` 以及字面 `text`。
- `query_receipt`：传 `instance_id/message_id`，仅查询原事务。处于未确认状态时可向原生 actor 补查持久收据，绝不重发输入。

空闲检查与领取在 TUI 同一事件循环完成；原生 actor 再检查会话运行、队列、审批及真实后台任务。普通原生权限和 Hook 继续生效。先将一次领取账本刷盘，再发专用原子接口；只有用户消息写入原生历史且持久化屏障成功才记 `native_acknowledged`。普通回合完成、Hook 拒绝、本地派发和连接断开不构成此确认。已领取的相同 ID 只读原收据，未知结果不得自动生成新 ID 重投。

纯显示动画只有在前后目标身份不变、输入／ACP／后台任务队列均无待处理事件且所有可能改变输入的恢复、搜索、拖选等路径均不可达时，才保留 `input_epoch`。首页预创建会话的命令同步代际 `0/1` 差异允许留待原生揭示会话流程处理；进入 Agent 视图仍要求命令同步代际一致。审批、待发送、待恢复、未确认回合、会话加载及其他危险待处理状态的守卫继续生效。

定制构建使用独立 leader 名称和精确客户端登记标记，拒绝旧客户端混入；旧 leader 不支持专用能力时不发输入，重连撤销旧租约。构建时必须显式设置上列 `GROK_VERSION`，使版本输出表明这不是官方发行二进制。Linux 宿主与 Windows 原生／宿主桥实现已补；定制 .6 工件与目标门禁仍待完成，不能将空摘要下的安全拒绝记作可用能力。其他平台实机验收按用户指令后置，不记为通过，也不豁免实现。

独立运行验收必须使用私有 `GROK_HOME`、`--no-auto-update` 和该私有配置中的 `[cli].auto_update = false`。上游未实现 `GROK_DISABLE_AUTOUPDATER` 环境变量，不能用它代替上述关闭方式；不修改用户默认配置。

## 验收边界

历史：`5629588b7` 的 [Linux／Windows 源码门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36714435483) 已成功；归档索引 SHA-256 `91fdefb1fd244752fd2fe1079422ae6f7d56b139d4a51e3918c1541768a46622`。它不覆盖后续 G06，也不代表 Linux／Windows 普通桥已实现。两平台接入的只读可行性核对见 `source.json.cross_platform_implementation_feasibility`；当时 Windows 方案仍为官方 API 合同推导；本轮实现边界见上文。以下各轮保留当时的来源与状态。

宿主 `r-hgmsd_pn` 的 `cargo check`、48 项桥回归、11 项 i18n 及构建通过；最初 peer 过滤器命中零项，另由 `r-joflf1h5` 的两项有效 peer 回归补齐。真实 GUI `r-wudipm8z` 绑定 `c3b509622` 基线加冻结源码摘要，签名宿主 SHA-256 `d52e438de1cd2790ab0f38a33b38562c21ff502b26b9d637f86c19c6ce587958`，原生 SHA-256 `edcdc3d8729cc657080e6a266e26a6590ec2b275f4545a5b93c1dd5e26bf08f1`；并非干净 HEAD 构建。

该 GUI 从普通 shell 启动标准首页，未使用 `--minimal` 或 owned leader 参数；英文两行、41,751 字节中文 400 行、非空原生草稿保护、真实审批拒绝后保留稿发送、重复点击与明确同文新一轮、发送期间编辑取消及双重重启同会话的新输入均有原文／回执／结果证据。共八个唯一输入和 ACK，七个 `end_turn`、一个用户拒绝审批后的 `cancelled`；审批文件未创建。英文及简体中文局部提示和结果可读，两代自然退出，私有认证副本删除并完成短目录清理。摘要 `r-wudipm8z-gui/acceptance-summary.safe.json` 的 SHA-256 为 `3c72d7d6bd6496ea5537b30e3cda385aa98afbd5875429f477e6b90aa47b63fa`；其中保留第 39 张截图误点、旧 Unknown 不自动恢复为 ACK 等边界。

关闭富输入后文本粘贴曾被拒绝，却提示用户粘贴到原生 CLI；最新源码改为打开富输入并恢复／追加本地草稿，不写 PTY、socket，也不自动提交。不支持桥的回退提示改为原生键入。英中文案已同步，新宿主 `r-zxk8iqk7` 的 check、52 项桥、两项 peer、两项既有粘贴回归、11 项 i18n 与构建均通过，收据 SHA-256 `e73eda855d6b092c8320e3af80e81f62067f73b64742f33cd004b7b93e73d66c`。随后 GUI `r-g91qynam` 的英文 131 字节与中文 128 字节合并草稿分别仅在明确提交后进入原生一次，ACK、精确答案及 `end_turn` 各一次；字面原生草稿 `123` 未被覆盖，无桥回退两侧计数不增。独立摘要 SHA-256 `5576b8c0b7378cbf7556a71b5b758fe70a0aa2da9b5844892b114a719620bd6b`。该轮英文回退完整、中文末尾截断作为历史保留。仅缩短中文一条文案后，`r-fdaad6hh` 的 check、52 项桥、两项 peer、两项既有粘贴、11 项 i18n 与构建再次通过并清理；suite SHA-256 `05ed738634c73dae51643bc619ffad1efcbdc9fdbb45733cd642a40c87365bfa`。最终中文布局 `r-ni936aln` 确认提示含句号及占位文本完整，零模型输入，不重计前轮功能正例；摘要 SHA-256 `0185ced924964c2fc47871344dad2796128079278a4338000ced2c8045855702`。源码对照证明只有中文 FTL 改变，原生与英文资源不变。Mac 功能与双语审计全部满足；此前“仅待 CI”判断过窄，已由仓外 `scope-correction.safe.json`（SHA-256 `33411686223414380743cc838f1d1179cc578d315c740a75ef486476449ae863`）更正。G01 仍须实现 Linux 宿主普通桥接入、Windows 原生传输及宿主接入；仅其他平台实机验收移交用户。源码与冻结工件已绑定 `5629588b77b15e27c9e046cc6326171370297dc6`，该提交 CI `36714435483` 正在运行，不能证明被 `cfg` 排除的能力已经实现。

历史 `r-rk5mi0us` 单次 Return 曾进入完整 SHA-256 计算，但 254 秒后宿主／原生账本仍为零，两次 State 的 epoch 稳定；不能将 SHA 认定为唯一原因。早期失败和 `.1/.2` 收据保留原范围。上游完整 `--tests` 仍因未公开的两份 `docs/internal` 文件缺失而失败，定向门禁明确选择库测试；必要的 `base64::Engine` 导入补丁不扩大验证范围。
