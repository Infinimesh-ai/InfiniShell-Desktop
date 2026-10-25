# InfiniShell Website

Astro 站点。先安装 Node.js 和 Bun;`predev`、`prebuild` 等脚本会调用 Bun 同步更新日志。
从仓库根目录进入 `website/` 后执行:

```bash
bun install --frozen-lockfile
bun run dev      # http://localhost:4321
bun run build    # 输出 dist/
```

依赖以 `bun.lock` 为准,与 GitHub Actions 和 Nixpacks 保持一致;旧 `package-lock.json` 不作为当前安装依据。

结构:

- `src/pages/index.astro` — Landing
- `src/pages/docs/[...slug].astro` — 文档动态路由
- `src/content/docs/*.mdx` — 文档内容(Content Collections)
- `src/components/` — Nav / Footer / Banner 等
- `src/styles/` — 设计 token 与全局样式

`website/CHANGELOG.md` 是仓库根更新日志的发布副本,供只包含 `website/` 的构建上下文使用。
在根目录更新日志后运行 `bun run sync:changelog`,同步副本后一起提交;不要分别维护两份内容。

网站仅支持英文与简体中文。语言选择、自动检测、文档字段和搜索提示均按这两种语言维护;
旧浏览器保存的不支持语言值会回退英文。
