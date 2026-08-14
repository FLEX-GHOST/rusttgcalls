//! Stack drives one Telegram group-call connection end-to-end.

use crate::models::{ConnState, errors::RustTgCallsError};
use crate::wrtc::native::direct_transport::DirectTransport;
use crate::wrtc::peer_factory::PeerFactory;
use bytes::Bytes;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

pub type ConnStateCallback = Arc<dyn Fn(ConnState) + Send + Sync>;
pub type ConnStateFn = ConnStateCallback;

/// Stack manages the direct ICE -> DTLS -> SRTP wire transport and media streams for one call.
pub struct Stack {
    direct: Arc<DirectTransport>,
    on_state_change: RwLock<Option<ConnStateCallback>>,
}

impl Stack {
    /// from_direct wraps an existing DirectTransport instance.
    pub fn from_direct(direct: Arc<DirectTransport>) -> Self {
        Self {
            direct,
            on_state_change: RwLock::new(None),
        }
    }

    /// new constructs a production WebRTC Stack instance with direct ICE -> DTLS -> SRTP transport.
    pub async fn new(factory: &PeerFactory) -> Result<Arc<Self>, RustTgCallsError> {
        let cert = factory.take_cert().await?;
        let direct = DirectTransport::new(cert).await?;
        Ok(Arc::new(Self {
            direct,
            on_state_change: RwLock::new(None),
        }))
    }

    pub fn audio_ssrc(&self) -> u32 {
        self.direct.audio_ssrc()
    }

    pub fn video_ssrc(&self) -> u32 {
        self.direct.video_ssrc()
    }

    pub fn set_on_state_change(&self, cb: ConnStateCallback) {
        *self.on_state_change.write() = Some(cb);
    }

    pub fn get_local_params_json(&self) -> Result<String, RustTgCallsError> {
        self.direct.build_local_params()
    }

    pub async fn connect(&self, remote_json: &str) -> Result<(), RustTgCallsError> {
        self.direct.connect(remote_json).await?;
        if let Some(ref cb) = *self.on_state_change.read() {
            cb(ConnState::Connected);
        }
        Ok(())
    }

    pub async fn send_rtp_bytes(
        &self,
        payload_type: u8,
        data: Bytes,
        duration: Duration,
    ) -> Result<(), RustTgCallsError> {
        self.direct.send_rtp_bytes(payload_type, data, duration).await
    }

    pub async fn write_rtp_frame(
        &self,
        payload_type: u8,
        data: Bytes,
        _timestamp_increment: u32,
    ) -> Result<(), RustTgCallsError> {
        self.direct
            .send_rtp_bytes(payload_type, data, Duration::from_millis(20))
            .await
    }

    pub fn reset_media_track_state(&self) {
        self.direct.reset_media_track_state();
    }

    pub fn state(&self) -> ConnState {
        self.direct.state()
    }

    pub async fn close(&self) -> Result<(), RustTgCallsError> {
        self.direct.close().await?;
        if let Some(ref cb) = *self.on_state_change.read() {
            cb(ConnState::Closed);
        }
        Ok(())
    }
}
