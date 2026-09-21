# CLI 自动升级绑定执行计划

## 状态与目标

第一阶段已在当前 dirty tree 实现：来源层直接构造 `ArtifactRole`、`ArgumentRef`、`LaunchBindingSpec` 与 `BoundInvocation`，生成确定性 binding digest；schema 2 journal、manifest、`launch-binding.json` 和 `exit-binding.json` 共同绑定 digest／generation／receipt。旧、缺失或不匹配 digest 均 fail closed，update 专用监督入口在 generation、manifest 和 OS spawn 前拒绝不支持的 pathname 执行。macOS 私有快照现会逐字节镜像源扩展属性，修复系统复制 API 改写 quarantine 字段造成的官方 CLI 摘要漂移；macOS atomic 15／15、当前签名快照实际执行 1／1、Claude 与 Grok 产品更新各 1／1、Python verifier 27／27、i18n 11／11、启用 feature 的 `cargo check -p warp` 与主二进制 build 已通过，详见 [receipt100](validation/macos-working-tree-100-auth-autoupdate-live.safe.json)；修复前失败边界仍见 [receipt99](validation/macos-working-tree-99-atomic-update-current.safe.json)，第一阶段 Runtime Host／来源门禁仍见 [receipt98](validation/macos-working-tree-98-current-runtime-binding-gates.safe.json)。

当前 macOS／Linux 的三款 native 来源使用 `NativeFile`：macOS 已实现签名私有 generation 快照，Linux 已实现 sealed memfd＋`execveat`。Windows 已实现程序／祖先 replacement lease、PE32／PE32+ 有界解析和挂起启动，但真实 CLI 的普通或 delay import 闭包尚未绑定，故 native 来源仍明确返回 `ManualOnly`。npm、Homebrew 与未知脚本／helper 闭包同样保持 `ManualOnly`，WinGet 仍不可识别。macOS 已真实通过 Claude `2.1.267`→`2.1.278` 与 Grok `1.0.34`→`1.0.40`，但 Codex 当前目标、Linux 上机、Windows 真实 CLI、三款渠道切换与事务恢复尚未闭环，因此消费者自动升级总体仍未完成。

[receipt104](validation/windows-working-tree-104-atomic-real-cli-failclosed.safe.json) 已把 Windows 边界从“未绑定导入闭包”推进为实际失败：三款官方 CLI 的真实签名与导入已盘点，但 cwd 叶目录可在持有句柄时改名，debug 事件链不能在 30 秒内结束，三款 updater 无退出收据，终止 debugger 后 Claude／Grok 根进程曾残留。产品 `validate_update_execution` 与来源接线继续在任何事务副作用前拒绝 Windows；现有 PE/debugger 实现只是受限候选，不是可调用的自动升级路径。[receipt105](validation/macos-working-tree-105-codex-01551-windows-asset-correction.safe.json) 后续确认 receipt104 把 `0.155.1` 官方包误与 legacy `0.147.0` 清单比较；当前 `0.155.1` 官方摘要与版本选择后的仓内清单一致，资产漂移这一独立阻断撤回，但不改变上述 Windows 实际失败与 `ManualOnly` 边界。

目标是让执行结果只有两种：

1. 执行已验证并绑定的对象及其受控依赖闭包；
2. 在任何副作用前明确拒绝，并保留原安装、事务和渠道状态。

禁止用执行前再次哈希、`/dev/fd` 路径、伪造 `argv[0]` 或缩短时间窗口代替绑定执行。

## 数据合同

更新计划需要从普通 `PathBuf` 提升为来源明确的绑定材料：

```text
LaunchBinding
├── NativeFile        已打开文件身份；仅原生单文件
├── SnapshotTree      私有 generation 快照及确定性树清单
├── WindowsLease      禁止替换的程序/祖先句柄租约
└── ManualOnly        只能检查和提示，不能自动执行
```

文件身份必须来自同一 handle，包含平台 file ID、大小和摘要。目录快照清单必须包含相对路径、文件类型、链接目标、mode、大小与摘要；拒绝特殊文件、越界链接、未知 hardlink 和可写祖先。

参数中的程序内路径必须使用结构化 artifact 引用，不能靠字符串替换猜测哪些参数应指向快照。

来源层最小合同如下；名称本身不是安全保证，只有 worker 真实实现相应执行语义后才能启用：

```rust
enum ArtifactRole {
    Program,
    Entry,
    Manager,
    Helper,
    Registration,
    InstallRoot,
    ConfigRoot,
    DependencyRoot,
}

enum ArgumentRef {
    Literal(OsString),
    ArtifactPath { role: ArtifactRole, relative: PathBuf },
    TargetVersion,
    PackageVersion { package: String },
    PathList(Vec<ArgumentRef>),
}

enum LaunchBinding {
    NativeFile { program: ArtifactRole },
    SnapshotTree { root: ArtifactRole, program: ArgumentRef },
    WindowsLease { program: ArtifactRole, ancestors: Vec<ArtifactRole> },
    ManualOnly { reason: Error },
}
```

`discover_*` 必须直接构造带角色的 `BoundInvocation`，禁止继续扫描参数或环境值中的占位字符串来猜测目标版本或安装路径。`capture_supervised_dependencies` 将其冻结为角色表、结构化参数和 binding digest；`spawn_with_environment_and_expected_files` 的后继 API 接收并持久化 `PreparedLaunchBinding`，journal 同时绑定 generation 与 binding digest。worker 不得对未知或未实现的 binding 回退 pathname 执行。

## 平台策略

### Linux

- 当前实现把同一身份冻结的 ELF 复制到 sealed memfd，核对摘要、大小、seal 与描述符标志后直接 `execveat(..., AT_EMPTY_PATH)`。
- `ENOSYS`、脚本或依赖闭包不完整时 fail closed；不得回退 pathname 或 `/proc/self/fd`。
- 用户可写 RPATH、RUNPATH 或同目录动态库必须进入受控快照，否则不开放自动执行。

### macOS

- SDK 没有可依赖的 `fexecve`／`execveat`；当前实现从已打开源文件构造 0700 私有 generation 快照并绑定原文件身份与摘要。
- 快照发布前核对内容、mode 与完整 Mach-O 签名，写 manifest 并同步后以无覆盖方式发布；当前源码的签名快照执行已实际通过。
- updater 必须证明实际修改目标来自显式安装根，而不是 `current_exe` 或快照路径。通用 worker 已验证环境仍指向原安装根；三款官方 updater 的产品事务仍须逐款验证。

### Windows

- 当前实现对原生 PE 使用禁止 `FILE_SHARE_WRITE`／`FILE_SHARE_DELETE` 的程序及祖先句柄，复核 file ID、reparse 和摘要后保持租约到挂起进程创建成功。
- PE32／PE32+、section／RVA、普通 import 与 delay import 均有固定边界；目前只接受依赖闭包为空的 PE。真实 CLI 都不能据此开放，非系统 DLL、KnownDLL 搜索优先级和延迟加载仍须形成可验证闭包。
- 进程 image 创建成功后才释放程序与 cwd 租约，再恢复主线程；真实 CLI 仍保持 `ManualOnly`，尚无 Windows 实际进程收据。

## 安装来源边界

- 官方 native：按平台绑定执行；无法证明 updater 目标与自身位置分离时不开放。
- Codex native：已有显式 `CODEX_INSTALL_DIR`，macOS／Linux 会将其绑定到原安装目标；Windows 和三平台真实产品事务仍须完成。
- Claude／Grok native：macOS／Linux 已能绑定执行对象并保持原配置／安装根环境；Grok 产品用例在执行前联网检查失败，Claude 用例已准备但未执行，不能据此声明 updater 目标闭环。
- npm：只有 Node、完整 `node_modules/npm`、helper 与外部工具闭包均可快照，且显式 `--prefix` 仍指向原安装目标时才开放。目标包 lifecycle script 未绑定时拒绝。
- 普通脚本：只有完整相对目录拓扑和显式目标参数都成立时支持；脚本以 `$0`／物理路径同时决定依赖与写入目标时拒绝。
- Homebrew：保持来源与版本检查，但自动执行可靠降级为手动更新。私有复制的 `brew` 会根据自身路径解析 repository/prefix，不能安全代表原安装；InfiniShell 也不能静默切换为自下载 cask。
- WinGet：当前缺少入口与包记录的可靠绑定，不能仅凭命令名或 PATH 映射为 `WindowsLease`。

`ManualOnly` 只是可靠降级，不满足“三款 CLI 消费者自动升级”总体完成门槛；至少每款官方默认安装路径仍须有一个真实验证通过的自动升级来源。

## 发布与恢复

1. 在受控状态目录同文件系统创建私有随机 scratch。
2. 从已打开源 handle 复制并构建确定性清单。
3. 校验快照、执行权限、依赖闭包与签名，逐文件及目录同步。
4. 写入并同步 manifest。
5. 无覆盖原子 rename 为 generation 目录。
6. 只有发布完成后才派生 supervisor；失败只留下可识别、可回收的 scratch。

每个阶段注入崩溃，恢复必须区分未启动、执行不确定、已退出和事务完成；不能重放安装命令或覆盖用户并发保存。

## 分阶段实施

1. **身份基础（当前已实现并定向验证）**：同 handle 捕获与验证、`ArtifactRole`／`ArgumentRef`／`LaunchBinding` 数据模型、binding digest、schema 2 journal 和 sidecar 绑定，以及无法绑定来源的 spawn 前 fail closed。
2. **原生执行（部分实现）**：Linux sealed memfd／`execveat`、macOS 私有签名快照和 Windows replacement lease／挂起启动均已有代码；macOS 当前源码实际执行通过，Linux 未上机，Windows 真实 PE 依赖闭包仍不支持。
3. **依赖闭包**：动态库、npm 树、helper 和结构化参数绑定。
4. **安装策略**：若官方 updater 无法脱离自身路径，仅对明确识别的官方 native 安装评估应用托管包下载、签名验证和原子发布；不得迁移 npm/Homebrew 来源。

## 验收

- 每个平台在“完成复核、尚未执行”屏障处并发替换 program、helper、父目录和依赖；只能执行绑定对象或明确拒绝。
- Linux 验证 ELF fd exec、脚本拒绝及无 `/dev/fd` 回退。
- macOS 验证签名保持、`current_exe` 行为及 updater 修改原目标而非快照。
- Windows 验证启动前 write/delete/rename 被租约阻止，进程创建后自更新仍可替换目标。
- npm 验证 `__dirname`、相对 require、完整依赖树和外部工具；Homebrew 验证产品不会派生 `brew`。
- 运行 `cargo test -p command`、受影响的 `warp --lib` 测试、i18n 门禁和 `cargo check -p warp`；再对包含实际修改的同一提交执行 macOS／Linux／Windows 真实升级、失败恢复和应用重启验收。

在上述证据完成前，现有离线驱动测试、开发机手工升级和执行前摘要复核都只能记作部分证据。
