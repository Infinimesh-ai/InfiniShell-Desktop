# 固定 Grok 内部技能维护响应原生形状探针

本次真实结果：`FAILED`。仅验证固定原生内部维护 RPC 的字段形状，不计为模型、SDK、本地子任务、权限或用户任务完成验收。

固定版本预期 1.0.30；二进制前置 SHA `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`，后置 SHA `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。使用一次独立 `agent --no-leader stdio`，只发送一次 initialize，未发送 authenticate/session new/load/prompt。模型输入为 0，合成技能文件写入次数 1。独立 HOME/GROK_HOME/TMPDIR/cwd，无复制或读取认证配置；OS 沙箱禁止全部网络与真实用户目录读取，仅精确放行固定二进制。

内部形状实际观测：`False`；未知帧 0。每帧仅保存固定字段匹配、类型、字节数、SHA 和允许的非负整数计数；未保存原生 stdout、stderr 或错误正文。首个失败 `internal_maintenance_shape_not_observed`，清理失败 `None`。原生进程退出码 `143`，终止信号 `None`，stdout EOF `True`，stderr EOF `True`，owned group 消失 `True`；这些事实分别记录，不以清理成功冒充退出码 0。临时夹具已移除 `True`。

完整安全投影见 [JSON](validation/grok-internal-skills-reload-native-shape.json)，5281 字节，SHA `873c448e35d8be1a3d9452436f6fbe5c6963da8c6140e752a57ad180b25a5b1b`。driver `/tmp/infinishell-grok-internal-skills-native-shape-driver-1.py` SHA `aab1f5b5706f2cb04e4f012d40007cc9480ef619f5741ac8e1ae9cb438741a5b`；实际耗时 52.632 秒。六类凭据形状的 serialized 与 decoded 字符串扫描均为零。没有修改生产源码、target、Cargo、GUI、Git 或旧验收档案；公开候选 snapshot 与已发布 binary 的完整映射仍未证明。无需本地化变更。

STOP；finished_reads_done=true。
