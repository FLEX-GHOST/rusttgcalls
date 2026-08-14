use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RustTgCallsError {
    #[error("rusttgcalls: call already exists for chat")]
    ConnectionExists,

    #[error("rusttgcalls: no call for chat")]
    ConnectionNotFound,

    #[error("rusttgcalls: ICE failed permanently")]
    ConnectionFailed,

    #[error("rusttgcalls: malformed telegram params: {0}")]
    InvalidParams(String),

    #[error("rusttgcalls: call mode not supported")]
    UnsupportedCallMode,

    #[error("rusttgcalls: ffmpeg failed to start: {0}")]
    FFmpegSpawn(String),

    #[error("rusttgcalls: ffmpeg exited non-zero: {0}")]
    FFmpegCrashed(String),

    #[error("rusttgcalls: webrtc error: {0}")]
    WebRTC(String),

    #[error("rusttgcalls: input file unreadable")]
    File,

    #[error("rusttgcalls: client closed")]
    Closed,

    #[error("rusttgcalls: internal error: {0}")]
    Internal(String),

    #[error("rusttgcalls: call not connected")]
    NotConnected,

    #[error("rusttgcalls: operation not valid for call mode")]
    WrongMode,

    #[error("rusttgcalls: no source currently playing")]
    NoSource,

    #[error("rusttgcalls: source is not seekable")]
    SeekUnsupported,
}
