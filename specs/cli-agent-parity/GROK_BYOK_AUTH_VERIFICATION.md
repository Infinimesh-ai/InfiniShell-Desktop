# Grok Build 的 API 认证边界

固定 Grok Build `1.0.30 (04b7ffed98c6)` 已验证每模型 `env_key` 配置可提供 ACP 的无交互 `xai.api_key` 认证。测试使用虚构值，并由 macOS 系统沙箱拒绝所有网络：`initialize` 公布该方法，`authenticate` 返回成功；去掉专用变量后只公布 `grok.com`，相同认证请求返回 `Authentication required`。两例均未创建会话、未请求模型、未产生 `auth.json`，标准输入关闭后正常退出。

[原生夹具](fixtures/grok-1.0.30-byok-authentication.json)保留固定二进制摘要与真实响应。适配器现在仅从原生公布的方法中选择 `cached_token` 或 `xai.api_key`，两者同时存在时维持已缓存登录优先；认证失败不改试交互登录，也不创建会话。应用不读取密钥，不代替用户修改 Grok 模型后端配置。缺少无交互认证时的中英文提示同步包含登录和 API 配置两种路径。

## 与模型及任务验收的区别

此认证证据不提升 Grok 的执行、审批、取消、恢复、附件或父子任务能力。Grok 官方模型的 402 额度限制没有因此消失；使用 Claude Messages 后端的原生 Grok Build 验证须单独记录，不能冒称 Grok 官方模型通过。

公开源码快照 [`482711333c7195dc16a272777f86086d615e2afb`](https://github.com/xai-org/grok-build/tree/482711333c7195dc16a272777f86086d615e2afb)与受测二进制不是同一构建提交，只作为辅助定位。源码显示每模型配置可以选择 Messages 后端和对应 `base_url`，但模型凭据还会进入进程共享凭据槽；断网认证成功不能证明完整任务的密钥出站边界。

因此真实 API 探针让原生 Grok 只持虚构密钥，由本机转发器向用户指定的唯一 HTTPS origin 注入实际凭据；原生进程受系统网络限制，转发器拒绝重定向和非 Messages 路径。该探针措施不构成产品自身具备相同隔离的证明。

[首轮极小模型探针](validation/macos-grok-byok-real-smoke.json)初始化、认证及创建会话成功；原生请求超过转发器的一次预算，唯一放行请求收到 HTTP 200，另一请求在本机被拒绝，主回合未产出结果。整轮未通过，不能将这一 HTTP 响应当模型回合成功。

[第二轮](validation/macos-grok-byok-real-smoke-2.json)在最多三次请求、每次不超过 4096 输出 token 的预算内通过：两次 Messages 请求分别为初始标题和主回合的请求形态（100/512 token 上限），主回合返回精确 `INFINISHELL_GROK_BYOK_OK` 及 ACP `stopReason=end_turn`。主回合记录输入 14817、输出 17 token，不包含辅助标题请求。实际没有工具执行或审批回调，标准输入关闭后原生退出码为 0，转发器已停止。

这证明固定 Grok Build 在该 Claude 后端下的一次真实原生请求链路。没有经过产品 leader 或生产适配器，不覆盖两轮、审批、取消、恢复或 GUI。断网认证、失败尝试、成功尝试和源码摘要分别保留在[证据清单](validation/macos-grok-byok-evidence-manifest.json)，未覆盖的能力继续关闭。

## 原生审批、取消与历史恢复

后续验证仍使用固定二进制、虚构原生密钥及受限转发器，在私有项目与 Grok 历史目录中执行。没有修改用户全局设置，原生进程均正常退出，转发器已停止。

[审批复验](validation/macos-grok-byok-approval-native-approval-2.json)完成两轮精确回执、单次 Write 允许和拒绝，并核对真实文件效果；[首次严格校验器失败](validation/macos-grok-byok-approval-native-approval.json)同时保留。两次合计 11 次 Messages 请求。审批必须同时核对原生 request ID、sessionId、toolCallId、optionId 与 kind；`enable-always-approve` 可能也声明 `allow_once`，不能只选第一个同 kind 的选项。实际选择 `allow-once/allow_once` 或 `reject-once/reject_once`。拒绝最终为 `cancelled/PermissionRejected`，不计成功。

[取消与恢复](validation/macos-grok-byok-recovery-native-recovery.json)共 4 次 Messages 请求：真实非空输出后发无 ID 的 `session/cancel` 通知，等待原 prompt 返回 `cancelled/MidTurnAbort`；同进程下一轮回收随机标记。随后 `session/close` 返回 closed、EOF 退出 0，新进程仅执行一次 `session/load`，按原 native session ID 加载私有历史，再次精确回收此前标记。new=1、load=1，没有新建替代或复用未退出进程。0 工具事件与审批回调。

这些新增证据仅覆盖 **Grok Build + 自定义 Claude 后端的原生 no-leader ACP**。实际验证的是 `session/load`，没有验证 `session/resume`；也不证明产品 leader、GUI、持久化与父子任务、运行中追加或官方 Grok 模型。产品执行门禁保持关闭，已确认的协议进入独立适配实现与后续真实生产验证。
