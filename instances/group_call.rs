//! GroupCall is the WebRTC call instance for one chat.

use crate::instances::call::Call;
use crate::media::frame_reader::{OpusFrameReader, VP8FrameReader};
use crate::media::source::Streams;
use crate::media::streamer::Streamer;
use crate::models::{
    ConnState, Device, MediaState, NetworkInfo, OPUS_PAYLOAD_TYPE, StreamType, VP8_PAYLOAD_TYPE,
    errors::RustTgCallsError,
};
use crate::utils::Dispatcher;
use crate::wrtc::native::stack::Stack;
use crate::wrtc::peer_factory::PeerFactory;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::timeout;

pub type OnStreamEndFn = Arc<dyn Fn(StreamType, Device, Option<RustTgCallsError>) + Send + Sync>;
pub type OnConnectionChangeFn = Arc<dyn Fn(NetworkInfo) + Send + Sync>;
pub type OnUpgradeFn = Arc<dyn Fn(MediaState) + Send + Sync>;

/// GroupCallEvents is the set of callbacks a GroupCall fires through the
/// shared dispatcher; the Client wires these to its public OnXxx callbacks.
#[derive(Clone, Default)]
pub struct GroupCallEvents {
    pub on_stream_end: Option<OnStreamEndFn>,
    pub on_connection_change: Option<OnConnectionChangeFn>,
    pub on_upgrade: Option<OnUpgradeFn>,
}

/// GroupCall is the WebRTC call instance for one chat.
pub struct GroupCall {
    chat_id: i64,
    stack: Arc<Stack>,
    events: GroupCallEvents,
    dispatcher: Option<Arc<Dispatcher>>,
    connect_timeout: Duration,

    audio_streamer: Mutex<Option<Arc<Streamer>>>,
    video_streamer: Mutex<Option<Arc<Streamer>>>,

    net_state: AtomicI32,
    closed: AtomicBool,
    switching: AtomicBool,
    connect_called: AtomicBool,
    paused: AtomicBool,
    muted: AtomicBool,
    ended_once: AtomicBool,
    had_audio: AtomicBool,
    had_video: AtomicBool,
    resume_ms: AtomicU64,

    connected_notify: Arc<Notify>,
}

impl GroupCall {
    /// NewGroupCall constructs a fresh call. Caller threads factory + dispatcher + events.
    pub async fn new(chat_id: i64, factory: &PeerFactory) -> Result<Arc<Self>, RustTgCallsError> {
        Self::new_with_events(
            chat_id,
            factory,
            None,
            Duration::from_secs(10),
            GroupCallEvents::default(),
        )
        .await
    }

    pub async fn new_with_events(
        chat_id: i64,
        factory: &PeerFactory,
        dispatcher: Option<Arc<Dispatcher>>,
        connect_timeout: Duration,
        events: GroupCallEvents,
    ) -> Result<Arc<Self>, RustTgCallsError> {
        let stack = Stack::new(factory).await?;
        let connected_notify = Arc::new(Notify::new());

        let connect_timeout = if connect_timeout.is_zero() {
            Duration::from_secs(10)
        } else {
            connect_timeout
        };

        let gc = Arc::new(Self {
            chat_id,
            stack,
            events,
            dispatcher,
            connect_timeout,
            audio_streamer: Mutex::new(None),
            video_streamer: Mutex::new(None),
            net_state: AtomicI32::new(ConnState::Connecting as i32),
            closed: AtomicBool::new(false),
            switching: AtomicBool::new(false),
            connect_called: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            ended_once: AtomicBool::new(false),
            had_audio: AtomicBool::new(false),
            had_video: AtomicBool::new(false),
            resume_ms: AtomicU64::new(0),
            connected_notify,
        });

        // Register stack state callback to keep GroupCall net_state 100% synchronized with Stack state
        let gc_clone = Arc::clone(&gc);
        gc.stack.set_on_state_change(Arc::new(move |s| {
            gc_clone.net_state.store(s as i32, Ordering::SeqCst);
            if s == ConnState::Connected {
                gc_clone.connected_notify.notify_waiters();
            }
            if let Some(ref cb) = gc_clone.events.on_connection_change {
                let cb_clone = cb.clone();
                let info = NetworkInfo { state: s };
                if let Some(ref disp) = gc_clone.dispatcher {
                    disp.submit(move || {
                        cb_clone(info);
                    });
                } else {
                    cb_clone(info);
                }
            }
        }));

        Ok(gc)
    }

    /// ChatId returns the chat id associated with this group call.
    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    /// AudioSSRC is exposed so callers can pass it as the Source param to
    /// phone.LeaveGroupCall.
    pub fn audio_ssrc(&self) -> u32 {
        self.stack.audio_ssrc()
    }

    /// currentStateLocked computes the MediaState a hypothetical OnUpgrade
    /// callback would report right now.
    pub fn current_state(&self) -> MediaState {
        let muted = self.muted.load(Ordering::SeqCst);
        let paused = self.paused.load(Ordering::SeqCst);
        let silent = muted || paused;
        let video_stopped = self.video_streamer.lock().is_none();
        MediaState {
            muted,
            paused: silent,
            video_stopped,
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

    /// handleStreamerEnd is the per-streamer onEnd callback.
    pub fn handle_streamer_end(
        &self,
        _stream_type: StreamType,
        _device: Device,
        err: Option<RustTgCallsError>,
    ) {
        if self.closed.load(Ordering::SeqCst) || self.switching.load(Ordering::SeqCst) {
            return;
        }

        if self.ended_once.swap(true, Ordering::SeqCst) {
            return;
        }

        let had_audio = self.had_audio.load(Ordering::SeqCst);
        let had_video = self.had_video.load(Ordering::SeqCst);

        let audio_lock = self.audio_streamer.lock();
        if let Some(ref a) = *audio_lock {
            a.stop();
        }
        let video_lock = self.video_streamer.lock();
        if let Some(ref v) = *video_lock {
            v.stop();
        }

        if let Some(ref cb) = self.events.on_stream_end {
            let cb_clone = cb.clone();
            let err_clone = err.clone();
            if let Some(ref disp) = self.dispatcher {
                disp.submit(move || {
                    if had_video {
                        cb_clone(StreamType::Video, Device::Camera, err_clone.clone());
                    }
                    if had_audio {
                        cb_clone(StreamType::Audio, Device::Microphone, err_clone);
                    }
                });
            } else {
                if had_video {
                    cb_clone(StreamType::Video, Device::Camera, err.clone());
                }
                if had_audio {
                    cb_clone(StreamType::Audio, Device::Microphone, err);
                }
            }
        }
    }
}

impl Call for GroupCall {
    /// Mode returns "webrtc" so Client can guard mode-specific operations.
    fn mode(&self) -> &'static str {
        "webrtc"
    }

    /// CreateLocalParams produces the local-side JSON.
    fn create_local_params(&self) -> Result<String, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        self.stack.get_local_params_json()
    }

    /// Connect feeds Telegram's response JSON.
    fn connect<'a>(
        &'a self,
        remote_json: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), RustTgCallsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(RustTgCallsError::Closed);
            }
            self.connect_called.store(true, Ordering::SeqCst);
            self.stack.connect(remote_json).await?;
            self.net_state
                .store(ConnState::Connected as i32, Ordering::SeqCst);
            self.connected_notify.notify_waiters();
            Ok(())
        })
    }

    /// SetSource installs the streaming source. Replaces atomically.
    fn set_source<'a>(
        &'a self,
        streams: Streams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), RustTgCallsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(RustTgCallsError::Closed);
            }

            // Gate: only wait if WebRTC is not yet connected
            if self.stack.state() != ConnState::Connected && self.net_state.load(Ordering::SeqCst) != (ConnState::Connected as i32) {
                let notify = self.connected_notify.clone();
                let conn_timeout = self.connect_timeout;
                if self.connect_called.load(Ordering::SeqCst) {
                    let _ = timeout(conn_timeout, notify.notified()).await;
                }
                if self.stack.state() != ConnState::Connected {
                    return Err(RustTgCallsError::Internal(
                        "WebRTC connection timed out before set_source".into(),
                    ));
                }
            }

            self.switching.store(true, Ordering::SeqCst);
            let prev = self.current_state();

            let mut audio_lock = self.audio_streamer.lock();
            let mut video_lock = self.video_streamer.lock();

            if let Some(old_audio) = audio_lock.take() {
                old_audio.stop();
            }
            if let Some(old_video) = video_lock.take() {
                old_video.stop();
            }
            self.stack.reset_media_track_state();

            self.had_audio
                .store(streams.audio.is_some(), Ordering::SeqCst);
            self.had_video
                .store(streams.video.is_some(), Ordering::SeqCst);

            let on_end_cb = self.events.on_stream_end.clone();
            let dispatcher_clone = self.dispatcher.clone();
            let stack_clone = Arc::clone(&self.stack);
            self.ended_once.store(false, Ordering::SeqCst);

            let has_audio = streams.audio.is_some();
            let has_video = streams.video.is_some();

            let sync_barrier = if has_audio && has_video {
                Some(Arc::new(tokio::sync::Barrier::new(2)))
            } else {
                None
            };

            if let Some(audio_src) = streams.audio {
                let reader = OpusFrameReader::new(audio_src);
                let streamer = Streamer::new_with_barrier(
                    crate::media::streamer::ReaderKind::Opus(reader),
                    stack_clone.clone(),
                    OPUS_PAYLOAD_TYPE,
                    sync_barrier.clone(),
                );
                if self.muted.load(Ordering::SeqCst) || self.paused.load(Ordering::SeqCst) {
                    streamer.pause();
                }
                let cb = on_end_cb.clone();
                let disp = dispatcher_clone.clone();
                streamer.on_end(move |err| {
                    if let Some(ref f) = cb {
                        let f_clone = Arc::clone(f);
                        if let Some(ref d) = disp {
                            d.submit(move || {
                                f_clone(StreamType::Audio, Device::Microphone, err);
                            });
                        } else {
                            f_clone(StreamType::Audio, Device::Microphone, err);
                        }
                    }
                });
                *audio_lock = Some(streamer);
            }

            if let Some(video_src) = streams.video {
                let reader = VP8FrameReader::new(video_src);
                let streamer = Streamer::new_with_barrier(
                    crate::media::streamer::ReaderKind::VP8(reader),
                    stack_clone,
                    VP8_PAYLOAD_TYPE,
                    sync_barrier,
                );
                if self.paused.load(Ordering::SeqCst) {
                    streamer.pause();
                }
                if !has_audio {
                    let cb = on_end_cb;
                    let disp = dispatcher_clone;
                    streamer.on_end(move |err| {
                        if let Some(ref f) = cb {
                            let f_clone = Arc::clone(f);
                            if let Some(ref d) = disp {
                                d.submit(move || {
                                    f_clone(StreamType::Video, Device::Camera, err);
                                });
                            } else {
                                f_clone(StreamType::Video, Device::Camera, err);
                            }
                        }
                    });
                }
                *video_lock = Some(streamer);
            }

            drop(audio_lock);
            drop(video_lock);

            self.switching.store(false, Ordering::SeqCst);
            self.fire_upgrade_if_changed(prev);
            Ok(())
        })
    }

    /// Pause pauses outgoing streams.
    fn pause(&self) -> Result<bool, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        if self.paused.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        let prev = self.current_state();
        if let Some(ref a) = *self.audio_streamer.lock() {
            a.pause();
        }
        if let Some(ref v) = *self.video_streamer.lock() {
            v.pause();
        }
        self.fire_upgrade_if_changed(prev);
        Ok(true)
    }

    /// Resume resumes outgoing streams.
    fn resume(&self) -> Result<bool, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        if !self.paused.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        let prev = self.current_state();
        if let Some(ref a) = *self.audio_streamer.lock() {
            a.resume();
        }
        if let Some(ref v) = *self.video_streamer.lock() {
            v.resume();
        }
        self.fire_upgrade_if_changed(prev);
        Ok(true)
    }

    /// Mute mutes audio.
    fn mute(&self) -> Result<bool, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        if self.muted.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        let prev = self.current_state();
        if let Some(ref a) = *self.audio_streamer.lock() {
            a.pause();
        }
        self.fire_upgrade_if_changed(prev);
        Ok(true)
    }

    /// Unmute unmutes audio.
    fn unmute(&self) -> Result<bool, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }
        if !self.muted.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        let prev = self.current_state();
        if let Some(ref a) = *self.audio_streamer.lock() {
            a.resume();
        }
        self.fire_upgrade_if_changed(prev);
        Ok(true)
    }

    /// Stop tears down call streamers and peer connection.
    fn stop(&self) -> Result<(), RustTgCallsError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(old_audio) = self.audio_streamer.lock().take() {
            old_audio.stop();
        }
        if let Some(old_video) = self.video_streamer.lock().take() {
            old_video.stop();
        }
        self.resume_ms.store(0, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
        self.muted.store(false, Ordering::SeqCst);

        // Explicitly close underlying WebRTC stack and transport resources
        let stack = Arc::clone(&self.stack);
        tokio::spawn(async move {
            let _ = stack.close().await;
        });

        Ok(())
    }

    fn seek_by(&self, _delta_ms: i64) -> Result<(), RustTgCallsError> {
        Err(RustTgCallsError::SeekUnsupported)
    }

    fn elapsed_ms(&self) -> u64 {
        self.resume_ms.load(Ordering::SeqCst)
    }

    fn state(&self) -> MediaState {
        self.current_state()
    }

    fn net_state(&self) -> ConnState {
        match self.net_state.load(Ordering::SeqCst) {
            0 => ConnState::Connecting,
            1 => ConnState::Connected,
            2 => ConnState::Disconnected,
            3 => ConnState::Failed,
            4 => ConnState::Closed,
            _ => ConnState::Disconnected,
        }
    }

    fn audio_ssrc(&self) -> Result<u32, RustTgCallsError> {
        Ok(self.audio_ssrc())
    }
}
