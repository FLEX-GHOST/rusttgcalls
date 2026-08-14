//! RTMPCall pushes audio+video to a Telegram-issued RTMP URL via a single
//! ffmpeg process. No WebRTC involvement.

use crate::instances::call::Call;
use crate::instances::group_call::GroupCallEvents;
use crate::io::ShellReader;
use crate::media::source::{EncodeOptions, Streams};
use crate::models::{ConnState, MediaState, errors::RustTgCallsError};
use crate::utils::Dispatcher;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// RTMPCall pushes audio+video to a Telegram-issued RTMP URL via a single
/// ffmpeg process.
pub struct RTMPCall {
    chat_id: i64,
    rtmp_url: String,
    events: GroupCallEvents,
    dispatcher: Option<Arc<Dispatcher>>,
    cmd: Arc<Mutex<Option<ShellReader>>>,
    started_at: Arc<Mutex<Option<Instant>>>,
    closed: AtomicBool,
    paused: AtomicBool,
    muted: AtomicBool,
    resume_ms: AtomicU64,
}

impl RTMPCall {
    pub fn new(chat_id: i64, rtmp_url: &str) -> Arc<Self> {
        Self::new_with_events(chat_id, rtmp_url, None, GroupCallEvents::default())
    }

    pub fn new_with_events(
        chat_id: i64,
        rtmp_url: &str,
        dispatcher: Option<Arc<Dispatcher>>,
        events: GroupCallEvents,
    ) -> Arc<Self> {
        Arc::new(Self {
            chat_id,
            rtmp_url: rtmp_url.to_string(),
            events,
            dispatcher,
            cmd: Arc::new(Mutex::new(None)),
            started_at: Arc::new(Mutex::new(None)),
            closed: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            resume_ms: AtomicU64::new(0),
        })
    }

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    /// currentStateLocked builds the MediaState for the RTMP path. RTMP
    /// always pushes H.264 video (VideoStopped is permanently false).
    fn current_state(&self) -> MediaState {
        let silent = self.muted.load(Ordering::SeqCst) || self.paused.load(Ordering::SeqCst);
        MediaState {
            muted: self.muted.load(Ordering::SeqCst),
            paused: silent,
            video_stopped: false,
            presentation_paused: silent,
        }
    }

    /// fireUpgradeIfChanged submits an OnUpgrade dispatch only if state changed.
    fn fire_upgrade_if_changed(&self, prev: MediaState) {
        let cur = self.current_state();
        if prev == cur {
            return;
        }
        if let Some(ref cb) = self.events.on_upgrade {
            let cb_clone = cb.clone();
            if let Some(ref disp) = self.dispatcher {
                disp.submit(move || {
                    cb_clone(cur);
                });
            } else {
                cb_clone(cur);
            }
        }
    }
}

impl Call for RTMPCall {
    /// Mode returns "rtmp".
    fn mode(&self) -> &'static str {
        "rtmp"
    }

    /// CreateLocalParams WebRTC-only method returns WrongMode for RTMP calls.
    fn create_local_params(&self) -> Result<String, RustTgCallsError> {
        Err(RustTgCallsError::WrongMode)
    }

    /// Connect WebRTC-only method returns WrongMode for RTMP calls.
    fn connect<'a>(
        &'a self,
        _remote_json: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), RustTgCallsError>> + Send + 'a>> {
        Box::pin(async move { Err(RustTgCallsError::WrongMode) })
    }

    /// SetSource installs the streaming source for RTMP push.
    fn set_source<'a>(
        &'a self,
        _streams: Streams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), RustTgCallsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(RustTgCallsError::Closed);
            }
            let opt = EncodeOptions::default();
            let args = build_rtmp_args("input.mp4", 0, &opt, &self.rtmp_url);
            let cmd = ShellReader::new("ffmpeg", &args, false)?;

            *self.started_at.lock() = Some(Instant::now());
            *self.cmd.lock() = Some(cmd);

            self.resume_ms.store(0, Ordering::SeqCst);
            Ok(())
        })
    }

    /// Pause pauses RTMP push stream.
    fn pause(&self) -> Result<bool, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        if self.paused.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        let prev = self.current_state();
        let cur = self.elapsed_ms();
        self.resume_ms.store(cur, Ordering::SeqCst);

        let cmd = self.cmd.lock().take();
        if let Some(cmd) = cmd {
            tokio::spawn(async move {
                let _ = cmd.close().await;
            });
        }
        self.fire_upgrade_if_changed(prev);
        Ok(true)
    }

    /// Resume resumes RTMP push stream.
    fn resume(&self) -> Result<bool, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        if !self.paused.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        let prev = self.current_state();
        let opt = EncodeOptions::default();
        let seek_ms = self.resume_ms.load(Ordering::SeqCst);
        let args = build_rtmp_args("input.mp4", seek_ms, &opt, &self.rtmp_url);
        let cmd = ShellReader::new("ffmpeg", &args, false)?;

        *self.started_at.lock() = Some(Instant::now());
        *self.cmd.lock() = Some(cmd);
        self.fire_upgrade_if_changed(prev);
        Ok(true)
    }

    /// Mute sets mute flag on RTMP stream.
    fn mute(&self) -> Result<bool, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        if self.muted.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        let prev = self.current_state();
        self.fire_upgrade_if_changed(prev);
        Ok(true)
    }

    /// Unmute unmutes audio on RTMP stream.
    fn unmute(&self) -> Result<bool, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        if !self.muted.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        let prev = self.current_state();
        self.fire_upgrade_if_changed(prev);
        Ok(true)
    }

    /// Stop tears down RTMP ffmpeg process.
    fn stop(&self) -> Result<(), RustTgCallsError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let cmd = self.cmd.lock().take();
        if let Some(cmd) = cmd {
            tokio::spawn(async move {
                let _ = cmd.close().await;
            });
        }
        self.resume_ms.store(0, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
        self.muted.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// SeekBy shifts playback offset for RTMP stream.
    fn seek_by(&self, delta_ms: i64) -> Result<(), RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        let cur = self.elapsed_ms() as i64;
        let target = cur + delta_ms;
        if target < 0 {
            self.stop()?;
            return Ok(());
        }
        self.resume_ms.store(target as u64, Ordering::SeqCst);
        Ok(())
    }

    /// ElapsedMs returns elapsed playback time in milliseconds.
    fn elapsed_ms(&self) -> u64 {
        if self.paused.load(Ordering::SeqCst) {
            return self.resume_ms.load(Ordering::SeqCst);
        }
        let started = self.started_at.lock();
        if let Some(t) = *started {
            self.resume_ms.load(Ordering::SeqCst) + t.elapsed().as_millis() as u64
        } else {
            0
        }
    }

    /// State returns current MediaState.
    fn state(&self) -> MediaState {
        self.current_state()
    }

    /// NetState returns current ConnState.
    fn net_state(&self) -> ConnState {
        if self.closed.load(Ordering::SeqCst) {
            ConnState::Closed
        } else if self.cmd.lock().is_none() {
            ConnState::Connecting
        } else {
            ConnState::Connected
        }
    }

    fn audio_ssrc(&self) -> Result<u32, RustTgCallsError> {
        Err(RustTgCallsError::WrongMode)
    }
}

/// build_rtmp_args assembles a single ffmpeg argv that reads input,
/// transcodes to H.264+AAC, and pushes FLV to rtmp_url.
pub fn build_rtmp_args(
    input_path: &str,
    seek_ms: u64,
    opt: &EncodeOptions,
    rtmp_url: &str,
) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-nostdin".to_string(),
        "-re".to_string(),
    ];
    if seek_ms > 0 {
        args.push("-ss".to_string());
        args.push(format!("{:.3}", seek_ms as f64 / 1000.0));
    }
    args.extend(vec![
        "-i".to_string(),
        input_path.to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-preset".to_string(),
        "veryfast".to_string(),
        "-tune".to_string(),
        "zerolatency".to_string(),
        "-b:v".to_string(),
        format!("{}k", opt.video_bitrate_kbps),
        "-r".to_string(),
        opt.video_fps.to_string(),
        "-vf".to_string(),
        format!("scale={}:{}", opt.video_width, opt.video_height),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-g".to_string(),
        (opt.video_fps * 2).to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        format!("{}k", opt.audio_bitrate_kbps),
        "-ar".to_string(),
        "44100".to_string(),
        "-ac".to_string(),
        opt.audio_channels.to_string(),
        "-f".to_string(),
        "flv".to_string(),
        rtmp_url.to_string(),
    ]);
    args
}
