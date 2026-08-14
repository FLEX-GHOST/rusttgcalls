//! Package media handles audio/video source preparation, transcoding via FFmpeg,
//! OGG/IVF frame parsing, and WebRTC sample streaming pacing.

pub mod frame_reader;
pub mod shell_source;
pub mod source;
pub mod streamer;
pub mod transcode;

pub use frame_reader::{OpusFrameReader, Sample, VP8FrameReader};
pub use shell_source::{from_shell, from_shells, tokenize_shell};
pub use source::{
    EncodeOptions, RawBytesSource, SeekableSource, Source, Streams, TRACK_AUDIO, TRACK_VIDEO,
    Track, from_raw_audio, get_ffmpeg_path, is_stderr_log_enabled, set_ffmpeg_path, set_stderr_log,
};
pub use streamer::Streamer;
pub use transcode::{
    ERR_NOT_SEEKABLE, SourcePath, TranscodeSource, audio_ff_args, from_file, from_file_offset,
    from_url, from_url_offset, video_ff_args,
};
