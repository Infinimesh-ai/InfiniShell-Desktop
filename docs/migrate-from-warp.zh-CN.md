# 迁移设置到 InfiniShell

[English](./migrate-from-warp.md)

本文给希望把**设置类配置**(自定义快捷键、主题、工作流、MCP 配置等)从历史安装
带到 InfiniShell 的用户。

可能的"源端"有两种,**两者的安全等级不同**,本文分两节说明。如果两边都有,
**请先迁 OpenWarp,再考虑迁 Warp**。

1. **OpenWarp** —— InfiniShell 自己之前的名字。
2. **上游 [Warp](https://github.com/warpdotdev/warp)** —— InfiniShell 所 fork 的项目。

本文**有意不覆盖**命令历史、SQLite 数据库、Drive 对象,以及系统密钥库中的凭证。这些要么
绑定到本机(Keychain / DPAPI / libsecret),要么 schema 与对方强耦合,跨过来
并不安全。

> **从 Zap 升上来的用户。** Zap 是 InfiniShell 的上一个名字。InfiniShell
> **不会**自动搬迁旧的 `.zap` / `dev.zap.Zap` 目录,而是直接使用全新的
> `.infinishell` / `dev.infinishell.InfiniShell` 配置。想把设置带过来,请按
> 下面的步骤手动迁移,并把源路径改为该 Zap 安装实际使用的位置。

---

## 磁盘布局总览

InfiniShell(以及 OpenWarp / 上游 Warp)把磁盘状态分成**三类目录**:

- **config** —— `settings.toml`、`keybindings.yaml`
- **data** —— `themes/`、`workflows/`、`launch_configurations/`、`tab_configs/`
- **home dotfile** —— `.mcp.json`、`skills/`

macOS 上三类目录都收敛到同一个 home dotfile 目录(`~/.warp/`、`~/.openwarp/`
或 `~/.infinishell/`);Linux 上按 XDG 规范分到**三个不同的位置**,Windows 上按
`directories` crate 的等价布局分。下面的迁移脚本会按平台把每个文件放到
正确的目标。

### InfiniShell 目标路径

| 类别 | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.infinishell/` | `${XDG_CONFIG_HOME:-~/.config}/infinishell/` | `%LOCALAPPDATA%\infinishell\InfiniShell\config\` |
| data | `~/.infinishell/` | `${XDG_DATA_HOME:-~/.local/share}/infinishell/` | `%APPDATA%\infinishell\InfiniShell\data\` |
| home dotfile | `~/.infinishell/` | `~/.infinishell/` | `%USERPROFILE%\.infinishell\` |

### OpenWarp 源路径

| 类别 | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.openwarp/` | `${XDG_CONFIG_HOME:-~/.config}/openwarp/` | `%LOCALAPPDATA%\openwarp\OpenWarp\config\` |
| data | `~/.openwarp/` | `${XDG_DATA_HOME:-~/.local/share}/openwarp/` | `%APPDATA%\openwarp\OpenWarp\data\` |
| home dotfile | `~/.openwarp/` | `~/.openwarp/` | `%USERPROFILE%\.openwarp\` |

### 上游 Warp 源路径

| 类别 | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.warp/` | `${XDG_CONFIG_HOME:-~/.config}/warp-terminal/` | `%LOCALAPPDATA%\warp\Warp-Terminal\config\` |
| data | `~/.warp/` | `${XDG_DATA_HOME:-~/.local/share}/warp-terminal/` | `%APPDATA%\warp\Warp-Terminal\data\` |
| home dotfile | `~/.warp/` | `~/.warp/` | `%USERPROFILE%\.warp\` |

> Linux 下目录名 `warp-terminal` 与 Linux 软件包名一致(例如 Debian/Ubuntu 下
> `/opt/warpdotdev/warp-terminal/`)。Windows 上的 organization 文件夹名可能因
> 打包方式而异;如果你在 `%APPDATA%\warp\Warp-Terminal`(或 `%LOCALAPPDATA%\warp\Warp-Terminal`)
> 找不到,请检查 Warp 实际使用的 `%APPDATA%` / `%LOCALAPPDATA%` 路径。

---

## 拷贝前:备份并记录本次改动

先退出两边应用。检查 `.mcp.json`、工作流和设置文件:即使不迁移系统密钥库,
这些文本文件仍可能包含凭证或私有端点。备份应留在 Git 和共享目录之外。

**每次迁移只执行一次**对应平台的准备代码,再在**同一个终端会话**里执行下方
一个源端的复制步骤。准备代码会备份既有目标目录,并记录本次新增的路径;
复制时跳过所有已存在的目标项。已有目录会整项跳过,不会合并内部文件。
不要删除这个跳过条件。

保留代码输出的备份路径。复制失败时立即停止,先检查备份与记录,不要对部分导入项直接重跑。
自动示例拒绝源、目标和备份路径的链接祖先、候选树中的符号链接及 Windows 重解析点;
读取或遍历失败也会停止。包含这些链接的配置需要单独人工迁移,不能取消检查或直接复制链接。

**macOS / Linux — Bash**

```bash
set -e
umask 077
assert_migration_path() {
  local checked="$1" links
  case "$checked" in
    /*) ;;
    *) printf '%s\n' '迁移路径必须是绝对路径' >&2; return 1 ;;
  esac
  while :; do
    if [ -L "$checked" ]; then
      printf '拒绝符号链接: %s\n' "$checked" >&2; return 1
    fi
    if [ -d "$checked" ] && { [ ! -r "$checked" ] || [ ! -x "$checked" ]; }; then
      printf '目录不可读取或遍历: %s\n' "$checked" >&2; return 1
    fi
    [ "$checked" = / ] && break
    checked="$(dirname "$checked")"
  done
  if [ -e "$1" ]; then
    links="$(find -P "$1" -type l -exec printf x \;)" || return 1
    if [ -n "$links" ]; then
      printf '候选目录含符号链接: %s\n' "$1" >&2; return 1
    fi
  fi
}

migration_backup_root="$HOME/InfiniShell-Migration-Backups"
assert_migration_path "$migration_backup_root"
mkdir -p "$migration_backup_root"
migration_backup="$(mktemp -d "$migration_backup_root/run-XXXXXXXX")"
mkdir "$migration_backup/before"
: > "$migration_backup/added-paths.nul"

case "$(uname -s)" in
  Darwin)
    dst_config="$HOME/.infinishell"
    dst_data="$dst_config"
    ;;
  Linux)
    dst_config="${XDG_CONFIG_HOME:-$HOME/.config}/infinishell"
    dst_data="${XDG_DATA_HOME:-$HOME/.local/share}/infinishell"
    ;;
  *) printf '%s\n' '此脚本仅适用于 macOS / Linux'; exit 1 ;;
esac
dst_home="$HOME/.infinishell"

backup_root() {
  assert_migration_path "$1" || return 1
  if [ -e "$1" ]; then cp -Rp "$1" "$migration_backup/before/$2"; fi
}
backup_root "$dst_config" config
if [ "$dst_data" != "$dst_config" ]; then backup_root "$dst_data" data; fi
if [ "$dst_home" != "$dst_config" ] && [ "$dst_home" != "$dst_data" ]; then
  backup_root "$dst_home" home
fi
mkdir -p "$dst_config" "$dst_data" "$dst_home"

copy_if_missing() {
  local source="$1/$3" destination="$2/$3"
  assert_migration_path "$source" || return 1
  assert_migration_path "$destination" || return 1
  if [ -e "$source" ] && [ ! -e "$destination" ]; then
    # 先记录路径,中途复制失败时也能保留并回滚部分内容。
    printf '%s\0' "$destination" >> "$migration_backup/added-paths.nul" || return 1
    cp -Rp "$source" "$destination"
  fi
}
printf '迁移备份: %s\n' "$migration_backup"
```

**Windows — PowerShell**

```powershell
$ErrorActionPreference = 'Stop'
function Assert-MigrationPath([string]$literalPath) {
  if (-not [IO.Path]::IsPathRooted($literalPath)) { throw '迁移路径必须是绝对路径' }
  $ancestors = [Collections.Generic.Stack[string]]::new()
  $current = $literalPath
  while ($current) {
    $ancestors.Push($current)
    $parent = Split-Path -Parent $current
    if ($parent -eq $current) { break }
    $current = $parent
  }
  $item = $null
  while ($ancestors.Count) {
    $current = $ancestors.Pop()
    $parentItem = $item
    try { $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop }
    catch [System.Management.Automation.ItemNotFoundException] {
      $item = $null
      if ($parentItem -is [IO.DirectoryInfo]) {
        $leaf = Split-Path -Leaf $current
        $item = Get-ChildItem -LiteralPath $parentItem.FullName -Force -ErrorAction Stop |
          Where-Object { $_.Name -eq $leaf }
      }
    }
    if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
      throw "拒绝链接或重解析点: $current"
    }
  }
  if (-not $item) { return }
  $pending = [Collections.Generic.Stack[IO.FileSystemInfo]]::new()
  $pending.Push($item)
  while ($pending.Count) {
    $item = $pending.Pop()
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
      throw "拒绝链接或重解析点: $($item.FullName)"
    }
    if ($item -is [IO.DirectoryInfo]) {
      foreach ($child in Get-ChildItem -LiteralPath $item.FullName -Force -ErrorAction Stop) {
        $pending.Push($child)
      }
    }
  }
}

$migrationBackupRoot = Join-Path $env:USERPROFILE 'InfiniShell-Migration-Backups'
Assert-MigrationPath $migrationBackupRoot
$migrationBackup = Join-Path $migrationBackupRoot ("run-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path "$migrationBackup\before" | Out-Null
$migrationAdded = @()
ConvertTo-Json -InputObject $migrationAdded | Set-Content -LiteralPath "$migrationBackup\added-paths.json" -Encoding UTF8

$dst_config = "$env:LOCALAPPDATA\infinishell\InfiniShell\config"
$dst_data = "$env:APPDATA\infinishell\InfiniShell\data"
$dst_home = "$env:USERPROFILE\.infinishell"
foreach ($entry in @{ config = $dst_config; data = $dst_data; home = $dst_home }.GetEnumerator()) {
  Assert-MigrationPath $entry.Value
  if (Test-Path -LiteralPath $entry.Value) {
    Copy-Item -LiteralPath $entry.Value -Destination (Join-Path "$migrationBackup\before" $entry.Key) -Recurse
  }
}
New-Item -ItemType Directory -Force -Path $dst_config, $dst_data, $dst_home | Out-Null

function Copy-IfMissing($srcDir, $dstDir, $name) {
  $from = Join-Path $srcDir $name
  $to = Join-Path $dstDir $name
  Assert-MigrationPath $from
  Assert-MigrationPath $to
  if ((Test-Path -LiteralPath $from) -and -not (Test-Path -LiteralPath $to)) {
    # 先记录路径,中途复制失败时也能保留并回滚部分内容。
    $script:migrationAdded += $to
    ConvertTo-Json -InputObject $script:migrationAdded | Set-Content -LiteralPath "$migrationBackup\added-paths.json" -Encoding UTF8
    Copy-Item -LiteralPath $from -Destination $to -Recurse
  }
}
Write-Output "迁移备份: $migrationBackup"
```

---

## 1. 从 OpenWarp 迁过来(老用户推荐路径)

OpenWarp 是本项目历史上使用过的名称。下面的文件可以作为迁移候选,实际兼容性
仍取决于源版本。拷贝前请复核自定义命令、MCP server 设置和提供商选项。

### 可以拷的内容

| 文件 / 目录 | 类别 | 控制什么 |
|---|---|---|
| `settings.toml` | config | 公开设置(TOML 设置文件)。 |
| `keybindings.yaml` | config | 自定义快捷键。 |
| `themes/` | data | 自定义主题。 |
| `workflows/` | data | 自定义 workflow。 |
| `launch_configurations/` | data | Launch 配置。 |
| `tab_configs/` | data | Tab 配置。 |
| `.mcp.json` | home dotfile | MCP server 配置。 |
| `skills/` | home dotfile | Agent skills。 |

### 操作步骤

> 先退出两边应用,执行上方对应平台的准备代码。

**macOS**

```bash
src_config="$HOME/.openwarp"
src_data="$src_config"
src_home="$src_config"

copy_if_missing "$src_config" "$dst_config" settings.toml
copy_if_missing "$src_config" "$dst_config" keybindings.yaml
copy_if_missing "$src_data" "$dst_data" themes
copy_if_missing "$src_data" "$dst_data" workflows
copy_if_missing "$src_data" "$dst_data" launch_configurations
copy_if_missing "$src_data" "$dst_data" tab_configs
copy_if_missing "$src_home" "$dst_home" .mcp.json
copy_if_missing "$src_home" "$dst_home" skills
```

**Linux**

```bash
src_config="${XDG_CONFIG_HOME:-$HOME/.config}/openwarp"
src_data="${XDG_DATA_HOME:-$HOME/.local/share}/openwarp"
src_home="$HOME/.openwarp"

copy_if_missing "$src_config" "$dst_config" settings.toml
copy_if_missing "$src_config" "$dst_config" keybindings.yaml
copy_if_missing "$src_data" "$dst_data" themes
copy_if_missing "$src_data" "$dst_data" workflows
copy_if_missing "$src_data" "$dst_data" launch_configurations
copy_if_missing "$src_data" "$dst_data" tab_configs
copy_if_missing "$src_home" "$dst_home" .mcp.json
copy_if_missing "$src_home" "$dst_home" skills
```

**Windows(PowerShell)**

```powershell
$src_config = "$env:LOCALAPPDATA\openwarp\OpenWarp\config"
$src_data = "$env:APPDATA\openwarp\OpenWarp\data"
$src_home = "$env:USERPROFILE\.openwarp"

Copy-IfMissing $src_config $dst_config settings.toml
Copy-IfMissing $src_config $dst_config keybindings.yaml
Copy-IfMissing $src_data $dst_data themes
Copy-IfMissing $src_data $dst_data workflows
Copy-IfMissing $src_data $dst_data launch_configurations
Copy-IfMissing $src_data $dst_data tab_configs
Copy-IfMissing $src_home $dst_home .mcp.json
Copy-IfMissing $src_home $dst_home skills
```

已有 InfiniShell 文件保持不变。若需要合并已有文件或目录,请先逐项比较和备份,
再手动编辑;上述命令不会合并。旧安装目录应继续保留,直到确认没有应用仍需使用它们。

---

## 2. 从上游 Warp 迁过来

上游 Warp 是另一个独立产品,有自己的磁盘身份(见上面"上游 Warp 源路径"表)。
InfiniShell 编译时 channel = `Oss`,对应独立的 app id(`dev.infinishell.InfiniShell`)和按平台分开的
目录布局。两边默认使用不同目录;这是存储约定,不是同一用户下不同进程之间的访问控制边界。

下表中的文本文件可以作为迁移候选,需要按已安装版本复核。Warp 独立演进,
二进制 / 私有存储可能绑定到 Warp 的认证和 bundle 身份。

### 可以拷的内容

和第 1 节相同的 8 项:

| 文件 / 目录 | 类别 | 控制什么 |
|---|---|---|
| `settings.toml` | config | 公开设置(TOML 设置文件)。 |
| `keybindings.yaml` | config | 自定义快捷键。 |
| `themes/` | data | 自定义主题。 |
| `workflows/` | data | 自定义 workflow。 |
| `launch_configurations/` | data | Launch 配置。 |
| `tab_configs/` | data | Tab 配置。 |
| `.mcp.json` | home dotfile | MCP server 配置。 |
| `skills/` | home dotfile | Agent skills。 |

### **不要**拷的内容

- **`user_preferences.json`** —— 这是私有存储,位于 macOS 上的
  `~/Library/Application Support/dev.warp.Warp/`(Linux / Windows 对应的 state
  目录),里面混杂了用户偏好、登录 token、机器绑定 ID 和云端缓存状态。整文件
  拷过去会泄漏身份信息,也会让 InfiniShell 误判登录状态。InfiniShell 默认值本身已经是隐私
  优先的,**不要碰它**。
- **`warp.sqlite`**(以及 `-wal` / `-shm` 伴生文件)—— schema 与上游 Warp 耦合,
  不保证能跑 InfiniShell 的 migrations。
- **Keychain / DPAPI / libsecret 中的条目** —— 绑定到 Warp 的 bundle / service
  名,对 InfiniShell 没有意义。

### 操作步骤

> 先退出两边应用,执行上方对应平台的准备代码。

**macOS**

```bash
src_config="$HOME/.warp"
src_data="$src_config"
src_home="$src_config"

copy_if_missing "$src_config" "$dst_config" settings.toml
copy_if_missing "$src_config" "$dst_config" keybindings.yaml
copy_if_missing "$src_data" "$dst_data" themes
copy_if_missing "$src_data" "$dst_data" workflows
copy_if_missing "$src_data" "$dst_data" launch_configurations
copy_if_missing "$src_data" "$dst_data" tab_configs
copy_if_missing "$src_home" "$dst_home" .mcp.json
copy_if_missing "$src_home" "$dst_home" skills
```

**Linux**

```bash
src_config="${XDG_CONFIG_HOME:-$HOME/.config}/warp-terminal"
src_data="${XDG_DATA_HOME:-$HOME/.local/share}/warp-terminal"
src_home="$HOME/.warp"

copy_if_missing "$src_config" "$dst_config" settings.toml
copy_if_missing "$src_config" "$dst_config" keybindings.yaml
copy_if_missing "$src_data" "$dst_data" themes
copy_if_missing "$src_data" "$dst_data" workflows
copy_if_missing "$src_data" "$dst_data" launch_configurations
copy_if_missing "$src_data" "$dst_data" tab_configs
copy_if_missing "$src_home" "$dst_home" .mcp.json
copy_if_missing "$src_home" "$dst_home" skills
```

**Windows(PowerShell)**

```powershell
$src_config = "$env:LOCALAPPDATA\warp\Warp-Terminal\config"
$src_data = "$env:APPDATA\warp\Warp-Terminal\data"
$src_home = "$env:USERPROFILE\.warp"

Copy-IfMissing $src_config $dst_config settings.toml
Copy-IfMissing $src_config $dst_config keybindings.yaml
Copy-IfMissing $src_data $dst_data themes
Copy-IfMissing $src_data $dst_data workflows
Copy-IfMissing $src_data $dst_data launch_configurations
Copy-IfMissing $src_data $dst_data tab_configs
Copy-IfMissing $src_home $dst_home .mcp.json
Copy-IfMissing $src_home $dst_home skills
```

Warp 自己的数据从始至终不会被改动,Warp 本体继续可用。

---

## 验证

启动 InfiniShell,检查导入的主题、快捷键、工作流与设置。API 密钥需要在提供商
设置界面重新输入;它们保存在系统密钥库中,本次不会迁移。发现缺失或不兼容的
设置时,先与原件及备份比较再修改。验证完成前保留源安装目录与备份。

## 只回滚本次迁移

先退出 InfiniShell。以下命令只把**本次记录为新增的路径**移入备份下一个新的
回滚目录,不会删除整个配置根目录,也不会替换迁移前就存在的文件。
如果导入项后来又被编辑,这些新内容会一起移入回滚目录并继续保留。
回滚会先把新增路径记录移入本次回滚目录,只允许领取一次;第二次执行会拒绝。
保留 `original-paths` 记录,以便将编号项对应回原路径。若中途失败,已移动项与记录都留在
该回滚目录;停止自动操作,按原路径与编号逐项人工核对剩余内容,不要把记录移回去重放。

**macOS / Linux — Bash**

```bash
set -e
umask 077
assert_migration_path() {
  local checked="$1" links
  case "$checked" in
    /*) ;;
    *) printf '%s\n' '迁移路径必须是绝对路径' >&2; return 1 ;;
  esac
  while :; do
    if [ -L "$checked" ]; then
      printf '拒绝符号链接: %s\n' "$checked" >&2; return 1
    fi
    if [ -d "$checked" ] && { [ ! -r "$checked" ] || [ ! -x "$checked" ]; }; then
      printf '目录不可读取或遍历: %s\n' "$checked" >&2; return 1
    fi
    [ "$checked" = / ] && break
    checked="$(dirname "$checked")"
  done
  if [ -e "$1" ]; then
    links="$(find -P "$1" -type l -exec printf x \;)" || return 1
    if [ -n "$links" ]; then
      printf '候选目录含符号链接: %s\n' "$1" >&2; return 1
    fi
  fi
}

# 将路径改为本次准备步骤输出的备份目录。
migration_backup="$HOME/InfiniShell-Migration-Backups/run-REPLACE_ME"
assert_migration_path "$migration_backup"
[ -f "$migration_backup/added-paths.nul" ] || {
  printf '%s\n' '新增路径记录不存在或已被领取,停止自动回滚' >&2; exit 1
}
migration_rollback="$(mktemp -d "$migration_backup/rollback-XXXXXXXX")"
# 先领取记录;部分失败后保留此目录,只允许人工核对剩余项。
mv "$migration_backup/added-paths.nul" "$migration_rollback/original-paths.nul"
printf '回滚记录: %s\n' "$migration_rollback"
migration_index=0
while IFS= read -r -d '' migration_path; do
  migration_index=$((migration_index + 1))
  assert_migration_path "$migration_path"
  if [ -e "$migration_path" ]; then
    mv "$migration_path" "$migration_rollback/$migration_index"
  fi
done < "$migration_rollback/original-paths.nul"
printf '本次新增内容已保留到: %s\n' "$migration_rollback"
```

**Windows — PowerShell**

```powershell
$ErrorActionPreference = 'Stop'
function Assert-MigrationPath([string]$literalPath) {
  if (-not [IO.Path]::IsPathRooted($literalPath)) { throw '迁移路径必须是绝对路径' }
  $ancestors = [Collections.Generic.Stack[string]]::new()
  $current = $literalPath
  while ($current) {
    $ancestors.Push($current)
    $parent = Split-Path -Parent $current
    if ($parent -eq $current) { break }
    $current = $parent
  }
  $item = $null
  while ($ancestors.Count) {
    $current = $ancestors.Pop()
    $parentItem = $item
    try { $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop }
    catch [System.Management.Automation.ItemNotFoundException] {
      $item = $null
      if ($parentItem -is [IO.DirectoryInfo]) {
        $leaf = Split-Path -Leaf $current
        $item = Get-ChildItem -LiteralPath $parentItem.FullName -Force -ErrorAction Stop |
          Where-Object { $_.Name -eq $leaf }
      }
    }
    if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
      throw "拒绝链接或重解析点: $current"
    }
  }
  if (-not $item) { return }
  $pending = [Collections.Generic.Stack[IO.FileSystemInfo]]::new()
  $pending.Push($item)
  while ($pending.Count) {
    $item = $pending.Pop()
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
      throw "拒绝链接或重解析点: $($item.FullName)"
    }
    if ($item -is [IO.DirectoryInfo]) {
      foreach ($child in Get-ChildItem -LiteralPath $item.FullName -Force -ErrorAction Stop) {
        $pending.Push($child)
      }
    }
  }
}

# 将路径改为本次准备步骤输出的备份目录。
$migrationBackup = "$env:USERPROFILE\InfiniShell-Migration-Backups\run-REPLACE_ME"
Assert-MigrationPath $migrationBackup
$migrationManifest = Join-Path $migrationBackup 'added-paths.json'
if (-not (Test-Path -LiteralPath $migrationManifest -PathType Leaf)) {
  throw '新增路径记录不存在或已被领取,停止自动回滚'
}
$migrationRollback = Join-Path $migrationBackup ("rollback-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $migrationRollback | Out-Null
# 先领取记录;部分失败后保留此目录,只允许人工核对剩余项。
Move-Item -LiteralPath $migrationManifest -Destination "$migrationRollback\original-paths.json"
Write-Output "回滚记录: $migrationRollback"
$migrationPaths = @(Get-Content -LiteralPath "$migrationRollback\original-paths.json" -Raw | ConvertFrom-Json)
$migrationIndex = 0
foreach ($migrationPath in $migrationPaths) {
  $migrationIndex++
  Assert-MigrationPath $migrationPath
  if (Test-Path -LiteralPath $migrationPath) {
    Move-Item -LiteralPath $migrationPath -Destination (Join-Path $migrationRollback "$migrationIndex")
  }
}
Write-Output "本次新增内容已保留到: $migrationRollback"
```

如果复制后手动改过原有文件,请先保留它的当前版本,再与 `before/` 中的备份逐项
比较,仅恢复需要撤销的改动。上述命令不会撤销这些手工修改。若缺少同一次迁移的
新增路径记录,请停止自动回滚并人工比较文件,不要删除整个 InfiniShell、OpenWarp
或 Warp 目录。上述命令始终不会修改源端目录。
