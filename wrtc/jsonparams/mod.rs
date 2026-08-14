//! Package jsonparams encodes and decodes the SDP-like JSON envelope that
//! Telegram's group-call signaling uses in place of standard SDP O/A.

pub mod decode;
pub mod types;

pub use decode::{ERR_UNSUPPORTED_MODE, parse_remote};
pub use types::{
    Candidate, Fingerprint, LocalParams, PayloadType, RTCPFb, RTPHdrExt,
    RemoteParams, SSRCGroup, Transport,
};
