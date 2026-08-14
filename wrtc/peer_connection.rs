//! PeerConnection wraps the underlying WebRTC Stack with lifecycle management,
//! monitor registration, and state event forwarding.

use crate::models::{ConnState, errors::RustTgCallsError};
use crate::wrtc::native::stack::{ConnStateCallback, Stack};
use crate::wrtc::peer_factory::PeerFactory;
use std::sync::Arc;
use tokio::sync::RwLock;

/// PeerConnection wraps a native::Stack with the method surface callers use
/// during the PeerConnection lifecycle. The wrapper owns no per-call tasks
/// beyond what the Stack already runs.
pub struct PeerConnection {
    stack: Arc<Stack>,
    on_state_change: Arc<RwLock<Option<ConnStateCallback>>>,
}

impl PeerConnection {
    /// NewPeerConnection constructs a PeerConnection by taking a Stack from
    /// the factory's pool.
    pub async fn new(factory: &PeerFactory) -> Result<Arc<Self>, RustTgCallsError> {
        let stack = Stack::new(factory).await?;
        let on_state_change = Arc::new(RwLock::new(None));

        let pc = Arc::new(Self {
            stack,
            on_state_change,
        });

        let pc_clone = Arc::clone(&pc);
        pc.stack.set_on_state_change(Arc::new(move |s: ConnState| {
            let pc = Arc::clone(&pc_clone);
            tokio::spawn(async move {
                let lock = pc.on_state_change.read().await;
                if let Some(cb) = lock.as_ref() {
                    cb(s);
                }
            });
        }));

        Ok(pc)
    }

    /// LocalParams returns the JoinGroupCall blob — credentials, fingerprint,
    /// SSRCs, codec/extension manifest.
    pub fn local_params(&self) -> Result<String, RustTgCallsError> {
        self.stack.get_local_params_json()
    }

    /// Connect applies Telegram's response, runs ICE+DTLS+SRTP setup, and
    /// returns once the connection is ready to carry media.
    pub async fn connect(&self, remote_json: &str) -> Result<(), RustTgCallsError> {
        self.stack.connect(remote_json).await
    }

    /// AudioSSRC reports the audio SSRC Telegram associates with this participant.
    pub fn audio_ssrc(&self) -> u32 {
        self.stack.audio_ssrc()
    }

    /// VideoSSRC reports the video SSRC announced in LocalParams.
    pub fn video_ssrc(&self) -> u32 {
        self.stack.video_ssrc()
    }

    /// OnConnectionStateChange registers a callback for state transitions.
    pub fn on_connection_state_change(&self, cb: ConnStateCallback) {
        let lock = Arc::clone(&self.on_state_change);
        tokio::spawn(async move {
            *lock.write().await = Some(cb);
        });
    }

    /// State exposes the current connection state.
    pub fn state(&self) -> ConnState {
        self.stack.state()
    }

    /// Close tears down the underlying connection.
    pub async fn close(&self) -> Result<(), RustTgCallsError> {
        self.stack.close().await
    }

    /// Inner Stack accessor.
    pub fn stack(&self) -> Arc<Stack> {
        Arc::clone(&self.stack)
    }
}
