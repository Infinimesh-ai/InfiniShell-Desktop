# 通知插件兼容性

当前插件版本、安装与禁用行为、升级恢复及授权规则统一见[支持说明](RELEASE_SUPPORT.md#通知插件与授权)；验收结论见[验证报告](VALIDATION_REPORT.md)。本路径供配套插件 README 继续引用。

安装成功与启用、原生 hooks 信任、事件可信度分别判断。缺少原生关联标识的通知不能单独证明成功；未知来源、用户自定义文件和并发编辑不得覆盖。CLI 升级与插件升级分别核验，不因发现新版本自动开放未经验证的能力。

原版本的通知线格式、兼容加载与安装实验见[固定提交历史原文](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/PLUGIN_COMPATIBILITY.md)。
