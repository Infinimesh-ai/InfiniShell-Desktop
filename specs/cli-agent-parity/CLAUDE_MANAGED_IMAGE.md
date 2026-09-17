# Claude 生产 PNG 适配器隔离验收准备

本夹具及生产 PNG 接入已通过 source17/18 Cargo 与定向门禁。source18 首轮真实完成随机图片识色与中文多行两轮、原生回放与完整结果关联；第一代正常关闭得到 stop_requested 收据，整轮 FAILED，第三轮恢复未执行。source19 正在修复并复验正常关闭，不宣称图片 GUI、SQLite 或跨平台通过。原始失败和清理证据见 [source18 记录](SOURCE18_NATIVE_FAILURES.md)。source14 的裸原生校准仍单列，不能替代生产适配器验收。

source20 第2轮已实际取得两轮完整结果、首代 `stdio_closed`/退出码0/资源清理，并恢复磁盘 PNG 引用与哈希；恢复连接 initialize 就绪暂无原生会话ID，夹具过早要求确认导致 `resume_ready_identity_failed`，第三轮未执行，整轮仍 FAILED。[原始记录](SOURCE20_NATIVE_DISCOVERY.md)保留这一边界。后续候选分别记录 requested 历史ID、实际 native ID 和 confirmed 标记，显式第三输入的真实 ACK/Started/result 均须确认原历史ID，不将输入的历史ID伪作原生输出。

## 路径与验收范围

测试入口为 `ai::cli_agent_runtime::claude::live_tests::managed_image_live_tests::real_claude_managed_png_lifecycle`，scope 为 `claude_managed_png_process_resume`。父 `claude_live_tests.rs` 已注册 `managed_image_live_tests`；夹具复用父模块的生产 `LiveSession::start(connect)`、权限检查、事件读取、正常关闭及监督器清理链。

图片由现有 `quadrant_png` 以随机 UUID 生成：64×64、四种实色、12,420 字节 PNG。颜色排列只存在于像素和夹具内存中，首轮提示只要求按左上、右上、左下、右下识别，第三轮提示只要求回忆原图片；两者均不提供排列答案。生产 `prepare_managed_input(Harness::Claude, ...)` 保存哈希命名图片到隔离根的 `local-cli-attachments/`，并返回 `Text` 与 `LocalImage` 类型化输入。夹具不会自行构造或发送原生 user JSON。

固定最多三次唯一原生输入，第一连接执行前两轮，恢复连接执行第三轮：

1. 提交 PNG 与固定英文识色提示，随后以同一消息 UUID 和完全相同的 `InputContent` 重投。真实原生回放、开始与结果账本必须仍只有这一轮。
2. 提交三行中文文本，完成后正常关闭第一连接。
3. 从隔离磁盘检查点读取原类型化附件引用，通过生产 `restore_managed_images` 读取并验证 PNG，再重建完全相同的类型化输入。此步骤不向模型重投图片。用新的 runtime generation 与同一 native session ID `Resume`，ready 后显式提交纯文本回忆问题。

这验证生产适配器、哈希附件持久引用读取及原生进程继续历史会话。它不验证 SQLite 任务恢复、应用完整重启、活跃进程重新关联、父子任务、权限上限、GUI 交互、非 PNG 格式或其他平台；相关报告字段始终为 `false`。HTTP 模型请求数不可见，三次预算只指唯一原生 user 输入。

## 严格成功条件

每轮必须取得同 generation、同 native session、同输入 UUID 的 `MessageAccepted`、唯一 `TurnStarted` 及 `TurnFinished(Completed)`。最终输出去掉首尾空白后必须精确匹配夹具预期；完整输出 SHA256 与字节数保留，不能用子串、前后缀或裁剪正文代替验收。原生账本另外要求恰好三条唯一 user 回放、三条 started、三条单输入 UUID 的成功 result，各输入至少一条同来源 assistant，原生 result 的完整文本 SHA256 必须等于运行时完整结果 SHA256。

生产适配器允许同 ID 重投重放已缓存的原生 ACK，出站写入仍为空。夹具最多接受一次精确匹配原 PNG UUID、turn、generation 和 native session 的缓存回执，单列 `cached_native_ack_replayed`，`receipt_source=CachedNativeProtocol` 且 `native_input_added=false`。它可在 image 或 multiline 读取阶段到达；恢复连接、错 ID、错 session、错 turn、第二次缓存回执均拒绝。缓存回执不增加三条真实原生输入或三条独立 ACK 计数，也不能掩盖重复原生 user、started 或 result。

两代连接都必须正常 `shutdown`，生产连接任务返回成功，再独立读取同 generation 的监督器退出收据：`version=1`、`exit_code=0`、`exit_reason=stdio_closed`、`cleanup_confirmed=true`。非零、空退出码、`host_disconnected`、`stop_requested`、连接返回错误或清理未确认均不通过。任何实际工具或权限请求都明确拒绝并使本验收失败；隔离项目权限为 `deny:["*"]`，不开放全局审批绕过或新的权限证明。

## 安全证据契约

专用运行器仅为子进程环境添加 `INFINISHELL_CLAUDE_MANAGED_IMAGE_TRACE=1` 和固定预算标记。生产实际读取处的 `CLAUDE_NATIVE_PROTOCOL_IDS` 使用连接 `options.generation`，不借当前输入补来源；默认其他运行器保持原 trace 字段不变。

专用 trace 追加 `runtime_generation`、`image_content_projection` 和 `result_text_sha256`。图像投影仅来自实际 stdout user 数组，包含 `array_sha256`、`text_sha256`、`text_bytes`、`image_sha256`、`image_bytes`、`block_types:[text,image]`、`media_type:image/png`。其他帧的图像投影为 null；结果 SHA 仅来自真实 result 字符串。期望数组 SHA 通过生产 `encode_input` 的 `Blocks` 计算，运行器与原生回放精确比较，正确格式但不同 SHA 仍失败。

公开 NDJSON 只接收固定事件和安全协议投影；不保存 base64、原生帧、图片正文、思考、任意工具名、环境、账号身份或凭据。生产类型化检查点保留在隔离私有根，不复制到公开报告。stdout 产物仅保存基础运行器脱敏后字节的 SHA、大小、摘要计数及凭据形态计数；异常路径也压缩本轮新建的 stdout，已存在的产物由既有路径校验拒绝且不会覆盖。凭据形态扫描只报告计数，任何命中均使本次成功结论降级。

## 准备中的预算与 EOF 处理

根代理生产改动的候选预算为文本不超过 1 MiB、完整图片出站帧不超过 `8 MiB − 64 KiB`，为原生回放附加字段保留空间；实际入站帧仍限制为 8 MiB。准备层与后端均检查预算，PNG 与技能混用拒绝。这些代码约束尚待对应提交门禁和真实生产验收，不属于已观察的 CLI 图片上限。

生产收尾在关闭 stdin 后并行等待 `child.finish` 与 stdout EOF drain，候选时限为 30 秒、总读取预算 `8 MiB + 1`；原始传输错误保留，不能只因清理收据正常而判成功。本夹具必须同时满足完整协议结果、连接成功返回与正常退出收据，不将 source13 图片校准旧失败追认为成功。

## 运行与当前验证记录

运行器复用 `run_claude_image_probe.run` 的私有 API 隔离方案和 `run_claude_adapter_live.run` 的固定版本、生产监督器链；不分叉凭据读取或认证实现。已有版本校验要求 `2.1.273 (Claude Code)`。参数路径必须显式指定；API 环境文件必须仅当前用户可读写，文件内容不要放入命令或报告。

```sh
python3 script/cli-agent-parity/run_claude_managed_image_live.py \
  --test-binary "$TEST_BINARY" \
  --claude "$FIXED_CLAUDE_BINARY" \
  --supervisor "$PRODUCTION_SUPERVISOR_BINARY" \
  --api-environment-file "$PRIVATE_API_ENV_FILE" \
  --model "$FIXED_NATIVE_MODEL" \
  --output "$PUBLIC_EVIDENCE_FILE" \
  --max-native-inputs 3 --timeout-seconds 900
```

上述命令是按已有在线/API 授权执行的入口示例，本文准备阶段未调用它。Rust 夹具整体时限 850 秒，单轮时限 180 秒，外层运行器固定 900 秒；不允许增加原生输入预算换取通过。

离线回归命令为 `PYTHONDONTWRITEBYTECODE=1 python3 script/cli-agent-parity/claude_managed_image_runner_tests.py`。目前 17 组通过，包含错图片/文本/数组 SHA、错 session/输入 UUID、额外原生帧、旧 generation、恢复重投、两代异常清理、缓存 ACK 身份和次数、凭据链恢复及异常 stdout 归档负例。Rust 仅完成指定文件格式化；尚未编译或实际运行。夹具和报告均不新增产品可见文案，无需本地化变更；根代理产品入口的中英文及布局门禁仍需独立完成。

待完成生产 PNG 前置能力门禁、冻结同一源码和监督器、Cargo/本地化门禁，再真实执行并独立归档其源码清单、固定 CLI 与二进制 SHA、三轮协议账本、附件 SHA 及两代退出收据。只有这些条件全部通过，才可报告本隔离生产路径通过；其他产品能力仍按各自验收证据判断。
