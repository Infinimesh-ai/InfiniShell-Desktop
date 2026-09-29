#!/usr/bin/env bash
# 在隔离 Xvfb 中采集固定 Grok 的英中静态视口；审批状态不在本脚本验收范围。
set -euo pipefail

binary="${1:?需要 GUI integration 可执行文件}"
grok="${INFINISHELL_TEST_GROK_EXE:?需要固定 Grok 文件}"
root="${WARP_INTEGRATION_TEST_ARTIFACTS_DIR:?需要证据根目录}"
source_commit="${WARP_TEST_GUI_SOURCE_COMMIT:?需要完整源码提交}"
[[ -f "$binary" && -x "$binary" && -f "$grok" && -x "$grok" ]] || {
  printf 'GUI 或 Grok 可执行文件不存在\n' >&2
  exit 1
}
[[ "$(basename "$grok")" == grok ]] || {
  printf '固定 Grok 文件名错误\n' >&2
  exit 1
}
mkdir -p "$root"

install_gui_dependencies() {
  local attempt
  local -a apt_options=(-o Acquire::Retries=2 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
  if sudo -n apt-get "${apt_options[@]}" install -y "$@"; then
    return 0
  fi
  for attempt in 1 2 3; do
    if timeout 120s sudo -n apt-get "${apt_options[@]}" -o Acquire::By-Hash=force -o APT::Update::Error-Mode=any update -q; then
      sudo -n apt-get "${apt_options[@]}" install -y "$@"
      return $?
    fi
    if [[ "$attempt" -lt 3 ]]; then sleep 5; fi
  done
  return 1
}

if ! command -v xvfb-run >/dev/null ||
  ! python3 -c 'import ctypes; [ctypes.CDLL(name) for name in ("libX11.so.6", "libXcursor.so.1", "libX11-xcb.so.1", "libXi.so.6", "libxkbcommon.so.0", "libxkbcommon-x11.so.0")]'; then
  install_gui_dependencies xvfb xauth libgl1-mesa-dri mesa-vulkan-drivers libx11-6 libxcursor1 libx11-xcb1 libxi6 libxkbcommon0 libxkbcommon-x11-0
fi
if ! command -v fc-list >/dev/null ||
  [[ "$(dpkg-query -W -f='${db:Status-Status}' fonts-noto-cjk 2>/dev/null || true)" != installed ]]; then
  install_gui_dependencies fontconfig fonts-noto-cjk
fi
fc-cache -f
for codepoint in 4e2d 6587 7b2c 4e00 884c 4e8c 56fa 5b9a 8bfb 53d6 6280 80fd 7981 7528; do
  [[ -n "$(fc-list --format='%{family}\n' ":charset=$codepoint")" ]] || {
    printf '缺少 V05 字形 U+%s\n' "$codepoint" >&2
    exit 1
  }
done

grok_dir="$(dirname "$grok")"
for locale in en zh-CN; do
  for size in compact normal; do
    output="$root/$locale/$size"
    mkdir -p "$output"
    # Grok 目录只进入 integration 子进程的 PATH，不改变 runner 或 Xvfb 的 PATH。
    timeout 180s xvfb-run -a -s '-screen 0 1600x1000x24' \
      env "PATH=$grok_dir:$PATH" \
      WARP_INTEGRATION=1 WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
      "WARP_TEST_GUI_LOCALE=$locale" "WARP_TEST_GUI_SIZE=$size" \
      "WARP_TEST_GUI_SOURCE_COMMIT=$source_commit" \
      "WARP_INTEGRATION_TEST_ARTIFACTS_DIR=$output" \
      "$binary" test_cli_grok_static_viewport
    python3 -B script/cli-agent-parity/verify_grok_static_viewport.py \
      --root "$output" --binary "$binary" --grok "$grok" \
      --source "$source_commit" --platform linux --locale "$locale" --size "$size"
  done
done
