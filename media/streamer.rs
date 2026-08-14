//! Streamer pulls Samples from a FrameReader at the sample's natural cadence
//! and pushes them to WebRTC RTP tracks. Mute (audio) skips send but keeps
//! the clock advancing. Pause blocks the pull loop on a channel without
//! tearing down the underlying ffmpeg process.

use crate::media::frame_reader::{OpusFrameReader, VP8FrameReader};
use crate::wrtc::native::stack::Stack;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncRead;
use tokio::sync::Notify;

/// stallTimeout bounds how long a single src.next() read can block before
/// the streamer force-closes the source. Catches ffmpeg processes stuck on
/// network I/O (e.g., HTTP source that dropped mid-stream) that would
/// otherwise hang the streamer indefinitely — OnStreamEnd never fires, the
/// bot never advances to the next song.
const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

pub enum ReaderKind<R: AsyncRead + Unpin + Send + 'static> {
    Opus(OpusFrameReader<R>),
    VP8(VP8FrameReader<R>),
}

/// Streamer pulls Samples from a FrameReader at the sample's natural cadence
/// and pushes them to a Writer. Mute (audio) skips WriteSample but keeps
/// the clock advancing. Pause (set via set_paused) blocks the pull loop on a
/// notify channel without tearing down the underlying ffmpeg process — the OS pipe
/// buffer absorbs ~1s of OGG bytes while paused; on resume the loop wakes
/// and drains them at real-time pace.
type StreamerCompletionCallback =
    Box<dyn Fn(Option<crate::models::errors::RustTgCallsError>) + Send + Sync>;

pub struct Streamer {
    muted: AtomicBool,
    paused: AtomicBool,
    running: AtomicBool,
    done_flag: AtomicBool,
    ns_sent: AtomicU64,
    pause_notify: Arc<Notify>,
    done_notify: Arc<Notify>,
    on_end: Mutex<Option<StreamerCompletionCallback>>,
}

impl Streamer {
    /// NewStreamer creates and immediately spawns the pacing task.
    pub fn new<R: AsyncRead + Unpin + Send + 'static>(
        reader: ReaderKind<R>,
        stack: Arc<Stack>,
        payload_type: u8,
    ) -> Arc<Self> {
        Self::new_with_barrier(reader, stack, payload_type, None)
    }

    /// new_with_barrier synchronizes multiple streamers (e.g. Audio + Video) to start together at T=0.
    pub fn new_with_barrier<R: AsyncRead + Unpin + Send + 'static>(
        mut reader: ReaderKind<R>,
        stack: Arc<Stack>,
        payload_type: u8,
        sync_barrier: Option<Arc<tokio::sync::Barrier>>,
    ) -> Arc<Self> {
        let streamer = Arc::new(Self {
            muted: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            running: AtomicBool::new(true),
            done_flag: AtomicBool::new(false),
            ns_sent: AtomicU64::new(0),
            pause_notify: Arc::new(Notify::new()),
            done_notify: Arc::new(Notify::new()),
            on_end: Mutex::new(None),
        });

        let s = streamer.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<crate::media::frame_reader::Sample, crate::models::errors::RustTgCallsError>>(256);
        let r_streamer = streamer.clone();

        // Background reader task: continuously prefetches frames into bounded buffer
        tokio::spawn(async move {
            while r_streamer.running.load(Ordering::Acquire) {
                while r_streamer.paused.load(Ordering::Relaxed) && r_streamer.running.load(Ordering::Acquire) {
                    tokio::select! {
                        _ = r_streamer.pause_notify.notified() => {}
                        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                    }
                }
                if !r_streamer.running.load(Ordering::Acquire) {
                    break;
                }

                let res = tokio::select! {
                    r = async {
                        match &mut reader {
                            ReaderKind::Opus(r) => r.next_sample().await,
                            ReaderKind::VP8(r) => r.next_sample().await,
                        }
                    } => r,
                    _ = tokio::time::sleep(STALL_TIMEOUT) => {
                        tracing::warn!("[Streamer] Read timed out after 30s");
                        Err(crate::models::errors::RustTgCallsError::Internal("streamer read timed out after 30s".into()))
                    }
                };
                let is_err = res.is_err();
                if tx.send(res).await.is_err() || is_err {
                    break;
                }
            }
        });

        // Dedicated pacing thread: consumes from buffer with drift-free monotonic deadline pacing
        tokio::spawn(async move {
            let mut first_sample = true;
            let mut end_error = None;
            let mut next_deadline = Instant::now();
            let mut next_sample_res = rx.recv().await;

            while s.running.load(Ordering::Acquire) {
                while s.paused.load(Ordering::Relaxed) && s.running.load(Ordering::Acquire) {
                    tokio::select! {
                        _ = s.pause_notify.notified() => {}
                        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                    }
                    first_sample = true;
                }
                if !s.running.load(Ordering::Acquire) {
                    break;
                }

                let sample_res = match next_sample_res.take() {
                    Some(res) => res,
                    None => break,
                };

                let sample = match sample_res {
                    Ok(sample) => sample,
                    Err(e) => {
                        end_error = Some(e);
                        break;
                    }
                };

                let dur = sample.duration;
                let data = sample.data;

                if first_sample {
                    if let Some(ref barrier) = sync_barrier {
                        barrier.wait().await;
                    }
                    next_deadline = Instant::now();
                    first_sample = false;
                }

                // Instantaneous transmission at T=0 of the frame's time-slot
                if !s.muted.load(Ordering::Relaxed) {
                    if let Err(e) = stack.send_rtp_bytes(payload_type, data, dur).await {
                        end_error = Some(e);
                        break;
                    }
                }

                s.ns_sent.fetch_add(dur.as_nanos() as u64, Ordering::Relaxed);
                next_deadline += dur;

                // Concurrently wait for next monotonic deadline AND pre-fetch the next sample from channel
                let now = Instant::now();
                let sleep_fut = async {
                    if next_deadline > now {
                        tokio::time::sleep(next_deadline - now).await;
                    } else if now.duration_since(next_deadline) > Duration::from_millis(100) {
                        next_deadline = now;
                    }
                };

                let (_, prefetched) = tokio::join!(sleep_fut, rx.recv());
                next_sample_res = prefetched;
            }

            s.done_flag.store(true, Ordering::Release);
            s.done_notify.notify_waiters();
            let cb = s.on_end.lock().unwrap().take();
            if let Some(f) = cb {
                f(end_error);
            }
        });

        streamer
    }

    /// Start starts the pacing loop.
    pub fn start(&self) {
        self.running.store(true, Ordering::Release);
    }

    /// SetPaused toggles pause state.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.pause_notify.notify_waiters();
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Release);
    }

    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        *self.on_end.lock().unwrap() = None;
        self.running.store(false, Ordering::Release);
        self.pause_notify.notify_waiters();
    }

    pub fn on_end<F>(&self, f: F)
    where
        F: Fn(Option<crate::models::errors::RustTgCallsError>) + Send + Sync + 'static,
    {
        *self.on_end.lock().unwrap() = Some(Box::new(f));
    }

    pub async fn done(&self) {
        if self.done_flag.load(Ordering::Acquire) {
            return;
        }
        self.done_notify.notified().await;
    }
}
