# source27 真实原生诊断复验

本轮在独立 detached 工作树 `.worktrees/cli-agent-parity-validation-27` 实际构建并各执行一次 Grok policy2 和 SDK10。父提交为 `436cc739234061c092cedd106432bbbfa3c2d645`，但候选含 9 个未提交诊断契约修改，其余 102 项清单内容保持父提交。111 项源码资源清单 SHA-256 为 `730a9aa633d9565eed15bc85ec4bfdbff45377e411f97ab6a23544c6326aaa8e`。这是未提交候选复验，`same_commit_verified=false`；不能计为包含实际修改的新提交、跨平台、完整三方链或 Goal 通过。source26 的失败不回填。

安全公开产物保存在 [official-27-native](validation/official-27-native/)。不归档认证、私有日志、原始模型或工具正文、私有响应信封及其精确 ID。本轮未操作 GUI，也未修改产品源码、冻结树、权限、采样参数或生产 pending ID 规则。

## 构建与独立身份先验

既有 source27 本地门禁已真实通过 `cargo check -p warp`、i18n 11 项、受影响 Rust 1410 项及 Python 15 组共 426 项。本轮没有重跑这些门禁。

本轮实际执行 `./script/run --dont-open --features local_cli_managed_tasks,rust-embed/debug-embed`，exit 0，用时 321.364 秒。随后 `codesign --verify --deep --strict` 实际 exit 0。构建前后及原生两阶段前后均重新核对 111 项清单、detached 父提交和恰好 9 项修改。

| 对象 | 实际 SHA-256 |
|---|---|
| source27 测试库 | `050ff3f3a6bdf742ab628a19ddf618331b830bbaa9cabe20349e9c677156fdbd` |
| 本轮实际构建主程序 | `f01f2525952847cba097feca5931566c41043948286402435012ce0a92dcda5e` |
| 固定 Grok 文件 | `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb` |

在隔离的无认证配置 HOME 中独立执行固定 Grok `--version`，实际匹配 `grok 1.0.30 (04b7ffed98c6)`。两个精确 `--exact --ignored --list` 入口均实际列出一次、exit 0；列表操作不执行原生测试。专用认证源仅核对普通文件、当前 UID、0600、nlink 1，不解析或回显内容。实际调用时由既有 runner 复制 opaque auth，保留原守卫时序。

## 零输入 policy2：FAILED

实际 runner exit 1、Rust 测试 exit 101，用时 12.911 秒；`max_native_inputs=0`、360 秒、16 次 TLS CONNECT、8 MiB TLS 上限及显式固定 profile/config 路径均按原参数执行一次。`initialize` 数值请求 1 成功后，`session/new` 数值请求 2 等待中收到未声明通知并拒绝。

通知只公开安全形状：method 为 string、UTF-8 原字节长度25、SHA-256 `5ad0b9eadd8fadb2225bf5c00b21c1cf42ab869332b86333e722aa248a106580`；params 为 object，session ID absent，无 id/result/error。未读取未知 method 原文或通知正文，不能据此猜测其真实语义。根代理审阅时按 `diagnostic_value` 实现更正了原“序列化25字节”的措辞；原证据的长度与SHA不变，定向比对过程见[协议差异复核](GROK_27_PROTOCOL_DIFFERENCE_REVIEW.md)。

新 `phase_failure` 分别保存下列三个阶段，最终 `probe_failed` 仍为 primary：

| 阶段 | 与固定错误字面量精确 SHA 比对的结果 |
|---|---|
| primary | `native_unknown_notification_rejected`，36 字节，SHA `553467e5c8ad28e8e0d6d88da80163ba14785868c0d169461e0d092631e96237` |
| cleanup | `native_exit_code_missing`，24 字节，SHA `315ae2f1192d0c3184a943fc25736613ca76cd8f3b2fc7fc2ecfe5d0d127e1d7` |
| drain | `rpc_error`，9 字节，SHA `21964f4ced55d1efcd8b529b6e33d8b2f76b542f2b65e72ab52b7355299a1d2c` |

这是错误类型的固定字面量核对，不是原生错误正文回收；`rpc_error` 不证明具体 API、账号或权限失败。runner 的认证副本删除、tunnel 停止、临时工作区删除及 cleanup 均为 true。受控 native receipt 缺少退出码的失败仍保留；runner 清理成功不能代替该 receipt。所有 policy 项继续为 unknown，父权限 ceiling、文件系统沙箱及可实施准备均 false。

## 单输入 SDK10：FAILED

实际 runner exit 1、Rust 测试 exit 101，用时 10.148 秒；450 秒上限下只提交 1 个原生输入。真实计数为 discovery 1、tools/list 1、legacy initialize 0、inspect 0、审批 0、SDK 请求 2。对端两个请求实际携带 `2026-07-28` metadata，MCP 状态达到 connected 1 / ready；没有工具调用、原生来源证明或完整工具链通过。

新公开事务记录实际发送顺序为 `initialize` ID 1、`authenticate` ID 2、`session/new` ID 3、`session/prompt` ID 4。向 SDK discovery/list 的回复 outer/inner ID 分别为 0、1，此时依然等待 prompt ID 4。随后同原生 string 响应 ID 再次被生产数值 ID 守卫拒绝：13 字节、SHA-256 `731058153ec89c5e512596d24d5ac95c7262a9cb4e5c58fbdcebd36677f9b38e`。该 ID 与 source26 SDK9 的公开摘要一致，不能因重复出现而接受它。

到达时的真实事务上下文为 generation `c616b3d3-a9a1-484b-9d0e-344cd26f199a`、next request ID 4、pending ID 4、pending kind `prompt`；响应为 JSON-RPC 2.0，无 method/error，result 为 object，stop reason absent。失败仍为 `production_runtime / protocol / invalid_response_id`。

本轮新的私有响应信封真实捕获 259 字节、1 条，SHA-256 `ed82aa0c7f5a228ef65737cef12ddb6099a764d6b7bd486d94c488d80fab8523`，runner 的 UID/0700/0600/nlink1/预算范围核对与 ledger 对账均 true。该文件只承载精确 ID 和字段形状，不承载 result 值或 error 正文；本代理仅独立核对文件元数据，未读取、回显或归档其内容。旧私有 error capture 为 not_observed / 0 字节，因此仍不能认定 API 或账号错误。

native cleanup receipt/read 为 true；运行时失败后 transport_closed=false 的原字段保持不变。runner 确认 tunnel 停止、认证副本删除及项目文件数 0。私有 settings 字节确有变化，既有安全审计仅允许已观察到的 marketplace 初始化；permission section unchanged=true，不宣称全配置字节不变或插件已安装。

## 收尾与范围

[独立 postcheck](validation/official-27-native/native-inputs-postcheck.json) 已实际通过：111 源码资源、恰好 9 项修改、测试库、独立构建主程序及固定 Grok 文件在两次原生执行后保持相同 SHA。所有本轮监督日志及所核对的私有 evidence、test output、wrapper audit、两个响应补证文件均为当前 UID 的独占 0600 文件；SDK 私有工作区为 0700。未读取私有日志或正文。

16 份安全公开档案和原生诊断均保留真实失败；公开档案的六类凭据形状 serialized/decoded 扫描零命中，定义与文件 SHA 见 [archive-audit.json](validation/official-27-native/archive-audit.json)。扫描不是所有秘密不存在的数学证明，也不含私有正文或截图 OCR。

本轮新增诊断确实定位了事务阶段并独立保存首个失败与收尾失败，尚未修复未知通知或 string 响应 ID 的接口不匹配。SDK/父子工具产品 gate 保持关闭。后续须先核验真实协议语义再修复，不能放宽数值 pending/审批来源关联，也不能把 MCP catalog ready 或 runner cleanup 成功计为目标通过。没有用户功能或界面文案变更；无需本地化变更。
