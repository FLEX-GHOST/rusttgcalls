use crate::models::errors::RustTgCallsError;
use crate::wrtc::jsonparams::types::RemoteParams;
use serde::Deserialize;

/// ErrUnsupportedMode signals that Telegram's response describes the group
/// call as an RTMP livestream or an MTProto broadcast stream rather than
/// a WebRTC call. The caller must surface this as "not joinable as a voice chat".
pub const ERR_UNSUPPORTED_MODE: &str = "call mode unsupported";

/// ParseRemote decodes Telegram's response JSON. Lenient: unknown top-level
/// keys are ignored. Missing required keys (transport.ufrag/pwd/fingerprints)
/// yield a typed error. RTMP/Stream responses yield UnsupportedCallMode.
pub fn parse_remote(raw: &str) -> Result<RemoteParams, RustTgCallsError> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        rtmp: serde_json::Value,
        #[serde(default)]
        stream: serde_json::Value,
    }

    let mut probe_buf = raw.as_bytes().to_vec();
    if let Ok(probe) = simd_json::from_slice::<Probe>(&mut probe_buf) {
        if !probe.rtmp.is_null() || !probe.stream.is_null() {
            return Err(RustTgCallsError::UnsupportedCallMode);
        }
    }

    let mut parse_buf = raw.as_bytes().to_vec();
    let rp: RemoteParams = simd_json::from_slice(&mut parse_buf)
        .or_else(|_| serde_json::from_str(raw))
        .map_err(|e| RustTgCallsError::InvalidParams(format!("decode remote params: {}", e)))?;

    if rp.transport.ufrag.is_empty() || rp.transport.pwd.is_empty() {
        return Err(RustTgCallsError::InvalidParams(
            "remote params missing ice creds".into(),
        ));
    }
    if rp.transport.fingerprints.is_empty() {
        return Err(RustTgCallsError::InvalidParams(
            "remote params missing fingerprint".into(),
        ));
    }

    Ok(rp)
}
