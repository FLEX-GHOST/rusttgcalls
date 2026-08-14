use serde::{Deserialize, Serialize};
use std::fmt;

/// MediaState mirrors the Telegram MTProto participant flags
/// (muted / video_paused / video_stopped / presentation_paused).
///
/// - Muted: the bot toggled /mute on its outgoing audio.
/// - Paused: outgoing media is not actively flowing — true whenever
///   Muted OR the call was paused via Pause. Mirrors Telegram's
///   video_paused: set whenever the participant's mic is silent,
///   regardless of why it is silent.
/// - VideoStopped: the current source has no video track (true after
///   Play / audio-only; false after VPlay / audio+video).
/// - PresentationPaused: same lifecycle as Paused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaState {
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub paused: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub video_stopped: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub presentation_paused: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ConnState {
    #[default]
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

impl fmt::Display for ConnState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnState::Connecting => write!(f, "connecting"),
            ConnState::Connected => write!(f, "connected"),
            ConnState::Disconnected => write!(f, "disconnected"),
            ConnState::Failed => write!(f, "failed"),
            ConnState::Closed => write!(f, "closed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub state: ConnState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct CallInfo {
    pub capture_time_ms: u64,
}
