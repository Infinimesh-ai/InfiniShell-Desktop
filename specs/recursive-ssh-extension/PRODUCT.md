# Recursive SSH Extension

## 1. 问题

InfiniShell 当前可以把本地 shell 发起的一次交互式 SSH 会话升级为带
remote-server 能力的远端会话，但不支持用户在远端 shell 中继续执行 SSH。
例如：

```text
local -> bastion -> staging -> database
```

第一跳 `local -> bastion` 可以获得远端命令执行、文件浏览、代码索引与 Agent
上下文；后续跳转则退化成普通 SSH。用户需要自己维护 `ProxyJump`、复制配置、
处理凭据或接受能力丢失。

目标是让用户继续使用熟悉的 `ssh <target>` 命令，InfiniShell 自动扩展每一个
交互式远端会话，且支持任意多级。

## 2. 产品原则

1. **用户命令不变。** 用户不需要学习 InfiniShell 专属连接语法。
2. **每一跳就地执行。** 在主机 A 输入的 `ssh B` 必须由 A 的 OpenSSH 执行，
   从而自然使用 A 的 DNS、`~/.ssh/config`、密钥、agent 与网络可达性。
3. **自动化连接基础设施，不扩散秘密。** 不自动复制私钥，不自动开启 agent
   forwarding，不静默接受变化的 host key。
4. **逐级增强，失败开放。** 任一扩展步骤失败时保留普通 SSH 会话，不阻断用户
   登录。
5. **运行时连接图是事实来源。** 收藏的服务器配置不是当前连接拓扑；相同别名
   在不同内网中可以代表不同主机。
6. **任意深度使用同一套递归协议。** 第二跳和第 N 跳不使用不同的特例实现。

## 3. 用户体验

### 3.1 自动扩展

当远端会话已启用 SSH Extension，用户执行一个交互式 SSH 命令时：

1. shell wrapper 识别该命令；
2. 使用当前主机的 OpenSSH 建立目标连接；
3. InfiniShell 检测目标平台并检查 remote-server；
4. 根据现有安装策略自动安装、询问或跳过；
5. 连接成功后，当前终端、文件能力、Agent 上下文和命令执行器切换到目标主机。

设置完成期间显示轻量状态：

```text
正在扩展 SSH：bastion -> staging
```

扩展失败时显示可恢复说明，但已经建立的交互式 SSH 会话继续工作。

### 3.2 路径展示

终端应显示当前访问路径：

```text
本地 > bastion > staging > database
```

传输型 `ProxyJump` 节点可以显示在详情中，但不作为独立的交互式工作主机，也不
要求安装 remote-server。

### 3.3 退出与恢复

在 `database` 执行 `exit` 后，当前能力恢复到 `staging`；继续退出则恢复到
`bastion`。退出一个子跳不得关闭仍在使用的父级会话或用户拥有的
ControlMaster。

### 3.4 保存访问路径

运行时发现的路径默认只保存在最近连接记录中。用户可以选择“保存访问路径”，
之后从 SSH Manager 直接连接最终目标。

保存路径只持久化主机引用、每跳别名、端口和连接策略；密码、passphrase、私钥
内容和 agent socket 不进入 SQLite 或云同步。打开保存路径时，第一跳若关联 SSH
Manager 节点，可以使用该节点在本地 Keychain 中的凭据；从第二跳开始只在已登录
的父级主机上执行 `ssh`，不把本地密码、私钥路径或 agent socket 传到父级。

## 4. 连接与身份语义

### 4.1 交互式跳与传输跳

- 用户进入主机后再执行 `ssh`：形成新的交互式连接节点。
- `ProxyJump` / `ProxyCommand`：由发起该连接的 OpenSSH 处理，属于一条连接边的
  内部传输细节。

InfiniShell 不重新实现完整的 `ssh_config` 解释器。需要展示解析信息时，在发起
该跳的主机上调用 `ssh -G`；实际连接仍由同一位置的 OpenSSH 完成。

### 4.2 主机身份

运行时主机身份优先使用 remote-server 返回的 `HostId` 和 SSH host key 指纹。
主机别名、hostname 与端口只作为连接信息，不能单独用于跨网络去重。

对于尚未取得稳定身份的连接，使用：

```text
(父路径标识, 目标别名, 用户, 端口)
```

作为临时身份。

## 5. 凭据与信任

1. 默认使用发起当前跳的 OpenSSH 凭据来源。
2. 本地保存的密码只能用于从本地发起且目标主机身份明确匹配的第一跳；不得跨越
   已登录主机注入到第二跳或更深跳点。
3. 未识别目标的密码、MFA 和硬件密钥提示保持交互式。
4. 不自动添加 `-A` 或 `ForwardAgent=yes`。
5. 新 host key 遵循发起主机的 OpenSSH 策略；发生 host key 变化时不得自动绕过。
6. 日志、遥测和错误报告不得包含命令全文、密码、私钥路径、agent socket 或
   `ssh_config` 内容。

## 6. 生命周期

### 6.1 父依赖

每个远端跳都依赖父跳。父跳断开时，其全部后代进入 `BlockedByParent`，不能各自
无限重连。父跳恢复后按从根到叶的顺序重建连接。

### 6.2 环路与深度

用户可以显式连接回路径中出现过的主机，但 InfiniShell 不得因此递归执行自动
恢复。第一版设置 8 层自动扩展上限；超过后保留普通 SSH 并显示说明。

### 6.3 并发

不同终端可以形成不同分支：

```text
local -> bastion -> staging
                  -> database
```

全局 manager 可以复用同一主机 daemon，但每个交互式会话、ControlMaster 所有权
和退出行为保持独立。

## 7. 分阶段交付

### 阶段一：可用的远端再 SSH

- 新增 `RecursiveSshExtension` Feature Flag；
- 支持带作用域的 SSH 控制引用；
- remote-server 协议支持一个父连接承载一个子连接字节流；
- bash、zsh、fish 远端 wrapper 支持再发起一个交互式 SSH；
- 子会话退出后恢复父级命令执行器；
- 所有失败路径回退到普通 SSH；
- 覆盖本地 -> A -> B。

### 阶段二：完整多级能力

- 任意多级连接图；
- 双向流控、半关闭、级联取消；
- 父依赖重连与拓扑顺序恢复；
- 深度和环路保护；
- 最近路径与保存路径；
- 第一跳凭据复用与后续跳凭据隔离；
- SFTP、项目批量执行、文件打开、Agent 与代码索引按当前跳路由；
- 覆盖本地 -> A -> B -> C。

两个阶段共享最终数据模型和协议；阶段一不得引入只能支持单跳的临时结构。

## 8. 平台范围

首个完整版本支持：

- 本地客户端：macOS、Linux；Windows 构建必须保持通过并安全禁用不支持路径；
- 远端：当前 remote-server 已支持的平台；
- shell：bash、zsh、fish。

PowerShell 远端 wrapper 和 Windows remote-server 作为后续平台扩展，不阻塞 POSIX
远端的完整多级能力。

## 9. 验收标准

1. 本地不能直接访问 B，但 A 可以访问 B 时，用户在 A 输入 `ssh B` 后，B 获得
   完整 remote-server 能力。
2. 本地 -> A -> B -> C 可以连续扩展，且 C 的命令、文件与 Agent 操作在 C 执行。
3. 从 C 逐级 `exit` 时，执行器和路径依次恢复为 B、A、本地。
4. A 断开时 B、C 不独立重连；A 恢复后按顺序恢复。
5. B 安装失败、平台不支持或协议不兼容时，交互式 B shell 仍可使用。
6. A、B、C 使用不同的 SSH 配置和凭据时，第一跳只读取本地凭据，后续跳分别读取
   A、B 上的 OpenSSH 凭据来源，不需要复制或转发私钥。
7. 不自动启用 agent forwarding，不跳过 host key 变化警告。
8. 同名内网主机不会仅因 `host:port` 相同而错误合并。
9. Linux focused tests、Windows 编译预检与 `cargo check` 通过。
