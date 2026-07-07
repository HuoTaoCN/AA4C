#!/usr/bin/env bash
# 本地起一个 aa4c-server（信令面，里程碑 C2），供本机或局域网内的客户端联调。
# 首次启动会在 $DATA_DIR 生成一份服务器身份（Ed25519 密钥 + 自签证书），此后固定。
#
# 用法：
#   bash scripts/dev-server.sh                # debug 构建，监听 0.0.0.0:42420
#   AA4C_SERVER_LISTEN=127.0.0.1:0 bash scripts/dev-server.sh   # 系统分配端口，仅本机联调
#
# 启动后从日志里取 `aa4c://<host>:<port>#<指纹>` 地址，把 <host> 换成客户端能连到
# 的地址（同机联调用 127.0.0.1；局域网/公网需自行确认可达性与端口开放），
# 填进客户端设置的服务器地址（`enable_remote` 开关同一页）。
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
DATA_DIR="${AA4C_SERVER_DATA_DIR:-$REPO/.dev-server-data}"

echo "==> 构建 aa4c-server（debug）"
cargo build -p aa4c-server

mkdir -p "$DATA_DIR"
echo "==> 身份数据目录：$DATA_DIR"
AA4C_SERVER_DATA_DIR="$DATA_DIR" \
  AA4C_SERVER_LISTEN="${AA4C_SERVER_LISTEN:-0.0.0.0:42420}" \
  "$REPO/target/debug/aa4c-server"
