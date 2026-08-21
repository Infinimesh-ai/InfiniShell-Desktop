#!/usr/bin/env bash
# 注册 InfiniShell 自定义合并驱动 + 启用 rerere。
# 第一次 clone 后跑一次,后续合并上游(merge / cherry-pick / rebase)就会:
# 1. .gitattributes 中标了 merge=infinishell-ours 的路径自动保留本地版本
# 2. rerere 记录每次冲突解析,下次相同冲突自动复用
set -euo pipefail

git config merge.infinishell-ours.name "Always keep InfiniShell version (custom driver)"
git config merge.infinishell-ours.driver true
git config rerere.enabled true
git config rerere.autoupdate true

echo "InfiniShell merge drivers + rerere configured."
echo "  rerere.enabled        = $(git config --get rerere.enabled)"
echo "  rerere.autoupdate     = $(git config --get rerere.autoupdate)"
echo "  merge.infinishell-ours   = $(git config --get merge.infinishell-ours.driver)"
