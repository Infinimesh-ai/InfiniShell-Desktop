# 跨平台预检的固定 Python 工具链

2026-09-16。本轮只修改 `.github/workflows/cross-platform-preflight.yml` 的两个前置安装步骤，没有连接 runner、派发 workflow、运行 Cargo 或修改产品。

## 首轮失败依据

[运行 35100708171](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35100708171) 在提交 `dbee1ecae81da54a1749de88a7caffb3099d7eb7` 上失败：Linux 的 `/usr/bin/python3` 未满足 3.11+，Windows 没有 `python` 命令。后续原生证据未生成，保留上传缺文件的错误；完整验收状态见 [首轮快照](validation/cross-platform-dbee1ecae-35100708171.json)。未从断言失败臆测 Linux 的精确 Python 版本。只读完整日志确认两台 runner 均为 `2.337.0`。

## 选型与固定依据

两 job 的 checkout 后加入 `actions/setup-python`，固定完整提交 `ece7cb06caefa5fff74198d8649806c4678c61a1`，对应官方 [v6.3.0](https://github.com/actions/setup-python/releases/tag/v6.3.0)。[官方 tag API](https://api.github.com/repos/actions/setup-python/git/ref/tags/v6.3.0) 返回该 commit 对象；未使用可移动主版本引用。

固定 CPython **3.13.15、x64**，不选预发行或自由线程版本，`check-latest: false`、`update-environment: true`。这满足已有 Python 3.11+ 检查，保留 Linux 的 `python3` 与 Windows 的原生 `python` 使用方式。官方 [固定 action.yml](https://github.com/actions/setup-python/blob/ece7cb06caefa5fff74198d8649806c4678c61a1/action.yml) 证实这些输入及 Node 24 执行环境；[固定 README](https://github.com/actions/setup-python/blob/ece7cb06caefa5fff74198d8649806c4678c61a1/README.md) 要求 runner 至少 2.327.1，本次已观察版本满足。

选取双方都可安装的补丁版本，而非只看 Python 3.11 分支号。官方 [固定版本清单](https://github.com/actions/python-versions/blob/96cf261124d1e3fbc49339879ac37f935c25653f/versions-manifest.json) 包含 3.13.15 的 Linux 22.04/24.04/26.04 x64 与 Windows x64 文件；最新 3.11.16 清单没有 Windows 文件。所选 [固定发布](https://github.com/actions/python-versions/releases/tag/3.13.15-31064747964) 的 API 元数据已核对：

| 文件 | 字节数 | API 声明的 SHA256 |
| --- | --- | --- |
| `python-3.13.15-linux-22.04-x64.tar.gz` | 103642328 | `de151f7c14aa962713a85289cc63f5fb3e52dd3385747d400bb9760abebdb640` |
| `python-3.13.15-linux-24.04-x64.tar.gz` | 102312718 | `4e544242f8a4ef647a6f511b67f9b00eefc9ef366644e3c40a27a6eff709ae2b` |
| `python-3.13.15-win32-x64.zip` | 29163819 | `73c2a2935597f8181e9bc60bc3a35cd2be28698d8f64b965055a29b43425a2b7` |

此处只核验官方元数据，未在本机下载这三个归档或冒称已执行目标平台 Python。setup-python 的发行清单查询仍由该 action 自身实现；固定 action 与 Python 版本不等于将下载归档的摘要额外写成 workflow 门禁。

## 安装范围与验证边界

复用 action 的 runner tool cache 和作业环境导出，不手工修改 runner 服务、系统 Python、全局 PATH 或安装配置。官方 [自托管说明](https://github.com/actions/setup-python/blob/ece7cb06caefa5fff74198d8649806c4678c61a1/docs/advanced-usage.md#using-setup-python-with-a-self-hosted-runner)要求工具缓存可写；Windows 首次安装还需要相应权限和 7zip，原生 MSI 自身可能更新安装注册表。未声称这次静态修改已证明两台机器满足首次安装条件；安装失败会使前置步骤明确失败，不降版本或跳测试。

本地验证：actionlint 1.7.12 通过 YAML、表达式和 action 输入检查。默认 actionlint 不认识仓库既有 `infinishell-ci` 标签；使用源树外临时配置仅声明该真实自托管标签后通过，未关闭规则。结构对比确认除两个安装步骤外，原有步骤、权限、触发条件、平台、错误处理及测试命令逐字不变；`git diff --check` 通过。

本轮无需产品本地化变更。下一次必须在包含此修改的实际提交上执行两平台预检，核对安装输出、实际 Python 版本、完整后续门禁及原生证据；首轮失败不能被本地 YAML 通过覆盖。
