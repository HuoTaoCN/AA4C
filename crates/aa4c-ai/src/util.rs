//! 端口探测 + 高熵密钥生成——照抄 `aa4c-download/src/util.rs` 的先例（那边的
//! 注释已经解释过："不为此单独引入新的 RNG 依赖，同 C6 `generate_token` 的
//! 先例"）。两个 crate 各自一份而不是共用一个 helper crate：加起来不到 20
//! 行，为它专门抽一层间接不值得（"三行重复胜过过早抽象"）。

use std::net::TcpListener;

use aa4c_types::{Aa4cError, Result};

/// 探测一个当前空闲的本地端口。绑定到端口 0 让操作系统分配、立即释放——
/// 与引擎真正 bind 之间存在竞态窗口，端口被抢由调用方的重试逻辑兜底。
pub(crate) fn probe_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(Aa4cError::Io)?;
    let port = listener.local_addr().map_err(Aa4cError::Io)?.port();
    drop(listener);
    Ok(port)
}

/// 生成一个高熵密钥：两个 UUID v4 拼成 32 字节 base58 编码。用作 `LLAMA_API_KEY`
/// ——环境变量传递，不走命令行参数（ARCHIVE_DESIGN.md §3.1 第 3 点 / §3.2）。
pub(crate) fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bs58::encode(bytes).into_string()
}
