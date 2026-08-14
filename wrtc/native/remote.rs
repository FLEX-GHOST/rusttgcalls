//! Remote parameters parsing and ICE candidate transformation.

use crate::models::errors::RustTgCallsError;
use crate::wrtc::jsonparams::decode::parse_remote;
use crate::wrtc::jsonparams::types::Candidate;

/// RemoteParams is the parsed shape of Telegram's JoinGroupCall response.
/// Only the fields the native stack actually consumes — codec list and
/// header-extension IDs in the response are ignored because we hardcode
/// the Telegram-mandated set in signaling.rs (the SFU expects PT=111 Opus +
/// PT=100 VP8 regardless of what it advertises back).
#[derive(Debug, Clone)]
pub struct RemoteParams {
    pub ufrag: String,
    pub pwd: String,
    pub fingerprints: Vec<crate::wrtc::jsonparams::Fingerprint>,
    pub candidates: Vec<Candidate>,
}

/// parse_remote_json unmarshals Telegram's response. We reuse
/// jsonparams::parse_remote so the lenient/strict semantics stay aligned.
pub fn parse_remote_json(raw: &str) -> Result<RemoteParams, RustTgCallsError> {
    let rp = parse_remote(raw)?;
    Ok(RemoteParams {
        ufrag: rp.transport.ufrag,
        pwd: rp.transport.pwd,
        fingerprints: rp.transport.fingerprints,
        candidates: rp.transport.candidates,
    })
}

/// build_ice_candidate translates one of Telegram's JSON candidate entries
/// into a canonical SDP candidate line string.
pub fn build_ice_candidate(c: &Candidate) -> Result<String, RustTgCallsError> {
    if c.ip.is_empty() || c.port.is_empty() {
        return Err(RustTgCallsError::InvalidParams(
            "candidate missing ip/port".into(),
        ));
    }
    let foundation = if c.foundation.is_empty() {
        "1"
    } else {
        &c.foundation
    };
    let component = if c.component.is_empty() {
        "1"
    } else {
        &c.component
    };
    let protocol = if c.protocol.is_empty() {
        "UDP"
    } else {
        &c.protocol
    };
    let priority = if c.priority.is_empty() {
        "2130706431"
    } else {
        &c.priority
    };
    let cand_type = if c.candidate_type.is_empty() {
        "host"
    } else {
        &c.candidate_type
    };

    let mut s = format!(
        "{} {} {} {} {} {} typ {}",
        foundation, component, protocol, priority, c.ip, c.port, cand_type
    );

    if !c.rel_addr.is_empty() && !c.rel_port.is_empty() {
        s.push_str(&format!(" raddr {} rport {}", c.rel_addr, c.rel_port));
    }
    if !c.generation.is_empty() {
        s.push_str(&format!(" generation {}", c.generation));
    }
    if !c.tcp_type.is_empty() {
        s.push_str(&format!(" tcptype {}", c.tcp_type));
    }

    Ok(s)
}
