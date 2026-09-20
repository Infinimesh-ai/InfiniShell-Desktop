# source30 Grok policy3 单次受控复验

本次实际运行结果为 **FAILED**：跑器 exit1、12.270 秒，Rust 唯一 ignored 测试 exit101。max_native_inputs 为 0，未发送任务提示或执行第二次复验。旧 policy1／2 的失败档案没有更改。本报告与 [安全公共归档](validation/official-30-policy3/archive-audit.json)记录新的实际结果，不声明完整权限能力或 Goal 完成。

运行源码为干净提交 `0059ef1bf8438c5af7545c3101b880ec2d0b10e9`。执行驱动在预编译及真实运行前后核对 111 个输入，与 source29 清单和 actual0059 Git 对象逐文件一致；whole worktree clean，用户两处排除改动没有复制。归档阶段不再读取冻结树或 target。本代理执行并归档此次复验，归档审计不冒充第二个主体重复执行。

cargo test -p warp --lib --no-run 真实 exit0，298.702 秒；实际 libtest 为 849545480 字节、SHA-256 `9a0e6b663d6be887598b7a1eb444f9bf9926b8fb37c06d2f0c9860c3535d90fb`。唯一精确 ignored 入口 `ai::cli_agent_runtime::grok::policy_preflight_live_tests::native_fixed_policy_interfaces` 通过 --list 恰好列出 1 项，列表未执行原生测试。无认证的隔离 --version 确认为 Grok 1.0.30 (04b7ffed98c6)。本轮没有重新编译 main。

产品 worker 为 866053040 字节、SHA-256 `ab33468aa09057ee58f71031aa5026430a8bdf3db9e34a776b02d1bbac1488ea`；固定 Grok 为 SHA-256 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。lib／worker／CLI 的实际字节身份由本次执行驱动在运行前后独立读取核对不变。根代理 main 构建原报告记录 323.458 秒、双 FTL 完整字节嵌入和 strict codesign exit0／双输出 0 字节；本代理只引用并原字节归档该 [bundle 报告](validation/official-30-policy3/bundle.json)，没有重复签名验证，不将调试签名计为发布公证。

公共事件中只出现一代 new 启动、2 次请求和2次对应响应。initialize id1 返回 ok。session/new id2 期间先收到 `_x.ai/mcp/servers_updated`；封闭字段表明空目录通知已被接受：method_summary 25 字节、SHA-256 `5ad0b9eadd8fadb2225bf5c00b21c1cf42ab869332b86333e722aa248a106580`，global_catalog_closed_empty=true，frame／params 为 object，没有 id、result、error 或 sessionId。随后 id2 实际响应为 rpc_error；响应与请求的 generation／id／sequence 均对应，两个公开请求没有未匹配的响应身份。

primary 失败摘要为 9 字节、SHA-256 `21964f4ced55d1efcd8b529b6e33d8b2f76b542f2b65e72ab52b7355299a1d2c`，独立按公开固定 status `rpc_error` 的 UTF-8 字节计算相符。cleanup 阶段失败摘要为 24 字节、SHA-256 `315ae2f1192d0c3184a943fc25736613ca76cd8f3b2fc7fc2ecfe5d0d127e1d7`，具体原因未由本次公共材料确认。没有 process_cleanup、probe_finished、resume_checked 或 drain_response_observed 事件。因此没有真实正常 stdio 退出收据，不能宣称原生 exit0／cleanup 已证或历史继续已验；未匹配公开 RPC 为 0 也不能替代本机进程收尾证明。未读取 RPC 错误正文，不把 rpc_error 推断为账号／API失败，也不将未知权限或接口能力判断为支持或不支持。

元数据 execution_boundary_passed 与 interface_investigation_completed 均为 false。same_commit_verified_by_runner=false 原值保持；同实际干净提交与二进制前后身份来自独立执行驱动的 [precheck](validation/official-30-policy3/precheck.json) 和 [postcheck](validation/official-30-policy3/postcheck.json)，未回填跑器自身声明。profile_loading、effective_mode、configuration_sources、builtin_catalog、wrapper_closure 仍为 unknown；parent_permission_ceiling_verified、filesystem_sandbox_verified、ready_for_policy_implementation 均为 false。仅一代启动，snapshots_match_explicit_inputs=false；私有 config／profile 的字节保持由受审跑器报告为 true。

受审跑器保持最多 12 个只读 RPC、2 个进程、360 秒、16 条 TLS／8 MiB，仅允许 auth.x.ai 和 cli-chat-proxy.grok.com。本轮实际 6 条 TLS、132205 字节。TLS 未解密，model_http_count_measured=false；0 个任务输入不等于独立测量的 0 次模型 HTTP 请求或费用。原授权认证文件只由受审 opaque-copy helper 复制，不解析值、原文件未改；执行驱动仅 stat／权限／inode／mtime 前后核对。跑器 opaque_auth_copy_removed、tunnels_stopped、private_workspace_removed、runner_cleanup_confirmed 全部为 true。这些是跑器资源清理证据，不填补缺失的原生 stdio／cleanup 收据。

新公共 events、metadata、network 通过本次封闭字段及类型校验，元数据诊断与事件逐项相同。连同准备／调用／后置身份报告，全部 JSON／NDJSON 原字节归档；六类固定凭据形状的原序列化及独立解码字符串扫描均为 0，公开编译日志仅核对 SHA、实际 lib 路径及形状计数，没有复制日志正文。未读取或归档私有 runner log、.test-output.txt、原生 stdout／RPC／信封、认证、配置或 SQLite 正文。配置及 profile 内容仅来自仓库合成夹具。

本次没有 GUI、平台、全工作区、索引、提交或 main 修改；未修改旧档案、主三文档、源码或用户配置。无需本地化变更。finished_reads_done=true，真实后置身份核对结束后共享 target 已释放；失败没有重试。
