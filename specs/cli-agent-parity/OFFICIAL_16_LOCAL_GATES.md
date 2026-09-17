# source16 本地门禁失败归档

本记录保留 source16 的真实失败边界：`cargo check -p warp` 成功，但 `cargo test -p warp --lib i18n::tests` 在编译 lib-test 时失败，i18n 测试没有执行，不能记为 11 项通过。定向 Rust 测试未运行；source16 未产出可交付的 lib-test 或主程序，也没有本轮真实 CLI、PNG、GUI 或跨平台验收。

## 来源与不可替换的快照

根代理执行的原始报告为本次证据来源，三份 JSON 按字节完整复制，不重新序列化。基础提交为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，79 个唯一路径的 source16 清单 SHA256 为 `d4694c9ded528c4c2c6c56f9a7646ca8356424f75dc44726990df773d523475d`。它是包含实际修改的中间脏快照，不能视为最终同提交验收。

根代理在 source16 唯一 Cargo 进程结束后核对了 79 个冻结文件，再修复测试断言并前进 source17（清单 SHA256 `addb4ca18ec6ab550e063bced9c879fe6bcba72da4bf138cd41a5d0c69ecb4bf`）。本归档只独立核对 source16 清单的原报告哈希、路径形态和计数；79 个实际文件一致性来自根代理确认，本归档没有复读被冻结文件，也没有用当前 source17 字节替代 source16。source17 的后续检查或修复不会追认 source16 成功。

## 实际门禁结果

| 门禁 | 退出码 | 实际时间 | 边界 |
| --- | ---: | ---: | --- |
| `cargo check -p warp` | 0 | 92.404 秒 | 生产编译检查成功 |
| `cargo test -p warp --lib i18n::tests` | 101 | 128.084 秒 | lib-test 编译 E0277；未运行 i18n 测试 |
| 受影响模块定向 Rust 测试 | 未运行 | 不适用 | 前置编译失败，不能计为通过 |
| Python 离线回归 | 0 | 4.384 秒 | 12 组、295 项通过；不是原生 CLI 或产品验收 |

失败定位为新增 `app/src/ai/cli_agent_runtime/claude_image_tests.rs:69` 的 JSON `&Value` 与 `Value` 比较，错误码 E0277。已允许读取的 1,017 字节 i18n 日志与根报告 SHA 相符；独立计数确认 1 条 E0277 错误行、1 次该文件行号定位、0 条测试成功摘要、0 条 `running 11 tests`。本归档不保存原始编译日志或进行门禁重跑。

## 原报告与日志身份

| 原报告 | 字节数 | SHA256 |
| --- | ---: | --- |
| [source16 inputs](validation/macos-official-16-inputs.json) | 14,848 | `d4694c9ded528c4c2c6c56f9a7646ca8356424f75dc44726990df773d523475d` |
| [source16 gates](validation/macos-official-16-gates.json) | 1,218 | `72530b6a2d785823d04e22567e899d2f13e5f927bdb1b51bad86023c733071dd` |
| [source16 python](validation/macos-official-16-python.json) | 3,732 | `86de837e667da07438dcbe95183fe91682ff7e81c3ee2313b38596fede43f408` |

`check` 日志 SHA256 为 `cca78b11a722ae07c049440231ca458eb03a22fbc444c74ee6f9ab92ff9f282b`，359 字节；`i18n` 日志 SHA256 为 `9a6c062f56d57399a989a36a3b7c83aa32670f5c0e35915d7a893fd80748b0c9`，1,017 字节。两份日志仅独立核对哈希、固定统计及凭据形态，不复制正文。Python 日志 SHA256 `a503116d736cb42f0829192c58e406405ef6233c3fe3384a42e3ed3111e775eb` 只引用根代理原报告，本子任务未读取或复制该日志。

[归档审计](validation/macos-official-16-archive-audit.json) 记录逐字节一致性、清单路径校验、日志身份、读取边界及六类凭据形态计数。三份原 JSON、两份许可日志和新增归档文件的 API key、JWT、Bearer、私钥、邮箱、凭据赋值形态均为 0，只保留计数，不输出候选值。本归档没有读取当前冻结树、目标目录、私有数据、认证文件或任意旧 stdout；也没有执行 Cargo、CLI、模型或 GUI。

本次新增内容是验证记录，不新增产品可见文案，无需本地化变更。生产 PNG 能力、英文和简体中文布局、同提交跨平台验证仍须按后续真实报告独立验收。
