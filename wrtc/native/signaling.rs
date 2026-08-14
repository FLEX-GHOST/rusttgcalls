use crate::models::{
    DEFAULT_CHANNEL_COUNT, OPUS_PAYLOAD_TYPE, OPUS_SAMPLE_RATE, VP8_PAYLOAD_TYPE,
    errors::RustTgCallsError,
};
use crate::wrtc::jsonparams::{
    Fingerprint, LocalParams, PayloadType, RTCPFb, RTPHdrExt, SSRCGroup,
};
use std::collections::HashMap;

pub const EXT_AUDIO_LEVEL_ID: u8 = 1;
pub const EXT_ABS_SEND_TIME_ID: u8 = 2;
pub const EXT_TRANSPORT_CC_ID: u8 = 3;
pub const EXT_SDES_MID_ID: u8 = 4;
pub const EXT_VIDEO_ORIENTATION_ID: u8 = 5;

pub const URI_AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const URI_ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const URI_TRANSPORT_CC: &str =
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const URI_SDES_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const URI_VIDEO_ORIENTATION: &str = "urn:3gpp:video-orientation";

pub fn build_local_params_json(
    ufrag: &str,
    pwd: &str,
    fingerprint_sha256: &str,
    audio_ssrc: u32,
    video_ssrc: u32,
) -> Result<String, RustTgCallsError> {
    if ufrag.is_empty() || pwd.is_empty() || fingerprint_sha256.is_empty() {
        return Err(RustTgCallsError::InvalidParams(
            "empty ufrag, pwd, or fingerprint".into(),
        ));
    }
    if audio_ssrc == 0 {
        return Err(RustTgCallsError::InvalidParams(
            "audio_ssrc cannot be zero".into(),
        ));
    }

    let mut opus_params = HashMap::new();
    opus_params.insert("minptime".to_string(), "10".to_string());
    opus_params.insert("useinbandfec".to_string(), "1".to_string());
    opus_params.insert("stereo".to_string(), "1".to_string());
    opus_params.insert("sprop-stereo".to_string(), "1".to_string());
    opus_params.insert("maxaveragebitrate".to_string(), "510000".to_string());

    let ssrc_groups = if video_ssrc != 0 {
        vec![SSRCGroup {
            semantics: "FID".to_string(),
            sources: vec![video_ssrc, video_ssrc.wrapping_add(1)],
        }]
    } else {
        vec![]
    };

    let payload_types = vec![
        PayloadType {
            id: OPUS_PAYLOAD_TYPE,
            name: "opus".to_string(),
            clockrate: OPUS_SAMPLE_RATE,
            channels: DEFAULT_CHANNEL_COUNT,
            parameters: Some(opus_params),
            rtcp_fbs: vec![RTCPFb {
                feedback_type: "transport-cc".to_string(),
                subtype: "".to_string(),
            }],
        },
        PayloadType {
            id: VP8_PAYLOAD_TYPE,
            name: "VP8".to_string(),
            clockrate: 90000,
            channels: 0,
            parameters: None,
            rtcp_fbs: vec![
                RTCPFb {
                    feedback_type: "goog-remb".to_string(),
                    subtype: "".to_string(),
                },
                RTCPFb {
                    feedback_type: "transport-cc".to_string(),
                    subtype: "".to_string(),
                },
                RTCPFb {
                    feedback_type: "ccm".to_string(),
                    subtype: "fir".to_string(),
                },
                RTCPFb {
                    feedback_type: "nack".to_string(),
                    subtype: "".to_string(),
                },
                RTCPFb {
                    feedback_type: "nack".to_string(),
                    subtype: "pli".to_string(),
                },
            ],
        },
    ];

    let rtp_hdrexts = vec![
        RTPHdrExt {
            id: EXT_AUDIO_LEVEL_ID,
            uri: URI_AUDIO_LEVEL.to_string(),
        },
        RTPHdrExt {
            id: EXT_ABS_SEND_TIME_ID,
            uri: URI_ABS_SEND_TIME.to_string(),
        },
        RTPHdrExt {
            id: EXT_TRANSPORT_CC_ID,
            uri: URI_TRANSPORT_CC.to_string(),
        },
        RTPHdrExt {
            id: EXT_SDES_MID_ID,
            uri: URI_SDES_MID.to_string(),
        },
        RTPHdrExt {
            id: EXT_VIDEO_ORIENTATION_ID,
            uri: URI_VIDEO_ORIENTATION.to_string(),
        },
    ];

    let lp = LocalParams {
        ufrag: ufrag.to_string(),
        pwd: pwd.to_string(),
        ssrc: audio_ssrc,
        fingerprints: vec![Fingerprint {
            hash: "sha-256".to_string(),
            setup: "passive".to_string(),
            fingerprint: fingerprint_sha256.to_string(),
        }],
        payload_types,
        rtp_hdrexts,
        ssrc_groups,
    };

    simd_json::to_string(&lp)
        .or_else(|_| serde_json::to_string(&lp))
        .map_err(|e| RustTgCallsError::Internal(e.to_string()))
}

