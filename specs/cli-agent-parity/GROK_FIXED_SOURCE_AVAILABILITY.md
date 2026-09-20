# Grok 固定 1.0.30 公开源码可用性核对

核对日期：2026-09-18（北京时间）。本报告只核对 xAI 官方仓库、官方产品页面及公开发布元数据，不执行原生 CLI，不读取认证、二进制、模型结果或冻结目录。

## 当前结论

尚未在本次实际查询范围内证实 Grok `1.0.30 (04b7ffed98c6)` 对应的精确公开源码映射。不能据此断言所有公开源码不存在，也不能将候选公开快照当作该固定原生版本的权限接口、内置工具全集或完整源码闭包证据。

原生版本和短构建 ID 来自父任务已保存证据；本次未重新执行 `--version` 或读取二进制。父任务提供的仓库提交为 `436cc739234061c092cedd106432bbbfa3c2d645`；本次只新增报告和安全投影，不将该提交的门禁或原生验收结论改变。

官方 README 说明该仓库定期从 monorepo 同步，根目录 `SOURCE_REV` 记录当前公开树对应的完整 monorepo SHA。因此公开 GitHub commit SHA 与原生短构建 ID 不同，本身不能证明源码不精确。应进一步核对 `SOURCE_REV`。[官方 README](https://github.com/xai-org/grok-build#readme)

## 实际定向查询

| 官方请求 | 实际结果 | 证据边界 |
| --- | --- | --- |
| [固定短构建 ID 对应公开 commit](https://api.github.com/repos/xai-org/grok-build/commits/04b7ffed98c6) | HTTP 422 | 只说明该公开 commit 请求未取得提交对象；未保留或解释错误正文。 |
| [标签前缀 v1.0.30](https://api.github.com/repos/xai-org/grok-build/git/matching-refs/tags/v1.0.30) | HTTP 200，0 项 | 未取得该前缀标签；不覆盖其他标签命名。 |
| [标签前缀 1.0.30](https://api.github.com/repos/xai-org/grok-build/git/matching-refs/tags/1.0.30) | HTTP 200，0 项 | 未取得该前缀标签。 |
| [标签首 100 项](https://api.github.com/repos/xai-org/grok-build/tags?per_page=100&page=1) | HTTP 200，0 项 | 本次第一页返回空列表；未遍历其他页面或所有可能 ref。 |
| [发布记录首 100 项](https://api.github.com/repos/xai-org/grok-build/releases?per_page=100&page=1) | HTTP 200，0 项 | 本次第一页返回空列表。 |
| [默认分支提交首 100 项](https://api.github.com/repos/xai-org/grok-build/commits?per_page=100&page=1) | HTTP 200，45 项 | 元数据中的提交消息均未匹配完整版本字符串 `1.0.30` 或短 ID `04b7ffed98c6`；未逐个读取历史 `SOURCE_REV`。 |
| [固定短 ID 的官方仓库 commit 搜索](https://api.github.com/search/commits?q=04b7ffed98c6+repo%3Axai-org%2Fgrok-build) | HTTP 200，0 项，incomplete_results=false | 搜索结果不覆盖未索引源码、历史文件内容或未公开 monorepo。 |
| [规范仓库元数据 URL](https://api.github.com/repos/xai-org/grok-build) | HTTP 200 | 官方公开仓库存在，默认分支为 `main`。 |

[官方发布页面](https://github.com/xai-org/grok-build/releases) 同样显示没有 release 记录。[官方产品入口](https://x.ai/cli) 当前重定向到 [Grok Build 产品页](https://x.ai/build)，提供源码和更新日志入口，本页没有为固定 `1.0.30` 提供精确源码映射。

## 候选快照及当前公开 HEAD

本次默认分支提交元数据返回的首项为：

- 公开 commit：`482711333c7195dc16a272777f86086d615e2afb`。
- commit 时间：`2026-09-15T13:17:35Z`。
- 该 commit 与父任务此前候选相同；候选和本次观察的公开 HEAD 因此共用同一个不可变文件 URL。
- [实际 SOURCE_REV 文件](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/SOURCE_REV) 经官方 contents API 两次 HTTP 200 核对，均为 ASCII 40 位 SHA 加换行，总计 41 字节。
- 完整 monorepo revision：`be7ce6e8cffe46d20bef9834b211616082ee866b`。
- 文件正文 SHA-256：`f42d40d90d620527204dd41d9a0bb4a585d69dc8525ad5e81633f33a83c7d6e4`。
- Git blob SHA：`0c594fbe842905b9db14f523b7acaa5ebc9a853a`。

`be7ce6e8cffe…` 不匹配原生短构建 ID `04b7ffed98c6`。这证明候选公开树声明的 revision 与原生报告的短 ID 不吻合；当前任务未独立核验原生短 ID 的生成规则，不能进一步声称该差异已经证明原生完整源码不存在。即使后续找到匹配的 `SOURCE_REV`，仍需分别核对已发布二进制的构建来源、配置、依赖闭包及真实权限行为。

## 查询失败及可复核性

所有可引用的 API 请求均在配套 JSON 中保存 URL、UTC 时间、HTTP 状态、响应正文长度及 SHA-256，并只保留明确白名单元数据；没有保存响应错误正文、请求认证头、token 或模型正文。

初次带末尾斜杠的仓库元数据 URL `https://api.github.com/repos/xai-org/grok-build/` 返回 HTTP 404；规范 URL 返回 HTTP 200。前者不能作为官方仓库不存在的证据。网页工具访问固定 commit 页面及 tags 页面返回 Internal Error，没有可确认的 HTTP 状态，不能计作 HTTP 404 或未命中证据。

此前一批 `gh api` 的返回未被执行工具保留，不能引用其状态或正文；它们不参与上述结论。报告依赖随后已保全的官方 API 请求和官方页面。

安全投影：[grok-fixed-source-availability-20260918.json](validation/grok-fixed-source-availability-20260918.json)。

## 对后续适配的影响

当前公开候选仍可用于提出待验证假设，不能冒充固定原生 `1.0.30` 已核验契约。未取得精确映射时，权限策略、内置工具 catalog 和源码闭包应继续标记未知；已有真实协议记录只支持各自观测到的接口范围。本报告不将源码、静态推断或空发布列表计为 native PASS，也不改变产品能力门禁。

本次没有读取各历史快照的 `SOURCE_REV`、全部 refs、未公开 monorepo或构建产物，未进行 native、Cargo、GUI 或跨平台验证。新增文档无需本地化变更。
