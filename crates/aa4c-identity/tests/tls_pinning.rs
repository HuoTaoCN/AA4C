//! TLS 证书固定的正反两向测试（V0.1_IMPLEMENTATION_PLAN.md M2 / TESTING.md 安全规则）。

use std::sync::Arc;

use aa4c_identity::{device_id_from_cert, Identity};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

fn new_identity() -> Identity {
    let dir = tempfile::tempdir().unwrap();
    Identity::load_or_generate(dir.path()).unwrap()
}

/// 启动一个回显一字节的 TLS 服务端，返回监听地址与 join handle。
async fn spawn_server(
    identity: &Identity,
    expect_peer: Option<&String>,
) -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<Option<String>>,
) {
    let config = identity.tls_server_config(expect_peer).unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.ok()?;
        let mut stream = acceptor.accept(tcp).await.ok()?;
        // 握手成功：读取对端（客户端）证书身份
        let peer_id = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certs| certs.first())
            .and_then(|cert| device_id_from_cert(cert).ok())?;
        let mut buf = [0u8; 1];
        stream.read_exact(&mut buf).await.ok()?;
        stream.write_all(&buf).await.ok()?;
        Some(peer_id)
    });
    (addr, handle)
}

async fn connect(
    client: &Identity,
    addr: std::net::SocketAddr,
    expect_peer: Option<&String>,
) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let config = client.tls_client_config(expect_peer).unwrap();
    let connector = TlsConnector::from(Arc::new(config));
    let tcp = TcpStream::connect(addr).await?;
    connector
        .connect(ServerName::try_from("aa4c").unwrap(), tcp)
        .await
}

#[tokio::test]
async fn handshake_succeeds_when_fingerprint_matches() {
    let server_id = new_identity();
    let client_id = new_identity();
    let (addr, server) = spawn_server(&server_id, None).await;

    let mut stream = connect(&client_id, addr, Some(server_id.device_id()))
        .await
        .expect("pinned handshake should succeed");
    stream.write_all(&[42]).await.unwrap();
    let mut buf = [0u8; 1];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf[0], 42);

    // 服务端从客户端证书中读到的身份 == 客户端 DeviceId（mTLS 双向认证）
    let seen = server
        .await
        .unwrap()
        .expect("server should see client cert");
    assert_eq!(&seen, client_id.device_id());
}

#[tokio::test]
async fn client_rejects_server_with_wrong_fingerprint() {
    let server_id = new_identity();
    let client_id = new_identity();
    let imposter = new_identity(); // 期望的是另一台设备的指纹
    let (addr, _server) = spawn_server(&server_id, None).await;

    let result = connect(&client_id, addr, Some(imposter.device_id())).await;
    let err = result.expect_err("handshake must fail on fingerprint mismatch");
    assert!(
        err.to_string().contains("pin mismatch"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn server_rejects_client_with_wrong_fingerprint() {
    let server_id = new_identity();
    let trusted_client = new_identity();
    let stranger = new_identity();
    // 服务端只接受 trusted_client 的指纹
    let (addr, server) = spawn_server(&server_id, Some(trusted_client.device_id())).await;

    // 陌生设备连接：服务端握手失败，回显任务返回 None
    let result = connect(&stranger, addr, Some(server_id.device_id())).await;
    let read_failed = match result {
        Err(_) => true, // 握手即失败
        Ok(mut stream) => {
            // TLS1.3 下客户端可能要到首次读写才看到服务端的拒绝
            stream.write_all(&[1]).await.ok();
            let mut buf = [0u8; 1];
            stream.read_exact(&mut buf).await.is_err()
        }
    };
    assert!(
        read_failed,
        "stranger should not complete an echo roundtrip"
    );
    assert!(
        server.await.unwrap().is_none(),
        "server must reject stranger"
    );
}
