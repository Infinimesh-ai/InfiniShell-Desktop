# Recursive SSH Extension 技术方案

对应产品规格：`PRODUCT.md`。

## 1. 总体结构

递归 SSH 保留两条相互独立的数据路径：

```text
用户终端：local PTY -> ssh A -> ssh B -> ssh C
控制侧车：client -> daemon A => daemon B => daemon C
```

终端字节继续走用户实际启动的嵌套 OpenSSH 进程。remote-server 协议通过父 daemon
提供的多路复用字节流连接子 daemon，不接管用户 PTY，也不要求开放额外 TCP
端口。

## 2. 领域模型

### 2.1 控制引用

将当前 `IsSSHWrapperSession::Yes { socket_path: PathBuf, ... }` 改为带作用域和所有权的
`ScopedControlPath`：

```rust
pub struct ScopedControlPath {
    path: PathBuf,
    owner_session_id: SessionId,
    scope: SshControlScope,
    ownership: ControlMasterOwnership,
    hop_depth: u32,
}

pub enum IsSSHWrapperSession {
    Yes {
        control_path: ScopedControlPath,
    },
    No,
}
```

远端 wrapper 发出的 socket 路径只能由其所在 daemon 使用。客户端收到 SSH hook
后立即向父 daemon 注册路径，换取不泄露路径语义的 `control_id`；后续 transport
只保存这个不透明引用和可随父连接更新的 `ParentConnectionHandle`。

### 2.2 连接图

在 `RemoteServerManager` 增加运行时路由信息：

```rust
pub struct RouteNode {
    pub session_id: SessionId,
    pub parent_session_id: Option<SessionId>,
    pub host_id: Option<HostId>,
    pub target_alias: String,
    pub port: Option<u16>,
    pub depth: u32,
    pub state: RouteNodeState,
}

pub enum RouteNodeState {
    Preparing,
    Connected,
    BlockedByParent,
    Reconnecting,
    Disconnected,
}
```

`SessionInfo::spawning_session_id` 是父子关系的输入。manager 持有可查询的图，终端
model 继续持有 shell session 栈，避免复制生命周期事实。

## 3. 协议扩展

### 3.1 顶层消息

隧道是连接级资源，必须绑定创建它的父 remote-server connection，不能使用当前
可故障转移的 host-scoped 请求。

隧道数据面不能进入现有的 `Notification -> ClientEvent -> 主线程` 路径。给两个顶层
envelope 各增加一个专用变体：

```proto
message ClientMessage {
  string request_id = 1;
  oneof message {
    // 原有变体保持不变。
    TunnelClientMessage tunnel = 5;
  }
}

message ServerMessage {
  string request_id = 1;
  oneof message {
    // 原有变体保持不变。
    TunnelServerMessage tunnel = 43;
  }
}
```

最小隧道协议为：

```proto
message TunnelClientMessage {
  string stream_id = 1;
  oneof message {
    RegisterSshControl register_control = 2;
    ReleaseSshControl release_control = 3;
    OpenSshStream open = 4;
    TunnelData data = 5;
    TunnelWindowUpdate window_update = 6;
    TunnelHalfClose half_close = 7;
    TunnelReset reset = 8;
  }
}

message TunnelServerMessage {
  string stream_id = 1;
  oneof message {
    RegisterSshControlResponse control_registered = 2;
    OpenSshStreamResponse opened = 3;
    TunnelData data = 4;
    TunnelWindowUpdate window_update = 5;
    TunnelExit exit = 6;
    TunnelReset reset = 7;
  }
}
```

字段编号在实现时以 proto 当前最高编号为准，只追加，不复用已有编号。
低频的平台检测、安装检查等请求仍可使用 session-scoped request；字节流的
Register/Open/Data/Window/Close 全部走上述连接级路径。

### 3.2 隧道标识与竞态

`tunnel_id` 由客户端生成，并在发送 `OpenSshTunnel` 前注册本地接收状态。这样服务端
即使在 open response 后立即推送数据，也不会出现“数据先于本地 map”竞态。

### 3.3 流控

每个 channel 维护独立 credit 和连续 offset：

- 默认窗口 256 KiB；
- 单帧最多 32 KiB；
- 消费数据后发送 `WindowUpdate(consumed_bytes)`；
- 没有 credit 时暂停读取子进程 stdout，而不是阻塞父协议 reader；
- credit 使用 checked arithmetic，并设窗口上限；
- offset 出现缺口、重复、超出窗口、未知 tunnel 或重复 half-close 时关闭该 tunnel，
  不关闭父连接。

流控属于第一阶段的正确性要求，不能推迟。每个父连接初始最多 16 个活动 tunnel，
控制消息使用独立有界队列并获得调度优先级；数据消息按 stream 公平轮询。

### 3.4 关闭语义

- `HalfClose(write)`：关闭对应子进程 stdin；
- 子进程 stdout EOF：服务端发送 half-close/read EOF；
- 子进程退出：发送 `SshTunnelClosed { exit_code, stderr_tail }`；
- 父 connection drop：daemon kill 全部归属该 connection 的 tunnel；
- 本地 `TunnelStream` drop：发送 `CloseSshTunnel`，不得影响用户交互式 SSH
  ControlMaster，除非它由 InfiniShell 拥有且对应 shell 已退出。

## 4. daemon 端实现

### 4.1 控制引用注册

`RegisterSshControl` 接受：

- 当前 shell session id；
- ControlMaster socket 路径；
- 所有权；
- hop depth。

daemon 校验：

- 路径必须是绝对 Unix socket 路径；
- 路径长度和字符串长度受限；
- `ssh -O check -o ControlPath=... placeholder@placeholder` 成功；
- 注册项绑定当前 proxy connection 和 session；
- 返回随机 `opaque_id`。

所有后续请求只接受 `opaque_id`，客户端不再传任意路径或任意远程命令。

### 4.2 检查与安装

daemon 端复用现有 SSH 安装逻辑，但执行位置变成父主机：

- detect platform；
- preinstall check；
- binary check；
- install/update；
- launch `remote-server-proxy`。

目标机无法直接下载时，客户端先为检测到的平台选择内置或缓存 tarball，再通过同一
受流控隧道把内容写入 daemon 生成的固定 staging 路径，随后由另一项固定 purpose
运行安装脚本。协议不接受客户端提供的 staging 路径或任意安装命令。

接口返回结构化 `RemotePlatform`、`PreinstallCheckResult`、`InstallSource` 与错误类型，
本地仍由 `RemoteServerController` 决定是否询问、安装或回退。

### 4.3 隧道进程

`OpenSshTunnel` 只允许启动固定用途：

```text
ssh -S <registered-control-path> placeholder@placeholder \
  <remote-server-binary> remote-server-proxy --identity-key <key>
```

协议不提供“任意 remote_command”字段，避免把隧道 API 扩大成新的任意命令执行
入口。

## 5. 客户端实现

### 5.1 RemoteSshTransport

新增 `RemoteSshTransport` 实现 `RemoteTransport`：

- 持有父 `RemoteServerClient`、`opaque_id` 与认证上下文；
- detect/check/install 映射为父 daemon 的 session-scoped 请求；
- connect 调用 `open_ssh_tunnel`，得到实现 `AsyncRead + AsyncWrite` 的
  `TunnelStream`；
- 用 `TunnelStream` 构造子 `RemoteServerClient`。

`RemoteSshTransport` 持有按父 session 更新的 `ParentConnectionHandle`，而不是缓存某次
连接产生的 `Arc<RemoteServerClient>`；父节点重连后 handle 原子切换到新 client。

现有本地 `SshTransport` 行为不变。`RemoteServerController` 根据
`ScopedControlPath.scope` 选择 transport。

### 5.2 RemoteServerClient 隧道管理

增加：

```rust
struct TunnelRegistry {
    tunnels: HashMap<TunnelId, TunnelState>,
}
```

读循环直接把 tunnel frame 分发到有界 channel，不生成 `ClientEvent`，也不进入 UI
主线程。普通控制消息与 tunnel 数据分别进入有界发送队列；writer 优先处理控制消息，
并在 tunnel 间公平轮询。每 tunnel 的异常只通知对应 stream，不能让普通
request/response pending map 失效。

### 5.3 连接资源抽象

现有 `Connection` 强制持有本地 `async_process::Child` 和本地 `ControlPath`，必须改为
类型擦除的生命周期资源：

```rust
pub trait TransportConnection: Send + Debug {
    fn terminate(&mut self);
    fn try_exit_status(&mut self) -> Option<RemoteServerExitStatus>;
    fn wait_for_exit(
        self: Box<Self>,
    ) -> Pin<Box<dyn Future<Output = Option<RemoteServerExitStatus>> + Send>>;
    fn stderr_tail(&self) -> Option<String>;
}
```

本地实现持有 SSH child；远端实现持有 `TunnelGuard`。ControlMaster teardown 下沉到
`RemoteTransport`：本地 transport 在本机执行 `ssh -O exit`，远端 transport 请求父
daemon 释放注册；用户拥有的 control 两边都不得执行 `-O exit`。

### 5.4 路由生命周期

- 连接子节点前确认父节点 `Connected`；
- 父断开时按拓扑标记全部后代 `BlockedByParent` 并取消 tunnel；
- 父重连完成后，从浅到深恢复仍有活动 shell 的子节点；
- 子 shell `ExitShell` 只 deregister 自己和后代，恢复父 session executor；
- 用户拥有的外部 ControlMaster 永不由 InfiniShell 关闭。

## 6. shell bootstrap

### 6.1 递归启用条件

替换只允许 `WARP_IS_LOCAL_SHELL_SESSION=1` 的判断。wrapper 在以下条件启用：

```text
本地 shell
或
当前远端 shell 已连接支持 recursive_ssh capability 的 remote-server
```

远端 bootstrap 注入：

```text
WARP_SSH_PARENT_SESSION_ID
WARP_SSH_HOP_DEPTH
WARP_RECURSIVE_SSH_EXTENSION=1
```

`InitializeResponse` 增加显式 `SSH_BYTE_STREAM_V1` capability。只有父 daemon 宣告支持
且运行时 Feature Flag 开启时才注入递归 wrapper 环境，不能仅按版本字符串推断。

超过深度、父 daemon 不支持或 Feature Flag 关闭时调用 `command ssh`。

### 6.2 SSH hook

扩展 hook：

```json
{
  "hook": "SSH",
  "value": {
    "socket_path": "...",
    "session_id": 1,
    "remote_session_id": 2,
    "parent_session_id": 1,
    "hop_depth": 2,
    "control_scope": "remote"
  }
}
```

旧客户端缺少字段时仍按本地 scope 解析；新客户端只有在 Feature Flag 开启且父
session 有已连接 daemon 时接受 `remote` scope。

bash、zsh、fish 必须保持等价实现，公共生成内容优先放在已有 bootstrap 生成层，
避免三个脚本长期漂移。

## 7. 持久化

### 7.1 表

阶段二新增 migration：

```sql
ssh_routes(id, name, target_node_id, created_at, updated_at, last_connected_at)
ssh_route_hops(route_id, position, node_id, target_alias, port, execution_scope)
```

`node_id` 可空，允许保存仅在父网络中发现、尚未加入 SSH Manager 的目标。
`port` 可空，空值保留 OpenSSH 别名配置的端口选择；`execution_scope` 只表达从前一跳
执行，不保存 socket 路径。

### 7.2 保存路径启动

- 第一跳关联 SSH Manager 节点时复用现有本地凭据解析；未关联时按普通 OpenSSH
  alias/key-auth 路径发起，不猜测或注入密码；
- 只有当前跳完成 shell bootstrap 且路由节点进入 `Connected` 后，才在该父 shell
  注入下一条 `ssh [-p port] <alias>`；
- alias 必须通过长度、空白、控制字符和前导选项校验，并经过 shell 参数转义；
- 任一跳失败、提前退出或未 bootstrap 时停止自动推进，但保留已建立的普通 SSH
  shell；
- 最后一跳成功后更新 `last_connected_at`，路径结构可同步，运行时 socket 和凭据不
  参与持久化。

### 7.3 同步

路径结构可以云同步；以下内容永不进入同步载荷：

- 密码和 passphrase；
- 私钥内容与临时路径；
- ControlMaster 路径；
- agent socket；
- host key 的未确认状态。

同步载荷中的 `routes` 使用可选字段保持向后兼容：字段缺失表示旧客户端不认识该
数据，应用时保留本地路径；只有新客户端显式发送空数组才表示清空保存路径。

## 8. 消费者迁移

以下功能从“按 terminal session 找 host”统一改为查询当前 route leaf：

- `RemoteServerCommandExecutor`；
- 远端文件打开与 SFTP；
- ProjectHostSessionRouter；
- Agent remote context 与 machine memory；
- repo detection、codebase indexing 与 remote file browser。

machine memory 的主机键不再仅依赖 `host:port`；取得稳定 HostId/指纹后迁移到稳定
身份，未取得时保留 route-scoped 临时键。

## 9. 错误与日志

- 网络、认证、目标不支持与用户取消：结构化非 actionable 错误，UI 展示并
  `warn`/`error`，不报 Sentry；
- 未知 tunnel、窗口下溢、父子图不一致：actionable 协议/状态错误，在最终 sink
  报告一次；
- Info 以上日志不得包含目标命令、用户名、socket 路径、配置正文或凭据；
- 高频 tunnel frame 仅允许 trace/debug 级别统计，不逐帧记录 payload。

## 10. 测试

### 10.1 单元测试

- `SshControlRef` 序列化与旧 hook 兼容；
- route 图插入、叶节点、级联阻塞、拓扑恢复、深度上限；
- tunnel 双向 credit、half-close、未知 id、overflow；
- tunnel offset 缺口/重复、队列上限和跨 stream 公平性；
- 父连接 drop 清理 tunnel；
- 不关闭 user-owned ControlMaster。

测试放在对应 `${file}_tests.rs`。

### 10.2 协议测试

- 所有新增 proto message round-trip；
- 旧消息仍可解码；
- tunnel data 超过全局/单帧限制被拒绝；
- 普通请求与 tunnel push 交错时 request_id 路由正确。

### 10.3 GUI/PTY 集成测试

使用 hermetic 三主机拓扑：

```text
client -> A -> B -> C
client -X-> B/C
A      -X-> C
```

每跳使用独立 sshd、known_hosts、用户和密钥。测试通过仓库 Builder/TestStep 框架
启动，不读取开发者真实 `~/.ssh`。

覆盖：

1. A -> B 自动扩展；
2. A -> B -> C 自动扩展；
3. C、B 逐级退出恢复；
4. 父连接中断和有序恢复；
5. B 不支持/安装失败时普通 SSH 可用；
6. bash、zsh、fish wrapper；
7. Feature Flag 关闭时行为完全等价于当前版本。

## 11. 验证顺序

1. focused unit/protocol tests；
2. 单个 GUI integration test；
3. `cargo check`；
4. 格式化与 diff 审计；
5. 推送精确 commit 后运行 Linux x64 与 Windows x64
   `cross-platform-preflight.yml`；
6. 最后运行需要的 workspace tests。
