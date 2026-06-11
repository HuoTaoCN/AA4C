//! TLS 1.3 证书固定（PROTOCOL.md §2）。
//!
//! 不使用 CA。信任模型：对端证书内 Ed25519 公钥的 BLAKE3 指纹必须等于期望的 DeviceId。
//! 配对（首次见面）场景 `expected = None`：接受任意有效自签名证书，
//! 由上层在握手后通过 [`device_id_from_cert`] 读取对端身份，再走 PIN 双向确认。

use std::sync::Arc;

use aa4c_types::{Aa4cError, DeviceId, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};

/// 从证书 DER 中提取 Ed25519 公钥并计算 DeviceId（BLAKE3 hex）。
///
/// 拒绝非 Ed25519 证书。
pub fn device_id_from_cert(cert: &CertificateDer<'_>) -> Result<DeviceId> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|e| Aa4cError::Protocol(format!("invalid peer certificate: {e}")))?;
    let spki = parsed.public_key();
    if spki.algorithm.algorithm != x509_parser::oid_registry::OID_SIG_ED25519 {
        return Err(Aa4cError::Protocol(
            "peer certificate is not Ed25519".into(),
        ));
    }
    let raw = spki.subject_public_key.data.as_ref();
    if raw.len() != 32 {
        return Err(Aa4cError::Protocol(format!(
            "unexpected ed25519 key length: {}",
            raw.len()
        )));
    }
    Ok(crate::device_id_from_public_key(raw))
}

fn check_pin(
    cert: &CertificateDer<'_>,
    expected: Option<&DeviceId>,
) -> std::result::Result<(), rustls::Error> {
    let actual = device_id_from_cert(cert)
        .map_err(|e| rustls::Error::General(format!("aa4c pin check: {e}")))?;
    match expected {
        Some(want) if &actual != want => Err(rustls::Error::General(format!(
            "aa4c pin mismatch: expected {want}, got {actual}"
        ))),
        _ => Ok(()),
    }
}

/// 客户端方向：校验服务端证书指纹。
#[derive(Debug)]
pub(crate) struct PinnedServerVerifier {
    expected: Option<DeviceId>,
    provider: Arc<CryptoProvider>,
}

impl PinnedServerVerifier {
    pub(crate) fn new(expected: Option<DeviceId>, provider: Arc<CryptoProvider>) -> Self {
        Self { expected, provider }
    }
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        check_pin(end_entity, self.expected.as_ref())?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// 服务端方向：要求客户端证书（mTLS）并校验指纹。
#[derive(Debug)]
pub(crate) struct PinnedClientVerifier {
    expected: Option<DeviceId>,
    provider: Arc<CryptoProvider>,
}

impl PinnedClientVerifier {
    pub(crate) fn new(expected: Option<DeviceId>, provider: Arc<CryptoProvider>) -> Self {
        Self { expected, provider }
    }
}

impl ClientCertVerifier for PinnedClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        check_pin(end_entity, self.expected.as_ref())?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) fn build_server_config(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    expect_peer: Option<DeviceId>,
) -> Result<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PinnedClientVerifier::new(expect_peer, provider.clone()));
    rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_err)?
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![cert], key)
        .map_err(tls_err)
}

pub(crate) fn build_client_config(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    expect_peer: Option<DeviceId>,
) -> Result<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PinnedServerVerifier::new(expect_peer, provider.clone()));
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_err)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![cert], key)
        .map_err(tls_err)?;
    Ok(config)
}

fn tls_err(e: rustls::Error) -> Aa4cError {
    Aa4cError::Network(format!("tls config: {e}"))
}
