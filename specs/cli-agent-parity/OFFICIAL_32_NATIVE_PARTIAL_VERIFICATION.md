# source32 构建与 Grok 原生部分复验

受测快照基于实际父提交 `0059ef1bf8438c5af7545c3101b880ec2d0b10e9`，119项输入、18项变更；dirty候选，不计最终实际修改提交或同提交平台通过。本地门禁见 [独立记录](OFFICIAL_32_LOCAL_GATES.md)。

main构建退出0／329.100秒，worker 866287536字节，SHA-256 `461cacd2ba2c4ed57de5b62ff6ea597e6feb58a669313bd9afd17043beb5a913`。严格签名检查退出0，完整英文和简体中文FTL逐字节核对嵌入；850624232字节测试库前后保持一致。[构建公共记录](validation/macos-official-32-bundle.json)仅证明构建与嵌入，不证明新GUI或真实CLI整链。

SDK12仅运行一次，1项模型输入、32次TLS连接及32MiB上限，29.281秒，Rust退出101、runner与driver退出1。实际观察现代2026-07-28发现1次、tools/list 1次、inspect业务调用1次，共3项SDK请求；SearchTool和UseTool各一次原生审批允许，未出现其他工具，正常transport结束、关闭、清理与退出收据均取得。最终验收失败属于acceptance：六种工具调用carrier均无原生session／prompt／toolCall身份字段，`full_native_origin_fields_observed`和`native_origin_ledger_relation_verified`为false，仍仅candidate_mapping。不能用当前活动prompt或唯一工具候选补造原生身份；真实SDK请求走通也不能证明父权限上限。原生内部skills-reload严格封闭成功形状为true，维护帧兼容确实使本次业务调用继续推进；旧SDK11整体失败保持不变。

SDK12在隔离目录中存在原生设置文件变化，`private_settings_unchanged`及`no_side_effects`均为false，不追认为设置完全保持。认证副本与连接已清理；HTTP模型调用次数不可观察且未由本探针强制限次，1项用户输入和TLS预算是实际边界。私有响应／错误／设置原文均未纳入交付。[SDK事件](validation/macos-official-32-sdk12-events.ndjson)、[SDK元数据](validation/macos-official-32-sdk12-metadata.json)保留各自真实范围。

policy4仅运行一次，0项模型输入，14RPC／2进程／360秒／16次TLS连接及8MiB为上限；13.074秒，Rust退出101、runner与driver退出1，接口调查与执行边界整体均未通过。实际仅发出3项RPC：initialize第1项与cached_token authenticate第2项成功，session/new第3项返回关联rpc_error，响应422字节；没有skills-reload或unknown-id被拒绝的证据。primary 9字节及cleanup 24字节原因原档仅保存摘要；与固定源码诊断值核对分别为rpc_error及native_exit_code_missing，后者是失败取消收据exit_code=null在公开投影中被拒绝，不能补成正常退出0。错误的标准code与正文原因仍未知，不推断登录失效、审批或策略接口不支持。认证副本、隧道和私有工作区清理确认，原始认证仅做stat前后核对，策略及config夹具字节保持。[权限探针事件](validation/macos-official-32-policy4-events.ndjson)、[元数据](validation/macos-official-32-policy4-metadata.json)保留实际请求计数，不能将预算作为已执行数量。

两探针独立HOME、端点和认证副本，precheck／单次执行／postcheck均分别留档，postcheck退出0且源输入、实际二进制、公共产物身份稳定；没有追加模型输入或重复执行。[公共归档核对](validation/macos-official-32-native-archive-audit.json)精确保存13项公共文件；严格JSON重复键／非有限数检查及序列化、解码字符串的六种凭据形状检查通过。未复制私有日志、认证正文、原生响应封包或设置正文。

原生六阶段插件迁移尚未执行。静态发现配方和manager最低版本为0.1.1，而hook session_start仍报0.1.0，实际footer比较可能持续显示更新提示；后续源码候选正在同步通知版本并固定父提交旧脚本夹具，不回填本快照。真实已安装hook、完整插件故障组合、普通终端、最终同提交三平台／全工作区、三方整链及SSH／tmux仍未完成。Goal保持ACTIVE。
