# aa4c-server：自建信令 + 中继服务器（CONNECT_DESIGN.md §1.1）。
#
# aa4c-server 是 workspace 里的一个 crate，构建需要整个仓库作为上下文：
#   docker build -f crates/aa4c-server/Dockerfile -t aa4c-server .
#   docker run -p 42420:42420 -v aa4c-server-data:/data aa4c-server
#
# 首次启动会在 /data 生成一份服务器身份（Ed25519 密钥 + 自签证书），此后固定——
# 卷必须持久化，否则每次重启都是"新服务器"（老客户端的 server_url 指纹会对不上）。

FROM rust:1.85 AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p aa4c-server

FROM debian:bookworm-slim
COPY --from=builder /build/target/release/aa4c-server /usr/local/bin/aa4c-server

ENV AA4C_SERVER_DATA_DIR=/data
ENV AA4C_SERVER_LISTEN=0.0.0.0:42420
VOLUME ["/data"]
EXPOSE 42420/tcp

ENTRYPOINT ["/usr/local/bin/aa4c-server"]
