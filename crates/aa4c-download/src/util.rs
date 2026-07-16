//! aria2 conf 与 Transmission settings.json 生成器共用的两个小工具：端口探测、
//! 高熵密钥生成。抽出来是因为 D2 加 Transmission 时发现这两段逻辑要照抄一遍
//! （随机端口 + 随机凭据是两个引擎共同的启动约定，DOWNLOAD_DESIGN.md §3.1/§3.6.2）。

use std::net::TcpListener;

use aa4c_types::{Aa4cError, Result};

/// 探测一个当前空闲的本地端口。绑定到端口 0 让操作系统分配、立即释放——
/// 与引擎真正 bind 之间存在竞态窗口，端口被抢由调用方的重试逻辑兜底
/// （见 `lib.rs`），这里不做任何预留尝试。
pub(crate) fn probe_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(Aa4cError::Io)?;
    let port = listener.local_addr().map_err(Aa4cError::Io)?.port();
    drop(listener);
    Ok(port)
}

/// 生成一个高熵密钥/凭据：两个 UUID v4 拼成 32 字节 base58 编码——不为此单独
/// 引入新的 RNG 依赖，同 C6 `generate_token` 的先例。
pub(crate) fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bs58::encode(bytes).into_string()
}
