//! TranscodeSource runs one or two ffmpeg processes to produce ogg/Opus and
//! ivf/VP8 streams from a file or URL input.

use crate::io::ShellReader;
use crate::media::source::{
    EncodeOptions, SeekableSource, Source, Streams, TRACK_AUDIO, TRACK_VIDEO, Track,
    get_ffmpeg_path, is_stderr_log_enabled,
};
use crate::models::{OPUS_FRAME_DURATION_MS, OPUS_SAMPLE_RATE, errors::RustTgCallsError};
use std::time::Duration;

/// ErrNotSeekable is returned by open_at when the source has no seekable input.
pub const ERR_NOT_SEEKABLE: &str = "media: source is not seekable";

/// SourcePath is implemented by Sources backed by a plain file path or URL.
pub trait SourcePath {
    fn input_path(&self) -> &str;
    fn input_args(&self) -> &[String];
    fn encode_opts(&self) -> EncodeOptions;
}

/// TranscodeSource runs one or two ffmpeg processes to produce ogg/Opus and
/// ivf/VP8 streams from a file or URL input.
pub struct TranscodeSource {
    pub path: String,
    pub input_args: Vec<String>,
    pub opt: EncodeOptions,
}

impl SourcePath for TranscodeSource {
    fn input_path(&self) -> &str {
        &self.path
    }

    fn input_args(&self) -> &[String] {
        &self.input_args
    }

    fn encode_opts(&self) -> EncodeOptions {
        self.opt.with_defaults()
    }
}

impl Source for TranscodeSource {
    fn tracks(&self) -> Track {
        self.opt.with_defaults().tracks
    }

    fn open(&self) -> Result<Streams, RustTgCallsError> {
        self.open_with(&self.input_args)
    }
}

impl SeekableSource for TranscodeSource {
    fn open_at(&self, offset: Duration) -> Result<Streams, RustTgCallsError> {
        if self.path.is_empty() {
            return Err(RustTgCallsError::SeekUnsupported);
        }
        let mut args = ffmpeg_input_prefix(&self.path);
        args.push("-i".to_string());
        args.push(self.path.clone());
        if offset.as_millis() > 0 {
            args.push("-ss".to_string());
            args.push(format!("{:.3}", offset.as_secs_f64()));
        }
        self.open_with(&args)
    }
}

impl TranscodeSource {
    fn open_with(&self, input: &[String]) -> Result<Streams, RustTgCallsError> {
        let o = self.opt.with_defaults();
        let ffmpeg = get_ffmpeg_path();
        let stderr_log = is_stderr_log_enabled();

        let mut audio_reader: Option<ShellReader> = None;
        let mut video_reader: Option<ShellReader> = None;

        if o.tracks.has(TRACK_AUDIO) {
            let args = audio_ff_args(input, &o);
            let r = ShellReader::new(&ffmpeg, &args, stderr_log)
                .map_err(|e| RustTgCallsError::FFmpegSpawn(format!("audio ffmpeg: {}", e)))?;
            audio_reader = Some(r);
        }

        if o.tracks.has(TRACK_VIDEO) {
            let args = video_ff_args(input, &o);
            let r = ShellReader::new(&ffmpeg, &args, stderr_log)
                .map_err(|e| RustTgCallsError::FFmpegSpawn(format!("video ffmpeg: {}", e)))?;
            video_reader = Some(r);
        }

        Ok(Streams {
            audio: audio_reader
                .map(|r| -> Box<dyn tokio::io::AsyncRead + Unpin + Send> { Box::new(r) }),
            video: video_reader
                .map(|r| -> Box<dyn tokio::io::AsyncRead + Unpin + Send> { Box::new(r) }),
            close: None,
        })
    }
}

pub fn audio_ff_args(input: &[String], o: &EncodeOptions) -> Vec<String> {
    let mut args = Vec::with_capacity(36 + input.len());
    args.push("-hide_banner".to_string());
    args.push("-loglevel".to_string());
    args.push("error".to_string());
    args.push("-nostdin".to_string());
    args.push("-fflags".to_string());
    args.push("+discardcorrupt+genpts".to_string());
    args.push("-err_detect".to_string());
    args.push("ignore_err".to_string());

    args.extend(input.iter().cloned());

    args.push("-map".to_string());
    args.push("0:a?".to_string());
    args.push("-vn".to_string());
    args.push("-sn".to_string());
    args.push("-dn".to_string());
    args.push("-c:a".to_string());
    args.push("libopus".to_string());
    args.push("-b:a".to_string());
    args.push(format!("{}k", o.audio_bitrate_kbps));
    args.push("-vbr".to_string());
    args.push("on".to_string());
    args.push("-compression_level".to_string());
    args.push("10".to_string());
    args.push("-frame_duration".to_string());
    args.push(OPUS_FRAME_DURATION_MS.to_string());
    args.push("-page_duration".to_string());
    args.push((OPUS_FRAME_DURATION_MS * 1000).to_string());
    args.push("-application".to_string());
    args.push("audio".to_string());
    args.push("-mapping_family".to_string());
    args.push("0".to_string());
    args.push("-ar".to_string());
    args.push(OPUS_SAMPLE_RATE.to_string());
    args.push("-ac".to_string());
    args.push(o.audio_channels.to_string());
    args.push("-f".to_string());
    args.push("ogg".to_string());
    args.push("pipe:1".to_string());
    args
}

/// videoFFArgs builds ffmpeg arguments for video VP8 stream.
pub fn video_ff_args(input: &[String], o: &EncodeOptions) -> Vec<String> {
    let rate = format!("{}k", o.video_bitrate_kbps);
    let max_rate = format!("{}k", (o.video_bitrate_kbps as f64 * 1.5) as u32);
    let buf_size = format!("{}k", o.video_bitrate_kbps * 2);
    let gop = o.video_fps.to_string(); // 1 keyframe every 1.0 second for crystal-clear playback and fast joiner sync
    let mut args = Vec::with_capacity(50 + input.len());
    args.push("-hide_banner".to_string());
    args.push("-loglevel".to_string());
    args.push("error".to_string());
    args.push("-nostdin".to_string());
    args.push("-fflags".to_string());
    args.push("+discardcorrupt+genpts".to_string());
    args.push("-err_detect".to_string());
    args.push("ignore_err".to_string());

    args.extend(input.iter().cloned());

    args.push("-map".to_string());
    args.push("0:v?".to_string());
    args.push("-an".to_string());
    args.push("-sn".to_string());
    args.push("-dn".to_string());
    args.push("-c:v".to_string());
    args.push("libvpx".to_string());
    args.push("-b:v".to_string());
    args.push(rate);
    args.push("-maxrate".to_string());
    args.push(max_rate);
    args.push("-bufsize".to_string());
    args.push(buf_size);
    args.push("-quality".to_string());
    args.push("realtime".to_string());
    args.push("-deadline".to_string());
    args.push("realtime".to_string());
    args.push("-cpu-used".to_string());
    args.push("16".to_string());
    args.push("-threads".to_string());
    args.push("4".to_string());
    args.push("-slices".to_string());
    args.push("4".to_string());
    args.push("-arnr-maxframes".to_string());
    args.push("0".to_string());
    args.push("-rc_lookahead".to_string());
    args.push("0".to_string());
    args.push("-lag-in-frames".to_string());
    args.push("0".to_string());
    args.push("-vf".to_string());
    args.push(format!("scale={}:{}", o.video_width, o.video_height));
    args.push("-r".to_string());
    args.push(o.video_fps.to_string());
    args.push("-g".to_string());
    args.push(gop.clone());
    args.push("-keyint_min".to_string());
    args.push(gop);
    args.push("-auto-alt-ref".to_string());
    args.push("0".to_string());
    args.push("-error-resilient".to_string());
    args.push("1".to_string());
    args.push("-f".to_string());
    args.push("ivf".to_string());
    args.push("pipe:1".to_string());
    args
}

/// ffmpegInputPrefix replicates the m3u8/http/local flag logic.
pub fn ffmpeg_input_prefix(input: &str) -> Vec<String> {
    let is_m3u8 = input.contains(".m3u8");
    let is_http = input.starts_with("http://") || input.starts_with("https://");
    if is_m3u8 {
        vec![
            "-user_agent".to_string(),
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36".to_string(),
            "-protocol_whitelist".to_string(),
            "file,http,https,tcp,tls".to_string(),
            "-rw_timeout".to_string(),
            "10000000".to_string(),
            "-http_persistent".to_string(),
            "1".to_string(),
            "-analyzeduration".to_string(),
            "0".to_string(),
            "-probesize".to_string(),
            "32k".to_string(),
        ]
    } else if is_http {
        vec![
            "-user_agent".to_string(),
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36".to_string(),
            "-reconnect".to_string(),
            "1".to_string(),
            "-reconnect_at_eof".to_string(),
            "1".to_string(),
            "-reconnect_streamed".to_string(),
            "1".to_string(),
            "-reconnect_on_network_error".to_string(),
            "1".to_string(),
            "-reconnect_on_http_error".to_string(),
            "4xx,5xx".to_string(),
            "-reconnect_delay_max".to_string(),
            "5".to_string(),
            "-rw_timeout".to_string(),
            "15000000".to_string(),
            "-timeout".to_string(),
            "15000000".to_string(),
            "-analyzeduration".to_string(),
            "0".to_string(),
            "-probesize".to_string(),
            "128k".to_string(),
        ]
    } else {
        vec![
            "-analyzeduration".to_string(),
            "0".to_string(),
            "-probesize".to_string(),
            "64k".to_string(),
        ]
    }
}

/// FromFile streams any ffmpeg-decodable file (mp4, mkv, webm, mp3, wav, ...).
/// Seekable. Pass EncodeOptions::default() for defaults.
pub fn from_file(path: &str, opt: EncodeOptions) -> Result<Streams, RustTgCallsError> {
    from_file_offset(path, Duration::from_secs(0), opt)
}

/// FromFileOffset streams from a file starting at a specific timestamp offset.
pub fn from_file_offset(path: &str, offset: Duration, opt: EncodeOptions) -> Result<Streams, RustTgCallsError> {
    let mut prefix = ffmpeg_input_prefix(path);
    prefix.push("-i".to_string());
    prefix.push(path.to_string());
    if offset.as_millis() > 0 {
        prefix.push("-ss".to_string());
        prefix.push(format!("{:.3}", offset.as_secs_f64()));
    }
    let src = TranscodeSource {
        path: path.to_string(),
        input_args: prefix,
        opt,
    };
    src.open()
}

/// FromURL streams from a URL (http(s), hls/.m3u8, rtmp, ...). Seekable.
/// Pass EncodeOptions::default() for defaults.
pub fn from_url(url: &str, opt: EncodeOptions) -> Result<Streams, RustTgCallsError> {
    from_url_offset(url, Duration::from_secs(0), opt)
}

/// FromURLOffset streams from a URL starting at a specific timestamp offset.
pub fn from_url_offset(url: &str, offset: Duration, opt: EncodeOptions) -> Result<Streams, RustTgCallsError> {
    let mut prefix = ffmpeg_input_prefix(url);
    prefix.push("-i".to_string());
    prefix.push(url.to_string());
    if offset.as_millis() > 0 {
        prefix.push("-ss".to_string());
        prefix.push(format!("{:.3}", offset.as_secs_f64()));
    }
    let src = TranscodeSource {
        path: url.to_string(),
        input_args: prefix,
        opt,
    };
    src.open()
}
