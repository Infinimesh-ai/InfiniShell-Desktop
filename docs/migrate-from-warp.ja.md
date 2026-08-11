# InfiniShell への設定の移行

[English](./migrate-from-warp.md) · [简体中文](./migrate-from-warp.zh-CN.md)

このガイドは、**設定系の構成**(カスタムキーバインド・テーマ・ワークフロー・
MCP 設定など)を以前のインストールから InfiniShell へ引き継ぎたい方向けです。

移行元として想定されるのは 2 つあり、**両者では安全性プロファイルが異なる**ため、
本書では別々のセクションで扱います。両方該当する場合は、**まず OpenWarp から
移行してから** Warp の移行を検討してください。

1. **OpenWarp** —— InfiniShell の以前の名称。
2. **上流の [Warp](https://github.com/warpdotdev/warp)** —— InfiniShell が fork して
   いるプロジェクト。

本書は コマンド履歴・SQLite データベース・Drive オブジェクト・認証情報の
いずれも **意図的に対象外** としています。これらはマシン依存のストア
(Keychain / DPAPI / libsecret)、または schema が結合したストアに格納されて
いるため、コピーは安全ではありません。

> **Zap から上がってきた場合。** Zap は InfiniShell の直前の名称です。
> InfiniShell は旧来の `.zap` / `dev.zap.Zap` ディレクトリを自動的には移行
> **しません**。新しい `.infinishell` / `dev.infinishell.InfiniShell` を
> まっさらな状態から使います。設定を引き継ぎたい場合は、本書の "OpenWarp" を
> "Zap" と読み替えて手動で実施してください。

---

## ディスク上のレイアウト

InfiniShell(および OpenWarp / 上流 Warp も同様)はディスク上の状態を **3 種類の
ディレクトリ**に分けて格納します:

- **config** —— `settings.toml`、`keybindings.yaml`
- **data** —— `themes/`、`workflows/`、`launch_configurations/`、`tab_configs/`
- **home dotfile** —— `.mcp.json`、`skills/`

macOS では 3 カテゴリーがいずれも単一の home dotfile ディレクトリ
(`~/.warp/`、`~/.openwarp/`、または `~/.infinishell/`)に集約されます。Linux では
XDG 規約に従って **3 つの異なる場所** に分かれ、Windows では `directories`
クレートの同等レイアウトで分かれます。下記の移行スクリプトはプラットフォーム
ごとに各ファイルを正しい保存先に配置します。

### InfiniShell の保存先

| カテゴリー | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.infinishell/` | `${XDG_CONFIG_HOME:-~/.config}/infinishell/` | `%LOCALAPPDATA%\infinishell\InfiniShell\config\` |
| data | `~/.infinishell/` | `${XDG_DATA_HOME:-~/.local/share}/infinishell/` | `%APPDATA%\infinishell\InfiniShell\data\` |
| home dotfile | `~/.infinishell/` | `~/.infinishell/` | `%USERPROFILE%\.infinishell\` |

### OpenWarp のソースパス

| カテゴリー | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.openwarp/` | `${XDG_CONFIG_HOME:-~/.config}/openwarp/` | `%LOCALAPPDATA%\openwarp\OpenWarp\config\` |
| data | `~/.openwarp/` | `${XDG_DATA_HOME:-~/.local/share}/openwarp/` | `%APPDATA%\openwarp\OpenWarp\data\` |
| home dotfile | `~/.openwarp/` | `~/.openwarp/` | `%USERPROFILE%\.openwarp\` |

### 上流 Warp のソースパス

| カテゴリー | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.warp/` | `${XDG_CONFIG_HOME:-~/.config}/warp-terminal/` | `%LOCALAPPDATA%\warp\Warp-Terminal\config\` |
| data | `~/.warp/` | `${XDG_DATA_HOME:-~/.local/share}/warp-terminal/` | `%APPDATA%\warp\Warp-Terminal\data\` |
| home dotfile | `~/.warp/` | `~/.warp/` | `%USERPROFILE%\.warp\` |

> Linux 上のディレクトリ名 `warp-terminal` は Linux パッケージ名と一致します
> (Debian/Ubuntu では `/opt/warpdotdev/warp-terminal/` など)。Windows 上の
> organization フォルダ名はパッケージ方法によって異なる場合があります。
> `%APPDATA%\warp\Warp-Terminal`(または `%LOCALAPPDATA%\warp\Warp-Terminal`)
> に見つからない場合は、お使いの Warp が実際に使用している
> `%APPDATA%` / `%LOCALAPPDATA%` のパスを確認してください。

---

## 1. OpenWarp からの移行(既存ユーザーの推奨経路)

OpenWarp は改名前の InfiniShell そのものです。2 回の改名コミット
(`feat: rename project Warp/OpenWarp → Zap`、およびその後の Zap → InfiniShell)は
識別子とディスク上のパス名を変更しただけで、**設定ファイルのフォーマットと schema は一切変わっていません**。
下記のファイルはそのままコピーできます。

### コピー対象

| ファイル / ディレクトリ | カテゴリー | 役割 |
|---|---|---|
| `settings.toml` | config | 公開設定(TOML ベースの設定ファイル)。 |
| `keybindings.yaml` | config | カスタムキーバインド。 |
| `themes/` | data | カスタムテーマ。 |
| `workflows/` | data | カスタムワークフロー。 |
| `launch_configurations/` | data | Launch 設定。 |
| `tab_configs/` | data | Tab 設定。 |
| `.mcp.json` | home dotfile | MCP サーバー設定。 |
| `skills/` | home dotfile | Agent skills。 |

### 手順

> コピー前に **InfiniShell を終了**してください。プロセスがファイルを掴んでいると
> 失敗します。

**macOS**

```sh
mkdir -p "$HOME/.infinishell"
for f in settings.toml keybindings.yaml themes workflows launch_configurations tab_configs skills .mcp.json; do
  if [ -e "$HOME/.openwarp/$f" ] && [ ! -e "$HOME/.infinishell/$f" ]; then
    cp -R "$HOME/.openwarp/$f" "$HOME/.infinishell/$f"
  fi
done
```

**Linux**

```sh
src_config="${XDG_CONFIG_HOME:-$HOME/.config}/openwarp"
src_data="${XDG_DATA_HOME:-$HOME/.local/share}/openwarp"
src_home="$HOME/.openwarp"

dst_config="${XDG_CONFIG_HOME:-$HOME/.config}/infinishell"
dst_data="${XDG_DATA_HOME:-$HOME/.local/share}/infinishell"
dst_home="$HOME/.infinishell"
mkdir -p "$dst_config" "$dst_data" "$dst_home"

copy() {
  if [ -e "$1/$3" ] && [ ! -e "$2/$3" ]; then
    cp -R "$1/$3" "$2/$3"
  fi
}

copy "$src_config" "$dst_config" settings.toml
copy "$src_config" "$dst_config" keybindings.yaml
copy "$src_data"   "$dst_data"   themes
copy "$src_data"   "$dst_data"   workflows
copy "$src_data"   "$dst_data"   launch_configurations
copy "$src_data"   "$dst_data"   tab_configs
copy "$src_home"   "$dst_home"   .mcp.json
copy "$src_home"   "$dst_home"   skills
```

**Windows(PowerShell)**

```powershell
$src_config = "$env:LOCALAPPDATA\openwarp\OpenWarp\config"
$src_data   = "$env:APPDATA\openwarp\OpenWarp\data"
$src_home   = "$env:USERPROFILE\.openwarp"

$dst_config = "$env:LOCALAPPDATA\infinishell\InfiniShell\config"
$dst_data   = "$env:APPDATA\infinishell\InfiniShell\data"
$dst_home   = "$env:USERPROFILE\.infinishell"
New-Item -ItemType Directory -Force -Path $dst_config, $dst_data, $dst_home | Out-Null

function Copy-IfMissing($srcDir, $dstDir, $name) {
  $from = Join-Path $srcDir $name
  $to   = Join-Path $dstDir $name
  if ((Test-Path $from) -and -not (Test-Path $to)) {
    Copy-Item -Path $from -Destination $to -Recurse
  }
}

Copy-IfMissing $src_config $dst_config settings.toml
Copy-IfMissing $src_config $dst_config keybindings.yaml
Copy-IfMissing $src_data   $dst_data   themes
Copy-IfMissing $src_data   $dst_data   workflows
Copy-IfMissing $src_data   $dst_data   launch_configurations
Copy-IfMissing $src_data   $dst_data   tab_configs
Copy-IfMissing $src_home   $dst_home   .mcp.json
Copy-IfMissing $src_home   $dst_home   skills
```

`[ ! -e ... ]` / `-not (Test-Path $to)` のガードは、InfiniShell 側で既にカスタマイズ
した内容を上書きしないためのものです。OpenWarp の値で上書きしたい場合は外して
ください。

InfiniShell が問題なく動くことを確認したら、上記の OpenWarp ディレクトリを削除して
ディスク領域を回収できます。もう誰も使っていません。

---

## 2. 上流 Warp からの移行

上流 Warp は独立した別プロダクトで、独自のディスク上アイデンティティを持ち
ます(上の「上流 Warp のソースパス」表を参照)。InfiniShell はビルド時の channel が
`Oss` で、独自の app id(`dev.infinishell.InfiniShell`)と、プラットフォームごとに分かれた
レイアウトを持ちます。両者のインストールは互いに相手のファイルを見ることが
できません —— これは Warp のアカウントやクラウド状態を InfiniShell に持ち込まない
ための仕様でもあります。

下記のテキスト形式ファイルは schema が安定しており、コピーしても安全です。
**それ以外はそうではありません** —— Warp は InfiniShell とは独立に進化しており、
バイナリ / プライベートストアは Warp の認証や bundle 識別子に紐付いている
場合があります。

### コピー対象

セクション 1 と同じ 8 項目です:

| ファイル / ディレクトリ | カテゴリー | 役割 |
|---|---|---|
| `settings.toml` | config | 公開設定(TOML ベースの設定ファイル)。 |
| `keybindings.yaml` | config | カスタムキーバインド。 |
| `themes/` | data | カスタムテーマ。 |
| `workflows/` | data | カスタムワークフロー。 |
| `launch_configurations/` | data | Launch 設定。 |
| `tab_configs/` | data | Tab 設定。 |
| `.mcp.json` | home dotfile | MCP サーバー設定。 |
| `skills/` | home dotfile | Agent skills。 |

### コピー**してはいけない**もの

- **`user_preferences.json`** —— macOS の
  `~/Library/Application Support/dev.warp.Warp/`(Linux / Windows ではそれに
  相当する state ディレクトリ)にあるプライベートストアで、ユーザー設定・
  認証トークン・マシン依存 ID・クラウドキャッシュが混在しています。ファイル
  ごとコピーすると ID 情報が漏れたり、InfiniShell の認証状態が壊れたりします。
  InfiniShell のデフォルトはもともとプライバシー優先なので、**触らないでください**。
- **`warp.sqlite`**(および `-wal` / `-shm` のサイドカー)—— schema が上流
  Warp に結合しており、InfiniShell の migration が通る保証はありません。
- **Keychain / DPAPI / libsecret のエントリ** —— Warp の bundle / service 名に
  紐付いており、InfiniShell からは利用できません。

### 手順

> コピー前に **Warp と InfiniShell を終了**してください。

**macOS**

```sh
mkdir -p "$HOME/.infinishell"
for f in settings.toml keybindings.yaml themes workflows launch_configurations tab_configs skills .mcp.json; do
  if [ -e "$HOME/.warp/$f" ] && [ ! -e "$HOME/.infinishell/$f" ]; then
    cp -R "$HOME/.warp/$f" "$HOME/.infinishell/$f"
  fi
done
```

**Linux**

```sh
src_config="${XDG_CONFIG_HOME:-$HOME/.config}/warp-terminal"
src_data="${XDG_DATA_HOME:-$HOME/.local/share}/warp-terminal"
src_home="$HOME/.warp"

dst_config="${XDG_CONFIG_HOME:-$HOME/.config}/infinishell"
dst_data="${XDG_DATA_HOME:-$HOME/.local/share}/infinishell"
dst_home="$HOME/.infinishell"
mkdir -p "$dst_config" "$dst_data" "$dst_home"

copy() {
  if [ -e "$1/$3" ] && [ ! -e "$2/$3" ]; then
    cp -R "$1/$3" "$2/$3"
  fi
}

copy "$src_config" "$dst_config" settings.toml
copy "$src_config" "$dst_config" keybindings.yaml
copy "$src_data"   "$dst_data"   themes
copy "$src_data"   "$dst_data"   workflows
copy "$src_data"   "$dst_data"   launch_configurations
copy "$src_data"   "$dst_data"   tab_configs
copy "$src_home"   "$dst_home"   .mcp.json
copy "$src_home"   "$dst_home"   skills
```

**Windows(PowerShell)**

```powershell
$src_config = "$env:LOCALAPPDATA\warp\Warp-Terminal\config"
$src_data   = "$env:APPDATA\warp\Warp-Terminal\data"
$src_home   = "$env:USERPROFILE\.warp"

$dst_config = "$env:LOCALAPPDATA\infinishell\InfiniShell\config"
$dst_data   = "$env:APPDATA\infinishell\InfiniShell\data"
$dst_home   = "$env:USERPROFILE\.infinishell"
New-Item -ItemType Directory -Force -Path $dst_config, $dst_data, $dst_home | Out-Null

function Copy-IfMissing($srcDir, $dstDir, $name) {
  $from = Join-Path $srcDir $name
  $to   = Join-Path $dstDir $name
  if ((Test-Path $from) -and -not (Test-Path $to)) {
    Copy-Item -Path $from -Destination $to -Recurse
  }
}

Copy-IfMissing $src_config $dst_config settings.toml
Copy-IfMissing $src_config $dst_config keybindings.yaml
Copy-IfMissing $src_data   $dst_data   themes
Copy-IfMissing $src_data   $dst_data   workflows
Copy-IfMissing $src_data   $dst_data   launch_configurations
Copy-IfMissing $src_data   $dst_data   tab_configs
Copy-IfMissing $src_home   $dst_home   .mcp.json
Copy-IfMissing $src_home   $dst_home   skills
```

Warp 自体のデータは一切手を加えません —— Warp 本体はそのまま使い続けられます。

---

## 確認方法

InfiniShell を起動すると、テーマピッカーにカスタムテーマ、キーバインドエディタに
カスタムキーバインド、ワークフローランチャーにカスタムワークフローが見える
はずです。`settings.toml` に存在するキーは、設定 UI 上でも移行元の値が反映
されます。

うまく反映されない場合、原因は上記 8 ファイルのいずれかに含まれています ——
テキストエディタで開いて確認するか、削除して InfiniShell のデフォルトに戻して
ください。

## ロールバック

本書の操作はいずれも破壊的ではありません。コピーしたファイルはどれも InfiniShell が
起動時にデフォルトから再生成できるものです。すべてを元に戻すには:

```sh
# macOS
rm -rf ~/.infinishell
```

```sh
# Linux
rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/infinishell"
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/infinishell"
rm -rf "$HOME/.infinishell"
```

```powershell
# Windows
Remove-Item -Recurse -Force "$env:APPDATA\infinishell"
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\infinishell"
Remove-Item -Recurse -Force "$env:USERPROFILE\.infinishell"
```

OpenWarp および Warp のソースディレクトリは、いずれも本ガイドからは変更されません。
