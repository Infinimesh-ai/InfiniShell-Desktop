# Grok 内部维护响应的安全形状诊断

本轮只为下一次 SDK 原生探针补充一个严格布尔诊断 `skills_reload_closed_success_shape`，没有接纳未知响应、维护请求或业务来源。生产仍以 invalid_response_id 拒绝字符串响应，不消费数值 pending，也不开放 SDK、权限或本地任务门禁。此前零输入原生探针的 FAILED 原样保留；它没有观测到维护响应，内层形状仍未得到真实验证。

Rust 专用 SDK fixture 在既有私有响应信封中计算该值。只有原始 frame 为精确三个字段 jsonrpc/id/result、JSON-RPC 2.0、固定维护 ID 精确匹配，result 闭合为单 result 键、其内层闭合为单 reloaded 键且 Value::as_u64 成功时才为 true。缺字段、method/error/额外外层或内层字段、伪造 session/generation、负数、浮点、布尔、字符串和超 u64 边界均不能满足。诊断不保存计数、内层结果或深层原文；唯一新增数据是布尔值。精确 ID 仍只留在既有受控私有文件。

既有当前用户 0700 目录、0600 普通文件、nlink=1、create_new/O_NOFOLLOW、单条首次捕获、64 KiB 信封和 128 个字段指纹预算保持不变。Python audit 要求新的七字段闭合 schema 与 strict bool；true 还必须与外层 ID、协议、单 result 字段的原始 UTF-8 指纹、object 类型及 error absent 相符。公开 metadata 只增加这一布尔值，保留文件长度/数量/SHA；空捕获不产生形状声明。公开投影将非布尔输入转为 null。

该 audit 只用于本轮新 SDK fixture 的证据。缺少新布尔字段的旧六字段信封会被新 audit 拒绝，不重算或改写 source27 原档案。内层值已在 Rust 丢弃，Python 无法仅凭摘要重建或独立验证内层；它依赖受控 fixture、身份与完整信封 SHA/字节账本。因此 true 仍只是该 frame 的形状诊断，不是 App-owned RPC、SDK origin、原生输入 ACK、工具完成、权限 ceiling 或任务成功证明。

实际离线验证：Python grok_sdk_origin_probe_runner_tests 一次运行 108 项全部 PASS（新增 7 项，0.645 秒），两个 Python 文件 py_compile PASS；所改 Rust 文件定向 rustfmt edition2024/skip_children=true PASS；三个源码文件 git diff --check PASS。新增 7 项 Rust 回归尚未执行，编译、原生 SDK11 与跨平台验收均未运行，不能计为 interface PASS。回归覆盖正/负闭合形状、u64 边界、深层原文不保留、生产未知 ID 拒绝/pending 不消费及首次捕获不被后来成功形状替换；Python 原有路径/权限、重复记录、64 KiB 与 128 字段预算回归继续真实通过。

没有运行 Cargo、原生 CLI、模型、GUI 或 Git mutation；没有读取认证、私有日志、native stdout/envelope 或模型正文。无需本地化变更：新增字段、异常和文档均为验收协议/开发诊断，不涉及用户界面。源码 SHA、全部新增测试名和验证边界见 [安全证据](validation/grok-internal-maintenance-shape-diagnostic.json)。后续需根任务统一冻结、编译，并以新真实 SDK11 证据判断该布尔值；本轮不推断实际原生 inner 值或成功。

STOP；finished_reads_done=true。
