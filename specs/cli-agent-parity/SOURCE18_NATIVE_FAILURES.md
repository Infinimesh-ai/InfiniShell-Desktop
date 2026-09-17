# Source18：两次真实 CLI 失败验收归档

本报告保留两个整轮失败，不能计为 P0–P5 完成。两次测试退出码均为 `101`。原始公开证据、元数据和限定的退出／macOS 清理回执按原字节复制；原测试输出、原生正文、凭据和其他私有文件未归档。独立核对只读取这里列出的公开文件与根代理明确授权的四份私有回执，没有运行 CLI、Cargo、GUI、网络或 Git，也没有读取 frozen worktree／target。

## 构建来源与边界

两次运行公开元数据中的 lib SHA 为 `cc12e15bd9209d9c4ca3c935ba89d23f5af13d9e685be16af65795e33fb3a153`；监督进程二进制 SHA 为 `0942eaadace89b284f249b4a20374d0dd2cb9803f805c363826c422b3ce409ba`。根代理将两者关联到 source18；本报告独立核对了两个元数据的这些字段一致，未重新读取二进制或 source18 冻结清单。Claude 元数据记录基线 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 且工作区有修改；Grok 明确记录 `same_commit_verified_by_runner=false`。这些证据没有证明最终包含实际修改的同一提交跨平台验收。

## Claude：生产 PNG 与中文两轮完成，第一次正常关闭失败

固定 CLI 为 Claude Code `2.1.273`，二进制 SHA `953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb`。公开轨迹共 36 条：实际提交、接收、开始和完成各 2 次，原生协议来源摘要 19 条；实际原生 `user` 和 `result` 各 2 条。两轮分别为 PNG 识色与中文多行输入，两次完整输出 SHA 均与夹具的期望 SHA 一致，并在真实原生 `result` 的成功摘要中出现。PNG 原生回放的数组、图片、文本 SHA／字节数及块类型与准备记录一致。该独立核对支持前两轮的具体结果，不能替代整轮验收。

第一次 `cleanup_checked` 明确 `normal_exit=false`，失败原因为 `normal_exit_not_confirmed`。限定退出回执实际记录 `exit_reason=stop_requested`、退出码 0、清理已确认；清理证明记录同一代次、真实等待状态 0、Job 已移除、资源 CID 已销毁。这里核对的是既有回执字节与字段关联，没有重新查询内核。强制停止来源不能当作正常 `stdio_closed` 来源，第三轮 Resume／附件持久恢复／显式记忆验证未执行，整轮 `acceptance_passed=false`。GUI、SQLite 与应用重启也未通过本轮证明。

原元数据中的 `production_image_gate_open=false`、`native_image_input_verified=false`、`persistence_verified=false` 为 bare 共享运行器继承字段，按原字节保留。这三个字段不用于判断实际生产代码 gate 的开闭；本报告以专用轨迹和明确的整轮验收字段说明已完成步骤与失败步骤。

文件：[轨迹](validation/claude-managed-image-live-1.ndjson)、[元数据](validation/claude-managed-image-live-1.metadata.json)、[退出回执](validation/claude-managed-image-live-1.exit.json)、[清理证明](validation/claude-managed-image-live-1.macos-cleanup.json)、[独立审计](validation/claude-managed-image-live-1.archive-audit.json)。

## Grok：两次精确单次审批有效，SDK 反向请求仍为零

固定 CLI 为 Grok `1.0.30 (04b7ffed98c6)`，二进制 SHA `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。公开轨迹 11 条，包含 6 条原生工具更新、2 条审批与 1 条终态。公开审批记录独立核对出 search_discovery 与 inspect 各一次 `allow_once`，重复次数 0，两者审批契约校验均为真。根代理提供实际原生种类为 SearchTool 与 UseTool；本报告不读取其私有原生工具帧。公开终态显示搜索完成、inspect 失败，只有 1 次原生输入，SDK 请求、SDK 初始化、tools/list 和 inspect 回调均为 0。

终态 `passed=false`、原生整体契约未确认、来源 `unknown`、来源账本关系未确认、产品 gate 关闭；该结果不能证明 Grok 支持或不支持 SDK，更不能开放父子任务和权限上限能力。根代理告知私有失败内容 183 字节中有 `not found`；本归档代理未读取或复制该内容，也没有重新计算其散列。这条细节仅为根代理来源，不能计为独立观察。

限定退出回执为 `stdio_closed`、退出码 0、清理已确认；同代次清理证明记录真实等待状态 0、Job 移除及 CID 销毁。正常收尾证明没有改变 SDK 探针失败结论。网络公开记录包含 8 次官方 TLS 隧道、4,626,084 字节，以及 40 次非官方目标拒绝；网络预算未耗尽。这些目标拒绝记录只保留散列，不推出其原始目标或失败因果。认证副本已移除、隧道已停止、原生启动参数未改写。私有设置审计声明权限部分未变，实际字节变化仅为 CLI marketplace 初始化；该声明来自原运行器元数据，本归档未读取设置文件。

文件：[轨迹](validation/grok-sdk-origin-probe-6.ndjson)、[元数据](validation/grok-sdk-origin-probe-6.metadata.json)、[网络摘要](validation/grok-sdk-origin-probe-6.network.json)、[退出回执](validation/grok-sdk-origin-probe-6.exit.json)、[清理证明](validation/grok-sdk-origin-probe-6.macos-cleanup.json)、[独立审计](validation/grok-sdk-origin-probe-6.archive-audit.json)。

## 归档审计

两份独立审计记录九份输入的来源、字节数、SHA、复制一致性及固定六类凭据形态扫描结果，全部为零匹配。扫描不是对未知凭据格式的完整识别能力。没有覆盖旧报告，没有将失败改写成成功；后续修复必须产生新的真实运行证据。
