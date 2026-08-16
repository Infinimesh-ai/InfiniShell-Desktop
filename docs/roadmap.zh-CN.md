# InfiniShell Desktop 路线图

InfiniShell Desktop 是 [InfiniShell 产品线](https://infinishell.dev/#lineup)中的
开源桌面软件：面向个人开发者与终端用户，在 Linux、macOS 和 Windows 上提供
本地优先的 AI 终端。用户可以接入自己的模型、CLI Agent 与 SSH 主机，密钥、历史
和 Agent 状态默认保留在自己的机器上。

本路线图只描述 **InfiniShell Desktop**。它不再用 Phase 把整个产品线写成一个
共享运行时的演进过程；具体功能的交付状态以代码、发布说明和对应 spec 为准。

## 产品线边界

三条产品线共享“目标理解、受控执行、经验沉淀”的 AI Native 运维理念，但它们是
边界清晰的不同产品，不要求共用同一套客户端、账号或 Agent 运行时。

| 产品 | 形态与用户 | 与本路线图的关系 |
|---|---|---|
| **InfiniShell Desktop** | 面向个人开发者与终端用户的开源桌面软件 | 本仓库、本路线图的范围 |
| **InfiniShell Box** | 面向个人运维与小团队的本地 AI 运维盒子 | 独立产品，不在本仓库排期 |
| **InfiniShell Enterprise** | 面向运维、安全、DevOps / SRE 团队的私有化企业中台 | 由独立项目承接 Web、多用户、权限、审计、知识库与协同能力 |

产品线之间可以复用经过验证的概念、协议或组件，但这种复用不能破坏 Desktop 的
本地优先、无需账号、独立可用和开放接入边界。

## 持续方向

### 1. 开放、本地的 Agent 体验

- 持续完善 BYOP 协议兼容性，覆盖流式输出、工具调用、推理、多模态、上下文压缩
  与长任务恢复。
- 让内置 Agent、第三方 CLI Agent、MCP 与 Skills 在 Blocks、审批、通知和会话
  生命周期中拥有一致体验。
- 保持系统提示词、工具权限、命令白名单与会话持久化可见、可配置、可审计。
- 不绑定单一模型、提供商或 InfiniShell 托管网关。

### 2. 个人与项目级 SSH 运维

- 稳定 macOS、Linux、Windows 客户端连接 POSIX 与 Windows PowerShell 远端的
  SSH、shell integration 和 remote-server 链路，并保留复杂 OpenSSH 配置的
  安全回退。
- 完善 SSH 主机管理、项目清单、仓库与规则上下文，让 Agent 能可靠理解目标机器。
- 强化单机与跨主机执行的审批、金丝雀、超时、结果聚合和失败边界。
- 逐步完善完全本地的项目与机器记忆，让历史经验可以复用但始终由用户掌控。

### 3. 本地优先与隐私收口

- 继续清理从上游继承的账号、团队、计费、分享、Drive 同步、云端会话、遥测和
  错误上报残留。
- 收敛本地 SQLite、系统密钥链和配置文件的存储边界，确保数据可理解、可迁移、
  可恢复。
- 审计网络访问路径；除用户明确配置的模型提供商、MCP、更新和远端连接外，不
  引入隐式外发。

### 4. 跨平台终端质量

- 维护三平台的构建、测试、打包与发布门禁，优先修复终端、PTY、shell integration、
  输入法和远程会话中的平台差异。
- 持续改善中英文界面、CJK 字体与文本布局、Markdown / 代码渲染、键盘操作、性能
  和可访问性。
- 选择性吸收 Warp 上游的终端内核改进，并用回归验证防止已剥离的云端依赖重新
  进入 Desktop。

### 5. 产品线协同但不混同

- 在主机、项目、Skills、MCP、审批和执行结果等领域保持可理解的共同语义，降低
  用户在 Desktop、Box 与 Enterprise 之间迁移认知的成本。
- 只有在需求和安全边界明确时才设计互通协议；不以“未来可能互通”为理由给
  Desktop 引入服务端依赖。
- Enterprise 的企业治理实践可以反哺 Desktop 的本地安全设计，但多租户、集中式
  控制面和组织权限仍留在 Enterprise。

## 本仓库的当前非目标

以下能力可能属于 InfiniShell 的其他产品线，但不属于 InfiniShell-Desktop 的当前
路线图：

- 把 Desktop 的 Agent 抽离成必须服务多个客户端的通用 Harness，或以此为前置条件
  重写现有 Rust Agent 栈；
- 在本仓库建设 Web 终端、IDE 客户端、企业控制台或统一云账号；
- 提供托管 Agent 集群、容器 / VM 沙箱调度或 Kubernetes 控制面；
- 实现企业多用户协作、RBAC、集中审计、组织级分享链接和统一任务调度；
- 建设 Slack、Discord、Telegram、Issue tracker 等企业入站渠道；
- 为产品线互通重新引入强制登录、云同步或 Desktop 必须依赖的中心服务。

这些内容若需跨产品协作，应由对应产品的 spec 定义边界、协议与安全模型，而不是
作为 Desktop Roadmap 的后续 Phase 隐含承诺。

---

[English](./roadmap.md)
