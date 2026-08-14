# 配置 BYOP Responses API

[English](./ai-provider-responses.md)

本文面向希望在 InfiniShell 中连接 OpenAI Responses API 或兼容网关的用户。
它说明设置页里的每个选项会改变什么、哪些组合可以一起使用,以及遇到常见
`400 Bad Request` 时应该先检查哪里。

> 第三方网关可能只实现 Responses API 的一部分。除非提供商明确声明支持某项
> 高级能力,建议先使用本文的兼容性默认配置,确认基本对话和工具调用正常后再
> 逐项开启。

## 打开提供商设置

打开 **设置 → AI → 提供商**,添加或展开一个提供商。按钮前的 `●` 表示当前
选中。

对于支持 `/v1/responses` 的 OpenAI 兼容端点,建议从下面这组配置开始:

| 选项 | 推荐值 |
|---|---|
| API 协议 | `OpenAI-Response` |
| Responses 状态 | `本地 / ZDR` |
| 传输 | `HTTP + SSE` |
| 自动上下文压缩 | `Off` |
| Pro 模式 | 关闭 |
| 跨轮推理上下文 | 关闭,使用提供商默认值 |
| 后台运行与断点恢复 | 关闭 |
| 程序化工具调用 | 关闭 |
| Extra Headers | 提供商未要求时留空 |

这套配置优先保证隐私和兼容性。高级开关不是“打开越多越强”;每个开关都要求
服务端实现相应的 Responses 协议。

## API 协议

API 协议决定 InfiniShell 使用哪个请求格式和响应解析器。它描述的是端点实际
接受的协议,不只是模型来自哪家公司。

| 选项 | 主要端点 | 适用场景 |
|---|---|---|
| `OpenAI` | `/v1/chat/completions` | 普通 OpenAI Chat Completions 兼容服务 |
| `OpenAI-Response` | `/v1/responses` | Responses、多轮推理和高级 Agent 能力 |
| `Gemini` | Gemini 原生协议 | 直接连接 Google Gemini |
| `Anthropic` | `/v1/messages` | 直接连接 Anthropic Messages API |
| `Ollama` | Ollama 原生协议 | 本机或自建 Ollama |
| `DeepSeek` | DeepSeek 原生协议 | 需要正确回放 `reasoning_content` 的 DeepSeek 推理模型 |

如果提供商只宣称“OpenAI compatible”而没有提到 Responses,先选 `OpenAI`。
只有在提供商明确支持 `/v1/responses`,或者实际测试成功后,才选
`OpenAI-Response`。

Base URL 通常应停在 API 版本目录,例如:

```text
https://provider.example/v1/
```

不要把 `/responses` 或 `/chat/completions` 手动追加到 Base URL;InfiniShell 会按
所选协议补全具体端点。API 密钥保存在系统密钥库中,请求时会发送给你配置的
提供商端点。不要在 Issue、日志或截图中暴露完整密钥。

## Responses 状态

Responses 状态决定多轮历史由本地客户端还是提供商保存。

| 模式 | InfiniShell 的行为 | 优点 | 注意事项 |
|---|---|---|---|
| `本地 / ZDR` | 发送 `store:false`,由客户端回放必要的消息、推理和工具 item | 隐私边界清晰,对兼容网关要求最低 | 会话变长后,每轮请求体会增大 |
| `提供商响应链` | 发送 `store:true`,后续轮次使用 `previous_response_id` | 每轮只需发送新增输入,长链更高效 | 提供商会保存响应状态,并且必须正确实现响应链 |
| `云会话` | 创建 Conversations 对象并持续使用 conversation ID | 适合明确需要服务端长期会话的应用 | 要求提供商实现 `/v1/conversations`,数据保留边界最大 |

选择建议:

- 隐私优先、第三方网关或不确定服务端能力:使用 `本地 / ZDR`。
- OpenAI 官方端点或已验证完整实现响应链的服务:可使用 `提供商响应链`。
- 只有明确需要服务端持久会话,并确认 Conversations API 可用时,才使用
  `云会话`。

`本地 / ZDR` 表示 InfiniShell 使用 `store:false` 并在本地管理会话状态。它不能
替代提供商自己的隐私政策,也不能保证第三方网关不保留访问日志。OpenAI 官方
关于三种状态管理方式的说明见
[Conversation state](https://developers.openai.com/api/docs/guides/conversation-state)。

## 传输

### HTTP + SSE

每轮通过 HTTP 创建 Response,再通过 SSE 流式接收事件。这是默认选择,也最容易
穿过反向代理、CDN 和企业网关。后台运行只能和此传输组合。

### WebSocket

通过持久 WebSocket 连接执行多轮 `response.create`,适合包含大量模型与工具往返
的长链 Agent 工作流。它减少的是续接和传输开销,不会让模型本身的推理速度
自动变快。

使用前应确认提供商和中间网关支持 Responses WebSocket。当前 InfiniShell 不会
在 WebSocket 模式发送 `background`;需要后台运行时请改回 `HTTP + SSE`。官方
协议说明见
[WebSocket Mode](https://developers.openai.com/api/docs/guides/websocket-mode)。

## 自动上下文压缩

该设置会把 Responses 请求的 `context_management.compact_threshold` 设为
`32k`、`64k` 或 `128k`。当渲染后的输入 token 数越过阈值时,服务端把较早状态
压缩成一个可供后续轮次继续使用的 compaction item。

| 选项 | 行为 |
|---|---|
| `Off` | 不发送自动压缩配置 |
| `32k` | 较早压缩,更积极控制请求大小 |
| `64k` | 在上下文保真和请求大小之间折中 |
| `128k` | 较晚压缩,保留更多原始上下文 |

该阈值不是模型的 Context 或 Output 上限。模型列表里的 Context/Output 描述模型
容量;这里描述何时开始压缩历史。

第三方网关未声明支持 compaction 时保持 `Off`。长会话出现上下文超限后,可以先
测试 `128k`,再根据模型容量和任务特点考虑 `64k`。官方说明见
[Compaction](https://developers.openai.com/api/docs/guides/compaction)。

## GPT-5.6 推理

### Pro 模式

开启后发送 `reasoning.mode: "pro"`。它让 GPT-5.6 为困难任务投入更多模型工作,
通常意味着更高的延迟和 token 消耗。适合复杂优化、高价值代码审查或深度分析;
日常对话和延迟敏感任务应保持关闭。

Pro 模式与 reasoning effort 是两个独立设置。开启 Pro 不会自动把 effort 调到最高。

### 跨轮推理上下文

开启后显式发送 `reasoning.context: "all_turns"`,让模型在目标、假设和优先级持续
稳定的多轮任务中复用可用推理 item。

关闭这个按钮表示 InfiniShell 不显式覆盖 `reasoning.context`,而不是关闭推理。
OpenAI 官方 GPT-5.6 当前默认使用 `all_turns`;第三方兼容端点可能采用不同默认值。
使用 `本地 / ZDR` 时,InfiniShell 会回放服务端返回的加密推理 item。

GPT-5.6 的推理模式和默认值以
[OpenAI Model guidance](https://developers.openai.com/api/docs/guides/latest-model)
为准。

## Agent 能力

### 后台运行与断点恢复

后台模式用于可能持续数分钟的任务。服务端异步执行 Response,客户端可以继续查询
状态并恢复事件流。

InfiniShell 只允许下面的组合:

```text
提供商响应链 或 云会话
+
HTTP + SSE
```

`本地 / ZDR` 或 WebSocket 模式下按钮不可用。后台执行需要服务端暂存状态,因此会
改变数据保留边界。官方说明见
[Background mode](https://developers.openai.com/api/docs/guides/background)。

### 程序化工具调用

Programmatic Tool Calling(PTC)允许模型生成一段受控 JavaScript,在托管运行环境
中调用被授权的工具并处理多个中间结果。它适合筛选、连接、排序、去重、聚合或
验证等控制流可预测的步骤,不是普通函数调用的必需项。

服务端必须同时支持 `programmatic_tool_calling`、`allowed_callers`、`program` 和
`program_output` 等协议元素。没有明确支持时请保持关闭。官方说明见
[Programmatic Tool Calling](https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling)。

## 常用配置组合

### 隐私与兼容性优先

```text
OpenAI-Response
本地 / ZDR
HTTP + SSE
其余高级选项关闭
```

适合第三方兼容网关,也是新提供商的推荐起点。

### 工具调用密集的长链 Agent

```text
OpenAI-Response
提供商响应链
WebSocket
按需开启跨轮推理上下文或程序化工具调用
```

适合已经验证 WebSocket 和响应链支持的服务。该组合不能使用后台模式。

### 超长后台任务

```text
OpenAI-Response
提供商响应链或云会话
HTTP + SSE
后台运行与断点恢复
按需开启 Pro 和自动压缩
```

适合服务端需要在客户端断线后继续执行的困难任务。

## 常见错误

| 错误 | 常见原因 | 处理方法 |
|---|---|---|
| `404` 或 `/responses` 不存在 | 提供商只实现 Chat Completions | 把 API 协议改为 `OpenAI` |
| `Unsupported tool type: programmatic_tool_calling` | 提供商不支持 PTC | 关闭“程序化工具调用”;升级 InfiniShell 以获得兼容回退 |
| `Invalid schema for function ... required ... Missing ...` | 提供商执行严格工具 schema 校验 | 升级到包含 Responses strict schema 修复的 InfiniShell;仍失败时向提供商报告其 schema 要求 |
| `/conversations` 返回 `404` 或 `400` | 提供商未实现 Conversations API | 改用 `本地 / ZDR` 或 `提供商响应链` |
| WebSocket 握手失败或连接频繁断开 | 提供商、代理或网络不支持长连接 | 改回 `HTTP + SSE` |
| Background 按钮不可用 | 当前使用本地状态或 WebSocket | 选择云状态并使用 `HTTP + SSE` |
| `context_management` 或 `compact_threshold` 不受支持 | 提供商未实现服务端压缩 | 把自动上下文压缩设为 `Off` |

排查时可以先恢复本文开头的推荐配置,确认基本对话和普通工具调用正常,然后每次只
开启一个高级选项。分享日志前请移除 API 密钥、Authorization header、用户输入和
其他敏感内容。
