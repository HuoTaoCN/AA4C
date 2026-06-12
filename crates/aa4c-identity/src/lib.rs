//! AA4C 设备身份：Ed25519 密钥、自签名证书、TLS 证书固定、配对 PIN。
//!
//! 接口契约见 API_DESIGN.md §4，协议规则见 PROTOCOL.md §2 / §6。
//!
//! - 设备私钥保存在 `<data_dir>/identity/device.key`（PEM，Unix 下权限 0600）
//! - DeviceId = BLAKE3(Ed25519 公钥 32 字节) 的 hex
//! - 证书每次启动由私钥重新自签生成（指纹固定在公钥上，证书本身可变）

#![forbid(unsafe_code)]

mod pairing;
mod pin;
mod tls;

use std::path::{Path, PathBuf};

use aa4c_types::{Aa4cError, DeviceId, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

pub use pairing::{EventSender, IncomingStream, PairingManager};
pub use pin::derive_pin;
pub use tls::device_id_from_cert;

const KEY_FILE: &str = "device.key";

/// 由 Ed25519 公钥原始字节计算 DeviceId（BLAKE3 hex，64 字符）。
pub fn device_id_from_public_key(public_key: &[u8]) -> DeviceId {
    blake3::hash(public_key).to_hex().to_string()
}

/// 本机身份：Ed25519 密钥对 + 自签名 TLS 证书。
pub struct Identity {
    device_id: DeviceId,
    public_key: Vec<u8>,
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
}

impl Identity {
    /// 加载或首次生成身份。
    ///
    /// 私钥存放于 `<data_dir>/identity/device.key`；目录不存在时自动创建。
    pub fn load_or_generate(data_dir: &Path) -> Result<Self> {
        let key_path = identity_dir(data_dir).join(KEY_FILE);
        let key_pair = if key_path.exists() {
            let pem = std::fs::read_to_string(&key_path)?;
            rcgen::KeyPair::from_pem(&pem)
                .map_err(|e| Aa4cError::Protocol(format!("invalid device key: {e}")))?
        } else {
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
                .map_err(|e| Aa4cError::Protocol(format!("keygen failed: {e}")))?;
            write_key_file(&key_path, &key_pair.serialize_pem())?;
            tracing::info!("generated new device identity");
            key_pair
        };
        Self::from_key_pair(&key_pair)
    }

    fn from_key_pair(key_pair: &rcgen::KeyPair) -> Result<Self> {
        let public_key = key_pair.public_key_raw().to_vec();
        let device_id = device_id_from_public_key(&public_key);

        let params = rcgen::CertificateParams::new(vec!["aa4c".into()])
            .map_err(|e| Aa4cError::Protocol(format!("cert params: {e}")))?;
        let cert = params
            .self_signed(key_pair)
            .map_err(|e| Aa4cError::Protocol(format!("self-sign failed: {e}")))?;

        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));
        Ok(Self {
            device_id,
            public_key,
            cert_der: cert.der().clone(),
            key_der,
        })
    }

    /// 设备指纹（BLAKE3(公钥) hex，64 字符）。
    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    /// Ed25519 公钥原始字节（32 字节），用于配对 PIN 推导与入库。
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// 本机证书 DER（用于调试/测试）。
    pub fn cert_der(&self) -> &CertificateDer<'static> {
        &self.cert_der
    }

    /// 监听端 TLS 配置（mTLS：要求客户端证书）。
    ///
    /// `expect_peer = None`（常规）：接受任意有效 Ed25519 客户端证书，
    /// 由上层在握手后读取对端证书校验 trusted；配对会话也走此路径。
    pub fn tls_server_config(
        &self,
        expect_peer: Option<&DeviceId>,
    ) -> Result<rustls::ServerConfig> {
        tls::build_server_config(
            self.cert_der.clone(),
            self.key_der.clone_key(),
            expect_peer.cloned(),
        )
    }

    /// 连接端 TLS 配置。
    ///
    /// `expect_peer = Some(id)`：证书固定，指纹不符即握手失败（传输场景必须传）。
    /// `expect_peer = None`：首次配对场景，接受任意有效证书，握手后由上层校验。
    pub fn tls_client_config(
        &self,
        expect_peer: Option<&DeviceId>,
    ) -> Result<rustls::ClientConfig> {
        tls::build_client_config(
            self.cert_der.clone(),
            self.key_der.clone_key(),
            expect_peer.cloned(),
        )
    }
}

fn identity_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("identity")
}

fn write_key_file(path: &Path, pem: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, pem)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_stable_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let first = Identity::load_or_generate(dir.path()).unwrap();
        let second = Identity::load_or_generate(dir.path()).unwrap();
        assert_eq!(first.device_id(), second.device_id());
        assert_eq!(first.public_key(), second.public_key());
        assert_eq!(first.device_id().len(), 64);
    }

    #[test]
    fn fresh_data_dir_yields_different_identity() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();
        assert_ne!(a.device_id(), b.device_id());
    }

    #[test]
    fn device_id_matches_cert_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::load_or_generate(dir.path()).unwrap();
        let from_cert = device_id_from_cert(identity.cert_der()).unwrap();
        assert_eq!(&from_cert, identity.device_id());
    }

    #[cfg(unix)]
    #[test]
    fn key_file_has_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        Identity::load_or_generate(dir.path()).unwrap();
        let meta = std::fs::metadata(dir.path().join("identity").join("device.key")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
