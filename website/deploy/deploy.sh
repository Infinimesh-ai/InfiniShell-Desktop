#!/usr/bin/env bash
# InfiniShell 官网部署脚本(在服务器上执行)
#
# 场景:站点由 1Panel 托管,nginx root 固定指向站点目录(例如
#   /opt/1panel/www/sites/infinishell.zerx.dev/index),无法改 root 用 symlink 方案。
# 故这里做"原子目录替换":CI 已把最新产物 rsync 到 <site>/.incoming,
# 本脚本把 .incoming 重命名为正式 index 目录,并保留上一版用于回滚。
#
# 约定布局($DEPLOY_PATH = 站点 index 目录,例如 .../infinishell.zerx.dev/index):
#   <parent>/.incoming   ← CI rsync 推上来的最新 dist 内容
#   <parent>/index       ← nginx 实际服务的目录(= $DEPLOY_PATH)
#   <parent>/.index.bak  ← 上一版,用于回滚
set -euo pipefail

# $DEPLOY_PATH 由调用方(SSH 命令)以第一个参数传入,即正式 index 目录
INDEX="${1:?用法: deploy.sh <site-index-dir>}"
PARENT="$(dirname "$INDEX")"
INCOMING="$PARENT/.incoming"
BACKUP="$PARENT/.index.bak"

log() { printf '[deploy] %s\n' "$*"; }

if [ ! -d "$INCOMING" ] || [ -z "$(ls -A "$INCOMING" 2>/dev/null)" ]; then
  log "错误:$INCOMING 不存在或为空,没有可发布的产物(先由 CI rsync 到 .incoming)。"
  exit 1
fi

# 删除上一版备份,把当前 index 移为备份(若存在)
if [ -e "$INDEX" ]; then
  log "备份当前版本 -> $BACKUP"
  rm -rf "$BACKUP"
  mv -Tf "$INDEX" "$BACKUP"
fi

# 原子上线:rename .incoming -> index(同一文件系统,mv 为原子操作)
log "发布新版本 -> $INDEX"
mv -Tf "$INCOMING" "$INDEX"

log "完成。当前服务目录:$INDEX"
log "如需回滚:rm -rf '$INDEX' && mv -Tf '$BACKUP' '$INDEX'"
