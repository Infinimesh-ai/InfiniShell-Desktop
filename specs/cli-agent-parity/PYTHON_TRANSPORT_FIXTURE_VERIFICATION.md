# Python 传输夹具的共享库隔离

此修复仅修改 `probe_claude_no_credentials_tests.py` 中以 `sys.executable` 派生的 Python 子进程。原生 Claude 的 `isolated_environment`、固定 CLI 准备器和探针本体不变。

## 失败证据与固定来源

同提交 `c55385a69a4cd50364a6464dfbec1c965db2ac92` 的 [CI 35106400715](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35106400715) 已安装 Python 3.13.15。Linux 日志记录解释器及 `LD_LIBRARY_PATH` 均位于 `/opt/actions-runner-infinishell/_work/_tool/Python/3.13.15/x64`，前六组离线测试通过，但 UTF-8 和非法 UTF-8 两个子进程夹具提前退出，分别表现为没有 initialize 响应和 BrokenPipeError。原日志未保存子进程 stderr，不能将推断写成已经采到动态加载器的原始报错。

[固定 setup-python 源码](https://github.com/actions/setup-python/blob/ece7cb06caefa5fff74198d8649806c4678c61a1/src/find-python.ts#L163-L172) 明确为 Linux 导出安装目录下的 `lib` 到 `LD_LIBRARY_PATH`；当前隔离环境没有保留这个变量。[固定 Ubuntu 构建器](https://github.com/actions/python-versions/blob/96cf261124d1e3fbc49339879ac37f935c25653f/builders/ubuntu-python-builder.psm1#L27-L37) 使用共享 Python 和构建目录的绝对 rpath，[固定安装脚本](https://github.com/actions/python-versions/blob/96cf261124d1e3fbc49339879ac37f935c25653f/installers/nix-setup-template.sh#L17-L35) 将文件复制到实际 runner 工具缓存，没有改写 sysconfig 数据。这与本次解释器夹具失去 loader 搜索路径后提前退出相符，实际 Linux 修复结果仍须新 workflow 验证。

另直接流式读取官方固定归档 [python-3.13.15-linux-22.04-x64.tar.gz](https://github.com/actions/python-versions/releases/download/3.13.15-31064747964/python-3.13.15-linux-22.04-x64.tar.gz) 中的 `lib/python3.13/_sysconfigdata__linux_x86_64-linux-gnu.py`，只用 AST 读取常量，没有执行 Linux 文件：

| 字段 | 实际归档值 |
| --- | --- |
| `Py_ENABLE_SHARED` | `1` |
| `LDLIBRARY` | `libpython3.13.so` |
| `LIBDIR` | `/opt/hostedtoolcache/Python/3.13.15/x64/lib` |
| `prefix` | `/opt/hostedtoolcache/Python/3.13.15/x64` |

因此只使用编译时 LIBDIR 也可能继续失败：它不同于本次 runner 的实际安装路径。

## 最小修复及验证范围

测试专用 helper 只在 Linux 共享解释器中添加一个经过核对的库目录：先读取当前 `sysconfig` 的绝对 LIBDIR 和单文件名 LDLIBRARY；若仍是外部构建前缀，则仅按已核实的 `lib` 布局重定位至当前 `sys.base_prefix/lib`。要求目录属于当前解释器安装、实际库文件存在且解析后仍在该目录。不复制调用方的 LD_LIBRARY_PATH，也不传递 LD_PRELOAD、Python 全局配置或凭据。

夹具失败时在异常附注中输出有界诊断：解释器、版本、所选库目录、真实退出码、stderr 与读取错误。提前退出仍失败，不增加 initialize 超时，也不把损坏 UTF-8 变成有效协议。

本机 macOS Python 3.14.6 已运行该文件全部 **9 项测试，0.282 秒通过**。其中保留两项真实 Python 管道测试，新增当前/重定位库目录边界、无效或缺失库拒绝，以及真实子进程 stderr/退出码 37 的诊断回归。库目录的 Linux 分支使用明确的临时文件布局测试，不能冒称 Linux 动态加载器已执行通过。新的 Linux 3.13.15 结果留待 workflow；未执行 Cargo、SSH、模型请求或原生 CLI，没有修改 Grok 冻结文件。
