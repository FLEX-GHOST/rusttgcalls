//! Package native implements pure WebRTC / RTP / DTLS / ICE abstractions for Telegram SFU calls.

pub mod certpool;
pub mod direct_transport;
pub mod factory;
pub mod keys;
pub mod remote;
pub mod signaling;
pub mod stack;
pub mod track;

pub use certpool::{CertPool, GeneratedCert, generate_cert};
pub use direct_transport::DirectTransport;
pub use factory::{Factory, FactoryOptions};
pub use keys::{DTLS_SRTP_EXPORTER_LABEL, ProtectionProfile};
pub use remote::{RemoteParams, build_ice_candidate, parse_remote_json};
pub use signaling::{
    EXT_ABS_SEND_TIME_ID, EXT_AUDIO_LEVEL_ID, EXT_SDES_MID_ID, EXT_TRANSPORT_CC_ID,
    EXT_VIDEO_ORIENTATION_ID, URI_ABS_SEND_TIME, URI_AUDIO_LEVEL, URI_SDES_MID, URI_TRANSPORT_CC,
    URI_VIDEO_ORIENTATION, build_local_params_json,
};
pub use stack::{ConnStateFn, Stack};
pub use track::{Kind, Track};
