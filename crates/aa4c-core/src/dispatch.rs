//! 配对分流适配器：把统一传输监听器读到的 `PairRequest` 接到 `PairingManager`。
//!
//! 传输层只认 [`IncomingPairDispatch`] trait，不感知配对语义；Core 在装配阶段
//! 注入本适配器，完成「传输监听 → 配对响应」的接线（AGENTS.md 低耦合约定）。

use std::sync::Arc;

use aa4c_identity::PairingManager;
use aa4c_transfer::{IncomingPairDispatch, IncomingTlsStream};
use aa4c_types::{DeviceId, DeviceInfo};

pub(crate) struct PairDispatch {
    pairing: Arc<PairingManager>,
}

impl PairDispatch {
    pub(crate) fn new(pairing: Arc<PairingManager>) -> Self {
        Self { pairing }
    }
}

impl IncomingPairDispatch for PairDispatch {
    fn dispatch(
        &self,
        stream: IncomingTlsStream,
        cert_id: DeviceId,
        device: DeviceInfo,
        public_key: [u8; 32],
    ) {
        if let Err(e) = self
            .pairing
            .handle_dispatched(stream, cert_id, device, public_key)
        {
            tracing::warn!(error = %e, "failed to dispatch incoming pairing");
        }
    }
}
