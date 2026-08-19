# Windows SSH 扩展开发指南

状态:实验性增强能力,需要持续补齐兼容性与回归覆盖

最后更新:2026-08-18

相关文档:[跨平台 SSH transport](cross-platform-ssh-transport.zh-CN.md)

## 文档目的

Windows SSH 扩展是 InfiniShell 相比上游额外维护的能力。这里的“扩展”不是一个
独立插件,而是随客户端发布、安装到 Windows SSH 远端并以
`remote-server-proxy` / `remote-server-daemon` 模式运行的 `infinishell.exe`。

本指南是该链路的开发与发布契约,用于:

- 在 Windows 原生环境构建和调试 remote-server;
- 不依赖 DMG 走通
  `client → SSH proxy → named pipe daemon → Initialize`;
- 验证安装包内置扩展、版本升级和 PowerShell bootstrap;
- 根据失败阶段快速缩小问题范围;
- 在同步上游或扩充能力时守住 Windows 特有的不变条件。

除最终安装体验外,不要把反复制作和安装 DMG 当作调试手段。协议测试和可复用的
开发 `.app` 应先给出确定结果,DMG 是最后一道打包验证。

## 支持范围

当前重点支持以下组合:

| 客户端 | 远端 | 交互 shell | 管理链路 |
|---|---|---|---|
| macOS / Linux | Windows OpenSSH | Windows PowerShell | 原生 OpenSSH exec channel |
| Windows | Windows OpenSSH | Windows PowerShell | Rust SSH worker 的复用 channel |
| Windows | POSIX SSH | `bash` / `zsh` | Rust SSH worker 的复用 channel |

本文集中描述 Windows 远端。Windows 客户端的 SSH 配置审计、认证、
`ProxyJump` / `ProxyCommand` 和安全回退边界见跨平台 transport 文档。

Windows 远端至少应覆盖:

- Windows PowerShell 5.1;
- OpenSSH Server 提供的非交互 exec 和交互 PTY;
- x86_64 remote-server 归档;
- 安装、版本升级、daemon 复用、文件 RPC 和 PowerShell shell bootstrap。

PowerShell 7、Windows ARM64、域账号、多用户主机、非 ASCII 用户目录和受严格
EDR/杀毒策略管理的机器仍应当作为独立兼容性维度验证,不能从基础用例通过推断为
已支持。

## 端到端架构

```text
InfiniShell client
  │
  ├─ interactive SSH PTY ──────────────────────────────┐
  │                                                    │
  └─ remote-server command transport                   │
       │                                               │
       ├─ 平台探测 / 二进制检查 / 归档安装             │
       │    PowerShell -EncodedCommand                 │
       │                                               │
       └─ 启动协议代理                                 │
            cmd.exe 原始 stdio                         │
                    │                                  │
                    ▼                                  ▼
       infinishell.exe remote-server-proxy       PowerShell PTY
                    │                                  │
                    │ Tokio overlapped named pipe      │
                    ▼                                  │
       infinishell.exe remote-server-daemon            │
                    │                                  │
                    ├─ Initialize / request-response   │
                    ├─ WriteFileChunk                  │
                    └─ host-scoped RPC                 │
                              │                        │
                              └─ 暂存 pwsh bootstrap ──┘
                                  短 dot-source 命令 + CR
```

链路分成五个阶段:

1. **探测**:在已经认证的 SSH 连接上识别 `Windows <arch>` 和 PowerShell。
2. **安装**:先检查与客户端版本对应的远端 exe;缺失时优先上传安装包内归档,
   再考虑远端下载回退。
3. **代理**:通过 Windows `cmd.exe` 启动 `remote-server-proxy`,保留 stdin/stdout
   原始字节流。
4. **daemon**:proxy 连接身份隔离的 named pipe;不存在时在启动 mutex 保护下
   拉起常驻 daemon。
5. **初始化与 bootstrap**:完成 `Initialize` 后通过 `WriteFileChunk` 写入
   PowerShell bootstrap,再向交互 PTY 发送很短的 dot-source 命令。

任一增强阶段失败都不应破坏原本可用的交互 SSH shell。

## 必须保持的不变条件

### 1. 安装前不能依赖扩展 RPC

remote-server 尚未安装或尚未完成 `Initialize` 时,不存在可用的 `WriteFile` /
`WriteFileChunk` 服务。因此扩展归档必须由已有 SSH transport 上传,不能借待安装
扩展上传自己。这不是性能取舍,而是启动依赖关系。

安装来源顺序为:

1. 客户端安装包内与平台、架构、版本完全匹配的归档;
2. InfiniShell GitHub Release 下载;
3. 下载失败且错误允许回退时,客户端缓存/上传路径。

安装包内归档可用时,正常连接不应访问 GitHub。日志出现下载或 404 通常表示
manifest 缺失、版本不匹配、归档缺失、SHA-256 不匹配或客户端根本没有运行预期
安装包。

### 2. Initialize 后优先使用 `WriteFileChunk`

PowerShell 交互 PTY 不适合承载数十 KB 的 bootstrap。大段粘贴可能被截断、丢块
或受终端编辑状态影响。当前正确流程是:

1. remote-server 完成 `Initialize`;
2. 使用文件浏览器同款 request/response `WriteFileChunk` 写入完整脚本;
3. 校验响应的 `next_offset` 等于脚本长度;
4. 只向 PTY 发送 dot-source 命令;
5. 上传失败时才回退到原始 PTY bootstrap,保证 shell 不会永久卡住。

不要把旧的 `WriteFile` + `FileModelEvent` 异步事件链作为阻塞 bootstrap 的依赖;
它不提供这里需要的直接完成语义。

### 3. PowerShell 交互提交符是 `CR`

dot-source 命令必须以 `\r` 结束。不能根据本地客户端 OS 选择换行符:macOS 客户端
的 `\n` 写入远端 PowerShell PTY 后可能只显示命令而不执行,界面会一直停在
“Starting shell…”。

非交互 PowerShell 脚本继续使用 UTF-16LE Base64 和 `-EncodedCommand`,避免引号、
反斜杠、空格及非 ASCII 路径被中间 shell 重解释。

### 4. 协议代理必须保持原始字节流

平台探测和安装脚本使用 PowerShell,但 `remote-server-proxy` 不能通过 PowerShell
承载。PowerShell 的输出缓冲和文本编码会破坏双向协议流;代理启动使用
`WindowsCmd` 方言。

proxy stdout 只能写协议字节。调试信息必须走现有日志设施,不得使用
`println!`、`Write-Host` 或其他会污染 stdout 的输出。

### 5. named pipe 必须支持同时读写

proxy 使用 Tokio `NamedPipeClient` 的 overlapped I/O。不要退回“同步 pipe handle
复制后一个线程阻塞读、另一个线程写”的实现:复制的同步 Windows handle 仍共享
同一个 file object,挂起的读可以阻塞写,最终表现为 `Initialize` 永远收不到响应。

任何 proxy I/O 重构至少要保留“读已挂起时写仍能完成”的 Windows 原生回归测试。
同时要检查:

- stdin EOF 能结束 proxy;
- daemon EOF 能关闭 stdout worker;
- 大响应不会因 channel 背压丢失;
- 一个慢客户端不会阻塞 daemon 接受其他连接。

### 6. 客户端、manifest 和扩展必须使用同一个版本标签

`GIT_RELEASE_TAG` 同时决定:

- 客户端报告的版本;
- manifest 的 `version`;
- 远端版本化 exe 文件名;
- daemon pipe/socket 身份中的版本散列;
- `InitializeResponse.server_version` 的兼容性检查。

无标签 OSS debug 构建使用无后缀 `infinishell.exe`。若远端已有这个文件,二进制
检查会复用它,即使本地安装包带了更新归档。它适合固定开发 slot,不适合验证升级。

涉及安装或升级的测试必须给 Windows 扩展和客户端注入同一个、每轮唯一的标签。
升级测试必须保留旧 exe,让新客户端自然安装版本化文件;先手工删除旧文件会掩盖
版本选择缺陷。

## 代码地图

| 职责 | 入口 |
|---|---|
| 客户端 SSH transport 与安装状态机 | `app/src/remote_server/ssh_transport.rs` |
| 安装包内归档选择、manifest 与 SHA-256 校验 | `app/src/remote_server/ssh_transport/installation/scp_fallback.rs` |
| 平台、版本、路径和安装命令 | `crates/remote_server/src/setup.rs` |
| Windows PowerShell 命令生成 | `crates/remote_server/src/setup/windows.rs` |
| Windows proxy / daemon | `app/src/remote_server/windows/` |
| daemon 通用连接处理 | `app/src/remote_server/daemon.rs` |
| Initialize 客户端与 manager | `crates/remote_server/src/client/`、`crates/remote_server/src/manager.rs` |
| server 侧协议处理 | `app/src/remote_server/server_model.rs` |
| PowerShell bootstrap 编排 | `app/src/terminal/writeable_pty/remote_server_controller.rs` |
| PTY dot-source 字节 | `app/src/terminal/writeable_pty/pty_controller.rs` |
| Windows remote-server 归档 | `script/windows/package_remote_server.ps1` |
| 内置归档 manifest 工具 | `script/prepare_bundled_remote_server_resources.py` |
| Windows 协议级 SSH E2E | `crates/remote_server/src/ssh_e2e_tests.rs` |

## Windows 原生开发环境

推荐测试机别名为 `win-infinishell-build`,并满足:

- OpenSSH Server 已启动,公钥认证可在 `BatchMode=yes` 下工作;
- Windows PowerShell 5.1 可用;
- Rust MSVC toolchain、Visual Studio Build Tools 和仓库依赖已安装;
- 测试账号可在自己的 `$HOME\.infinishell` 下读写和启动进程;
- 测试机上的仓库 checkout 与本地分支同步。

先从客户端确认基础能力:

```bash
ssh -o BatchMode=yes -o ConnectTimeout=10 win-infinishell-build \
  'powershell.exe -NoLogo -NoProfile -NonInteractive -Command "$PSVersionTable.PSVersion; $env:PROCESSOR_ARCHITECTURE"'
```

Windows 上确认源码与工具链:

```powershell
Set-Location "$HOME\InfiniShell-Desktop"
git branch --show-current
rustc -Vv
cargo -V
Get-Service sshd
```

不要把开发用 daemon 与机器上其他 InfiniShell 会话混在一起。测试代码会记录启动前
的进程集合,只清理本轮产生、且 exe 路径完全匹配的进程。手工清理时也应使用相同
约束,不要按进程名无差别终止:

```powershell
$binary = (Resolve-Path '.\target\debug\infinishell.exe').Path
Get-Process | Where-Object { $_.Path -eq $binary } | Stop-Process -Force
```

## 推荐开发闭环

### 第 0 步:选择唯一版本和隔离数据 profile

每轮涉及安装的开发使用一个新标签。示例格式沿用可被现有 macOS bundle 版本逻辑
接受的形式:

```bash
export WINDOWS_SSH_DEV_TAG='v0.2026.08.18.20.50.oss_01'
export WARP_DATA_PROFILE='windows-ssh-dev'
```

标签必须同时传给 Windows 构建、partial manifest 和 macOS 客户端。不要在这些
步骤之间重新生成标签。

### 第 1 步:在 Windows 原生构建扩展

```powershell
Set-Location "$HOME\InfiniShell-Desktop"
$env:GIT_RELEASE_TAG = 'v0.2026.08.18.20.50.oss_01'
cargo build -p warp --bin infinishell --features standalone
.\target\debug\infinishell.exe --version
```

`standalone` 保证 remote-server 以控制台程序运行并从 exe 相邻资源目录读取资源。
不要给 remote-server 启用会切换到 Windows GUI subsystem 的 `release_bundle`。

先运行 Windows 特有单测和归档结构测试:

```powershell
cargo test -p warp named_pipe_async_client_can_write_while_read_is_pending -- --nocapture
pwsh -File script/windows/test_package_remote_server.ps1
```

若改动 daemon accept、pipe 桥接或大消息处理,同时运行
`app/src/remote_server/windows/mod_tests.rs` 中全部 Windows 测试。

### 第 2 步:先跑协议级 Initialize

在 macOS/Linux 客户端仓库设置测试机和 Windows 原生构建产物。路径必须是测试账号
实际可执行的绝对 Windows 路径:

```bash
export WARP_WINDOWS_SSH_E2E_HOST='win-infinishell-build'
export WARP_WINDOWS_SSH_E2E_REMOTE_BINARY='C:\Users\<user>\InfiniShell-Desktop\target\debug\infinishell.exe'

cargo test -p remote_server \
  windows_ssh_proxy_daemon_initialize_round_trip \
  -- --ignored --nocapture
```

该测试不是 mock。它真实覆盖:

1. SSH exec channel;
2. `remote-server-proxy`;
3. Windows named pipe;
4. 常驻 daemon;
5. 第一次 `Initialize`;
6. 同一连接上的第二次 `Initialize`;
7. `WriteFileChunk` 写入、独立读取校验和删除;
8. 只清理本轮测试 daemon。

这个测试失败时不要继续 GUI 或 DMG。先根据 proxy stderr 和失败阶段定位协议问题。

### 第 3 步:验证真实归档安装

快速协议循环可以直接用仓库 `resources/` 生成只含 Windows x86_64 的归档:

```powershell
Set-Location "$HOME\InfiniShell-Desktop"
$archive = Join-Path $PWD 'target\debug\infinishell-windows-x86_64.zip'
pwsh -File script/windows/package_remote_server.ps1 `
  -BinaryPath '.\target\debug\infinishell.exe' `
  -ResourcesDirectory '.\resources' `
  -DestinationPath $archive
```

把归档复制到客户端后运行生产安装脚本 E2E:

```bash
export WARP_WINDOWS_SSH_E2E_LOCAL_ARCHIVE='/absolute/path/infinishell-windows-x86_64.zip'

cargo test -p remote_server \
  windows_ssh_archive_install_and_initialize_round_trip \
  -- --ignored --nocapture
```

该测试先用 SCP 上传归档,再执行生产 PowerShell 安装脚本,最后执行完整
`Initialize → WriteFileChunk`。归档几十 MB 本身不是跳过测试的理由;应记录冷安装
耗时并与 `SCP_INSTALL_TIMEOUT` / `INSTALL_TIMEOUT` 比较,不要凭文件大小推断超时。

正式发布归档不能直接使用源码 `resources/`;必须由发布资源准备脚本生成完整资源树,
并通过 manifest 校验。

### 第 4 步:准备客户端 partial bundle

在客户端创建一个只包含本轮 Windows 归档的 partial 资源目录:

```bash
dev_input="$PWD/target/windows-ssh-dev/$WINDOWS_SSH_DEV_TAG/input"
dev_bundle="$PWD/target/windows-ssh-dev/$WINDOWS_SSH_DEV_TAG/bundled-remote-server"
mkdir -p "$dev_input"
cp "$WARP_WINDOWS_SSH_E2E_LOCAL_ARCHIVE" \
  "$dev_input/infinishell-windows-x86_64.zip"

python3 script/prepare_bundled_remote_server_resources.py create-partial \
  "$dev_input" \
  "$WINDOWS_SSH_DEV_TAG" \
  "$dev_bundle"

python3 script/prepare_bundled_remote_server_resources.py verify \
  --allow-partial \
  "$dev_bundle" \
  "$WINDOWS_SSH_DEV_TAG"

export WARP_BUNDLED_REMOTE_SERVER_PARTIAL_DIR="$dev_bundle"
```

manifest 的版本、平台/架构文件名和 SHA-256 都必须匹配。不要手写 manifest,否则
开发测试很容易误用上一次构建的归档。

### 第 5 步:不制作 DMG 调试完整 GUI 链路

第一次为当前架构准备可复用的开发 `.app` 壳。Apple Silicon 示例:

```bash
client_target='aarch64-apple-darwin'
client_features='release_bundle,extern_plist,autoupdate,gui,nld_classifier_v3,nld_heuristic_v2,fast_dev'

pushd app
GIT_RELEASE_TAG="$WINDOWS_SSH_DEV_TAG" \
  cargo bundle \
  --profile dev \
  --bin infinishell \
  --target "$client_target" \
  --features "$client_features"
popd

dev_app="$PWD/target/$client_target/debug/bundle/osx/InfiniShell.app"
script/macos/add_framework_rpath "$dev_app/Contents/MacOS/infinishell"

GIT_RELEASE_TAG="$WINDOWS_SSH_DEV_TAG" \
WARP_BUNDLED_REMOTE_SERVER_PARTIAL_DIR="$WARP_BUNDLED_REMOTE_SERVER_PARTIAL_DIR" \
NO_LICENSES=1 SKIP_SETTINGS_SCHEMA=1 \
  script/prepare_bundled_resources \
  "$dev_app/Contents/Resources" \
  oss dev "$client_features" "$client_target"

codesign --force --deep --sign - "$dev_app"
WARP_DATA_PROFILE=windows-ssh-dev \
  "$dev_app/Contents/MacOS/infinishell"
```

Intel Mac 把 target 改为 `x86_64-apple-darwin`。`fast_dev` 跳过登录流程;
`WARP_DATA_PROFILE` 隔离开发设置和状态。

后续只改客户端 Rust 代码时,复用 `.app` 壳并替换二进制即可:

```bash
GIT_RELEASE_TAG="$WINDOWS_SSH_DEV_TAG" \
  cargo build -p warp \
  --profile dev \
  --bin infinishell \
  --target "$client_target" \
  --features "$client_features"

ditto \
  "$PWD/target/$client_target/debug/infinishell" \
  "$dev_app/Contents/MacOS/infinishell"
script/macos/add_framework_rpath "$dev_app/Contents/MacOS/infinishell"
codesign --force --deep --sign - "$dev_app"
```

修改 Windows exe 或归档后必须重新执行第 1、3、4 步并刷新
`Contents/Resources/remote-server`;只替换 macOS 客户端二进制不会更新内置扩展。

在开发 app 内执行 `ssh win-infinishell-build`。成功链路至少应观察到:

- 版本化远端 exe 缺失时进入安装/升级;
- `Initialize` 返回相同标签和非空 `host_id`;
- `Staging remote PowerShell bootstrap`;
- PowerShell `Precmd` 和 `Bootstrapped` 事件;
- 创建 remote-server command executor;
- 文件浏览、代码审查等增强能力可用,而不只是出现交互提示符。

验证升级时,先确认远端保留无标签或上一标签的 exe,再启动新客户端:

```powershell
Get-ChildItem "$HOME\.infinishell\remote-server\infinishell*.exe" |
  Select-Object Name, Length, LastWriteTime
```

新标签 exe 应与旧文件共存,且新 exe 的 `--version` 与客户端标签一致。

### 第 6 步:协议与 GUI 都通过后再制作 DMG

最终使用仓库标准 bundle 流程,继续传递完全相同的标签和 remote-server 资源目录。
示例只构建当前 Apple Silicon 架构:

```bash
GIT_RELEASE_TAG="$WINDOWS_SSH_DEV_TAG" \
WARP_BUNDLED_REMOTE_SERVER_PARTIAL_DIR="$WARP_BUNDLED_REMOTE_SERVER_PARTIAL_DIR" \
  script/macos/bundle \
  --debug \
  --nouniversal \
  --arch aarch64 \
  --channel oss \
  --release-tag "$WINDOWS_SSH_DEV_TAG" \
  --features fast_dev \
  --selfsign \
  --dmg-name-suffix windows-ssh-dev
```

对外发布时不能使用 partial bundle、`--debug`、`fast_dev` 或 ad-hoc/开发签名;
必须包含发布矩阵要求的全部 remote-server 产物并走正式签名、公证流程。

DMG 生成后必须挂载回读,不能只检查构建目录:

```bash
python3 script/prepare_bundled_remote_server_resources.py verify \
  --allow-partial \
  '/Volumes/<volume>/InfiniShell.app/Contents/Resources/remote-server' \
  "$WINDOWS_SSH_DEV_TAG"

codesign --verify --deep --strict \
  '/Volumes/<volume>/InfiniShell.app'
```

同时核对 `Info.plist` 版本、macOS 二进制架构、zip 内 `infinishell.exe` 大小和 DMG
SHA-256。

## 失败阶段与排查路径

| 现象 | 最可能阶段 | 首要检查 |
|---|---|---|
| `Failed to install SSH extension` | 归档选择、上传或 PowerShell 安装 | manifest 版本/SHA、归档是否在 app 内、安装脚本 stderr、远端磁盘和 EDR |
| 出现 GitHub 404 | 内置归档没有被选中 | 客户端标签、manifest、目标 OS/arch、运行的 app 是否为本轮构建 |
| 卡在 `Initializing` | proxy、named pipe 或 Initialize | Windows pending-read 测试、协议 E2E、proxy stderr、daemon 是否提前退出 |
| Initialize 成功但卡在 `Starting shell…` | bootstrap 暂存或 PTY 提交 | `WriteFileChunk.next_offset`、远端 `.ps1`、dot-source 是否以 `\r` 结束 |
| shell 可用但文件浏览不可用 | manager/session 关联或 RPC | `host_id`、session 到 client 索引、remote command executor 是否创建 |
| 每次连接都重新安装 | 版本/路径检查失败 | 新 exe `--version`、客户端标签、binary check exit code |
| 新客户端继续使用旧扩展 | 无标签开发 slot 被复用 | 为客户端和扩展设置同一个新 `GIT_RELEASE_TAG` |
| 出现第二次认证提示 | transport 没有复用已认证 session | broker/capability、回退边界、ProxyJump 目标连接数量 |
| 协议随机损坏或 protobuf 解析失败 | stdout 被文本污染 | proxy/daemon 中的打印、PowerShell 文本管道、编码转换 |
| 首个请求成功、后续请求挂起 | pipe 半双工/背压/生命周期 | overlapped I/O、EOF、bounded channel、并发连接测试 |

建议按以下顺序收集证据:

1. 普通终端中的同一条 `ssh` 是否可用;
2. 客户端版本标签和实际 app 路径;
3. app 内 manifest、归档 SHA 和 zip 内容;
4. 远端版本化 exe 列表及目标 exe `--version`;
5. 安装、proxy、Initialize、bootstrap 中最后一个成功阶段;
6. 协议 E2E 的 stderr;
7. Windows daemon 进程路径和数量。

报告中不得包含私钥、密码、capability token、完整 `known_hosts` 或未脱敏的用户与
代理端点。

## 已知风险与后续工作

### 发布阻断级

- named pipe ACL 需要在多用户 Windows 主机上做安全审计,确认其他本地用户不能
  连接或冒充当前身份的 daemon;
- Windows ARM64 归档和原生运行尚需持续验证,不能只验证 x86_64 zip 文件名;
- PowerShell 5.1、PowerShell 7 作为默认 shell 时的探测、编码与 PTY 行为需要
  独立 fixtures;
- 任一版本升级不得复用不兼容 daemon 或旧 exe;
- EDR/杀毒隔离 exe、阻止 named pipe 或延迟首次启动时,必须给出可诊断错误并保留
  交互 shell。

### 高优先级兼容性

- 用户目录包含空格、中文、组合字符、单引号和尾随反斜杠;
- 多窗口同时首次安装同一版本,以及安装时另一个会话连接;
- 慢网络、短读写、大响应、客户端突然退出和 sshd 强制断线;
- Windows 更新或 OpenSSH 更新后的 `cmd.exe` / PowerShell 命令行规则变化;
- global `bundled_resources` 的“最后一次安装覆盖”语义与多个版本 daemon 并存;
- daemon crash 后的 pipe、mutex、数据库与临时安装目录恢复;
- Rust SSH worker 和原生 OpenSSH 两种客户端 transport 的行为一致性。

### 工程化改进

- 为 Windows 构建机增加 protocol E2E 的受控 CI job;
- 给冷安装、热连接、Initialize 和首个 RPC 分阶段记录受控基准;
- 建立 Windows x86_64 / ARM64、PowerShell 5.1 / 7、直连 / ProxyJump 测试矩阵;
- 让开发 `.app` 准备流程成为独立脚本,减少手工替换二进制和资源的机会;
- 增加安装来源、版本选择和失败阶段的脱敏诊断导出,但不要记录协议内容或凭据。

## 修改检查清单

改动 Windows SSH 扩展前后逐项确认:

- [ ] 改动属于探测、安装、proxy、daemon、协议还是 bootstrap 中的明确一层;
- [ ] PowerShell 脚本使用 `-EncodedCommand`,协议流没有经过 PowerShell;
- [ ] 没有给 proxy/daemon stdout 添加日志;
- [ ] named pipe 仍为 overlapped async I/O,读挂起时写可完成;
- [ ] 安装前没有依赖扩展 RPC;
- [ ] Initialize 后的大文件使用直接 request/response 的 `WriteFileChunk`;
- [ ] PowerShell 交互执行字节以 `\r` 提交;
- [ ] 客户端、manifest、归档和扩展使用同一个唯一标签;
- [ ] 升级测试保留旧 exe,没有通过清空远端状态制造假通过;
- [ ] `cargo check -p warp` 通过;
- [ ] Windows pipe 回归测试通过;
- [ ] 基础协议 E2E 通过;
- [ ] 生产归档安装 E2E 通过;
- [ ] fast-dev `.app` 的真实 GUI 链路通过;
- [ ] 失败时普通交互 SSH shell 仍然可用;
- [ ] 上述门禁全部通过后才制作和安装 DMG;
- [ ] DMG 挂载回读后的签名、版本、manifest、SHA 和架构通过。

若修改会影响 POSIX remote-server 或 Windows 客户端 Rust SSH worker,还必须执行
[跨平台 SSH transport](cross-platform-ssh-transport.zh-CN.md) 中对应的完整回归矩阵。
