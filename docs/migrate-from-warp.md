# Migrating settings to InfiniShell

[简体中文](./migrate-from-warp.zh-CN.md)

This guide is for people who want to bring **settings-style configuration**
(custom keybindings, themes, workflows, MCP config, etc.) into InfiniShell from a
previous install.

There are two source installs this might apply to:

1. **OpenWarp** — InfiniShell's own previous name.
2. **Upstream [Warp](https://github.com/warpdotdev/warp)** — the project InfiniShell
   is forked from.

The two cases have **different safety profiles** and are covered separately
below. Always migrate from OpenWarp first if both apply.

This guide deliberately does **not** migrate command history, the SQLite
database, Drive objects, or system-keychain credentials. Those live in stores that are
either machine-bound (Keychain / DPAPI / libsecret) or schema-coupled and are
not safe to copy across forks.

> **Coming from Zap?** Zap was InfiniShell's immediately preceding name.
> InfiniShell does **not** migrate `.zap` / `dev.zap.Zap` directories
> automatically — it starts from a clean `.infinishell` /
> `dev.infinishell.InfiniShell` config. To carry settings across, follow the
> steps below, replacing the source paths with the actual locations used by your Zap installation.

---

## How on-disk state is laid out

InfiniShell (and OpenWarp / upstream Warp before it) splits its on-disk state into
**three categories of directory**:

- **config** — `settings.toml`, `keybindings.yaml`
- **data** — `themes/`, `workflows/`, `launch_configurations/`, `tab_configs/`
- **home dotfile** — `.mcp.json`, `skills/`

On macOS all three categories coincide under a single home dotfile directory
(`~/.warp/`, `~/.openwarp/`, or `~/.infinishell/`). On Linux and Windows they live in
**three different places** following XDG conventions on Linux and the
`directories` crate layout on Windows. The migration scripts below take care
of placing each file in the correct destination per platform.

### InfiniShell destination paths

| Category | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.infinishell/` | `${XDG_CONFIG_HOME:-~/.config}/infinishell/` | `%LOCALAPPDATA%\infinishell\InfiniShell\config\` |
| data | `~/.infinishell/` | `${XDG_DATA_HOME:-~/.local/share}/infinishell/` | `%APPDATA%\infinishell\InfiniShell\data\` |
| home dotfile | `~/.infinishell/` | `~/.infinishell/` | `%USERPROFILE%\.infinishell\` |

### OpenWarp source paths

| Category | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.openwarp/` | `${XDG_CONFIG_HOME:-~/.config}/openwarp/` | `%LOCALAPPDATA%\openwarp\OpenWarp\config\` |
| data | `~/.openwarp/` | `${XDG_DATA_HOME:-~/.local/share}/openwarp/` | `%APPDATA%\openwarp\OpenWarp\data\` |
| home dotfile | `~/.openwarp/` | `~/.openwarp/` | `%USERPROFILE%\.openwarp\` |

### Upstream Warp source paths

| Category | macOS | Linux | Windows |
|---|---|---|---|
| config | `~/.warp/` | `${XDG_CONFIG_HOME:-~/.config}/warp-terminal/` | `%LOCALAPPDATA%\warp\Warp-Terminal\config\` |
| data | `~/.warp/` | `${XDG_DATA_HOME:-~/.local/share}/warp-terminal/` | `%APPDATA%\warp\Warp-Terminal\data\` |
| home dotfile | `~/.warp/` | `~/.warp/` | `%USERPROFILE%\.warp\` |

> The Linux directory name `warp-terminal` matches the Linux package name
> (e.g. `/opt/warpdotdev/warp-terminal/` on Debian/Ubuntu). The exact Windows
> organization folder may differ depending on how Warp was packaged; if you
> can't find `%APPDATA%\warp\Warp-Terminal` (or `%LOCALAPPDATA%\warp\Warp-Terminal`),
> check the actual `%APPDATA%` / `%LOCALAPPDATA%` location your Warp install
> uses.

---

## Before copying: back up and record the changes

Quit both applications. Review `.mcp.json`, workflows and settings first: these
text files may contain credentials or private endpoints even though OS-keychain
credentials are not migrated. Keep backups outside Git and shared folders.

Run the preparation block for your platform **once per migration attempt**, then
run exactly one source section below in the **same terminal session**. It backs
up the existing destination directories, creates a manifest of newly copied paths,
and skips every existing destination item. Existing directories are skipped as a
whole; their contents are not merged. Do not remove the skip condition.

Keep the printed backup path. If a copy fails, stop and inspect the backup and
manifest; do not rerun copying over a partially imported item. The examples reject
linked ancestors of source, destination and backup paths, symlinks in candidate
trees, and Windows reparse points. Read or traversal errors also stop the attempt.
Configurations containing such links need a separate manual migration; do not
disable the checks or copy the links directly.

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

## 1. From OpenWarp (recommended path for existing users)

OpenWarp is an earlier name in this project's history. The files below are
candidates for migration; compatibility still depends on the source version.
Review custom commands, MCP server settings and provider options before copying.

### Files to copy

| File or folder | Category | What it controls |
|---|---|---|
| `settings.toml` | config | Public settings (the TOML-backed settings file). |
| `keybindings.yaml` | config | Custom keybindings. |
| `themes/` | data | Custom themes. |
| `workflows/` | data | Custom workflows. |
| `launch_configurations/` | data | Launch configurations. |
| `tab_configs/` | data | Tab configurations. |
| `.mcp.json` | home dotfile | MCP server configuration. |
| `skills/` | home dotfile | Agent skills. |

### Steps

> Run the preparation block above first, with both applications closed.

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

**Windows (PowerShell)**

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

Existing InfiniShell files are preserved. To merge an existing file or directory,
compare it with the source and back up that specific item before editing; the
copy commands above intentionally do not merge it. Keep the old installation's
directories until you have verified that no application still needs them.

---

## 2. From upstream Warp

Upstream Warp is a separate product with its own on-disk identity (see the
"Upstream Warp source paths" table above). InfiniShell is built with channel `Oss`,
which gives it its own app ID (`dev.infinishell.InfiniShell`) and its own per-platform layout.
The applications use separate default directories; this is a storage convention,
not an access-control boundary between processes running as the same user.

The text-format files below are migration candidates and need review against
your installed version. Warp evolves independently, and binary / private stores
can be tied to Warp's authentication and bundle identity.

### What to copy

Same eight items as above:

| File or folder | Category | What it controls |
|---|---|---|
| `settings.toml` | config | Public settings (the TOML-backed settings file). |
| `keybindings.yaml` | config | Custom keybindings. |
| `themes/` | data | Custom themes. |
| `workflows/` | data | Custom workflows. |
| `launch_configurations/` | data | Launch configurations. |
| `tab_configs/` | data | Tab configurations. |
| `.mcp.json` | home dotfile | MCP server configuration. |
| `skills/` | home dotfile | Agent skills. |

### What **not** to copy

- **`user_preferences.json`** — a private store under
  `~/Library/Application Support/dev.warp.Warp/` (macOS) or the equivalent
  state directory on Linux/Windows. Mixes user preferences with auth tokens,
  machine-bound IDs and cached cloud state. Copying it can leak identity and
  confuse InfiniShell's auth state. InfiniShell defaults are already privacy-friendly.
- **`warp.sqlite`** (and its `-wal` / `-shm` sidecars) — schema is coupled
  to upstream Warp and not guaranteed to be compatible with InfiniShell's migrations.
- **Keychain / DPAPI / libsecret entries** — bound to the Warp bundle /
  service name, useless to InfiniShell.

### Steps

> Run the preparation block above first, with both applications closed.

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

**Windows (PowerShell)**

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

Your original Warp data is never touched — Warp itself keeps working.

---

## Verifying

Start InfiniShell and check the imported themes, keybindings, workflows and
settings. Re-enter API keys through the provider settings UI; they are stored in
the OS keychain and are not included in this migration. A setting that is missing
or incompatible should be compared with the original and the backup before editing.
Keep both the source installation and the backup until verification is complete.

## Rolling back only this attempt

Quit InfiniShell first. The following commands move **only paths recorded as new
by this attempt** into a fresh rollback directory inside its backup. They do not
delete the configuration roots or replace files that existed before migration.
Any later edits inside a newly imported item move with it and remain available
in the rollback directory. Rollback first moves the manifest into that directory
and claims it only once; a second invocation is rejected. Keep `original-paths`
to map numbered items back to their original locations. On partial failure, the
manifest and already moved items remain in that rollback directory. Stop automatic
operations and compare the remaining paths and numbered items manually; do not
move the manifest back to replay the rollback.

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

If you manually edited a pre-existing file after copying, compare that specific
file with `before/` and restore only the intended changes after preserving its
current version. The commands above do not undo such edits. Without the manifest
from the same migration attempt, stop and compare files manually; do not delete
entire InfiniShell, OpenWarp or Warp directories. The source directories are never
changed by these commands.
