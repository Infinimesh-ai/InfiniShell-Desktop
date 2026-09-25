# Codex 插件持久来源与失败恢复

当前支持范围见[插件与授权](RELEASE_SUPPORT.md#通知插件与授权)，验证结论见[验证报告](VALIDATION_REPORT.md)。本路径供配套插件 README 继续引用。

配置提交使用限定字段重读比较和原子文件替换，不是跨进程原子 CAS。发现并发变化时停止；失败恢复仅覆盖仍匹配本次写入值的状态。不能确认所有权的旧缓存与阶段记录保留在事务目录，不能直接执行原生 Git upgrade 覆盖受控来源。

原始实验、恢复边界和 `expectedVersion` 限制见[固定提交历史原文](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/CODEX_PLUGIN_CACHE_REFRESH.md)。历史结果只对应其标注版本与平台，不扩展当前兼容范围。
