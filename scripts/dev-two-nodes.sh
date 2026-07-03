#!/usr/bin/env bash
# 在同一台 Mac 上跑两个互相隔离的 AA4C 桌面实例，做同步链路本地联调
# （真 mDNS 发现 / 真 TLS / 真 GUI）。用后端的联调钩子 AA4C_DATA_DIR /
# AA4C_DEVICE_NAME（见 apps/desktop/src-tauri/src/lib.rs setup）。
#
# 用法：
#   bash scripts/dev-two-nodes.sh          # 构建当前代码并起 A、B 两个窗口
#   bash scripts/dev-two-nodes.sh --no-build  # 跳过构建，直接用已构建的二进制
#
# 走查清单（两个窗口分别操作）：
#   1. 两窗口「首页」互相出现在「附近设备」→ 各自点「配对」，两边确认同一 PIN
#   2. 配对成功弹窗里选「是，我的设备」把对方升为完全信任（或到「设置」改）
#   3. 在 B「同步」页「添加同步文件夹」选一个带文件的目录；A 点「刷新设备」
#   4. A「同步」页应出现黄色「可下载」条目 → 点它 → 完成后转绿「本地有」
#   5. 造冲突：让 A、B 各有一个同名不同内容的文件在共享范围内 → A 刷新后该名字
#      显示「多版本」并按序号并列，各自可分别拉取
#
# 收尾：关掉两个窗口即可；数据目录在 $ROOT 下，可整目录删除。
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="${AA4C_DEV_ROOT:-/tmp/aa4c-dev}"
BIN="$REPO/target/debug/aa4c-desktop"

if [[ "${1:-}" != "--no-build" ]]; then
  echo "==> 构建前端 + 桌面二进制（debug，内嵌当前 dist）"
  ( cd "$REPO/apps/desktop" && pnpm install --silent && pnpm tauri build --debug --no-bundle )
fi
[[ -x "$BIN" ]] || { echo "找不到二进制：$BIN（先不带 --no-build 跑一次）" >&2; exit 1; }

mkdir -p "$ROOT/a" "$ROOT/b"
echo "==> 数据目录：$ROOT/a  |  $ROOT/b"

AA4C_DATA_DIR="$ROOT/a" AA4C_DEVICE_NAME="联调-A" "$BIN" &
PA=$!
sleep 2
AA4C_DATA_DIR="$ROOT/b" AA4C_DEVICE_NAME="联调-B" "$BIN" &
PB=$!

echo "==> 已启动：A(pid $PA) / B(pid $PB)。按 Ctrl-C 关闭两者。"
trap 'kill "$PA" "$PB" 2>/dev/null || true' INT TERM
wait
