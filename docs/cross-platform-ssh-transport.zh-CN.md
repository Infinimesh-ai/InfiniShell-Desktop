# 跨平台 SSH transport

状态:已实现,分阶段放量  
实现基线:`2082841bb`、`1e6fdfaa4`、`363914f1d`  
最后更新:2026-08-16  
英文版:[cross-platform-ssh-transport.md](cross-platform-ssh-transport.md)

## 目标与不变条件

目标是将 InfiniShell 的 shell 集成和 remote-server 扩展到 Windows,同时不破坏
过去在 macOS、Linux 或 Windows 上已经能工作的 SSH 连接方式。

因此,实现遵守四个不变条件:

1. 原有 macOS/Linux OpenSSH 与 ControlMaster 链路继续可用。
2. Windows 客户端不依赖 ControlMaster,也能增强兼容的 POSIX 或 Windows
   远端会话。
3. `ProxyJump` 和进程形式的 `ProxyCommand` 配置继续可用。
4. 只要 InfiniShell 无法如实保留某个 SSH 选项的行为或安全语义,就在
   认证前把原始参数交还给原生 OpenSSH,而不是静默忽略该选项。

## 能力矩阵

| 客户端 | POSIX 远端(`bash` / `zsh`) | Windows 远端(PowerShell) |
|---|---|---|
| macOS / Linux | 原有 OpenSSH ControlMaster 扩展 + POSIX bootstrap | 版本化 PowerShell 能力探测 + PowerShell bootstrap + remote-server |
| Windows | Rust SSH worker + POSIX bootstrap + remote-server | Rust SSH worker + PowerShell bootstrap + Windows remote-server |

这个矩阵描述的是兼容的、交互式的单目标会话。每一格在不适用增强链路时
仍保留原生 SSH。发生回退可能表示终端连接仍然正常,但当前会话无法使用
InfiniShell shell 集成或 remote-server。

## 选路与连接流程

### macOS 与 Linux 客户端

原有 shell wrapper 继续使用 OpenSSH ControlMaster。第一次探测读取 `$SHELL`;
如果结果无法识别 `bash` 或 `zsh`,就通过已经认证的 master connection
执行一个固定、版本化的 PowerShell 能力探测。有效的 Windows 结果直接选择
PowerShell bootstrap;未知结果则继续普通 SSH。

### Windows 客户端

PowerShell wrapper 只拦截带一个目标、不含已配置 `RemoteCommand` 的交互命令。
`Warp-Test-IsWindows` 检测的是本地客户端平台,不用它判断远端系统。在
Windows 上,wrapper 调用随应用打包的 `infinishell-ssh rust-ssh-session` worker。

worker 随后:

1. 通过用户的 OpenSSH 可执行文件运行 `ssh -G`,获取有效配置;
2. 审计返回的每一个选项,不支持的非中性值转原生回退;
3. 通过直连 TCP、`ProxyCommand` 或 `ProxyJump` 字节桥建立连接;
4. 验证主机密钥并完成认证;
5. 在该目标 session 上探测远端 shell;
6. 启动对应的 POSIX 或 PowerShell bootstrap 与交互 PTY;
7. 开启环回 broker,用 capability token 授权在同一目标 session 上增建 exec
   channel。

这避免了 remote-server 命令触发第二个目标连接和第二次认证提示。
`ProxyJump` 必然包含到跳板机的独立连接,但目标 session 仍然被复用。

## Transport 与 remote-server 细节

Windows 安装包在同一 worker 协议后包含两个 Rust 后端:

- 基于 `ssh2` 的兼容后端;
- 新的异步 `russh` 后端,同时受编译期 `russh_transport` feature 与运行时
  `RusshTransport` feature flag 控制。

关闭 `RusshTransport` 时保留 `ssh2` 后端。截至本文档日期,该运行时开关尚未
加入默认放量列表,因此可以在不删除既有链路的前提下分阶段开启。

两个增强后端都覆盖 InfiniShell 需要的会话行为:

- 普通 `known_hosts` 验证和严格主机密钥确认;
- OpenSSH agent 认证,包括 Unix socket,以及所选后端支持时的 Windows OpenSSH
  named pipe/Pageant;
- 有序 identity file、加密私钥、RSA 签名选择、keyboard-interactive 和密码认证;
- 协商算法;配置与后端都支持时使用 ML-KEM;
- PTY 分配、环境变量传递、终端缩放、escape 处理、keepalive 与压缩;
- remote-server 命令 transport 所用的、受 capability 保护的环回 broker。

Windows remote-server 产物与客户端分开打包,使用 PowerShell 感知的路径和归档处理
完成安装,并通过 Windows daemon/proxy 实现启动。POSIX remote-server 保持原有链路。

## 安全回退边界

wrapper 把非交互操作、转发/隧道模式、多目标和显式远端命令留给原生
OpenSSH。worker 遇到无法保留语义的配置时,也会在认证前回退。例如:

- GSSAPI、host-based authentication、SSH certificate 与 security-key provider 策略;
- agent/X11/端口转发以及依赖 shell 的 `ProxyCommand` 表达式;
- `known_hosts` 中的 `@cert-authority` 或 `@revoked` 条目;
- `UpdateHostKeys=yes`、`ObscureKeystrokeTiming=yes`、
  `StrictHostKeyChecking=no` 等安全敏感的非中性值;
- 所选后端与用户算法配置交集为空,例如当前 `russh` 后端遇到仅
  SNTRUP 的密钥交换配置。

只有在尚未弹出提示、也未成功认证时才允许回退。越过这个边界后,失败会从当前
尝试返回,不会突然启动第二个 SSH 连接。如果认证后的远端 shell 探测或
bootstrap 不可用,同一目标 session 可以继续作为普通交互 shell。

`UpdateHostKeys` 证明请求和 OpenSSH 按键混淆 chaff 需要当前 `russh` 公开 API
尚未暴露的协议钩子。它们作为兼容性缺口跟踪,不会用更弱的语义近似实现。

## 回归契约

以下行为一旦回归,应视为发布阻断项:

- 过去可通过原生 SSH 连接的命令必须仍然能连接;
- 回退时必须保留原始 SSH 参数和用户解析后的 OpenSSH 配置;
- `ProxyJump` 和进程形式的 `ProxyCommand` 必须继续工作;
- 增强的目标 session 不能产生意外的第二次认证提示或第二个目标连接;
- 主机密钥不匹配或被拒绝时绝不允许绕过;
- remote-server 安装或启动失败不能破坏原本可用的交互 SSH shell。

## 验证

跨平台验证前先运行本地门禁:

```bash
cargo check -p warp --lib --locked
cargo check -p warp --bin infinishell-ssh --locked \
  --features rust_ssh_worker,russh_transport
cargo nextest run -p warp --locked \
  --features rust_ssh_worker,russh_transport \
  -E 'test(remote_server::rust_ssh)'
cargo fmt --all -- --check
```

在 Windows 构建机上还应验证 remote-server 打包:

```powershell
pwsh -File script/windows/test_package_remote_server.ps1
```

手工发布覆盖至少包含:

| 用例 | 必要观察结果 |
|---|---|
| 直连密钥认证 | 最多一次提示;增强 shell 与 broker 正常 |
| 密码 / keyboard-interactive | 提示可用,且不会被回退重复 |
| 主机密钥变更 | 拒绝连接 |
| 严格 `ask` 下的新主机 | 只有明确同意后才记录密钥 |
| `ProxyJump` 和进程形式 `ProxyCommand` | 目标可连接,增强命令复用 session |
| POSIX 与 Windows 远端 | 选择正确 bootstrap 和 remote-server 产物 |
| 不支持的 SSH 选项 | 原生 SSH 收到原始命令并仍可用 |
| remote-server 安装失败 | 交互 shell 在无增强状态下仍可用 |

精确的 merge commit 在远端可用后,应使用 Linux 与 Windows runner 运行跨平台
云验证。如果 commit 尚只在本地,未获明确授权时不应仅为触发验证而 push。

### 基线验证记录(2026-08-16)

整合当前本地 `main` 后的功能树已通过:

- `cargo check -p warp --lib --locked`;
- 同时启用两个 Rust transport feature 的 `infinishell-ssh` worker check;
- 全部 38 个 `remote_server::rust_ssh` 聚焦测试;
- `cargo fmt --all -- --check`;
- PowerShell 下的 `script/windows/test_package_remote_server.ps1`。

在 macOS 上面向 `x86_64-pc-windows-msvc` 交叉编译 worker 时,在进入项目 Rust
代码前就停在 `aws-lc-sys 0.39.1`,原因是 macOS 主机没有 Windows SDK 头文件
(`stdlib.h` 与 `windows.h`)。原生 Windows 和 Linux 云任务需等精确 merge commit
push 后才能运行;这是环境/授权缺口,不代表 Windows 运行时已验证通过。

## 合并后观察记录

在发布 issue 或后续文档中使用下表。请按“客户端/远端”组合分开记录,避免
健康的 POSIX 链路掩盖 Windows 回归。

| 信号 | 期望结果 | 跟进阈值 |
|---|---|---|
| 原生 SSH 连接成功 | 相比上一版无回归 | 任一确认回归都阻断放量 |
| 到可用提示符的时间 | 按每个矩阵格记录中位数和 p95,并与上一版对比 | 可重现的显著回归暂停放量 |
| 增强 bootstrap 激活 | 受支持的 shell/配置组合成功 | 某一矩阵格反复失败时必须增加 fixture 与测试 |
| remote-server 安装/启动 | 使用匹配 OS/架构的产物 | 系统性平台/架构失败阻断放量 |
| broker 命令执行 | 使用已认证的目标 session | 任何第二次目标登录/提示都是阻断项 |
| 首个 broker 命令延迟 | 分别记录冷安装和热 session 的中位数/p95 | 可重现的显著回归需先 profiling 再放量 |
| 安全原生回退 | 普通 SSH 仍可用 | 任何参数丢失/改写都是阻断项 |
| 回退率与原因 | 每次回退都对应明确的兼容边界 | 新增或上升的未解释原因需增加 fixture 与测试 |
| ProxyJump/ProxyCommand | 直连和跳板机 fixture 通过 | 任一过去可用的 fixture 失败都是阻断项 |
| 主机密钥处理 | 新密钥/不匹配/撤销场景保留策略 | 任何弱化都是安全阻断项 |

InfiniShell 不为这个 transport 新增云遥测。在受控测试 fixture 中收集延迟与回退
观察结果,并通过验证矩阵和脱敏 issue 报告跟踪现场表现。有效报告应包含:

- 客户端 OS 和 InfiniShell 版本;
- `ssh -V` 输出以及远端 OS/默认 shell;
- 直连、`ProxyJump` 或 `ProxyCommand` 连接类型;
- 脱敏的 `ssh -G <host>` 结果和可见回退消息;
- 同一命令是否仍能在普通终端工作;
- 失败发生在认证前、认证后、bootstrap、安装还是 broker 命令执行阶段。

绝不要附带私钥、密码、capability token、完整 `known_hosts` 文件,或未脱敏的
主机/用户/路径/代理端点。

## 代码地图

- PowerShell 客户端 wrapper:
  `app/assets/bundled/bootstrap/pwsh_ssh_wrapper.ps1`
- POSIX 到 Windows 能力探测:
  `app/assets/bundled/bootstrap/ssh_remote_shell_probe.sh`
- Rust worker 与配置门禁:
  `app/src/remote_server/rust_ssh.rs`
- 分阶段 `russh` 后端:
  `app/src/remote_server/rust_ssh/russh_backend.rs`
- 客户端 remote-server transport:
  `app/src/remote_server/ssh_transport.rs`
- Windows daemon/proxy:
  `app/src/remote_server/windows/`
- 远端安装/平台 setup:
  `crates/remote_server/src/setup.rs` 与
  `crates/remote_server/src/setup/windows.rs`
- Windows 打包:
  `script/windows/package_remote_server.ps1` 与
  `script/windows/bundle.ps1`
