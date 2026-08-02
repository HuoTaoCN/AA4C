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

/// 真实进程全链路测试的两个前置件——`service.rs`/`suggest.rs` 的集成测试共用
/// （AI2.0/AI3.1 实证结论：真实 `llama-server` + 微型 GGUF，不 mock）。放这里
/// 而不是各自复制一份：两处测试都要用，超过"三行重复"的阈值。
#[cfg(test)]
pub(crate) fn require_llama_server() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("AA4C_TEST_LLAMA_SERVER_BIN") {
        return std::path::PathBuf::from(p);
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let exe_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "llama-server not found in PATH and AA4C_TEST_LLAMA_SERVER_BIN not set — see \
         ARCHIVE_DESIGN.md §3.1/HANDOFF.md."
    );
}

#[cfg(test)]
pub(crate) fn require_tiny_model() -> std::path::PathBuf {
    match std::env::var("AA4C_TEST_TINY_GGUF") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => panic!("AA4C_TEST_TINY_GGUF not set — see ARCHIVE_DESIGN.md §3.1 第 6 点。"),
    }
}
