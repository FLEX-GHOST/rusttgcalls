//! Package jsonparams encodes and decodes the SDP-like JSON envelope that
//! Telegram's group-call signaling uses in place of standard SDP O/A.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// LocalParams is what we send to Telegram via phone.JoinGroupCall.
/// Field shape matches signaling requirements:
/// - "ssrc" is the audio SSRC only; video SSRC is inferred server-side.
/// - "ssrc-groups" is always emitted (as an empty array if there are no
///   video FID/SIM groups) — never as a cross-media FID:[audio, video] pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalParams {
    pub ufrag: String,
    pub pwd: String,
    pub fingerprints: Vec<Fingerprint>,
    #[serde(rename = "ssrc-groups")]
    pub ssrc_groups: Vec<SSRCGroup>,
    #[serde(rename = "payload-types")]
    pub payload_types: Vec<PayloadType>,
    #[serde(rename = "rtp-hdrexts")]
    pub rtp_hdrexts: Vec<RTPHdrExt>,
    pub ssrc: u32,
}

/// RemoteParams is what Telegram returns. Schema is more lenient than
/// LocalParams; unknown keys are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteParams {
    pub transport: Transport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transport {
    pub ufrag: String,
    pub pwd: String,
    pub fingerprints: Vec<Fingerprint>,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fingerprint {
    pub hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub setup: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SSRCGroup {
    pub semantics: String,
    pub sources: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadType {
    pub id: u8,
    pub name: String,
    pub clockrate: u32,
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    pub channels: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, String>>,
    #[serde(rename = "rtcp-fbs", default, skip_serializing_if = "Vec::is_empty")]
    pub rtcp_fbs: Vec<RTCPFb>,
}

fn is_zero_u16(v: &u16) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RTCPFb {
    #[serde(rename = "type")]
    pub feedback_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtype: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RTPHdrExt {
    pub id: u8,
    pub uri: String,
}

/// Candidate mirrors a libnice/ICE candidate. All String for forward
/// compatibility with Telegram's quoted-numeric serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub generation: String,
    pub component: String,
    pub protocol: String,
    pub port: String,
    pub ip: String,
    pub foundation: String,
    pub id: String,
    pub priority: String,
    #[serde(rename = "type")]
    pub candidate_type: String,
    pub network: String,
    #[serde(rename = "rel-addr", default, skip_serializing_if = "String::is_empty")]
    pub rel_addr: String,
    #[serde(rename = "rel-port", default, skip_serializing_if = "String::is_empty")]
    pub rel_port: String,
    #[serde(rename = "tcptype", default, skip_serializing_if = "String::is_empty")]
    pub tcp_type: String,
}
