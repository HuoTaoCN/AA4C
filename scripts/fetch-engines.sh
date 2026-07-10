#!/usr/bin/env bash
# 把 aria2c 二进制放进 apps/desktop/src-tauri/binaries/，供 Tauri sidecar 机制
# 打包（tauri.conf.json 的 bundle.externalBin）。一旦声明了 externalBin，
# `tauri_build::build()` 会在**任何** `cargo build`/`cargo check`/`cargo test`
# 涉及 aa4c-desktop 时校验该文件存在——这不是可选步骤，见
# V0.4_IMPLEMENTATION_PLAN.md D1 步骤 8 与 HANDOFF.md 环境要求。
#
# 用法：
#   scripts/fetch-engines.sh              # 正式模式：按下方写死的校验和下载 +
#                                          # 校验当前平台对应产物
#   scripts/fetch-engines.sh <triple>     # 正式模式，显式指定三元组（macOS
#                                          # universal 构建需要 aarch64-apple-darwin
#                                          # 与 x86_64-apple-darwin 都下载好，供
#                                          # Tauri 自己 lipo 合并，见 release.yml）
#   scripts/fetch-engines.sh --from-path  # 开发模式：复制 PATH 里的系统 aria2c
#                                          # （brew/apt/choco 装的那个），按当前
#                                          # 平台 target-triple 改名——不校验，
#                                          # 只为让本地 `cargo build --workspace` /
#                                          # `pnpm tauri dev` 跑起来，**不用于发布**。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT/apps/desktop/src-tauri/binaries"
mkdir -p "$BIN_DIR"

# 引擎版本与校验和：升级 = 改这里 + 手动跑一次 .github/workflows/engines.yml
# 产出新的 engines release，diff 可审（DOWNLOAD_DESIGN.md §3.1/§9）。
ARIA2_VERSION="1.37.0"
ENGINES_TAG="engines/aria2-${ARIA2_VERSION}"

# 首次 engines.yml 跑完后，把 dist/SHA256SUMS 的值填进来。不用关联数组
# （macOS 系统自带 bash 仍是 3.2，不支持 `declare -A`，见 scripts/dev-server.sh
# 一路对 macOS 默认 shell 的迁就）。
checksum_for() {
  case "$1" in
    x86_64-pc-windows-msvc) echo "be2099c214f63a3cb4954b09a0becd6e2e34660b886d4c898d260febfe9d70c2" ;;
    aarch64-apple-darwin) echo "a79bdf829a479a77d2f0af775b2a61e1174466417af9c70d5eb75d891918ae08" ;;
    x86_64-apple-darwin) echo "5481440a7c7bbde3cb0e339267684af6d1eb187f4fb7c299f721b7536b9d86df" ;;
    x86_64-unknown-linux-gnu) echo "726c98e6a331f1e1e50b1d3711bf257b6472832d9b2dbae140dd9fbd113ee376" ;;
    *) echo "" ;;
  esac
}

detect_triple() {
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) echo "aarch64-apple-darwin" ;;
    Darwin-x86_64) echo "x86_64-apple-darwin" ;;
    Linux-x86_64) echo "x86_64-unknown-linux-gnu" ;;
    MINGW*|MSYS*|CYGWIN*) echo "x86_64-pc-windows-msvc" ;;
    *)
      echo "error: unsupported platform $(uname -s)-$(uname -m)" >&2
      exit 1
      ;;
  esac
}

exe_suffix() {
  case "$1" in
    *windows*) echo ".exe" ;;
    *) echo "" ;;
  esac
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

if [[ "${1:-}" == "--from-path" ]]; then
  triple="$(detect_triple)"
  suffix="$(exe_suffix "$triple")"
  src="$(command -v aria2c || true)"
  if [[ -z "$src" ]]; then
    echo "error: aria2c not found in PATH — install it first" >&2
    echo "  macOS:   brew install aria2" >&2
    echo "  Linux:   apt install aria2" >&2
    echo "  Windows: choco install aria2" >&2
    exit 1
  fi
  dest="$BIN_DIR/aria2c-${triple}${suffix}"
  # 先删再拷贝：源文件（brew 装的 aria2c）常是只读权限位，`cp` 覆盖一个已存在的
  # 只读目标会失败，rm -f 让这个脚本可以重复安全运行。
  rm -f "$dest"
  cp "$src" "$dest"
  chmod +x "$dest"
  echo "dev mode: copied $src -> $dest (NOT verified, NOT for release)"
  exit 0
fi

triple="${1:-$(detect_triple)}"
suffix="$(exe_suffix "$triple")"
expected="$(checksum_for "$triple")"
if [[ -z "$expected" ]]; then
  echo "error: no checksum recorded yet for $triple." >&2
  echo "  Run .github/workflows/engines.yml (workflow_dispatch) once, then fill" >&2
  echo "  scripts/fetch-engines.sh's checksum_for() from the resulting SHA256SUMS." >&2
  echo "  For local development in the meantime, use: $0 --from-path" >&2
  exit 1
fi

dest="$BIN_DIR/aria2c-${triple}${suffix}"
url="https://github.com/HuoTaoCN/AA4C/releases/download/${ENGINES_TAG}/aria2c-${triple}${suffix}"

# 先删再下载：同一目录常常已经躺着一份 `--from-path` 模式留下的文件（那份是从
# Homebrew 装的 aria2c 直接 cp 来的，继承了只读权限位），`curl -o` 覆盖一个
# 只读目标会写失败（"Failure writing output to destination"），本地实测踩到过。
rm -f "$dest"
echo "downloading $url"
curl -fsSL -o "$dest" "$url"

actual="$(sha256_of "$dest")"
if [[ "$actual" != "$expected" ]]; then
  echo "error: checksum mismatch for $dest" >&2
  echo "  expected: $expected" >&2
  echo "  actual:   $actual" >&2
  rm -f "$dest"
  exit 1
fi
chmod +x "$dest"
echo "verified $dest (sha256 $actual)"
