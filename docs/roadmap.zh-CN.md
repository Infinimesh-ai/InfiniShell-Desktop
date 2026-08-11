# 后续计划

InfiniShell 的 Agent 能力将以独立开源服务的形式实现,不绑定 Warp 客户端。终端只是它的其中一个载体 —— TUI、IDE 插件、云端 worker 都可以驱动同一个引擎。

## Phase 1 — 自研 Agent Harness Core

- 独立设计并从零实现一个开源 Agent 服务—— 涵盖 Agent 循环、工具运行时、会话 / 历史状态、提示词模板、提供商路由;不绑定 Warp 现有客户端代码。InfiniShell 作为它的首个官方载体。
- 定义稳定的 IPC / JSON-RPC 协议:提示词、流式 Token、工具调用、文件 diff、状态、附件。
- Harness 以可复用的开源服务形式发布,headless 守护进程、独立 TUI、IDE 插件、其它终端都能接入。
- 默认纯本地运行;凭证、历史、Skills 与 MCP 服务器配置均保留在本地。
- 协议带版本号与能力协商,客户端与 Harness 可独立升级。
- 工具注册表可插拔:内置 shell / read / edit / search 等工具,并允许用户通过统一 RPC 接口提供外部工具。

## Phase 2 — 托管 Agent 运行时

- 同一份 Harness 可在服务器端运行,接受任意客户端下发任务。
- 异步任务委派:下发长耗时任务、跟踪进度、随时返回查看结果。
- 每个任务一个隔离沙箱(容器 / VM),支持预安装工具链与 setup 脚本。
- 仓库感知执行:clone、开分支、跑测试、lint、类型检查;输出可证的终端日志与测试结果。
- Git 工作流集成:创建分支、提交与 PR,diff 与日志可引用追溯。
- 任务级密钥与网络策略:默认无出网,需要时显式 allowlist。
- 多任务并发,带配额、调度与取消机制。
- 仓库 / 组织 / 项目级记忆文件(`AGENTS.md` 或等价物)跨次运行生效。
- 完全可自托管:单节点 Docker、多节点集群、或自带 Kubernetes部署都可,不依赖任何 SaaS。

## Phase 3 — 多载体协作

- 在 InfiniShell 终端、独立 TUI、IDE 插件与 Web UI 间共享同一身份 / 账号。
- 会话接力:Web 开启任务,终端里接着干;或者把终端会话交给桌面端评审。
- 后台 Agent 与多 Agent 团队:Lead Agent 拆解任务,分发给并发的子 Agent。
- Routines:定时 / API 调用 / 仓库事件 / CI / Issue tracker 事件触发任务。
- 入站渠道:从 Slack / Discord / Telegram / Webhook 推任务进 Harness。
- 出站集成:GitHub / GitLab / Gitea、Issue tracker、CI、MCP 服务器、代码评审。
- 远程运行可观测:实时日志、中间 diff、运行中介入与取消。
- 会话 / 任务可分享链接,带权限范围,供团队评审。
- 端到端开源:Harness、沙箱运行时、Web UI、集成全部可自托管。

> 本路线图仅代表当前探索方向,会随实际落地与社区反馈调整。

---

[English](./roadmap.md) · [日本語](./roadmap.ja.md)
