# Grok 内部技能维护响应兼容修复

source31 的真实 SDK11 仍为 FAILED：一次原生输入，Rust 测试 exit 101，runner/driver exit 1，耗时 16.715 秒；仅完成一次 discovery 与 tools/list，inspect 为 0，来源和产品门禁均 false。新的严格诊断已真实观察到 `skills_reload_closed_success_shape=true`，私有信封范围与原生账本对应校验均 true。只读公共 metadata 即可核对该结论，本次没有读取私有 envelope、原生帧、错误正文或认证正文。

这次诊断确认固定 Grok 1.0.30（04b7ffed98c6，CLI SHA256 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`）确实转发一个固定 `skills-reload` string ID 的内部维护成功响应。它的外层只含 `jsonrpc/id/result`，JSON-RPC 2.0，外层 result 只含 result，该内层只含 `reloaded:u64`，没有 method、error 或额外字段。原生 reloaded 值没有回显或归档；计数不表示用户回合或工具执行完成。

公开候选源码已定位 watcher 的内部请求写入共享 ACP 输入并由 stdout 返回：[agent/app.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/app.rs#L152-L258)。[维护 handler](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/extensions/session_admin.rs#L607-L615) 的 ExtMethodResult.success 是该双层结构；[agent_ops.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/agent/mvp_agent/agent_ops.rs#L72-L79) 计数的是 resident session 的异步派发。候选与已发布二进制的完整构建来源仍未证明；实际内层形状依据新 SDK11 的严格布尔证据，而非候选推测。

生产修复仅改 grok.rs 与 grok_tests.rs：在现有 JSON-RPC、身份预算及 method 分支后、数值响应 ID 解析前，严格识别上述闭合成功结构并返回空 Effects。它不消费 pending、不建立数值响应缓存、不刷新超时、不输出生命周期、工具或 SDK 业务事件，也不提升权限、ceiling 或能力。重复合法维护响应仍为空操作；其他 string ID、error、额外身份字段、缺失包装或非 u64 值仍进入既有拒绝路径。method 与 result 混合的异常帧保持既有 server-message 拒绝规则，未修改审批与客户端工具路由。

新增 6 项回归覆盖 u64 两端、重复维护期间认证 pending 不变、运行中 prompt 无 ACK/终态/SDK 计数、非整数和溢出、三个对象层的额外 S/P/T/代际字段以及混合帧。定向 rustfmt（edition 2024、skip_children）与静态 diffcheck 已通过；尚未运行 Cargo、Rust 测试或修复后的原生 probe，由根任务在下一候选统一验证。本轮是协议兼容修复，无需本地化变更；不能据此补记 SDK 来源、子任务权限或三方完整验收 PASS。

SDK11 原执行器把 UID 写死 501，在实际 UID 502 的前置阶段失败，未创建执行标记、未启动 CLI、未读取认证正文；该失败保留。根任务另建 current-UID 等值守卫的独立执行器，明确授权后执行一次真实 SDK11；本次只核其公共身份报告，不冒称独立重跑。111 输入、exact 7M、lib `c4c1c54f805ebc8dbdd745a0702bd883960ac28d1c9efdb609cb785af6037ca0` 与 main `5d0320978e1f8df4f88e8ee20fd78c3f925c31ec42aa0ac67d36f9f61db3bfa2` 前后不变。隧道停止与认证副本移除均 true，postcheck 成功；这些清理证据不把 test 101 改成成功退出。

SDK11 的对象仍是父提交 `0059ef1bf8438c5af7545c3101b880ec2d0b10e9` 的 source31 未提交候选，same_commit=false；本次两源码修复尚不在该二进制中。原 FAILED 档案没有改写。六类凭据形状扫描覆盖所读公共 JSON 的序列化与解码字符串、代码和本报告，均 0 命中。新档案只保存安全摘要和 SHA，不归档私有内容。

- [证据与变更范围](validation/grok-internal-maintenance-compatibility-fix.json)
