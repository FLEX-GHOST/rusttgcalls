//! Package wrtc wraps pure Rust WebRTC abstractions for Telegram SFU signaling,
//! JSON SDP/ICE parameter marshalling, peer connection lifecycle, and RTP packetization.

pub mod jsonparams;
pub mod keepalive;
pub mod native;
pub mod peer_connection;
pub mod peer_factory;

pub use jsonparams::{
    ERR_UNSUPPORTED_MODE,
    decode::parse_remote,
    types::{
        Candidate, Fingerprint, LocalParams, PayloadType, RTCPFb, RTPHdrExt, RemoteParams,
        SSRCGroup, Transport,
    },
};
pub use keepalive::FactoryMonitor;
pub use native::signaling;
pub use native::signaling::build_local_params_json;
pub use native::stack;
pub use native::stack::{ConnStateCallback, Stack};
pub use peer_connection::PeerConnection;
pub use peer_factory::PeerFactory;
