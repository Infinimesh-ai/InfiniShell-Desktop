# macOS 加新语种的需要做的事

为了让 macOS 能正常翻译，当新增语种(如 `zh-TW`、`fr` 等)时，还需要:

1. **更新 macOS plist 配置**: 在所有需要声明本地化的 plist 中添加新语种代码到 `CFBundleLocalizations` 数组:
   - `app/assets/resources/mac/CLI-Info.plist` —— 修改 `<key>CFBundleLocalizations</key>` 下的 `<array>`
   - `app/src/bin/local.rs` —— 修改 `embed_plist::embed_info_plist_bytes!` 宏内的 plist XML
   - `app/src/bin/oss.rs` —— 修改 `embed_plist::embed_info_plist_bytes!` 宏内的 plist XML
   - 例:添加 `<string>zh-TW</string>` 到 `<array>` 中
2. **更新构建脚本**: 在 `script/update_plist` 脚本的 `plutil -insert/replace CFBundleLocalizations` 命令中添加新语种代码
