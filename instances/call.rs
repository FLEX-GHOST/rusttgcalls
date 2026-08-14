//! Package instances holds the per-chat call state. One Call trait,
//! two implementations: GroupCall (WebRTC, default) and RTMPCall (FFmpeg
//! push to a Telegram-issued RTMP URL).

use crate::media::source::Streams;
use crate::models::{ConnState, MediaState, errors::RustTgCallsError};
use std::future::Future;
use std::pin::Pin;

/// Call is the per-chat interface the top-level Client multiplexes over.
pub trait Call: Send + Sync {
    /// CreateLocalParams produces the local-side JSON. WebRTC mode only;
    /// RTMPCall returns ErrWrongMode.
    fn create_local_params(&self) -> Result<String, RustTgCallsError>;

    /// Connect feeds Telegram's response JSON. WebRTC mode only.
    fn connect<'a>(
        &'a self,
        remote_json: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), RustTgCallsError>> + Send + 'a>>;

    /// SetSource installs the streaming source. Replaces atomically.
    fn set_source<'a>(
        &'a self,
        streams: Streams,
    ) -> Pin<Box<dyn Future<Output = Result<(), RustTgCallsError>> + Send + 'a>>;

    fn pause(&self) -> Result<bool, RustTgCallsError>;
    fn resume(&self) -> Result<bool, RustTgCallsError>;
    fn mute(&self) -> Result<bool, RustTgCallsError>;
    fn unmute(&self) -> Result<bool, RustTgCallsError>;
    fn stop(&self) -> Result<(), RustTgCallsError>;

    /// SeekBy shifts playback by delta_ms relative to the current position
    /// (positive forward, negative backward). Underflow below 0 triggers
    /// EOF via the OnStreamEnd path. Forward overshoots past the source
    /// duration are detected naturally by ffmpeg yielding zero frames.
    /// Returns ErrSeekUnsupported if the active source is not seekable
    /// and ErrNoSource if nothing is currently playing.
    fn seek_by(&self, delta_ms: i64) -> Result<(), RustTgCallsError>;

    fn elapsed_ms(&self) -> u64;
    fn state(&self) -> MediaState;
    fn net_state(&self) -> ConnState;
    fn audio_ssrc(&self) -> Result<u32, RustTgCallsError>;

    /// Mode returns either "webrtc" or "rtmp" so the Client can guard
    /// mode-specific operations.
    fn mode(&self) -> &'static str;
}
