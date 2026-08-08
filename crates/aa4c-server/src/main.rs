//! `aa4c-server` 命令行入口：薄封装，业务逻辑全在 `aa4c_server::run`（库，供测试内嵌复用）。
//!
//! 环境变量：
//! - `AA4C_SERVER_DATA_DIR`：身份数据目录，默认 `./aa4c-server-data`
//! - `AA4C_SERVER_LISTEN`：监听地址，默认 `[::]:42420`（双栈，同时接受 IPv6 与 IPv4）

use std::net::SocketAddr;
use std::path::PathBuf;

use aa4c_server::{run, ServerConfig};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let data_dir = std::env::var("AA4C_SERVER_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./aa4c-server-data"));
    let listen_addr: SocketAddr = std::env::var("AA4C_SERVER_LISTEN")
        .ok()
        .and_then(|s| s.parse().ok())
        // 默认双栈（里程碑 R1）：`[::]` 同时接受 IPv6 与 IPv4（`IPV6_V6ONLY` 由
        // `Server::run` 显式关闭）。要退回只听 IPv4 就显式设 `0.0.0.0:42420`。
        .unwrap_or_else(|| "[::]:42420".parse().unwrap());

    let server = run(ServerConfig {
        data_dir,
        listen_addr,
    })
    .await
    .expect("aa4c-server failed to start");

    tracing::info!(
        device_id = %server.device_id(),
        "把 aa4c://<你的可达地址>:{}#{} 填进客户端设置",
        server.local_addr().port(),
        &server.device_id()[..16],
    );

    // 无显式关闭逻辑：接受循环在后台任务里跑，进程结束（Ctrl-C / 容器停止）即退出。
    std::future::pending::<()>().await;
}
