# ssh-machine-memory: 每台远端机器的 AI 记忆

技术实现细节见 `TECH.md`。

## 1. 问题

Zap 的重度使用场景之一是通过 SSH 运维远端服务器（legacy SSH 手敲 `ssh xxx`、
SSH Manager 面板、warpified SSH 三条路径）。当前 Agent 对远端机器的认知是
**一次性的**：

1. 每次会话 Agent 都要重新探测机器画像（发行版、服务路径、部署方式），
   浪费轮次，且探测命令本身有打扰生产机器的成本。
2. 上一次会话踩过的坑（如"这台机器的 nginx 是源码编译装在 /opt 下"、
   "systemctl 名字是 nginx-custom"）不会延续到下一次会话，同样的弯路会重复走。
3. 用户无法只表达目的（"去 web-01 重启 nginx"）——Agent 不知道 web-01 是谁、
   怎么连、上面有什么。

现有的记忆机制都不按机器划分：全局 Rules（`AIFact::Memory`）无作用域，
Project Rules（WARP.md/AGENTS.md）按本地目录作用域，`ssh_servers.notes`
是人工笔记且不注入 Agent 上下文。

## 2. 方案概述

为每台远端机器维护一份 **AI 维护的 Markdown 记忆**，形成闭环：

```
连接机器 → 注入该机记忆到 system prompt → Agent 带着经验干活
    ↑                                            ↓
  下次连接 ← 本地存储/同步 ← 会话结束后台复盘 + 会话中主动记录
```

分三阶段落地（详见 TECH.md 任务拆分）：

- **Phase 1（MVP）**：SQLite 新表 `ssh_machine_memories` + SSH 会话时把该机
  记忆注入 system prompt + 新 Agent 工具 `update_machine_memory` 让模型在会话中
  主动写记忆。不需要任何后台任务即形成"越用越懂这台机器"的闭环。
- **Phase 2（后台复盘）**：远端 shell 退出（`ExitShell`）时，若该 SSH 会话期间
  发生过 Agent 交互，后台发一次 oneshot LLM 调用：旧记忆 + 本次会话摘要 →
  合并后的新记忆，写回存储。
- **Phase 3（意图直达 + 同步 + UI）**：把"已知机器索引"（每台一行）注入本地
  会话，使"去 web-01 重启 nginx"可以直接关联目标机器；记忆随 `zap_sync`
  gist 加密同步；SSH Manager 面板可查看/清空记忆。

## 3. 关键决策记录

| # | 决策 | 理由 |
|---|------|------|
| A | **记忆存本地，不存远端机器** | 记忆最有用的时刻是连接之前（选机器、做计划）；Warp 注入本质是往 PTY 打脚本、不落文件，往生产机写 AI 记忆文件有污染/泄露/多人互覆盖问题；本地存储天然获得 zap_sync 加密同步 |
| B | **machine_key = 归一化的 `host:port`**（剥离 `user@` 前缀、小写、缺省端口 22） | 记忆属于机器而非账号；`InteractiveSshCommand.host` 是原始位置参数（可能含 `user@`、可能是 ssh_config 别名），别名情况通过存储 DCS 回报的真实 hostname 作为 `hostname_alias` 辅助归并（Phase 2+）。不用 `HostId`——legacy SSH 的 HostId 是每会话合成的，不稳定 |
| C | **与 `ssh_servers.notes` 分离**（新表，而非复用 notes 列） | notes 是人写给人看的，memory 是 AI 写给 AI 看的；生命周期、写入方、大小上限都不同 |
| D | **记忆格式为单份 Markdown 文档**（非结构化条目列表） | 与 Claude Code memory / WARP.md 同形态，LLM 读写皆友好；结构由复盘 prompt 引导（系统画像/服务与部署/操作惯例/踩坑记录），不在 schema 层强制 |
| E | **大小上限：存储 16 000 字符，注入 6 000 字符** | 防止无限膨胀；复盘 prompt 负责压缩合并（参考 byop_compaction 的做法） |
| F | **默认开启，可全局关闭**（新 AI 设置项，且尊重现有 memory 总开关） | 与现有 Rules 记忆的开关习惯一致 |
| G | **禁止把凭据写进记忆** | 复盘与工具的 prompt 明确要求不得记录密码/token/私钥内容；凭据已由 SSH Manager keychain 体系管理 |
| H | Phase 1 只覆盖 legacy SSH 会话（`is_legacy_ssh`），warpified SSH 通过 `hostname_alias` 在 Phase 2+ 归并 | legacy SSH 是运维主路径，`SessionContext` 已带 `ssh_connection_info`；warpified 路径的 host 来源不同（`SessionInfo.hostname`），分期降低风险 |

## 4. 用户体验

- 用户在 SSH 会话里正常与 Agent 协作，无新增交互负担。Agent 学到该机器的
  持久事实时调用 `update_machine_memory`（工具调用在 UI 中可见，与其他工具一致）。
- 退出 SSH 会话后，后台静默复盘（Phase 2），无 UI 打断；失败静默放弃，不重试。
- 下次连接同一台机器，Agent 开场即具备该机器画像，不再重复探测。
- Phase 3 后：在本地会话说"去 web-01 重启 nginx"，Agent 能从机器索引认出
  web-01（含其记忆摘要），指引或直接发起连接。
- SSH Manager 面板（Phase 3）：服务器详情中"记忆"区块，可查看、清空。

## 5. 非目标

- 不做记忆的远端存储或远端读取。
- 不做跨机器的记忆推理（"集群级"记忆）。
- 不改变现有 warpify / bootstrap 注入流程。
- 不在 Phase 1-2 提供记忆的手工编辑 UI（可通过清空重建；编辑 UI 视 Phase 3 反馈决定）。
