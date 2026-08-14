//! Track is a bitmask selecting which tracks a Source provides.
//! Streams is the encoded output of a Source: ogg/Opus audio and/or IVF/VP8 video.

use crate::models::DEFAULT_CHANNEL_COUNT;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, RwLock};
use std::time::Duration;
use tokio::io::AsyncRead;

static FFMPEG_BINARY: LazyLock<RwLock<String>> =
    LazyLock::new(|| RwLock::new("ffmpeg".to_string()));

static STDERR_LOG: AtomicBool = AtomicBool::new(false);

/// SetFFmpegPath overrides the binary used for transcoding. Empty resets to "ffmpeg".
pub fn set_ffmpeg_path(path: &str) {
    let mut lock = FFMPEG_BINARY.write().unwrap();
    if path.is_empty() {
        *lock = "ffmpeg".to_string();
    } else {
        *lock = path.to_string();
    }
}

/// get_ffmpeg_path returns configured ffmpeg binary path or "ffmpeg".
pub fn get_ffmpeg_path() -> String {
    FFMPEG_BINARY.read().unwrap().clone()
}

/// SetStderrLog toggles the live-tee of ffmpeg stderr to the package logger.
pub fn set_stderr_log(on: bool) {
    STDERR_LOG.store(on, Ordering::SeqCst);
}

/// StderrLogEnabled reports the current setting (read by io.ShellReader).
pub fn is_stderr_log_enabled() -> bool {
    STDERR_LOG.load(Ordering::SeqCst)
}

/// Track is a bitmask selecting which tracks a Source provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Track(pub u8);

pub const TRACK_AUDIO: Track = Track(1 << 0);
pub const TRACK_VIDEO: Track = Track(1 << 1);

impl Track {
    pub fn has(&self, other: Track) -> bool {
        (self.0 & other.0) != 0
    }
}

impl std::ops::BitOr for Track {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Track(self.0 | rhs.0)
    }
}

/// Streams is the encoded output of a Source: ogg/Opus audio and/or IVF/VP8
/// video. A None reader means that track is absent. Close releases any
/// underlying ffmpeg processes and pipes.
#[derive(Default)]
pub struct Streams {
    pub audio: Option<Box<dyn AsyncRead + Unpin + Send>>,
    pub video: Option<Box<dyn AsyncRead + Unpin + Send>>,
    pub close: Option<Box<dyn FnOnce() + Send>>,
}

impl Streams {
    pub fn close(mut self) {
        if let Some(f) = self.close.take() {
            f();
        }
    }
}

/// Source is the public input abstraction. A Source is created lazily;
/// Open spawns whatever processes are needed and returns the encoded
/// audio+video byte streams.
pub trait Source: Send + Sync {
    fn tracks(&self) -> Track;
    fn open(&self) -> Result<Streams, crate::models::RustTgCallsError>;
}

/// SeekableSource is a Source that can begin playback at an offset. Only
/// file/URL transcode sources implement it; pre-encoded passthrough sources
/// and stdin-fed reader sources do not.
pub trait SeekableSource: Source {
    fn open_at(&self, offset: Duration) -> Result<Streams, crate::models::RustTgCallsError>;
}

/// EncodeOptions tunes the ffmpeg encode for transcoding sources. Zero
/// values become sensible defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeOptions {
    pub video_bitrate_kbps: u32,
    pub video_width: u32,
    pub video_height: u32,
    pub video_fps: u32,
    pub audio_bitrate_kbps: u32,
    pub audio_channels: u16,
    pub tracks: Track,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            video_bitrate_kbps: 800,
            video_width: 1280,
            video_height: 720,
            video_fps: 30,
            audio_bitrate_kbps: 128,
            audio_channels: DEFAULT_CHANNEL_COUNT,
            tracks: TRACK_AUDIO,
        }
    }
}

impl EncodeOptions {
    /// Preset for high-fidelity audio only.
    pub fn audio_only() -> Self {
        Self {
            tracks: TRACK_AUDIO,
            ..Default::default()
        }
    }

    /// Default preset: 480p at 60 FPS (ultra-smooth 60 FPS real-time streaming without slow motion).
    pub fn video_default() -> Self {
        Self {
            video_width: 854,
            video_height: 480,
            video_fps: 60,
            video_bitrate_kbps: 1200,
            tracks: TRACK_AUDIO | TRACK_VIDEO,
            ..Default::default()
        }
    }

    /// Alias for smooth 60 FPS video.
    pub fn video_60fps() -> Self {
        Self::video_default()
    }

    /// Fast low-latency preset (360p at 30 FPS).
    pub fn video_fast() -> Self {
        Self {
            video_width: 640,
            video_height: 360,
            video_fps: 30,
            video_bitrate_kbps: 800,
            tracks: TRACK_AUDIO | TRACK_VIDEO,
            ..Default::default()
        }
    }

    /// Preset for 720p at 30 FPS.
    pub fn video_720p_30fps() -> Self {
        Self {
            video_width: 1280,
            video_height: 720,
            video_fps: 30,
            video_bitrate_kbps: 1500,
            tracks: TRACK_AUDIO | TRACK_VIDEO,
            ..Default::default()
        }
    }

    /// Preset for 720p at 60 FPS (high frame rate).
    pub fn video_720p_60fps() -> Self {
        Self {
            video_width: 1280,
            video_height: 720,
            video_fps: 60,
            video_bitrate_kbps: 2500,
            tracks: TRACK_AUDIO | TRACK_VIDEO,
            ..Default::default()
        }
    }

    /// Preset for 1080p Full HD at 60 FPS.
    pub fn video_1080p_60fps() -> Self {
        Self {
            video_width: 1920,
            video_height: 1080,
            video_fps: 60,
            video_bitrate_kbps: 4000,
            tracks: TRACK_AUDIO | TRACK_VIDEO,
            ..Default::default()
        }
    }

    pub fn with_defaults(mut self) -> Self {
        if self.video_bitrate_kbps == 0 {
            self.video_bitrate_kbps = 800;
        }
        if self.video_width == 0 {
            self.video_width = 1280;
        }
        if self.video_height == 0 {
            self.video_height = 720;
        }
        if self.video_fps == 0 {
            self.video_fps = 30;
        }
        if self.audio_bitrate_kbps == 0 {
            self.audio_bitrate_kbps = 128;
        }
        if self.audio_channels == 0 {
            self.audio_channels = DEFAULT_CHANNEL_COUNT;
        }
        if self.tracks.0 == 0 {
            self.tracks = TRACK_AUDIO;
        }
        if self.tracks.has(TRACK_VIDEO) {
            self.tracks.0 |= TRACK_AUDIO.0;
        }
        self
    }
}

/// RawBytesSource allows pure in-memory streaming of pre-encoded Opus or VP8 bytes.
#[derive(Clone)]
pub struct RawBytesSource {
    audio_data: Option<bytes::Bytes>,
    video_data: Option<bytes::Bytes>,
    tracks: Track,
}

impl RawBytesSource {
    pub fn new_audio(data: bytes::Bytes) -> Self {
        Self {
            audio_data: Some(data),
            video_data: None,
            tracks: TRACK_AUDIO,
        }
    }

    pub fn new_media(audio: Option<bytes::Bytes>, video: Option<bytes::Bytes>) -> Self {
        let mut t = Track(0);
        if audio.is_some() {
            t.0 |= TRACK_AUDIO.0;
        }
        if video.is_some() {
            t.0 |= TRACK_VIDEO.0;
        }
        Self {
            audio_data: audio,
            video_data: video,
            tracks: t,
        }
    }
}

impl Source for RawBytesSource {
    fn tracks(&self) -> Track {
        self.tracks
    }

    fn open(&self) -> Result<Streams, crate::models::RustTgCallsError> {
        let audio: Option<Box<dyn AsyncRead + Unpin + Send>> = self
            .audio_data
            .clone()
            .map(|d| Box::new(std::io::Cursor::new(d)) as Box<dyn AsyncRead + Unpin + Send>);
        let video: Option<Box<dyn AsyncRead + Unpin + Send>> = self
            .video_data
            .clone()
            .map(|d| Box::new(std::io::Cursor::new(d)) as Box<dyn AsyncRead + Unpin + Send>);

        Ok(Streams {
            audio,
            video,
            close: None,
        })
    }
}

/// from_raw_audio creates a Source that streams pure in-memory Opus bytes without disk or process overhead.
pub fn from_raw_audio(data: bytes::Bytes) -> RawBytesSource {
    RawBytesSource::new_audio(data)
}
