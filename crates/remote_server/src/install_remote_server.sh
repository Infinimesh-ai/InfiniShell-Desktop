#!/usr/bin/env bash
# 在远端主机安装 Zap CLI 二进制,用于 remote-server-proxy;并将 artifact 的
# `resources/` 树(bundled skills、settings schema)安装到全局无版本目录:
#
#   {install_dir}/
#   ├── {binary_name}{version_suffix}   ← 可执行文件
#   └── bundled_resources/              ← artifact 的 resources 树(如有)
#
# setup.rs 会在运行时替换这些占位符:
#   {download_base_url}          - 例如 https://github.com/zerx-lab/warp/releases/latest/download
#   {install_dir}                - 例如 ~/.zap/remote-server
#   {binary_name}                - 例如 infinishell
#   {version_suffix}             - 例如 -v0.2026...,没有 release tag 时为空
#   {bundled_resources_dir_name} - 全局 resources 目录名(例如 bundled_resources)
#   {no_http_client_exit_code}   - curl/wget 都不可用时的退出码
#   {staging_tarball_path}       - SCP fallback 预上传 tarball 路径,常规下载路径为空
set -e

arch=$(uname -m)
case "$arch" in
  x86_64|amd64)  arch_name=x86_64 ;;
  aarch64|arm64) arch_name=aarch64 ;;
  *) echo "unsupported arch: $arch" >&2; exit 2 ;;
esac

os_kernel=$(uname -s)
case "$os_kernel" in
  Darwin) os_name=macos ;;
  Linux)  os_name=linux ;;
  *) echo "unsupported OS: $os_kernel" >&2; exit 2 ;;
esac

install_dir="{install_dir}"
# Avoid `${var/pattern/replacement}` for tilde expansion. Two
# interpreter quirks make it dangerous in this script:
#   1. bash 3.2 (macOS /bin/bash) keeps inner double-quotes around the
#      replacement literal, so `"$HOME"` ends up as 6 literal
#      characters and the install lands under a directory tree
#      literally named `"`.
#   2. bash 5.2+ enables `patsub_replacement` by default, which makes
#      `&` in the replacement expand to the matched pattern, so a
#      `$HOME` containing `&` resolves to a `~`-substituted path.
# Use `case` + `${var#\~}` instead — works on bash 3.2 and bash 5.2+
# without surprises.
case "$install_dir" in
  "~"|"~/"*) install_dir="${HOME}${install_dir#\~}" ;;
esac
mkdir -p "$install_dir"

tmpdir=$(mktemp -d "$install_dir/.install.XXXXXX")
# 尽力清理 staging 目录。这里失败不能覆盖真正的安装结果:
# trap 触发时二进制要么已经移动到最终路径,要么脚本已经因为
# 其他原因失败,后者的错误更值得暴露给调用方。
cleanup() {
  rm -rf "$tmpdir" 2>/dev/null || true
}
trap cleanup EXIT

staging_tarball_path="{staging_tarball_path}"
if [ -n "$staging_tarball_path" ]; then
  # SCP fallback:tarball 已由客户端预先上传。
  # 与上面 install_dir 相同的波浪号展开注意事项。
  case "$staging_tarball_path" in
    "~"|"~/"*) staging_tarball_path="${HOME}${staging_tarball_path#\~}" ;;
  esac
  mv "$staging_tarball_path" "$tmpdir/zap.tar.gz"
else
  url="{download_base_url}/zap-$os_name-$arch_name.tar.gz"
  if command -v curl >/dev/null 2>&1; then
    curl -fSL --connect-timeout 15 "$url" -o "$tmpdir/zap.tar.gz"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$tmpdir/zap.tar.gz" "$url"
  else
    echo "error: neither curl nor wget is available" >&2
    exit {no_http_client_exit_code}
  fi
fi

tar -xzf "$tmpdir/zap.tar.gz" -C "$tmpdir"

bin="$tmpdir/{binary_name}"
if [ ! -f "$bin" ]; then
  bin=$(find "$tmpdir" -type f \( -name 'infinishell' -o -name 'warp-oss' -o -name 'oz*' \) ! -path '*/resources/*' ! -name '*.tar.gz' | head -n1)
fi
if [ -z "$bin" ]; then echo "no binary found in tarball" >&2; exit 1; fi
chmod +x "$bin"

# 将 resources 树安装到 daemon 读取的全局无版本目录。`$tmpdir` 位于
# `$install_dir` 内,因此 `mv` 是同文件系统 rename。先装 resources 再装
# 二进制:中断的安装不会留下缺 resources 的新二进制 —— 二进制缺失会重新
# 触发本脚本。tarball 不带 resources 不算错误:daemon 只是没有 bundled skills。
resources="$(dirname "$bin")/resources"
if [ -d "$resources" ]; then
  rm -rf "$install_dir/{bundled_resources_dir_name}"
  mv "$resources" "$install_dir/{bundled_resources_dir_name}"
fi

mv "$bin" "$install_dir/{binary_name}{version_suffix}"
