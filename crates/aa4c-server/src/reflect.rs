//! 自托管「打洞探测」端点（里程碑 C5，CONNECT_DESIGN.md §2 连接阶梯第 3 档）。
//!
//! 设备用自己的 QUIC 端点（与后续真实设备间连接**同一个本地端口**，见
//! `aa4c-transfer::TransferService::reflexive_addr`）连一次这里，服务器把观测到的对端
//! 源地址（quinn 的 `Connection::remote_address()`，即经过 NAT 之后的反射地址）经一条
//! uni 流原样回给它——这是自建版的 STUN binding response，避免额外引入公共 STUN 依赖
//! （CONNECT_DESIGN.md §1.1「仅自建」）。
//!
//! 与设备间传输 QUIC 用不同的 ALPN（`aa4c-reflect` vs `aa4c`），纯防御性区分——两者本来
//! 就是完全独立的 `quinn::Endpoint`（不同进程），不存在真正的协议混淆风险。
//! 不做身份鉴权：反射地址本身不敏感（探测方已经知道自己在往哪发），接受任意合法
//! Ed25519 客户端证书即可，同 TCP 信令面的既有惯例。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use aa4c_identity::Identity;
use aa4c_proto::net;
use aa4c_types::{Aa4cError, Result};
use quinn::crypto::rustls::QuicServerConfig;

const ALPN: &[u8] = b"aa4c-reflect";

/// 绑定反射端点并启动接受循环（后台任务）。`port` 与 TCP 信令端口同号（UDP/TCP 端口
/// namespace 独立，不冲突）。
///
/// 返回实际绑定端口与**端点句柄**：句柄给 [`crate::Server::shutdown`] 用——关掉端点，
/// 这里的 `accept()` 会拿到 `None` 自然退出，UDP 端口随之释放。内嵌进桌面端之后这一步
/// 是必须的：用户在设置里关掉「内置服务器」，端口就该真的还回去。
pub(crate) fn spawn(identity: Arc<Identity>, port: u16) -> Result<(u16, quinn::Endpoint)> {
    let mut rustls_server = identity.tls_server_config(None)?;
    rustls_server.alpn_protocols = vec![ALPN.to_vec()];
    let quic_server = QuicServerConfig::try_from(rustls_server)
        .map_err(|e| Aa4cError::Network(format!("reflect quic server config: {e}")))?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server));

    // 双栈监听（TRUST_DESIGN.md §6.1，里程碑 R1）：反射服务尤其需要——它的职责就是
    // 告诉对端"我看到你的地址是什么"，只绑 IPv4 的话 IPv6 客户端根本问不到自己的地址。
    let socket = aa4c_proto::net::bind_udp_dual_stack(port)
        .map_err(|e| Aa4cError::Network(format!("reflect quic bind: {e}")))?;
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(server_config),
        socket,
        quinn::default_runtime()
            .ok_or_else(|| Aa4cError::Network("no async runtime for reflect".into()))?,
    )
    .map_err(|e| Aa4cError::Network(format!("reflect quic endpoint: {e}")))?;
    let bound_port = endpoint
        .local_addr()
        .map_err(|e| Aa4cError::Network(format!("reflect local_addr: {e}")))?
        .port();

    // 克隆一份进接受循环，原件返回给调用方留作停机句柄（quinn 的 Endpoint 是共享句柄，
    // 关闭任意一份即关闭整个端点）。
    let accept_endpoint = endpoint.clone();
    tokio::spawn(async move {
        loop {
            let Some(incoming) = accept_endpoint.accept().await else {
                break;
            };
            tokio::spawn(async move {
                match incoming.await {
                    Ok(connection) => {
                        // 还原 IPv4 映射地址（里程碑 R1）：双栈监听之后，一个普通
                        // IPv4 客户端的源地址会变成 `::ffff:a.b.c.d`。这个值会被对端
                        // 当作**打洞候选**用，不还原就会和 `local_candidate_endpoints`
                        // 报的纯 IPv4 形式对不上——去重失效，白白多打几轮探测。
                        let observed = net::normalize_mapped(connection.remote_address());
                        if let Err(e) = reply(&connection, observed).await {
                            tracing::debug!(error = %e, "reflect reply failed");
                        }
                    }
                    Err(e) => tracing::debug!(error = %e, "reflect handshake failed"),
                }
            });
        }
    });
    Ok((bound_port, endpoint))
}

/// 把观测到的源地址（文本形式，`SocketAddr::to_string()`）经一条新 uni 流写回并结束，
/// 不需要 bincode——这是全部有效负载，字符串够用。
///
/// `finish()` 只是标记流结束（发 FIN），不保证数据已经送达对端；`reply()` 返回后调用方
/// 会让 `connection` 句柄归还（accept 循环里那条 task 结束），quinn 在最后一个句柄丢弃时
/// 会立即关闭连接——如果紧接着就丢，FIN 帧和 CONNECTION_CLOSE 帧几乎同时发出，客户端
/// 有极小概率先收到关闭而没读全数据。这里显式等对端主动关闭（或空闲超时兜底），
/// 避免这个时序竞态（探测客户端读完就会关闭它那侧，见 `TransferService::reflexive_addr`）。
async fn reply(connection: &quinn::Connection, observed: SocketAddr) -> Result<()> {
    let mut send = connection
        .open_uni()
        .await
        .map_err(|e| Aa4cError::Network(format!("reflect open uni: {e}")))?;
    send.write_all(observed.to_string().as_bytes())
        .await
        .map_err(|e| Aa4cError::Network(format!("reflect write: {e}")))?;
    send.finish()
        .map_err(|e| Aa4cError::Network(format!("reflect finish: {e}")))?;
    let _ = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quinn::crypto::rustls::QuicClientConfig;
    use std::net::Ipv4Addr;

    /// 反射端点应如实回报「服务器观测到的客户端源地址」——回环环境下 IP 固定是
    /// 127.0.0.1，端口应等于客户端 QUIC 端点自己绑定的本地端口（回环没有 NAT 改写，
    /// 这条断言在真实 NAT 环境下不成立，但足以证明协议本身接线正确）。
    #[tokio::test]
    async fn reflect_endpoint_reports_observed_source_address() {
        let server_dir = tempfile::tempdir().unwrap();
        let server_identity = Arc::new(Identity::load_or_generate(server_dir.path()).unwrap());
        let (port, _endpoint) = spawn(server_identity, 0).unwrap();
        let server_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));

        let client_dir = tempfile::tempdir().unwrap();
        let client_identity = Identity::load_or_generate(client_dir.path()).unwrap();
        let mut rustls_client = client_identity.tls_client_config(None).unwrap();
        rustls_client.alpn_protocols = vec![ALPN.to_vec()];
        let quic_client = QuicClientConfig::try_from(rustls_client).unwrap();
        let mut client_config = quinn::ClientConfig::new(Arc::new(quic_client));
        client_config.transport_config(Arc::new(quinn::TransportConfig::default()));

        let client_endpoint =
            quinn::Endpoint::client(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
        let client_port = client_endpoint.local_addr().unwrap().port();

        let connection = client_endpoint
            .connect_with(client_config, server_addr, "aa4c")
            .unwrap()
            .await
            .unwrap();
        let mut recv = connection.accept_uni().await.unwrap();
        let bytes = recv.read_to_end(64).await.unwrap();
        let observed: SocketAddr = std::str::from_utf8(&bytes).unwrap().parse().unwrap();
        connection.close(0u32.into(), b"done");

        assert_eq!(observed.ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(observed.port(), client_port);
    }
}
