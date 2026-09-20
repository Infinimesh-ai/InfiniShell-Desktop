# Grok 权限预检的缓存认证握手修复

本次修复仅补齐无模型预检探针的接口步骤。生产适配器在 `initialize` 后选择原生公布的 `cached_token`，发送固定 headless `authenticate`，再建立或加载会话；原探针跳过了这一步。该差异不能确定 [source30 policy3](OFFICIAL_30_GROK_POLICY_VERIFICATION.md) 的 `rpc_error` 根因，旧失败证据保持原样。

## 实际变更

- 新建与继续分别执行 `initialize → authenticate → session/new 或 session/load → x.ai/session/info → x.ai/session/state → x.ai/mcp/list → x.ai/debug/agent`，每阶段 7 次 RPC，总上限 14 次、2 个进程、0 次 prompt。
- 只有当前 `initialize` 响应唯一公布精确的 `cached_token` 才继续。缺失、重复、名称变体或格式错误均拒绝，不选择交互认证或 API key。认证参数固定为 `{"methodId":"cached_token","_meta":{"headless":true}}`，不带凭据、目录或历史会话。
- 认证响应仍按当前进程的待处理 RPC 关联；它不绑定原生会话，不消费 New/Load 的待处理请求，也不证明权限上限。新增闭合公共事件 `auth_method_selected` 仅包含代次、固定方法及公布/headless 布尔值。Python 审计核对初始化响应、选择、认证请求、认证响应和建立/加载请求的顺序。
- 网络仍只允许既有两个官方主机，上限 16 次 TLS 连接、8 MiB、360 秒。模式、配置来源、内置工具闭包和父任务权限上限继续为未知；本次没有修改产品能力判据。

## 验证与边界

`python3 -B -m unittest grok_policy_preflight_runner_tests` 最终真实运行 51 项，全部通过，内部耗时 0.066 秒，无跳过。新增 10 项 Python 回归覆盖两阶段顺序、严格事件投影、缺失/重复认证选择、错误响应、旧代与乱序响应及 14 次预算。首次运行有 1 项新增测试把拒绝代码预期写成 `response_before_request`；实际先按身份关联拒绝为 `response_uncorrelated`。仅修正该测试预期后重验通过，没有放宽实现。

新增 10 项 Rust 回归（该文件共 50 项）覆盖真实序列化请求构造、固定参数守卫、方法公布、会话绑定隔离、重复/旧响应、两阶段方法序列和预算。两份 Rust 文件通过 `rustfmt --edition 2024 --config skip_children=true`；本次未编译或执行 Rust 测试，需由后续统一冻结门禁验证。

四份授权源码通过 Python 语法解析及静态空白/冲突标记检查（Python 解析仅适用于两份 Python 文件）。差异检查使用改前公开源码与当前字节，不运行 Git 或修改索引。新增报告及 JSON 的六类凭据形状在原始字节与 JSON 解码字符串中均为 0；这不代表扫描用户配置或认证文件。

本次没有真实 CLI 复验、认证值读取、网络请求、Cargo、主程序构建、GUI 或跨平台验证，也未读取或修改 source31 冻结树和共享 target。四份新改动尚未按同一实际修改提交验证，不追认旧原生失败。无需本地化变更；本次没有产品可见文案变化。

四份源码的改前/改后 SHA、实际测试结果和静态检查记录见 [安全修复记录](validation/grok-policy-auth-handshake-fix.json)。
