use crate::models::errors::RustTgCallsError;
use crate::wrtc::native::certpool::{CertPool, GeneratedCert, generate_cert};
use rtc::ice::network_type::NetworkType as WebrtcNetworkType;
use rtc::peer_connection::configuration::{RTCConfiguration, RTCConfigurationBuilder};
use std::sync::Arc;
use tokio::sync::Mutex;

/// NetworkType is the candidate network-type tag enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkType {
    UDP4 = 1,
    UDP6 = 2,
    TCP4 = 3,
    TCP6 = 4,
}

impl From<NetworkType> for WebrtcNetworkType {
    fn from(n: NetworkType) -> Self {
        match n {
            NetworkType::UDP4 => WebrtcNetworkType::Udp4,
            NetworkType::UDP6 => WebrtcNetworkType::Udp6,
            NetworkType::TCP4 => WebrtcNetworkType::Tcp4,
            NetworkType::TCP6 => WebrtcNetworkType::Tcp6,
        }
    }
}

/// FactoryOptions configures the per-process Factory. ICE timing,
/// per-pair binding budgets, and STUN/TURN servers are handled internally.
#[derive(Clone, Debug, Default)]
pub struct FactoryOptions {
    pub network_types: Vec<NetworkType>,
    pub cert_pool_size: usize,
    pub shared_udp_mux: bool,
}

/// PeerFactory hosts the per-process configuration the native Stack draws
/// from on each NewPeerConnection. A single PeerFactory is shared across
/// every concurrent call.
pub struct PeerFactory {
    options: FactoryOptions,
    cert_pool: Arc<Mutex<CertPool>>,
}

impl PeerFactory {
    /// NewFactory configures and constructs a PeerFactory.
    pub fn new() -> Result<Self, RustTgCallsError> {
        Self::new_with_options(FactoryOptions::default())
    }

    /// NewFactory constructs a PeerFactory with custom options.
    pub fn new_with_options(options: FactoryOptions) -> Result<Self, RustTgCallsError> {
        let pool_size = if options.cert_pool_size == 0 {
            8
        } else {
            options.cert_pool_size
        };
        let cert_pool = Arc::new(Mutex::new(CertPool::new(pool_size)));

        Ok(Self {
            options,
            cert_pool,
        })
    }

    /// take_cert draws a pre-generated RTCCertificate from the cert pool.
    /// Releases the Mutex lock before performing any fallback keygen so parallel calls never block each other.
    pub async fn take_cert(&self) -> Result<GeneratedCert, RustTgCallsError> {
        let cert_opt = {
            let mut guard = self.cert_pool.lock().await;
            guard.try_take()
        };
        if let Some(cert) = cert_opt {
            return Ok(cert);
        }

        tokio::task::spawn_blocking(generate_cert)
            .await
            .map_err(|e| RustTgCallsError::Internal(e.to_string()))?
            .map_err(|e| RustTgCallsError::Internal(e.to_string()))
    }

    /// create_rtc_config returns default RTCConfiguration.
    pub fn create_rtc_config(&self) -> RTCConfiguration {
        RTCConfigurationBuilder::new().build()
    }

    /// options returns the active FactoryOptions configuration.
    pub fn options(&self) -> &FactoryOptions {
        &self.options
    }
}
