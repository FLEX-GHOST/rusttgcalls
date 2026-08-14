//! FromShell parses cmdline as a shell command and spawns it directly.
//! MultiShellSource allows dual custom ffmpeg processes (audio + video).

use crate::io::ShellReader;
use crate::media::source::{Streams, TRACK_AUDIO, TRACK_VIDEO, Track};
use crate::models::errors::RustTgCallsError;
use std::time::Duration;

/// ShellSource streams from a custom ffmpeg shell command line.
pub struct ShellSource {
    pub binary: String,
    pub args: Vec<String>,
    pub track: Track,
}

impl ShellSource {
    pub fn tracks(&self) -> Track {
        if self.track.0 == 0 {
            TRACK_AUDIO
        } else {
            self.track
        }
    }

    pub fn open(&self) -> Result<Streams, RustTgCallsError> {
        self.open_with(&self.args)
    }

    pub fn open_at(&self, offset: Duration) -> Result<Streams, RustTgCallsError> {
        if offset.is_zero() {
            self.open_with(&self.args)
        } else {
            self.open_with(&inject_seek(&self.args, offset))
        }
    }

    fn open_with(&self, args: &[String]) -> Result<Streams, RustTgCallsError> {
        let bin = if self.binary.is_empty() {
            "ffmpeg"
        } else {
            &self.binary
        };
        let reader = ShellReader::new(bin, args, false)?;
        let mut streams = Streams::default();
        if self.tracks().has(TRACK_AUDIO) {
            streams.audio = Some(Box::new(reader));
        } else if self.tracks().has(TRACK_VIDEO) {
            streams.video = Some(Box::new(reader));
        }
        Ok(streams)
    }
}

/// MultiShellSource builds a source from two separate ffmpeg commands
/// (one for audio, one for video).
pub struct MultiShellSource {
    pub audio_bin: String,
    pub video_bin: String,
    pub audio_args: Vec<String>,
    pub video_args: Vec<String>,
    pub parallel: bool,
}

impl MultiShellSource {
    /// WithParallelSpawn opts into starting both ffmpeg legs concurrently.
    pub fn with_parallel_spawn(mut self) -> Self {
        self.parallel = true;
        self
    }

    pub fn tracks(&self) -> Track {
        let mut t = Track(0);
        if !self.audio_args.is_empty() {
            t.0 |= TRACK_AUDIO.0;
        }
        if !self.video_args.is_empty() {
            t.0 |= TRACK_VIDEO.0;
        }
        t
    }

    pub fn open(&self) -> Result<Streams, RustTgCallsError> {
        self.open_with(&self.audio_args, &self.video_args)
    }

    pub fn open_at(&self, offset: Duration) -> Result<Streams, RustTgCallsError> {
        if offset.is_zero() {
            self.open_with(&self.audio_args, &self.video_args)
        } else {
            let audio = if !self.audio_args.is_empty() {
                inject_seek(&self.audio_args, offset)
            } else {
                Vec::new()
            };
            let video = if !self.video_args.is_empty() {
                inject_seek(&self.video_args, offset)
            } else {
                Vec::new()
            };
            self.open_with(&audio, &video)
        }
    }

    fn open_with(
        &self,
        audio_args: &[String],
        video_args: &[String],
    ) -> Result<Streams, RustTgCallsError> {
        let mut streams = Streams::default();

        if !audio_args.is_empty() {
            let bin = if self.audio_bin.is_empty() {
                "ffmpeg"
            } else {
                &self.audio_bin
            };
            let r = ShellReader::new(bin, audio_args, false)?;
            streams.audio = Some(Box::new(r));
        }

        if !video_args.is_empty() {
            let bin = if self.video_bin.is_empty() {
                "ffmpeg"
            } else {
                &self.video_bin
            };
            let r = ShellReader::new(bin, video_args, false)?;
            streams.video = Some(Box::new(r));
        }

        Ok(streams)
    }
}

/// FromShell parses cmdline as a shell command and auto-fills missing flags.
pub fn from_shell(cmdline: &str, track: Track) -> Result<Streams, RustTgCallsError> {
    let tokens = tokenize_shell(cmdline);
    if tokens.is_empty() {
        return Err(RustTgCallsError::FFmpegSpawn("empty command".into()));
    }
    validate_output_codec(&tokens[1..], track)?;
    let binary = tokens[0].clone();
    let args = ensure_ffmpeg_flags(&tokens[1..], track);
    let src = ShellSource {
        binary,
        args,
        track,
    };
    src.open()
}

/// FromShells builds a MultiShellSource from two separate ffmpeg commands.
pub fn from_shells(audio_cmd: &str, video_cmd: &str) -> Result<Streams, RustTgCallsError> {
    let mut audio_bin = String::new();
    let mut video_bin = String::new();
    let mut audio_args = Vec::new();
    let mut video_args = Vec::new();

    if !audio_cmd.is_empty() {
        let tokens = tokenize_shell(audio_cmd);
        if !tokens.is_empty() {
            validate_output_codec(&tokens[1..], TRACK_AUDIO)?;
            audio_bin = tokens[0].clone();
            audio_args = ensure_ffmpeg_flags(&tokens[1..], TRACK_AUDIO);
        }
    }

    if !video_cmd.is_empty() {
        let tokens = tokenize_shell(video_cmd);
        if !tokens.is_empty() {
            validate_output_codec(&tokens[1..], TRACK_VIDEO)?;
            video_bin = tokens[0].clone();
            video_args = ensure_ffmpeg_flags(&tokens[1..], TRACK_VIDEO);
        }
    }

    let src = MultiShellSource {
        audio_bin,
        video_bin,
        audio_args,
        video_args,
        parallel: false,
    };
    src.open()
}

/// ensure_ffmpeg_flags injects input-side fast-probe and output-side Opus/VP8 flags.
pub fn ensure_ffmpeg_flags(args: &[String], track: Track) -> Vec<String> {
    let mut has_analyzeduration = false;
    let mut has_probesize = false;
    let mut has_err_detect = false;
    let mut has_ca = false;
    let mut has_cv = false;
    let mut has_f = false;
    let mut has_pipe = false;

    let mut input_idx = None;

    for (i, a) in args.iter().enumerate() {
        if a == "-analyzeduration" {
            has_analyzeduration = true;
        }
        if a == "-probesize" {
            has_probesize = true;
        }
        if a == "-err_detect" {
            has_err_detect = true;
        }
        if a == "-c:a" || a == "-acodec" {
            has_ca = true;
        }
        if a == "-c:v" || a == "-vcodec" {
            has_cv = true;
        }
        if a == "-f" {
            has_f = true;
        }
        if a == "pipe:1" || a == "-" {
            has_pipe = true;
        }
        if a == "-i" && input_idx.is_none() {
            input_idx = Some(i);
        }
    }

    let mut input_inject = Vec::new();
    if input_idx.is_some() {
        if !has_analyzeduration {
            input_inject.extend(vec!["-analyzeduration".to_string(), "0".to_string()]);
        }
        if !has_probesize {
            input_inject.extend(vec!["-probesize".to_string(), "64k".to_string()]);
        }
        if !has_err_detect {
            input_inject.extend(vec!["-err_detect".to_string(), "ignore_err".to_string()]);
        }
    }

    let mut output_inject = Vec::new();
    if track.has(TRACK_AUDIO) {
        if !has_ca {
            output_inject.extend(vec!["-c:a".to_string(), "libopus".to_string()]);
        }
        output_inject.extend(vec![
            "-application".to_string(),
            "audio".to_string(),
            "-frame_duration".to_string(),
            "20".to_string(),
            "-page_duration".to_string(),
            "20000".to_string(),
            "-mapping_family".to_string(),
            "0".to_string(),
            "-ar".to_string(),
            "48000".to_string(),
            "-ac".to_string(),
            "2".to_string(),
        ]);
        if !has_f {
            output_inject.extend(vec!["-f".to_string(), "ogg".to_string()]);
        }
    } else if track.has(TRACK_VIDEO) {
        if !has_cv {
            output_inject.extend(vec![
                "-c:v".to_string(),
                "libvpx".to_string(),
                "-deadline".to_string(),
                "realtime".to_string(),
            ]);
        }
        if !has_f {
            output_inject.extend(vec!["-f".to_string(), "ivf".to_string()]);
        }
    }

    let mut out = Vec::new();
    if let Some(idx) = input_idx {
        out.extend(args[..idx].to_vec());
        out.extend(input_inject);
        out.extend(args[idx..].to_vec());
    } else {
        out.extend(args.to_vec());
    }

    if has_pipe {
        out.retain(|x| x != "pipe:1");
    }
    out.extend(output_inject);
    out.push("pipe:1".to_string());
    out
}

/// validate_output_codec checks for incompatible raw PCM or YUV formats.
pub fn validate_output_codec(args: &[String], track: Track) -> Result<(), RustTgCallsError> {
    let raw_pcm = [
        "s16le", "s16be", "s24le", "s32le", "f32le", "f64le", "u8", "alaw", "mulaw",
    ];
    for i in 0..args.len() {
        let next = args.get(i + 1).map(|s| s.as_str()).unwrap_or("");
        let a = &args[i];
        if (a == "-acodec" || a == "-c:a") && next.starts_with("pcm_") && track.has(TRACK_AUDIO) {
            return Err(RustTgCallsError::InvalidParams(format!(
                "raw PCM codec {} not supported; expected libopus in OGG",
                next
            )));
        }
        if a == "-f" && raw_pcm.contains(&next) && track.has(TRACK_AUDIO) {
            return Err(RustTgCallsError::InvalidParams(format!(
                "raw PCM container {} not supported; expected -f ogg",
                next
            )));
        }
    }
    Ok(())
}

/// inject_seek prepends or inserts `-ss <offset>` before `-i`.
pub fn inject_seek(args: &[String], offset: Duration) -> Vec<String> {
    let offset_str = format!("{:.3}", offset.as_secs_f64());
    let mut cleaned = Vec::new();
    let mut input_idx = None;
    let mut skip_next = false;

    for (i, a) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if a == "-ss" && input_idx.is_none() && i + 1 < args.len() {
            skip_next = true;
            continue;
        }
        if a == "-i" && input_idx.is_none() {
            input_idx = Some(cleaned.len());
        }
        cleaned.push(a.clone());
    }

    let mut out = Vec::new();
    if let Some(idx) = input_idx {
        out.extend(cleaned[..idx].to_vec());
        out.push("-ss".to_string());
        out.push(offset_str);
        out.extend(cleaned[idx..].to_vec());
    } else {
        out.push("-ss".to_string());
        out.push(offset_str);
        out.extend(cleaned);
    }
    out
}

/// tokenize_shell splits shell strings into argv tokens, supporting
/// double-quoted segments and escaped quote characters (\", \\).
pub fn tokenize_shell(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i] as char;

        if c == '\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1] as char;
            if next == '"' || next == '\\' {
                cur.push(next);
                i += 2;
                continue;
            }
        }

        match c {
            '"' => {
                in_quote = !in_quote;
            }
            ' ' | '\t' | '\n' if !in_quote => {
                if !cur.is_empty() {
                    out.push(cur.clone());
                    cur.clear();
                }
            }
            _ => cur.push(c),
        }
        i += 1;
    }

    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
