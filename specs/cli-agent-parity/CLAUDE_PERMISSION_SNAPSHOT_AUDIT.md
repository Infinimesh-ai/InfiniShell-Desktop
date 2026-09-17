# Claude 2.1.273 权限快照审计

本记录仅覆盖固定原生二进制、隔离无凭据控制请求与产品代码审查。没有发送用户或模型输入，没有执行工具，没有开启 Claude 父任务派发，也没有修改用户配置。原生 arm64 文件 SHA-256 为 `953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb`，来源及签名清单见 [无凭据验证记录](CLAUDE_NO_CREDENTIALS_VERIFICATION.md)。本轮使用其核对后的私有普通文件副本。

## 已取得的真实接口证据

| 场景 | 真实结果 | 证据 |
| --- | --- | --- |
| 空配置 | `initialize`、`get_settings`、`list_permission_rules` 均按本次 request ID 成功；规则和工作目录增量为空；EOF 自行退出 0 | [空配置](fixtures/claude-2.1.273-permission-query-empty-macos.json) |
| 非空配置，sandbox 显式 false / true | 两组均返回原样权限及 sandbox 配置、`flagSettings` 来源，以及 CLI 参数增加的 allow/deny 和 additional directory；EOF 分别 8 / 9 ms 自行退出 0 | [受控配置](fixtures/claude-2.1.273-permission-query-controlled-macos.json) |
| 同连接重复初始化 | 两次无配置字段的 `initialize` 均成功，包含当前 `current_permission_mode`；EOF 自行退出 0 | [重复初始化](fixtures/claude-2.1.273-permission-query-repeat-initialize-macos.json) |

`get_settings` 返回 `effective`、`sources` 和部分 `applied` 模型参数；`list_permission_rules` 返回 `state.rules`（behavior/source/rule/editability）、`workspaceDirectories`、`originalCwd`、`managedOnly`。固定发布文件内的实现还列出 session-only grants、解析错误和 `notInEffect` 字段；这些分支本轮没有制造并执行，不能计为真实通过。

此前产品只记录 `permissionMode`，是尚未接入上述查询的实现缺口，不能据此断言上游没有权限查询接口。

## 仍然不能推导的权限证明

1. `sandbox.enabled=true` 的配置回显不是实际工具已受约束的证明。固定版本另有 `claude sandbox status`（`statusVersion=2`，含 supported/enabled/strictMode/filesystemPolicy），但该命令观察新进程，不是正在运行的父会话，也没有导出该父会话完整已解析的文件及网络边界。
2. 初始化只证明读取了当时的 mode。后续无模型实验已验证同进程 mode 切换、flag 合并及自有文件漂移（见下）；session-only grant/revoke、动态 sandbox 例外和企业策略收紧仍未实测。
3. 三个控制查询没有共同的原生事务版本。顺序查询或重复相等不能自动升格为外部变更下的原子快照。
4. `can_use_tool` 不拦截所有自动批准工具；`allowedTools` 也不是工具集合限制。参见 [官方权限顺序](https://code.claude.com/docs/en/agent-sdk/permissions)。Bash sandbox 与内置文件工具的权限机制不同，参见 [官方 sandbox 说明](https://code.claude.com/docs/en/sandboxing)。滚动文档不能代替固定版本的执行验证。

## 最窄后续实现

- 在原生连接内加入绑定 request ID、runtime generation 和 native session 的只读查询，持久化严格的权限投影。不要记录完整 settings；其中可能含环境变量和认证 helper 等秘密。
- 派发前重新读取父策略，创建尚未发送初始输入的子会话并比较，再次核对父策略与当前派发动作后才释放首条输入。旧持久化快照不能代表派发时父上限。
- 首个候选组合仅考虑显式 sandbox disabled；缺省不推断 false。未知权限或 sandbox 字段、尚未验证的 managedOnly、来源相关路径规则及环境强制限制均应明确拒绝。固定父子 cwd 可复用，但仍需比较 additional directories 和规则来源。
- `session` 来源不得丢失。忽略额外 allow 是否更严、如何保持 deny/ask 规则语义需要独立证明；将规则搬入 flagSettings 可能改变路径锚点，不能只比较字符串。
- Codex 已保存的外层绑定和 `permissions` JSON 结构保持不变；Claude 新形状使用明确标记和严格兼容解析，增加旧记录往返回归。

下一轮可无模型复现 mode/规则变更后的查询、来源和路径锚点、父子非空投影比较及漂移拒绝。真正工具审批、sandbox 强制执行及完整父子模型生命周期仍未验证。上述为最初查询审计阶段的边界；后续运行时接入见文末，未修改共享协调器。


## 可复现动态观察（2026-09-17）

新增 [探针](../../script/cli-agent-parity/probe_claude_permission_snapshot.py) 与 [离线测试](../../script/cli-agent-parity/probe_claude_permission_snapshot_tests.py)。真实运行使用同一固定 darwin-arm64 原生副本，没有凭据、用户消息、模型回合、工具执行或用户配置写入。[脱敏完整权限投影](fixtures/claude-2.1.273-permission-snapshot-dynamic-macos.json) 保留 4 个原生进程、32 个精确 request ID、12 个原生状态通知；4 个进程均经 stdin EOF 自行退出 0，未使用强制清理。脚本 `passed` 只指下表查询/负向比较契约。

| 场景 | 真实观察 | 不能推导的结论 |
| --- | --- | --- |
| 父子同配置、同 cwd、同 `--add-dir` | 两个原生 PID 的权限投影一致；原生 `originalCwd` 和 `workspaceDirectories` 返回本次含中文/空格绝对路径，目录来源是 `cliArg` | 不代表两个进程具有完整等价 sandbox、原子父上限或真实工具授权 |
| 同连接 `set_permission_mode` | `default → dontAsk → default` 精确成功，响应 `{mode:...}`，重复空 initialize 读到当前实际模式；新父模式拒绝旧子投影 | 不覆盖其它模式及模型中的审批行为 |
| `apply_flag_settings` | 追加 deny 实际进入 effective 和 live 规则；`allow:[]` **不会撤销**旧 allow，规则数组合并而非替换；新父规则拒绝旧子，新子从当前受控 flag 构建后资料相等 | 该接口不是 session-only grant/revoke；不能用空数组推断已撤销权限 |
| 四种自有文件/参数来源 | 顺序为 userSettings、projectSettings、localSettings、flagSettings，规则保留来源和原始相对字符串 | native 响应没有导出每种规则的规范文件路径；控制输入中记录的路径不是上游返回的授权解析锚点；未执行文件工具证明相对路径语义 |
| `update_settings` 尝试权限写入 | 已启用 local 来源仍精确拒绝 `update_settings keys not allowed: permissions`，本次 local 文件未变 | 不能通过此控制接口声称实现规则撤销 |
| 自有 local 文件 `ask → deny` | get_settings 已返回新 deny；同一次观察的 list_permission_rules 仍保留旧 ask，明确产生 `settings_live_rules_mismatch`；旧父子比较拒绝 | 不推断永久滞后。探针只限 3 秒观察，不能把重复查询/短暂相等当作原子快照 |

最早探索版错误地期待每次读取都为 control_response，在成功切换 mode 后遇到真实 `system/status` 而断言失败；未执行模型，私有进程已清理。最终探针仅接受实际已见的 `system/status(status:null,permissionMode)` 和空 `background_tasks_changed`，关联其 native session ID；额外 user/assistant/result/stream_event、未知通知、错 request ID、重复响应、非空后台任务、超时或强制退出均不能计通过。

固定发行文件的 schema/dispatcher 有 `get_settings`、`list_permission_rules`、`apply_flag_settings`、`set_permission_mode` 和受限 `update_settings`。`updatedPermissions` 的 addRules/replaceRules/removeRules + destination=session 属于已有待审批 `can_use_tool` 的回复路径；没有已验证的独立 session grant/revoke 控制方法。本轮没有真实待审批，因此不伪造回复、不声称该路径已测，也不将有限 schema 审计扩大为所有接口普遍不存在。managedOnly=true、policy 来源和 `notInEffect` 的真实分支仍未测试。

探针只写已知 permissions/sandbox/rule/directory 字段。未知字段只记已知对象位置与数量，不写键名或值；解析错误仅记存在标志，`applied`、完整 settings、env 及任意原生错误正文均不写入产物。未知对象或类型、模式变动、settings/live 不一致、managedOnly、sandbox 未显式 false 都拒绝资料比较。14 项离线测试覆盖秘密字段不会进入产物、未知字段、session 规则保留、错 request ID、模型/越权请求禁止及旧快照拒绝。本次无需本地化变更。

运行方式（Python 3.11+；仅已有固定摘要原生文件）：

```sh
python3 script/cli-agent-parity/probe_claude_permission_snapshot_tests.py
python3 script/cli-agent-parity/probe_claude_permission_snapshot.py --executable /absolute/private/claude --output /absolute/private/permission-snapshot.json
```

Linux/Windows 可复用 prepare_claude_cli.py 的固定原生文件和 RUNNER_TEMP；本轮未在这两个平台执行，不算跨平台通过。Windows 使用原生 Python 的绝对路径参数，无 Bash 依赖。报告输出必须源码外、不能覆盖既有证据；配置由探针新建临时域并清理，二进制前后均核对固定摘要。

## 生产观察接线与本地验证

只新增串行观察，不开放 Claude 父派发：绑定当前连接/generation 的 initialize → get_settings → list_permission_rules → 再次空 initialize。任何错误、过期回包、模式变化或投影不一致只令权限观察不可用于派发；正常 Inherit 对话仍按原始策略工作，等待有界，不能永久挂起。重复空 initialize 不得重新注册/覆盖 MCP、skills、hooks。已持久化 Codex ParentPermissionCeiling 旧 JSON 保持原样。

后续有界观察入口供派发前刷新复用，但当前没有原生事务版本，不能提前声明原子性。模型/工具真实审批、会话 grant/revoke、动态 sandbox、managed-only、来源路径授权语义，以及父收紧与子首条输入之间的原子边界，仍是允许派发前的明确缺口。

生产适配器已接入上述串行观察，在首次原生 initialize 后等待最多 5 秒。所有回包按本连接代次的唯一 request ID 对应；两次 initialize 的原生 PID 必须一致，握手没有原生 session ID 时仍保持未确认关联，后续真实生命周期事件再确认。观察错误或超时释放正常 Inherit 聊天，并丢弃迟到观察回包；未知配置字段和任意原生错误正文不进入持久化。原生权限模式通知使旧观察失效，不产生任务完成事件。

冻结 5 文件快照在独立验证树通过 `cargo check -p warp`（135.016s）、国际化 11 项和相关模块 987 项（11.969s 测试执行时间）。新增 22 项回归包含真实无凭据投影夹具、权限规则滞后、秘密过滤、模式/PID/cwd 漂移、旧代回包、观察超时与重复 initialize 后 MCP/技能保留。见 [源摘要与命令记录](validation/macos-claude-permission-observation-gates-1.json)。这是生产代码的本地回归，不是实际 Rust 适配器的 Claude 模型生命周期或父权限委派验收。复用已有就绪数据，不新增界面文案，无需本地化变更。
