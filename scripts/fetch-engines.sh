#!/usr/bin/env bash
# 把 aria2c / transmission-daemon 二进制放进 apps/desktop/src-tauri/binaries/，
# 供 Tauri sidecar 机制打包（tauri.conf.json 的 bundle.externalBin）。一旦声明
# 了 externalBin，`tauri_build::build()` 会在**任何** `cargo build`/`cargo check`/
# `cargo test` 涉及 aa4c-desktop 时校验该文件存在——这不是可选步骤，见
# V0.4_IMPLEMENTATION_PLAN.md D1 步骤 8 与 HANDOFF.md 环境要求。
#
# 两个引擎的产物形态不同（DOWNLOAD_DESIGN.md §3.6.5）：aria2 是每个三元组一个
# 裸二进制；Transmission 是每个三元组一个 zip（daemon 可执行文件 + 它依赖的
# 若干动态库/DLL，见 engines.yml 的 transmission-windows/macos/linux 三个 job）。
# 裸二进制原地改名进 binaries/；zip 解包后，可执行文件同样改名进 binaries/，
# 其余库文件放进 binaries/transmission-daemon-<triple>-libs/，供 tauri.conf.json
# 的 bundle.resources 一并打进安装包（这一步尚未接线，见 HANDOFF.md）。
#
# 用法：
#   scripts/fetch-engines.sh              # 正式模式：按下方写死的校验和下载 +
#                                          # 校验当前平台对应产物（两个引擎都会取）
#   scripts/fetch-engines.sh <triple>     # 正式模式，显式指定三元组（macOS
#                                          # universal 构建需要 aarch64-apple-darwin
#                                          # 与 x86_64-apple-darwin 都下载好，供
#                                          # Tauri 自己 lipo 合并，见 release.yml）
#   scripts/fetch-engines.sh --from-path  # 开发模式：复制 PATH 里的系统 aria2c
#                                          # （brew/apt/choco 装的那个），按当前
#                                          # 平台 target-triple 改名——不校验，
#                                          # 只为让本地 `cargo build --workspace` /
#                                          # `pnpm tauri dev` 跑起来，**不用于发布**。
#                                          # 同时会尝试复制 PATH 里的
#                                          # transmission-daemon，这是 best-effort
#                                          # （找不到只警告不报错，BT 能力运行时
#                                          # 优雅降级）；-libs/ 占位目录则总会创建，
#                                          # 满足 tauri.conf.json bundle.resources
#                                          # 的 glob 校验（不这样做本地 cargo check
#                                          # 都会因为 glob 零匹配直接编译报错）。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT/apps/desktop/src-tauri/binaries"
mkdir -p "$BIN_DIR"

# 引擎版本与校验和：升级 = 改这里 + 手动跑一次 .github/workflows/engines.yml
# 产出新的 engines release，diff 可审（DOWNLOAD_DESIGN.md §3.1/§9）。
ARIA2_VERSION="1.37.0"
ENGINES_TAG="engines/aria2-${ARIA2_VERSION}"
TRANSMISSION_VERSION="4.1.3"
TRANSMISSION_TAG="engines/transmission-${TRANSMISSION_VERSION}"

# 首次 engines.yml 跑完后，把 dist/SHA256SUMS 的值填进来。不用关联数组
# （macOS 系统自带 bash 仍是 3.2，不支持 `declare -A`，见 scripts/dev-server.sh
# 一路对 macOS 默认 shell 的迁就）。
checksum_for() {
  case "$1" in
    x86_64-pc-windows-msvc) echo "be2099c214f63a3cb4954b09a0becd6e2e34660b886d4c898d260febfe9d70c2" ;;
    aarch64-apple-darwin) echo "34f5dd97cd307d355306d0fbdcd0c14e1b4fdba54f210e94ca4a03bd0c9e965a" ;;
    x86_64-apple-darwin) echo "2af49a6dc10d696cdc329bbac8f0d6d3948b39322cff0e31f2334012a893bea9" ;;
    x86_64-unknown-linux-gnu) echo "ca1edb54e583f1e476f3a5084b8458d31821a12948d88217dd842ebbf7daf825" ;;
    *) echo "" ;;
  esac
}

# 同样来自 engines.yml 产出的 SHA256SUMS，但这里校验的是 zip 本身（Transmission
# 每个三元组的产物是 daemon + 依赖库打成的一个 zip，不是裸二进制，见文件头注释）。
checksum_for_transmission() {
  case "$1" in
    x86_64-pc-windows-msvc) echo "1295b252da08e6cc06c388f3e011c540ce8eee96d13c7bf8a388a74f7e80dca7" ;;
    aarch64-apple-darwin) echo "a3086f57fd403fa52e3cf79ebc7ee7db9d6d71cdcd6be5137689d56476dcebec" ;;
    x86_64-apple-darwin) echo "5b9e208ebf7e87e9327d250351300f5e3413e71c396dcbfb085878aff54ab222" ;;
    x86_64-unknown-linux-gnu) echo "532ef742820352014d56eee7e9d64249655a59ac590b62f2123a504edb874b32" ;;
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

  # transmission-daemon 二进制本身是 best-effort：找不到只警告，不能让本地
  # dev 环境因为没装 Transmission 而跑不起来（BT 能力运行时优雅降级成不可用，
  # 见 DOWNLOAD_DESIGN.md §3.6.5）。本地开发机上依赖库已经由 brew/apt 装好在
  # 系统路径里，直接复制可执行文件本身即可，不需要像正式下载模式那样搬运
  # 依赖库。
  tsrc="$(command -v transmission-daemon || true)"
  if [[ -n "$tsrc" ]]; then
    tdest="$BIN_DIR/transmission-daemon-${triple}${suffix}"
    rm -f "$tdest"
    cp "$tsrc" "$tdest"
    chmod +x "$tdest"
    echo "dev mode: copied $tsrc -> $tdest (NOT verified, NOT for release)"
  else
    echo "warning: transmission-daemon not found in PATH — BT downloads will be unavailable in this dev build" >&2
    echo "  macOS:   brew install transmission-cli" >&2
    echo "  Linux:   apt install transmission-daemon" >&2
  fi

  # 不管上面找没找到 transmission-daemon 本体，-libs/ 占位目录都要建：
  # tauri.conf.json 的 bundle.resources 用 glob 引用它（正式下载模式才会填真
  # 的依赖库，见下方主流程），而 tauri-build 在**任何** cargo build/check/test
  # 涉及 aa4c-desktop 时都会校验这个 glob 至少匹配一个文件，一个没匹配到就
  # 直接编译报错——dev 模式下这些库根本不会被用到（系统装的
  # transmission-daemon 走系统库路径，不依赖这个占位目录），放个占位文件纯粹
  # 是为了不让 build.rs 炸。
  tlibs_dir="$BIN_DIR/transmission-daemon-${triple}-libs"
  mkdir -p "$tlibs_dir"
  : > "$tlibs_dir/.dev-placeholder"

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

# --- Transmission：每个三元组一个 zip（daemon + 依赖库），形态和上面的裸二进制
# 不一样，需要单独的下载 + 校验 + 解包逻辑（DOWNLOAD_DESIGN.md §3.6.5）。
t_expected="$(checksum_for_transmission "$triple")"
if [[ -z "$t_expected" ]]; then
  echo "error: no checksum recorded yet for transmission-daemon $triple." >&2
  echo "  Run .github/workflows/engines.yml (workflow_dispatch) once, then fill" >&2
  echo "  scripts/fetch-engines.sh's checksum_for_transmission() from the resulting SHA256SUMS." >&2
  echo "  For local development in the meantime, use: $0 --from-path" >&2
  exit 1
fi

t_zip="$BIN_DIR/.transmission-daemon-${triple}.zip.tmp"
t_url="https://github.com/HuoTaoCN/AA4C/releases/download/${TRANSMISSION_TAG}/transmission-daemon-${triple}.zip"

rm -f "$t_zip"
echo "downloading $t_url"
curl -fsSL -o "$t_zip" "$t_url"

t_actual="$(sha256_of "$t_zip")"
if [[ "$t_actual" != "$t_expected" ]]; then
  echo "error: checksum mismatch for $t_zip" >&2
  echo "  expected: $t_expected" >&2
  echo "  actual:   $t_actual" >&2
  rm -f "$t_zip"
  exit 1
fi
echo "verified $t_zip (sha256 $t_actual)"

t_extract_dir="$BIN_DIR/.transmission-daemon-${triple}.extract.tmp"
rm -rf "$t_extract_dir"
mkdir -p "$t_extract_dir"
unzip -q "$t_zip" -d "$t_extract_dir"
rm -f "$t_zip"

t_exe_name="transmission-daemon${suffix}"
t_exe_src="$(find "$t_extract_dir" -name "$t_exe_name" -type f | head -n1)"
if [[ -z "$t_exe_src" ]]; then
  echo "error: $t_exe_name not found inside $t_url" >&2
  rm -rf "$t_extract_dir"
  exit 1
fi

t_dest="$BIN_DIR/transmission-daemon-${triple}${suffix}"
t_libs_dir="$BIN_DIR/transmission-daemon-${triple}-libs"
rm -f "$t_dest"
rm -rf "$t_libs_dir"
mkdir -p "$t_libs_dir"

mv "$t_exe_src" "$t_dest"
chmod +x "$t_dest"
# zip 里除可执行文件外的其余文件都是它依赖的动态库/DLL（见 engines.yml 对应三个
# job 的打包逻辑），原样搬进 -libs/ 目录，供 tauri.conf.json 的 bundle.resources
# 一并打进安装包（尚未接线，见 HANDOFF.md）。
find "$t_extract_dir" -type f -exec mv {} "$t_libs_dir/" \;
rm -rf "$t_extract_dir"

echo "verified transmission-daemon-${triple}: $t_dest + $(find "$t_libs_dir" -type f | wc -l | tr -d ' ') lib file(s) in $t_libs_dir"
